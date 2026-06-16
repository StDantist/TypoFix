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
    GetKeyboardLayout, LoadKeyboardLayoutW, MapVirtualKeyExW, ToUnicodeEx, HKL, MAPVK_VK_TO_VSC,
    MAPVK_VSC_TO_VK_EX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use crate::keystate::fill_key_state;

const VK_SPACE: u32 = 0x20;
/// Не показувати сповіщення оболонці при завантаженні розкладки.
const KLF_NOTELLSHELL: u32 = 0x0000_0080;

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

/// `langid` (молодше слово HKL) → наш [`LayoutId`]. Невідомі — як hex-рядок.
pub fn layout_id_for_hkl(hkl: HKL) -> LayoutId {
    let langid = (hkl as usize & 0xFFFF) as u16;
    match langid {
        0x0409 => LayoutId::new("en"),
        0x0422 => LayoutId::new("uk"),
        other => LayoutId::new(format!("0x{other:04x}")),
    }
}

/// Поточна активна розкладка ОС як наш [`LayoutId`].
pub fn current_layout_id() -> LayoutId {
    layout_id_for_hkl(current_hkl())
}

/// Наш [`LayoutId`] → системний `HKL` (через `LoadKeyboardLayoutW`).
///
/// Повертає `None`, якщо мову не знаємо або ОС не має такої розкладки.
/// `LoadKeyboardLayoutW` лише **завантажує** її у процес (не активує — для цього
/// є окремий `WM_INPUTLANGCHANGEREQUEST` в `inject`).
pub fn hkl_for_layout_id(id: &LayoutId) -> Option<HKL> {
    let klid: &str = match id.as_str() {
        "en" => "00000409",
        "uk" => "00000422",
        _ => return None,
    };
    let wide: Vec<u16> = klid.encode_utf16().chain(std::iter::once(0)).collect();
    let hkl = unsafe { LoadKeyboardLayoutW(wide.as_ptr(), KLF_NOTELLSHELL) };
    if hkl.is_null() {
        None
    } else {
        Some(hkl)
    }
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
/// **нашій** розкладці. `None`, якщо розкладки немає в ОС або клавіша «німа».
pub fn char_for_layout(scancode: u32, modifiers: Modifiers, id: &LayoutId) -> Option<char> {
    let hkl = hkl_for_layout_id(id)?;
    char_for(scancode, modifiers, hkl)
}

/// Символ для `scancode`+`modifiers` у **активній** розкладці ОС.
pub fn char_for_active_layout(scancode: u32, modifiers: Modifiers) -> Option<char> {
    char_for(scancode, modifiers, current_hkl())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `LoadKeyboardLayoutW("00000409")` має дати ненульовий HKL, а через нього
    /// `ToUnicodeEx` — стабільні ASCII-символи US-розкладки. Це детермінований,
    /// безпечний запит (без ін'єкції вводу й без зміни активної розкладки).
    #[test]
    fn us_layout_ascii_letters_and_digits() {
        let Some(hkl) = hkl_for_layout_id(&LayoutId::new("en")) else {
            // US-розкладка не встановлена в системі — нема що перевіряти.
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
    fn hkl_langid_roundtrips_to_layout_id() {
        if let Some(hkl) = hkl_for_layout_id(&LayoutId::new("en")) {
            assert_eq!(layout_id_for_hkl(hkl), LayoutId::new("en"));
        }
    }

    #[test]
    fn unknown_layout_id_has_no_hkl() {
        assert!(hkl_for_layout_id(&LayoutId::new("zz")).is_none());
    }

    #[test]
    fn current_layout_id_is_queryable() {
        // Не знаємо, яка саме активна, але виклик має не панікувати й дати щось.
        let id = current_layout_id();
        assert!(!id.as_str().is_empty());
    }
}
