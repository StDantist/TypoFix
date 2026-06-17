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
use windows_sys::Win32::Globalization::{
    GetLocaleInfoEx, LCIDToLocaleName, LOCALE_SLOCALIZEDLANGUAGENAME,
};
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardLayoutList, MapVirtualKeyExW, ToUnicodeEx, HKL, MAPVK_VK_TO_VSC,
    MAPVK_VSC_TO_VK_EX,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
};

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

/// RAII-guard: тримає `AttachThreadInput(me, target, TRUE)` і ГАРАНТОВАНО
/// відчіпляє у `Drop` (навіть на ранньому виході/панці). Симетричність —
/// критична: лишити потік приєднаним = зламати ввід обом.
struct InputAttach {
    me: u32,
    target: u32,
    attached: bool,
}

impl InputAttach {
    /// Спробувати приєднатися до `target`. Гард на власний потік і нульовий tid.
    fn new(me: u32, target: u32) -> Self {
        let attached = target != 0
            && target != me
            && unsafe {
                AttachThreadInput(me, target, 1 /* TRUE */)
            } != 0;
        Self {
            me,
            target,
            attached,
        }
    }
}

impl Drop for InputAttach {
    fn drop(&mut self) {
        if self.attached {
            unsafe {
                AttachThreadInput(self.me, self.target, 0 /* FALSE */)
            };
        }
    }
}

/// `HKL` **активного вікна переднього плану** (а не системний дефолт).
///
/// **Готча (емпірично підтверджено `layoutprobe`):** `GetKeyboardLayout(fg_tid)`
/// для UWP/консольних вікон бреше — `GetForegroundWindow` віддає обгортку
/// `ApplicationFrameWindow`, чий потік має дефолтну розкладку, а не реальну.
/// Рятує **метод M2** ([`m2_hkl`]): `GetGUIThreadInfo(fgTid).hwndFocus` дає
/// справжнє фокусне вікно (всередині UWP-хоста) → читаємо розкладку ЙОГО потоку.
/// Для звичайних вікон M1/M2/M3 рівноцінні; M2 обрано саме заради UWP/console.
pub fn current_hkl() -> HKL {
    m2_hkl()
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

/// Діагностика (для `live_spike`): сирі біти HKL активного вікна як `usize`.
/// Не для продакшн-логіки — лише друк/дебаг.
pub fn current_hkl_bits() -> usize {
    current_hkl() as usize
}

// =========================================================================
// ДІАГНОСТИКА: 4 методи визначення розкладки активного вікна (для layoutprobe).
// Мета — емпірично знайти, який метод СЛІДУЄ за реальною розкладкою на цій
// машині (UWP/консольні вікна ламають частину з них). Це НЕ продакшн-шлях —
// після вибору переможця `current_hkl` перепишемо на нього.
// =========================================================================

/// Результат одного методу: змаплений [`LayoutId`] + сирі біти HKL.
#[derive(Debug, Clone)]
pub struct MethodResult {
    pub id: LayoutId,
    pub hkl_bits: usize,
}

/// Результати всіх 4 методів за один прохід (порівнювати поряд).
#[derive(Debug, Clone)]
pub struct LayoutProbe {
    /// M1: `GetKeyboardLayout(tid(GetForegroundWindow))` — поточний (ламається на UWP).
    pub m1: MethodResult,
    /// M2: через `GetGUIThreadInfo`→`hwndFocus`→його потік.
    pub m2: MethodResult,
    /// M3: `AttachThreadInput` + `GetKeyboardLayout(fgThread)`.
    pub m3: MethodResult,
    /// M4: `GetKeyboardLayout(0)` (наш потік — контроль).
    pub m4: MethodResult,
}

fn result_of(hkl: HKL) -> MethodResult {
    MethodResult {
        id: layout_id_for_hkl(hkl),
        hkl_bits: hkl as usize,
    }
}

/// tid потоку, що володіє вікном переднього плану (0, якщо вікна немає).
fn foreground_tid() -> u32 {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            0
        } else {
            GetWindowThreadProcessId(hwnd, std::ptr::null_mut())
        }
    }
}

/// M1 — поточний (зламаний) метод.
fn m1_hkl() -> HKL {
    unsafe { GetKeyboardLayout(foreground_tid()) }
}

/// M2 — розкладка потоку, що володіє `hwndFocus` (через `GetGUIThreadInfo`).
fn m2_hkl() -> HKL {
    unsafe {
        let hwnd = GetForegroundWindow();
        let fg_tid = if hwnd.is_null() {
            0
        } else {
            GetWindowThreadProcessId(hwnd, std::ptr::null_mut())
        };
        let mut gti: GUITHREADINFO = std::mem::zeroed();
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let focus_hwnd = if fg_tid != 0 && GetGUIThreadInfo(fg_tid, &mut gti) != 0 {
            if gti.hwndFocus.is_null() {
                hwnd
            } else {
                gti.hwndFocus
            }
        } else {
            hwnd
        };
        let focus_tid = if focus_hwnd.is_null() {
            fg_tid
        } else {
            GetWindowThreadProcessId(focus_hwnd, std::ptr::null_mut())
        };
        GetKeyboardLayout(focus_tid)
    }
}

