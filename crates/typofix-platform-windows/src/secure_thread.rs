//! Окремий **виділений** потік детекції секретних полів: WinEvent focus-хуки +
//! UIA-перерахунок із власним message-pump і COM (STA).
//!
//! ## Чому окремий потік (🔴 критичний інваріант стабільності)
//! UIA-перевірка (`GetFocusedElement` → `IsPassword`) і нативний
//! `SendMessageTimeoutW` — **синхронні й повільні** (на IDE/Electron/Chromium UIA
//! вмикає accessibility-дерево й може коштувати десятки–сотні мс). Якщо їх крутити
//! на потоці LL-хука (`crate::hook`), вони блокують його message-pump → ОС бачить
//! `LowLevelHooksTimeout` → **увесь системний ввід лагає** (репро власника: IDE з
//! file-watcher сипле штормом `EVENT_OBJECT_FOCUS`, курсор лагав 30–40 с).
//! Тому: важкий перерахунок живе ТУТ, на власному потоці з власним pump; потік
//! LL-хука лишається дешевим (його `is_secure_field` лише читає атомік `secure::CACHE`).
//!
//! ## Дебаунс (коалесинг шторму)
//! IDE сипле фокус-події пачками. Кожна подія НЕ запускає перерахунок одразу —
//! вона лише (пере)зводить короткий таймер ([`FOCUS_DEBOUNCE_MS`]); перерахунок
//! іде ОДИН раз після «осідання» фокуса. Так навіть тут UIA не молотиться даремно.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer, TranslateMessage,
    EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND, MSG, WINEVENT_OUTOFCONTEXT, WM_QUIT, WM_TIMER,
};

/// Затримка коалесингу фокус-подій (мс): перерахунок секретності запускаємо лише
/// після короткого осідання фокуса, щоб шторм подій не молотив UIA даремно. Малий,
/// щоб реакція на «Tab у поле пароля» лишалась миттєвою на людський масштаб.
const FOCUS_DEBOUNCE_MS: u32 = 60;

thread_local! {
    /// Активний дебаунс-таймер (0 = немає). NULL-window-таймер кладе `WM_TIMER` у
    /// чергу саме цього потоку, тож вікна не треба. Живе у `thread_local`, бо вся
    /// робота йде на одному потоці.
    static DEBOUNCE_TIMER: Cell<usize> = const { Cell::new(0) };
}

/// Дескриптор працюючого потоку детекції секретних полів. Drop коректно зупиняє
/// pump (`WM_QUIT`) і знімає WinEvent-хуки.
pub struct SecureHandle {
    tid: Arc<AtomicU32>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl SecureHandle {
    /// Запустити потік і дочекатися готовності (tid опубліковано → хуки стоять,
    /// pump крутиться).
    pub fn start() -> Self {
        let tid = Arc::new(AtomicU32::new(0));
        let tid_for_thread = Arc::clone(&tid);
        let join = std::thread::Builder::new()
            .name("typofix-secure".into())
            .spawn(move || secure_thread_main(tid_for_thread))
            .expect("spawn secure thread");

        while tid.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        Self {
            tid,
            join: Some(join),
        }
    }
}

impl Drop for SecureHandle {
    fn drop(&mut self) {
        let tid = self.tid.load(Ordering::Acquire);
        if tid != 0 {
            // WM_QUIT валить GetMessageW (повертає 0) → pump виходить, хуки знімаються.
            unsafe { PostThreadMessageW(tid, WM_QUIT, 0, 0) };
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// (Пере)звести дебаунс-таймер: попередній знімаємо, ставимо новий на
/// [`FOCUS_DEBOUNCE_MS`]. Викликається з WinEvent-callback цього ж потоку.
fn schedule_recompute() {
    DEBOUNCE_TIMER.with(|t| unsafe {
        let existing = t.get();
        if existing != 0 {
            KillTimer(std::ptr::null_mut(), existing);
        }
        // NULL hwnd + NULL callback → WM_TIMER лягає у чергу цього потоку.
        let id = SetTimer(std::ptr::null_mut(), 0, FOCUS_DEBOUNCE_MS, None);
        t.set(id);
    });
}

/// WinEvent-callback (на ЦЬОМУ потоці): будь-яка зміна фокуса/переднього вікна
/// лише відкладає перерахунок (дебаунс), нічого важкого тут не робимо.
unsafe extern "system" fn secure_winevent_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: windows_sys::Win32::Foundation::HWND,
    _id_object: i32,
    _id_child: i32,
    _id_thread: u32,
    _ms_event_time: u32,
) {
    schedule_recompute();
}

/// Тіло потоку: COM(STA) → WinEvent-хуки фокуса → pump із дебаунс-перерахунком.
fn secure_thread_main(tid_out: Arc<AtomicU32>) {
    // COM на ЦЬОМУ потоці (тут є message-pump → STA). Скидаємо кеш (нова сесія) і
    // рахуємо для поточного фокуса одразу.
    crate::secure::com_init();
    crate::secure::reset_cache();
    crate::secure::recompute();

    unsafe {
        // Зміна вікна на передньому плані + зміна фокуса між контролами (Tab у поле
        // пароля). Обидві → лише (пере)звести дебаунс; перерахунок — після осідання.
        let fg_hook: HWINEVENTHOOK = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(secure_winevent_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        let focus_hook: HWINEVENTHOOK = SetWinEventHook(
            EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_FOCUS,
            std::ptr::null_mut(),
            Some(secure_winevent_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        // Публікуємо tid лише після встановлення хуків (споживач певен, що готові).
        tid_out.store(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            Ordering::Release,
        );

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            // Дебаунс-таймер осів → знімаємо його (SetTimer періодичний!) і робимо
            // перерахунок РАЗ. NULL-window WM_TIMER не диспатчимо у window-proc.
            if msg.message == WM_TIMER {
                DEBOUNCE_TIMER.with(|t| {
                    let id = t.get();
                    if id != 0 {
                        KillTimer(std::ptr::null_mut(), id);
                        t.set(0);
                    }
                });
                crate::secure::recompute();
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !fg_hook.is_null() {
            UnhookWinEvent(fg_hook);
        }
        if !focus_hook.is_null() {
            UnhookWinEvent(focus_hook);
        }
    }

    crate::secure::com_shutdown();
}
