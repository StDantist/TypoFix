//! Живий smoke-тест round-trip над виділенням (B1) — **АВТОНОМНИЙ**.
//!
//! На відміну від `live_spike` (ручний), цей бінарник сам відтворює сценарій
//! «реального користувача» через Win32-автоматизацію й сам перевіряє результат:
//!
//! 1. Кладе у системний clipboard відомий sentinel-рядок.
//! 2. Запускає `notepad.exe`, чекає, поки воно стане foreground.
//! 3. Інжектить `hello world` (через `Action::TypeUnicode` → `inject::SendInput`).
//! 4. Синтет. Ctrl+A (виділити все).
//! 5. `get_selection_text()` → очікує `Some("hello world")`.
//! 6. Перевіряє, що clipboard ВІДНОВЛЕНО (sentinel на місці) — privacy-гарантія.
//! 7. Готча Taras: при ще активному виділенні інжектить `TypeUnicode("HELLO
//!    WORLD")`, тоді Ctrl+A+Ctrl+C і читає clipboard → очікує рівно
//!    `HELLO WORLD` (виділення затерлося вводом, без дублювання).
//! 8. Вбиває notepad (force, без діалогу збереження).
//!
//! ⚠️ ПОБІЧНІ ЕФЕКТИ: друкує у вікно з фокусом, перезаписує clipboard (потім
//! відновлює sentinel), force-kill усіх `notepad.exe`. Запускати лише в
//! GUI-сесії на тест-машині:
//! `cargo run -p typofix-platform-windows --bin selection_smoke`
//! Код виходу: 0 — усі перевірки пройшли; 1 — є провал/середовище без GUI.

#[cfg(windows)]
fn main() {
    std::process::exit(win::run());
}

#[cfg(windows)]
mod win {
    use std::ptr;
    use std::time::Duration;

    use typofix_platform::{Action, Platform};
    use typofix_platform_windows::{foreground_window_info, get_selection_text, WindowsPlatform};

    use windows_sys::Win32::Foundation::{GlobalFree, BOOL, HANDLE, HWND, LPARAM};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_CONTROL,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    const CF_UNICODETEXT: u32 = 13;
    const VK_A: u16 = 0x41;
    const VK_C: u16 = 0x43;
    /// Sentinel, який не трапиться у тексті тесту — щоб перевірити відновлення.
    const SENTINEL: &str = "TYPOFIX::clipboard::sentinel::Привіт-42";

