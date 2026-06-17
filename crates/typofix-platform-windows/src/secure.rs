//! Детекція секретних (пароль) полів із **кешуванням на зміні фокуса**.
//!
//! ## Архітектура (hot-path!)
//! `Platform::is_secure_field()` кличеться ЩОКРОКУ (на кожне натискання) — там не
//! можна робити `SendMessageTimeout`/UIA(COM). Тому секретність обчислюється РАЗ
//! на ЗМІНУ ФОКУСА (WinEvent-хук, див. [`crate::hook`]) і кешується в [`CACHE`];
//! `is_secure_field` лише читає атомік (дешево, без блокувань).
//!
//! ## Порядок перевірки (у [`recompute`])
//! 1. Дешева **нативна** перевірка (`ES_PASSWORD`/`EM_GETPASSWORDCHAR`) —
//!    [`crate::window::native_focus_is_secure`].
//! 2. Якщо `false` → **UI Automation** `UIA_IsPasswordPropertyId` по СФОКУСОВАНОМУ
//!    елементу ([`uia_focus_is_password`], `GetFocusedElement` — дістає й WINDOWLESS
//!    елементи: **веб/Electron** `<input type=password>`, WPF/WinForms PasswordBox).
//!    ⚠️ НЕ ловить WinRAR v7 (його поле не виставляє IsPassword ніде — див. CLAUDE.md).
//!
//! ## COM
//! `recompute` крутиться на хук-потоці (де є message-pump). Той потік один раз
//! робить `CoInitializeEx(APARTMENTTHREADED)` ([`com_init`]) і ліниво створює
//! `CUIAutomation` (кеш у `thread_local`, перевикористання). Звільнення інтерфейсів
//! — вручну (`Release`), `CoUninitialize` при зупинці ([`com_shutdown`]).

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::core::{GUID, HRESULT};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows_sys::Win32::System::Variant::{VariantClear, VARIANT, VT_BOOL};
use windows_sys::Win32::UI::Accessibility::{CUIAutomation, UIA_IsPasswordPropertyId};

/// Кеш секретності поточного фокуса. Пише хук-потік (на зміні фокуса), читає
/// потік рушія (`is_secure_field`). Один процес → один екземпляр платформи.
static CACHE: AtomicBool = AtomicBool::new(false);

/// Прочитати закешовану секретність поточного фокуса (дешево, hot-path).
pub fn cached_is_secure() -> bool {
    CACHE.load(Ordering::Relaxed)
}

/// Перерахувати секретність поточного фокусного поля й оновити [`CACHE`].
/// Викликати на зміні фокуса (і раз на старті хука). Нативна перевірка → UIA-фолбек.
pub fn recompute() {
    let secure = match crate::window::foreground_focus_hwnd() {
        // Дешева нативна перевірка фокусного контрола; якщо не спрацювала —
        // UIA по СФОКУСОВАНОМУ елементу (дістає й windowless WPF/XAML/веб).
        Some(hwnd) => crate::window::native_focus_is_secure(hwnd) || uia_focus_is_password(),
        None => false,
    };
    CACHE.store(secure, Ordering::Relaxed);
}

/// Доказ плумбінгу для прикладу `secure_synth`: ініціалізувати COM (якщо треба) і
/// віддати UIA-перевірку сфокусованого елемента. Не для прод-шляху (там кеш).
pub fn debug_uia_focus_is_password() -> bool {
    com_init();
    uia_focus_is_password()
}

/// Скинути кеш у «не секретне» (на старті/зупинці хук-потоку — щоб не лишався
/// стан із попередньої сесії).
pub fn reset_cache() {
    CACHE.store(false, Ordering::Relaxed);
}

// ===========================================================================
// COM / UI Automation (ручні vtable поверх windows-sys — крейт не тягне `windows`)
// ===========================================================================

thread_local! {
    /// Лінивий екземпляр `IUIAutomation` на хук-потоці (перевикористовуємо).
    static UIA: Cell<*mut c_void> = const { Cell::new(std::ptr::null_mut()) };
    /// Чи `CoInitializeEx` на цьому потоці зробили саме ми (для парного Uninit).
    static COM_OWNED: Cell<bool> = const { Cell::new(false) };
}

/// IID `IUIAutomation` `{30CBE57D-D9D0-452A-AB13-7AC5AC4825EE}`.
const IID_IUIAUTOMATION: GUID = GUID::from_u128(0x30cbe57d_d9d0_452a_ab13_7ac5ac4825ee);