/// M3 — `AttachThreadInput` до fg-потоку, тоді `GetKeyboardLayout(fgThread)`.
fn m3_hkl() -> HKL {
    unsafe {
        let fg_tid = foreground_tid();
        let me = GetCurrentThreadId();
        let _attach = InputAttach::new(me, fg_tid);
        // Свідомо НЕ 0: читаємо саме цільовий потік (на відміну від current_hkl).
        GetKeyboardLayout(fg_tid)
    }
}

/// M4 — наш потік (контроль; має бути стабільним).
fn m4_hkl() -> HKL {
    unsafe { GetKeyboardLayout(0) }
}

/// Порахувати всі 4 методи за один прохід.
pub fn probe_layout_methods() -> LayoutProbe {
    LayoutProbe {
        m1: result_of(m1_hkl()),
        m2: result_of(m2_hkl()),
        m3: result_of(m3_hkl()),
        m4: result_of(m4_hkl()),
    }
}

/// [`LayoutId`] усіх **уже встановлених** розкладок (для вибору цілей у probe).
pub fn installed_layout_ids() -> Vec<LayoutId> {
    installed_hkls()
        .into_iter()
        .map(layout_id_for_hkl)
        .collect()
}

/// Одна встановлена в ОС розкладка з **людською** назвою мови — для UI, де
/// користувач бачить, які розкладки є і яку пару TypoFix використовує.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledLayout {
    /// Локалізована назва мови («Українська», «English», «Русский»…); якщо
    /// дістати з ОС не вдалося — `0x{langid:04x}` як fallback.
    pub name: String,
    /// `PRIMARYLANGID` (молодші 10 біт langid): `en`=0x09, `uk`=0x22 — збігається
    /// для будь-якого варіанта мови. Дає змогу UI звʼязати розкладку з нашою парою.
    pub primary_langid: u16,
    /// Чи саме ця розкладка зараз активна (= [`current_hkl`]).
    pub is_active: bool,
}

/// Перелік **усіх уже встановлених** розкладок ОС із людськими назвами.
///
/// Через `GetKeyboardLayoutList` (нічого НЕ інсталює). Дублі однакового langid
/// (напр. дві англійські розкладки) **лишаються обидва** — користувач має бачити
/// варіанти. Назва — через `LCIDToLocaleName`→`GetLocaleInfoEx`
/// (`LOCALE_SLOCALIZEDLANGUAGENAME`); недоступна → hex-langid.
pub fn installed_layouts() -> Vec<InstalledLayout> {
    let current = current_hkl();
    installed_hkls()
        .into_iter()
        .map(|hkl| {
            let langid = (hkl as usize & 0xFFFF) as u16;
            InstalledLayout {
                name: language_name_for_langid(langid).unwrap_or_else(|| format!("0x{langid:04x}")),
                primary_langid: primary_langid(langid),
                is_active: hkl == current,
            }
        })
        .collect()
}

/// Локалізована назва мови за `langid` через Win32 locale-API. `None`, якщо ОС
/// не дала ні BCP-47-імені локалі, ні назви мови.
fn language_name_for_langid(langid: u16) -> Option<String> {
    unsafe {
        // langid → BCP-47 ім'я локалі (напр. "uk-UA"). LCID = langid (SORT_DEFAULT).
        let mut locale = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
        let n = LCIDToLocaleName(langid as u32, locale.as_mut_ptr(), locale.len() as i32, 0);
        if n <= 0 {
            return None;
        }
        // Ім'я локалі → локалізована назва мови ("Українська"/"English"/...).
        let mut buf = [0u16; 128];
        let got = GetLocaleInfoEx(
            locale.as_ptr(),
            LOCALE_SLOCALIZEDLANGUAGENAME,
            buf.as_mut_ptr(),
            buf.len() as i32,
        );
        if got <= 0 {
            return None;
        }
        // `got` включає завершальний NUL — відкидаємо його.
        let s = String::from_utf16_lossy(&buf[..(got as usize - 1)]);
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
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

    #[test]
    fn installed_layouts_lists_machine_layouts() {
        let layouts = installed_layouts();
        // Друк для живої діагностики (видно під `-- --nocapture`).
        for l in &layouts {
            println!(
                "розкладка: name={:?} primary_langid=0x{:02x} active={}",
                l.name, l.primary_langid, l.is_active
            );
        }
        // На реальній машині розкладок ≥1; рівно стільки ж, скільки HKL.
        assert_eq!(layouts.len(), installed_layout_ids().len());
        if !layouts.is_empty() {
            // Активна рівно одна (та сама, що current_layout_id).
            assert_eq!(layouts.iter().filter(|l| l.is_active).count(), 1);
            // Кожна назва непорожня (людська або hex-fallback).
            assert!(layouts.iter().all(|l| !l.name.is_empty()));
        }
    }
}
