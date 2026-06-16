//! Інформація про активне вікно: процес + шлях до exe + (best-effort) fullscreen.
//! Потрібно для per-window буфера та виключень за апкою/папкою.

use typofix_platform::WindowInfo;
use windows_sys::Win32::Foundation::{CloseHandle, HWND, RECT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, SM_CXSCREEN,
    SM_CYSCREEN,
};

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

#[cfg(test)]
mod tests {
    use super::*;

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
