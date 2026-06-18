//! Інформація про активне вікно: процес + шлях до exe + (best-effort) fullscreen.
//! Потрібно для per-window буфера та виключень за апкою/папкою.

use typofix_platform::WindowInfo;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetSystemMetrics, GetWindowLongPtrW,
    GetWindowRect, GetWindowThreadProcessId, SendMessageTimeoutW, GUITHREADINFO, GWL_STYLE,
    SMTO_ABORTIFHUNG, SM_CXSCREEN, SM_CYSCREEN,
};

/// Біт стилю стандартного EDIT-контролу «текст приховано» (`ES_PASSWORD` із
/// `WinUser.h`). `windows-sys` його не реекспортує, тож тримаємо константу тут.
const ES_PASSWORD: isize = 0x0020;

/// `EM_GETPASSWORDCHAR` (`WinUser.h`): повертає символ-маску EDIT-контролу або 0,
/// якщо поле не маскує ввід. Ловить поля, що стали секретними через
/// `EM_SETPASSWORDCHAR` у рантаймі (без біта `ES_PASSWORD` у стилі).
const EM_GETPASSWORDCHAR: u32 = 0x00D2;

/// `WindowInfo` для вікна на передньому плані. Якщо переднього вікна немає
/// (напр. безголова сесія) — [`WindowInfo::default`].
pub fn foreground_window_info() -> WindowInfo {
    let hwnd = unsafe { GetForegroundWindow() };
    window_info_for_hwnd(hwnd)
}

/// `WindowInfo` для конкретного `hwnd` (використовує WinEvent на зміну фокуса).
pub fn window_info_for_hwnd(hwnd: HWND) -> WindowInfo {
    if hwnd.is_null() {
        return WindowInfo::default();
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let exe_path = process_image_path(pid).unwrap_or_default();
    let process_name = process_name_from_path(&exe_path);
    WindowInfo {
        process_name,
        exe_path,
        is_fullscreen: is_window_fullscreen(hwnd),
    }
}

/// Повний шлях до exe процесу за його PID (через `QueryFullProcessImageNameW`).
///
/// `PROCESS_QUERY_LIMITED_INFORMATION` — мінімальне право, доступне навіть для
/// процесів іншого користувача того ж рівня цілісності (елевовані лишаються
/// недоступними — це наш non-goal, UIPI).
pub fn process_image_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 32768]; // макс. довжина Windows-шляху (\\?\)
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT::default(),
            buf.as_mut_ptr(),
            &mut size,
        );
        CloseHandle(handle);
        if ok == 0 || size == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

/// Останній компонент шляху (ім'я exe), напр. `C:\…\notepad.exe` → `notepad.exe`.
pub fn process_name_from_path(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// Best-effort: чи вікно займає увесь **первинний** монітор.
///
/// Спрощено (тільки первинний монітор; багатомоніторні кейси — follow-up): якщо
/// прямокутник вікна ⊇ розмір екрана. Використовується для авто-паузи у
/// fullscreen-апках/іграх.
fn is_window_fullscreen(hwnd: HWND) -> bool {
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return false;
        }
        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        rect.left <= 0 && rect.top <= 0 && rect.right >= screen_w && rect.bottom >= screen_h
    }
}

/// Чи стиль вікна має біт `ES_PASSWORD` (текст приховано). Чисто й тестовно без
/// WinAPI: парсинг `GWL_STYLE`, що повернув `GetWindowLongPtrW`.
fn style_is_password(style: isize) -> bool {
    (style & ES_PASSWORD) != 0
}

/// Чи EDIT-контрол `hwnd` маскує ввід через `EM_GETPASSWORDCHAR` (символ ≠ 0).
/// Через `SendMessageTimeout` (короткий таймаут + `SMTO_ABORTIFHUNG`), бо це
/// синхронний крос-процесний виклик — не блокуємо рушій, якщо ціль зависла.
fn focus_has_password_char(hwnd: HWND) -> bool {
    unsafe {
        let mut result: usize = 0;
        let ok = SendMessageTimeoutW(
            hwnd,
            EM_GETPASSWORDCHAR,
            0,
            0,
            SMTO_ABORTIFHUNG,
            40,
            &mut result,
        );
        ok != 0 && result != 0
    }
}