    /// Запустити сценарій; повернути код виходу (0 = всі перевірки зелені).
    pub fn run() -> i32 {
        println!("=== TypoFix selection_smoke (АВТОНОМНИЙ live-тест B1) ===\n");

        // (0) Покласти sentinel у clipboard ДО всього.
        if !set_clipboard_text(SENTINEL) {
            eprintln!("СЕРЕДОВИЩЕ: не вдалося відкрити/записати clipboard — немає GUI-сесії?");
            return 1;
        }
        println!("[setup] clipboard ← sentinel");

        // (1) Запустити notepad.
        let mut child = match std::process::Command::new("notepad.exe").spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("СЕРЕДОВИЩЕ: не вдалося запустити notepad.exe: {e}");
                return 1;
            }
        };
        // Знайти вікно Notepad і ПРИМУСОВО вивести вперед (Windows блокує
        // крадіжку фокуса; обходимо через AttachThreadInput). Поллимо ~10 с,
        // бо Win11-Notepad (packaged) стартує повільно.
        let forced = force_foreground_notepad(Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(400));
        let fg = foreground_window_info().process_name;
        println!("[setup] foreground-процес: {fg:?} (примусово={forced})");
        let fg_ok = fg.to_lowercase().contains("notepad");
        if !fg_ok {
            eprintln!(
                "СЕРЕДОВИЩЕ: notepad не вдалося зробити активним вікном (foreground={fg:?}). \
                 Можливо немає інтерактивного робочого столу / вікно не знайдено — перериваю чесно."
            );
            let _ = child.kill();
            kill_all_notepad();
            set_clipboard_text(SENTINEL);
            return 1;
        }

        // Платформа для інжекту (apply → inject::type_unicode). Хуки нам не
        // заважають: події з каналу просто ігноруємо.
        let mut platform = WindowsPlatform::new();
        std::thread::sleep(Duration::from_millis(300));

        // (2) Надрукувати "hello world".
        platform.apply(&Action::TypeUnicode("hello world".into()));
        std::thread::sleep(Duration::from_millis(400));

        // (3) Виділити все (Ctrl+A).
        tap_with_ctrl(VK_A);
        std::thread::sleep(Duration::from_millis(250));

        // (4) Прочитати виділення.
        let selection = get_selection_text();
        let sel_ok = selection.as_deref() == Some("hello world");
        println!(
            "\n(а) get_selection_text() = {selection:?}  →  {}",
            verdict(sel_ok)
        );

        // (5) Clipboard має лишитись sentinel (privacy-гарантія відновлення).
        let after = read_clipboard_text();
        let clip_ok = after.as_deref() == Some(SENTINEL);
        println!(
            "(б) clipboard після виклику = {}  →  {}",
            after
                .as_deref()
                .map(|s| format!("{s:?}{}", if s == SENTINEL { " (sentinel)" } else { "" }))
                .unwrap_or_else(|| "None".into()),
            verdict(clip_ok)
        );

        // (6) Готча Taras: друк поверх ще активного виділення.
        platform.apply(&Action::TypeUnicode("HELLO WORLD".into()));
        std::thread::sleep(Duration::from_millis(400));
        // Незалежний readback: Ctrl+A + Ctrl+C + читання clipboard.
        tap_with_ctrl(VK_A);
        std::thread::sleep(Duration::from_millis(150));
        tap_with_ctrl(VK_C);
        std::thread::sleep(Duration::from_millis(250));
        let content = read_clipboard_text();
        let overwrite_ok = content.as_deref() == Some("HELLO WORLD");
        println!(
            "(в) вміст після друку поверх виділення = {content:?}  →  {}",
            verdict(overwrite_ok)
        );
        if !overwrite_ok {
            println!(
                "    ⚠️  Виділення НЕ затерлося вводом — для ApplyCase знадобиться \
                 DeleteChars/Backspace ПЕРЕД друком (передати Taras)."
            );
        } else {
            println!("    ✓ Виділення затерлося вводом — DeleteChars перед друком НЕ потрібен.");
        }

        // (7) Прибирання: вбити notepad без діалогу збереження + відновити sentinel.
        drop(platform); // зняти хуки до завершення
        let _ = child.kill();
        kill_all_notepad();
        set_clipboard_text(SENTINEL);
        println!("\n[cleanup] notepad вбито (force), clipboard відновлено на sentinel.");

        let all_ok = sel_ok && clip_ok && overwrite_ok;
        println!(
            "\n=== ПІДСУМОК: {} ===",
            if all_ok {
                "✅ усі перевірки пройшли"
            } else {
                "❌ є провали (див. вище)"
            }
        );
        if all_ok {
            0
        } else {
            1
        }
    }

    fn verdict(ok: bool) -> &'static str {
        if ok {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    }

    /// Зібрати один keyboard-INPUT (VK-режим, без scancode).
    fn kbd(vk: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// Синтетично натиснути Ctrl+`vk` (down Ctrl → down vk → up vk → up Ctrl).
    fn tap_with_ctrl(vk: u16) {
        let inputs = [
            kbd(VK_CONTROL, 0),
            kbd(vk, 0),
            kbd(vk, KEYEVENTF_KEYUP),
            kbd(VK_CONTROL, KEYEVENTF_KEYUP),
        ];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    /// Записати UTF-16 текст у clipboard (CF_UNICODETEXT). `false` при помилці.
    fn set_clipboard_text(s: &str) -> bool {
        let mut wide: Vec<u16> = s.encode_utf16().collect();
        wide.push(0);
        let bytes = wide.len() * 2;
        unsafe {
            let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes);
            if hmem.is_null() {
                return false;
            }
            let p = GlobalLock(hmem) as *mut u16;
            if p.is_null() {
                GlobalFree(hmem);
                return false;
            }
            ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
            GlobalUnlock(hmem);
            if OpenClipboard(ptr::null_mut()) == 0 {
                GlobalFree(hmem);
                return false;
            }
            EmptyClipboard();
            if SetClipboardData(CF_UNICODETEXT, hmem).is_null() {
                GlobalFree(hmem); // не прийнято — памʼять усе ще наша
                CloseClipboard();
                return false;
            }
            CloseClipboard();
        }
        true
    }

    /// Прочитати CF_UNICODETEXT з clipboard як String.
    fn read_clipboard_text() -> Option<String> {
        unsafe {
            if OpenClipboard(ptr::null_mut()) == 0 {
                return None;
            }
            let handle: HANDLE = GetClipboardData(CF_UNICODETEXT);
            let result = if handle.is_null() {
                None
            } else {
                let p = GlobalLock(handle) as *const u16;
                if p.is_null() {
                    None
                } else {
                    let mut len = 0usize;
                    while *p.add(len) != 0 {
                        len += 1;
                    }
                    let slice = std::slice::from_raw_parts(p, len);
                    let text = String::from_utf16_lossy(slice);
                    GlobalUnlock(handle);
                    Some(text)
                }
            };
            CloseClipboard();
            result
        }
    }

    /// Знайти топ-рівневе видиме вікно з «Notepad» у заголовку й примусово
    /// вивести його на передній план. Поллить до `timeout`. `true`, якщо вдалося.
    ///
    /// Примусовий fg робимо через `AttachThreadInput` до потоку поточного
    /// foreground-вікна — інакше Windows ігнорує `SetForegroundWindow` від
    /// процесу без права на фокус.
    fn force_foreground_notepad(timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(hwnd) = find_notepad_window() {
                unsafe {
                    let fg = GetForegroundWindow();
                    let target_tid = GetWindowThreadProcessId(fg, ptr::null_mut());
                    let self_tid = GetCurrentThreadId();
                    AttachThreadInput(self_tid, target_tid, 1);
                    ShowWindow(hwnd, SW_RESTORE);
                    BringWindowToTop(hwnd);
                    SetForegroundWindow(hwnd);
                    AttachThreadInput(self_tid, target_tid, 0);
                }
                std::thread::sleep(Duration::from_millis(250));
                if foreground_window_info()
                    .process_name
                    .to_lowercase()
                    .contains("notepad")
                {
                    return true;
                }
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    /// EnumWindows-пошук першого видимого вікна з «notepad» у заголовку.
    fn find_notepad_window() -> Option<HWND> {
        let mut found: Option<HWND> = None;
        unsafe {
            EnumWindows(Some(enum_proc), &mut found as *mut Option<HWND> as LPARAM);
        }
        found
    }

    /// Колбек EnumWindows: матчимо видиме вікно із «notepad» у заголовку.
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1; // продовжити
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            let title = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
            if title.contains("notepad") || title.contains("блокнот") {
                let out = &mut *(lparam as *mut Option<HWND>);
                *out = Some(hwnd);
                return 0; // знайшли — зупинити перебір
            }
        }
        1
    }

    /// Force-kill усіх notepad.exe (без діалогу збереження). Тест-прибирання.
    fn kill_all_notepad() {
        let _ = std::process::Command::new("taskkill")
            .args(["/f", "/im", "notepad.exe"])
            .output();
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("selection_smoke доступний лише на Windows.");
    std::process::exit(1);
}
