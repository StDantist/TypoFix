//! АВТОНОМНИЙ live-доказ, що ПРОДАКШН `foreground_focus_is_secure()` коректно
//! детектує СПРАВЖНЄ нативне поле пароля. Створює власне топ-вікно з дочірнім
//! EDIT-контролом `ES_PASSWORD`, виводить його на передній план, фокусує
//! password-edit → очікує `is_secure_field()==TRUE`; тоді фокусує звичайний EDIT
//! (без `ES_PASSWORD`) → очікує `false`. Код виходу 0 = обидва вердикти зелені.
//!
//! Це герметична альтернатива WinRAR: WinRAR v7 ховає пароль у `ComboBox` без
//! `ES_PASSWORD` (детекція його не ловить — див. `CLAUDE.md`), тож для перевірки
//! САМОГО механізму потрібне гарантовано-нативне `ES_PASSWORD`-поле.
//!
//! ⚠️ Перевіряє ЛИШЕ нативний шлях (`foreground_focus_is_secure`); UIA з рантайму
//! прибрано назавжди (вмикав a11y-дерево цільової апки → лаг).

#[cfg(windows)]
fn main() {
    std::process::exit(win::run());
}

#[cfg(windows)]
mod win {
    use std::time::Duration;

    use typofix_platform_windows::foreground_focus_is_secure;
    use windows_sys::core::w;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, CreateWindowExW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
        GetWindowThreadProcessId, PeekMessageW, SetForegroundWindow, ShowWindow, TranslateMessage,
        MSG, PM_REMOVE, SW_SHOW, WS_BORDER, WS_POPUP, WS_VISIBLE,
    };

    const ES_PASSWORD: u32 = 0x0020;
    const ES_AUTOHSCROLL: u32 = 0x0080;

    pub fn run() -> i32 {
        println!("=== TypoFix secure_synth (доказ детекції нативного ES_PASSWORD) ===\n");
        unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let edit = w!("EDIT");

            // Топ-рівневе поле ПАРОЛЯ (ES_PASSWORD) — без реєстрації власного класу.
            let pw = CreateWindowExW(
                0,
                edit,
                std::ptr::null(),
                WS_POPUP | WS_VISIBLE | WS_BORDER | ES_PASSWORD | ES_AUTOHSCROLL,
                120,
                120,
                360,
                30,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            // Топ-рівневе ЗВИЧАЙНЕ поле (без ES_PASSWORD) — контроль.
            let plain = CreateWindowExW(
                0,
                edit,
                std::ptr::null(),
                WS_POPUP | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL,
                120,
                170,
                360,
                30,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinst,
                std::ptr::null(),
            );
            if pw.is_null() || plain.is_null() {
                eprintln!("СЕРЕДОВИЩЕ: CreateWindowExW edit не вдалось — немає GUI-сесії?");
                return 1;
            }

            // (1) Фокус на полі ПАРОЛЯ → нативна перевірка TRUE (ES_PASSWORD у стилі).
            ShowWindow(pw, SW_SHOW);
            force_fg(pw);
            SetFocus(pw);
            pump(Duration::from_millis(300));
            let secure_pw = foreground_focus_is_secure();
            println!("[ES_PASSWORD edit] native={}", up(secure_pw));

            // (2) Фокус на ЗВИЧАЙНОМУ полі → false.
            ShowWindow(plain, SW_SHOW);
            force_fg(plain);
            SetFocus(plain);
            pump(Duration::from_millis(300));
            let secure_plain = foreground_focus_is_secure();
            println!("[звичайний edit]   native={}", up(secure_plain));

            DestroyWindow(pw);
            DestroyWindow(plain);

            if secure_pw && !secure_plain {
                println!("\n✅ Коректно: ES_PASSWORD→секретне (native), звичайне→ні.");
                0
            } else {
                eprintln!("\n❌ Несподівано: native(pw={secure_pw},plain={secure_plain})");
                1
            }
        }
    }

    fn up(b: bool) -> &'static str {
        if b {
            "TRUE"
        } else {
            "false"
        }
    }

    unsafe fn force_fg(hwnd: HWND) {
        let fg = GetForegroundWindow();
        let target_tid = GetWindowThreadProcessId(fg, std::ptr::null_mut());
        let self_tid = GetCurrentThreadId();
        AttachThreadInput(self_tid, target_tid, 1);
        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);
        AttachThreadInput(self_tid, target_tid, 0);
    }

    /// Прокрутити чергу повідомлень ~`dur`, щоб вікно/фокус устаканились.
    unsafe fn pump(dur: Duration) {
        let end = std::time::Instant::now() + dur;
        let mut msg: MSG = std::mem::zeroed();
        while std::time::Instant::now() < end {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("secure_synth: лише Windows.");
}
