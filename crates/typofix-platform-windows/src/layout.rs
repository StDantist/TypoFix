//! Запит розкладки з ОС через `ToUnicodeEx` (залізне правило №5: істина про
//! символи — з ОС, а не з TOML) + мапінг між нашим [`LayoutId`] і системним
//! `HKL`.
//!
//! ## Готча dead-key стану (критично)
//! `ToUnicodeEx` **мутує внутрішній per-thread стан dead-key** драйвера
//! розкладки. Якщо клавіша — мертва (діакритика, напр. `^`), виклик лишає її
//! «висіти», і наступний реальний символ буде зіпсований. Тому ми:
//! - передаємо **власний** очищений `key state` (ніколи не читаємо/не пишемо
//!   глобальний `GetKeyboardState`);
//! - **зливаємо** можливий мертвий стан пробілом до і після запиту
//!   ([`flush_dead_key`]). Так запит лишається без побічних ефектів.

use typofix_platform::{LayoutId, Modifiers};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardLayoutList, MapVirtualKeyExW, ToUnicodeEx, HKL, MAPVK_VK_TO_VSC,
    MAPVK_VSC_TO_VK_EX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::keystate::fill_key_state;

const VK_SPACE: u32 = 0x20;

/// `PRIMARYLANGID` із langid (молодші 10 біт). Збігається для БУДЬ-ЯКОГО варіанта
/// мови (напр. усі укр. розкладки `0x0422`/`0x0822`/… мають primary `0x22`).
fn primary_langid(langid: u16) -> u16 {
    langid & 0x03FF
}

/// `PRIMARYLANGID` розкладки `hkl` (молодше слово HKL → primary).
fn primary_langid_of_hkl(hkl: HKL) -> u16 {
    primary_langid((hkl as usize & 0xFFFF) as u16)
}

/// Наш [`LayoutId`] → цільовий `PRIMARYLANGID`, або `None` для невідомої мови.
fn primary_langid_for_id(id: &LayoutId) -> Option<u16> {
    match id.as_str() {
        "en" => Some(0x09), // LANG_ENGLISH
        "uk" => Some(0x22), // LANG_UKRAINIAN
        _ => None,
    }
}

/// `HKL`, активний у потоці вікна на передньому плані (а не нашого потоку).
pub fn current_hkl() -> HKL {
    unsafe {
        let hwnd = GetForegroundWindow();
        let tid = if hwnd.is_null() {
            0
        } else {
            GetWindowThreadProcessId(hwnd, std::ptr::null_mut())
        };
        GetKeyboardLayout(tid)
    }
}

/// `langid` (молодше слово HKL) → наш [`LayoutId`], матч за `PRIMARYLANGID`.
/// Невідомі — як hex-рядок повного langid.
pub fn layout_id_for_hkl(hkl: HKL) -> LayoutId {
    let langid = (hkl as usize & 0xFFFF) as u16;
    match primary_langid(langid) {
        0x09 => LayoutId::new("en"),
        0x22 => LayoutId::new("uk"),
        _ => LayoutId::new(format!("0x{langid:04x}")),
    }
}

/// Поточна активна розкладка ОС як наш [`LayoutId`].
pub fn current_layout_id() -> LayoutId {
    layout_id_for_hkl(current_hkl())
}

/// Список `HKL` усіх **уже встановлених** розкладок (через `GetKeyboardLayoutList`).
/// Нічого не інсталює.
fn installed_hkls() -> Vec<HKL> {
    unsafe {
        let count = GetKeyboardLayoutList(0, std::ptr::null_mut());
        if count <= 0 {
            return Vec::new();
        }
        let mut list: Vec<HKL> = vec![std::ptr::null_mut(); count as usize];
        let got = GetKeyboardLayoutList(count, list.as_mut_ptr());
        list.truncate(got.max(0) as usize);
        list
    }
}

/// Наш [`LayoutId`] → `HKL` серед **уже встановлених** розкладок (матч за
/// `PRIMARYLANGID`).
///
/// **НІКОЛИ не встановлює розкладку.** Якщо потрібної мови в системі немає або
/// мова невідома — `None` (краще не діяти, ніж засмічувати систему). Це залізна
/// готча: жодного `LoadKeyboardLayoutW`.
pub fn installed_hkl_for_layout_id(id: &LayoutId) -> Option<HKL> {
    let target = primary_langid_for_id(id)?;
    installed_hkls()
        .into_iter()
        .find(|&hkl| primary_langid_of_hkl(hkl) == target)
}

/// Злити можливий мертвий (dead-key) стан розкладки, «натиснувши» пробіл доти,
/// доки `ToUnicodeEx` не поверне невідʼємне (не -1 = не мертва клавіша).
fn flush_dead_key(hkl: HKL) {
    let state = [0u8; 256];
    let mut buf = [0u16; 8];
    let sc = unsafe { MapVirtualKeyExW(VK_SPACE, MAPVK_VK_TO_VSC, hkl) };
    for _ in 0..4 {
        let rc = unsafe {
            ToUnicodeEx(
                VK_SPACE,
                sc,
                state.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as i32,
                0,
                hkl,
            )
        };
        if rc >= 0 {
            break;
        }
    }
}