/// Справжнє фокусне вікно (контрол) переднього плану, якщо є.
///
/// `GetForegroundWindow` → його GUI-потік (`GetWindowThreadProcessId`) →
/// `GetGUIThreadInfo(thread).hwndFocus` (працює навіть усередині UWP-хоста — той
/// самий M2-метод, що в `layout.rs`). `None`, якщо вікна/фокуса немає.
pub(crate) fn foreground_focus_hwnd() -> Option<HWND> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() {
            return None;
        }
        let fg_tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        if fg_tid == 0 {
            return None;
        }
        let mut gti: GUITHREADINFO = std::mem::zeroed();
        gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetGUIThreadInfo(fg_tid, &mut gti) == 0 || gti.hwndFocus.is_null() {
            return None;
        }
        Some(gti.hwndFocus)
    }
}

/// Чи ім'я класу контрола — EDIT-подібне (`Edit`/`RichEdit*`/`RICHEDIT50W`/
/// `RichEditD2DPT`…). 🔴 **Гейт precision (фікс false-positive secure):**
/// `ES_PASSWORD`(біт `0x20`) і `EM_GETPASSWORDCHAR` мають сенс ЛИШЕ для
/// EDIT-контролів. На НЕ-edit вікні той самий біт `0x20` означає геть інше
/// (BS_*/SS_*/LBS_*…), а `EM_GETPASSWORDCHAR` — невизначене повідомлення → хибний
/// `secure=true`, що **глушив би ВСІ перемикання** у звичайному контролі. Тому
/// нативну перевірку застосовуємо лише до edit-подібних класів; решту (вкл. справжні
/// windowless пароль-поля WPF/веб) добирає UIA-фолбек. Невдача читання класу → `false`.
fn class_is_edit_like(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
    class.contains("edit")
}

/// **Нативна** (дешева, без COM) перевірка секретності контрола: лише для
/// EDIT-подібного класу ([`class_is_edit_like`]) — `ES_PASSWORD` у `GWL_STYLE`
/// **АБО** маскування вводу (`EM_GETPASSWORDCHAR` ≠ 0; ловить поля з пароль-символом,
/// виставленим у рантаймі без біта стилю — інсталятори, логіни). НЕ-edit клас → `false`
/// (нативно не secure; UIA-фолбек добере справжні пароль-поля інших типів).
pub(crate) fn native_focus_is_secure(hwnd: HWND) -> bool {
    if !class_is_edit_like(hwnd) {
        return false;
    }
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    style_is_password(style) || focus_has_password_char(hwnd)
}

/// **Лише нативна** жива перевірка секретності фокусного поля переднього плану
/// (`ES_PASSWORD`/passwordchar). НЕ використовує UIA, НЕ блокує (для прямих
/// запитів/прикладів). Продакшн-шлях кешує результат (нативна + UIA) на зміні
/// фокуса — див. [`crate::secure`]. Невдача → `false`.
pub fn foreground_focus_is_secure() -> bool {
    foreground_focus_hwnd()
        .map(native_focus_is_secure)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_style_bit_is_parsed() {
        // ES_PASSWORD виставлено → секретне.
        assert!(style_is_password(0x0020));
        // Типовий EDIT-стиль із ES_PASSWORD серед інших бітів (WS_CHILD|WS_VISIBLE|…).
        assert!(style_is_password(0x50800020u32 as i32 as isize));
        // Без біта → не секретне (звичайне поле / ALL інші стилі).
        assert!(!style_is_password(0x0000));
        assert!(!style_is_password(0x50800000u32 as i32 as isize));
    }

    #[test]
    fn process_name_extraction() {
        assert_eq!(
            process_name_from_path(r"C:\Windows\notepad.exe"),
            "notepad.exe"
        );
        assert_eq!(process_name_from_path("/usr/bin/foo"), "foo");
        assert_eq!(process_name_from_path("bare.exe"), "bare.exe");
        assert_eq!(process_name_from_path(""), "");
    }

    /// Власний процес завжди запитується успішно й дає шлях до тест-бінарника.
    #[test]
    fn own_process_path_is_readable() {
        let pid = unsafe { windows_sys::Win32::System::Threading::GetCurrentProcessId() };
        let path = process_image_path(pid).expect("власний шлях має читатись");
        assert!(
            path.to_ascii_lowercase().ends_with(".exe"),
            "очікували .exe, отримали {path}"
        );
        let name = process_name_from_path(&path);
        assert!(!name.is_empty());
    }

    #[test]
    fn invalid_pid_returns_none() {
        // PID 0 (System Idle) — недоступний для QueryFullProcessImageName.
        assert!(process_image_path(0).is_none());
    }

    #[test]
    fn foreground_query_does_not_panic() {
        // У безголовій сесії може бути default — головне, що не панікує.
        let _ = foreground_window_info();
    }
}