/// Vtable `IUIAutomation` (потрібен `GetFocusedElement`, решта — заглушки
/// потрібного зміщення; НІКОЛИ не викликаємо їх). **`GetFocusedElement`, а не
/// `ElementFromHandle`** — бо фокус може бути на WINDOWLESS-елементі (WPF/XAML/
/// веб-input усередині одного render-HWND), якого `ElementFromHandle(hwndFocus)`
/// не дістане (поверне лише вікно-хост).
#[repr(C)]
struct IUIAutomationVtbl {
    query_interface: *const c_void,
    add_ref: *const c_void,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    compare_elements: *const c_void,
    compare_runtime_ids: *const c_void,
    get_root_element: *const c_void,
    element_from_handle: *const c_void,
    element_from_point: *const c_void,
    get_focused_element: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

/// Vtable `IUIAutomationElement` (потрібен `GetCurrentPropertyValue`, слот 10).
#[repr(C)]
struct IUIAutomationElementVtbl {
    query_interface: *const c_void,
    add_ref: *const c_void,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    set_focus: *const c_void,
    get_runtime_id: *const c_void,
    find_first: *const c_void,
    find_all: *const c_void,
    find_first_build_cache: *const c_void,
    find_all_build_cache: *const c_void,
    build_updated_cache: *const c_void,
    get_current_property_value:
        unsafe extern "system" fn(*mut c_void, i32, *mut VARIANT) -> HRESULT,
}

/// Ініціалізувати COM-апартамент на поточному (хук) потоці. Викликати РАЗ перед
/// pump'ом. STA, бо потік має message-pump.
pub fn com_init() {
    unsafe {
        let hr = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        // S_OK (0) / S_FALSE (1) → ми ініціалізували. RPC_E_CHANGED_MODE → вже
        // ініціалізовано в іншій моделі (не наше — не будемо Uninit'ити).
        COM_OWNED.with(|c| c.set(hr >= 0));
    }
}

/// Звільнити UIA й COM на поточному потоці (при зупинці хука).
pub fn com_shutdown() {
    UIA.with(|cell| {
        let p = cell.replace(std::ptr::null_mut());
        if !p.is_null() {
            unsafe {
                let vtbl = *(p as *const *const IUIAutomationVtbl);
                ((*vtbl).release)(p);
            }
        }
    });
    if COM_OWNED.with(|c| c.replace(false)) {
        unsafe { CoUninitialize() };
    }
}

/// Лінивий `IUIAutomation` для поточного потоку (`null` при невдачі).
fn uia_instance() -> *mut c_void {
    UIA.with(|cell| {
        let existing = cell.get();
        if !existing.is_null() {
            return existing;
        }
        let mut ptr: *mut c_void = std::ptr::null_mut();
        let hr = unsafe {
            CoCreateInstance(
                &CUIAutomation,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_IUIAUTOMATION,
                &mut ptr,
            )
        };
        if hr < 0 || ptr.is_null() {
            return std::ptr::null_mut();
        }
        cell.set(ptr);
        ptr
    })
}

/// UIA-перевірка: чи СФОКУСОВАНИЙ елемент має `IsPassword == true`.
/// Ловить поля без нативного `ES_PASSWORD`, що виставляють UIA-семантику пароля
/// (WPF/WinForms PasswordBox, частина веб/Electron). Будь-яка невдача → `false`.
fn uia_focus_is_password() -> bool {
    let automation = uia_instance();
    if automation.is_null() {
        return false;
    }
    unsafe {
        // GetFocusedElement() → IUIAutomationElement* (descends into windowless).
        let vtbl = *(automation as *const *const IUIAutomationVtbl);
        let mut element: *mut c_void = std::ptr::null_mut();
        let hr = ((*vtbl).get_focused_element)(automation, &mut element);
        if hr < 0 || element.is_null() {
            return false;
        }

        // GetCurrentPropertyValue(UIA_IsPasswordPropertyId) → VARIANT(VT_BOOL).
        let el_vtbl = *(element as *const *const IUIAutomationElementVtbl);
        let mut var: VARIANT = std::mem::zeroed();
        let hr2 =
            ((*el_vtbl).get_current_property_value)(element, UIA_IsPasswordPropertyId, &mut var);
        let is_pw = hr2 >= 0
            && var.Anonymous.Anonymous.vt == VT_BOOL
            && var.Anonymous.Anonymous.Anonymous.boolVal != 0;
        VariantClear(&mut var);
        ((*el_vtbl).release)(element);
        is_pw
    }
}