/// Який символ дасть фізичний `scancode` (set 1) з даними `modifiers` у розкладці
/// `hkl`. `None`, якщо клавіша не дає друкованого символу.
///
/// Це і є рантайм-джерело істини про розкладку (підміняє/доповнює TOML).
/// Обробляє dead-keys: для мертвої клавіші повертає **саму діакритику**
/// (`buf[0]`), попередньо зливши стан, щоб не зіпсувати наступний ввід.
pub(crate) fn char_for(scancode: u32, modifiers: Modifiers, hkl: HKL) -> Option<char> {
    let vk = unsafe { MapVirtualKeyExW(scancode, MAPVK_VSC_TO_VK_EX, hkl) };
    if vk == 0 {
        return None;
    }

    let mut state = [0u8; 256];
    fill_key_state(&mut state, modifiers);

    // Чистимо стан до запиту (раптом висить чужа мертва клавіша).
    flush_dead_key(hkl);

    let mut buf = [0u16; 8];
    let rc = unsafe {
        ToUnicodeEx(
            vk,
            scancode,
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
            hkl,
        )
    };

    let unit = match rc {
        // -1 — мертва клавіша: символ діакритики в buf[0], але стан треба злити.
        -1 => {
            flush_dead_key(hkl);
            buf[0]
        }
        // 0 — клавіша не дає символу (модифікатор, функціональна тощо).
        0 => return None,
        // >=1 — звичайний символ; беремо перший UTF-16 code unit (для наших
        // розкладок ключі однокодові; лігатури не підтримуємо).
        _ => buf[0],
    };

    char::from_u32(unit as u32)
}

/// Високорівневий запит без `HKL`: символ для `scancode`+`modifiers` у вказаній
/// **нашій** розкладці — лише якщо вона **вже встановлена** в системі. `None`,
/// якщо розкладки немає (нічого не інсталюємо) або клавіша «німа».
pub fn char_for_layout(scancode: u32, modifiers: Modifiers, id: &LayoutId) -> Option<char> {
    let hkl = installed_hkl_for_layout_id(id)?;
    char_for(scancode, modifiers, hkl)
}

/// Символ для `scancode`+`modifiers` у **активній** розкладці ОС.
pub fn char_for_active_layout(scancode: u32, modifiers: Modifiers) -> Option<char> {
    char_for(scancode, modifiers, current_hkl())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Якщо US-розкладка **вже встановлена**, `ToUnicodeEx` дає стабільні
    /// ASCII-символи. Запит без ін'єкції й **без встановлення** розкладки; якщо
    /// її немає — тест нічого не перевіряє (і нічого не інсталює).
    #[test]
    fn us_layout_ascii_letters_and_digits() {
        let Some(hkl) = installed_hkl_for_layout_id(&LayoutId::new("en")) else {
            // US-розкладка не встановлена — нема що перевіряти (НЕ інсталюємо).
            return;
        };
        // A (0x1E): 'a' / Shift → 'A'.
        assert_eq!(char_for(0x1E, Modifiers::empty(), hkl), Some('a'));
        assert_eq!(char_for(0x1E, Modifiers::SHIFT, hkl), Some('A'));
        // Цифровий ряд '1' (0x02): '1' / Shift → '!'.
        assert_eq!(char_for(0x02, Modifiers::empty(), hkl), Some('1'));
        assert_eq!(char_for(0x02, Modifiers::SHIFT, hkl), Some('!'));
        // Пробіл (0x39).
        assert_eq!(char_for(0x39, Modifiers::empty(), hkl), Some(' '));
        // Caps Lock піднімає регістр літери так само, як Shift.
        assert_eq!(char_for(0x1E, Modifiers::CAPS, hkl), Some('A'));
    }

    #[test]
    fn installed_hkl_roundtrips_to_layout_id() {
        // Для будь-якої встановленої розкладки матч за PRIMARYLANGID симетричний.
        if let Some(hkl) = installed_hkl_for_layout_id(&LayoutId::new("en")) {
            assert_eq!(layout_id_for_hkl(hkl), LayoutId::new("en"));
            assert_eq!(primary_langid_of_hkl(hkl), 0x09);
        }
    }

    #[test]
    fn unknown_layout_id_has_no_hkl() {
        // Невідома мова → None, і жодного встановлення.
        assert!(installed_hkl_for_layout_id(&LayoutId::new("zz")).is_none());
    }

    #[test]
    fn primary_langid_matches_any_variant() {
        // Усі англ. варіанти (US 0x0409, UK 0x0809…) → primary 0x09.
        assert_eq!(primary_langid(0x0409), 0x09);
        assert_eq!(primary_langid(0x0809), 0x09);
        // Усі укр. варіанти → primary 0x22.
        assert_eq!(primary_langid(0x0422), 0x22);
    }

    #[test]
    fn current_layout_id_is_queryable() {
        // Не знаємо, яка саме активна, але виклик має не панікувати й дати щось.
        let id = current_layout_id();
        assert!(!id.as_str().is_empty());
    }
}
