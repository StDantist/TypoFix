//! Живий перехоплювач вводу: `WH_KEYBOARD_LL` + `WH_MOUSE_LL` + WinEvent на
//! зміну ПЕРЕДНЬОГО ВІКНА (для емісії `FocusChange`), усе на одному потоці з
//! власним message-pump.
//!
//! ## Чому окремий потік із насосом
//! Низькорівневі хуки ОС **доставляють події лише потоку, що їх установив, і
//! лише поки той потік качає черга-повідомлень** (`GetMessage`/`DispatchMessage`).
//! Без насоса callbacks мовчать. Тому: окремий потік ставить усі хуки, крутить
//! pump, а зупиняється через `WM_QUIT` (`PostThreadMessage` із [`HookHandle`]).
//!
//! ## 🔴 Що тут НЕ робиться (інваріант стабільності)
//! Детекція секретних полів тут НЕ живе — вона на ОКРЕМОМУ потоці
//! ([`crate::secure_thread`]), щоб LL-pump лишався дешевим. (UIA з неї прибрано
//! назавжди — вмикав a11y цільової IDE й лагав; лишилась лише дешева нативна
//! перевірка.) Тут — лише `EVENT_SYSTEM_FOREGROUND` → емісія `FocusChange`.
//!
//! ## Цикл проти власного вводу (критично)
//! Хук бачить і фізичний ввід, і НАШ `SendInput`. Власні події позначені
//! `LLKHF_INJECTED` (+ підпис [`crate::inject::INJECT_SIGNATURE`]) → ставимо
//! `is_synthetic = true`; ядро їх ігнорує, тож перенабір не породжує перенабір.
//!
//! ## Стан у callbacks
//! LL-callbacks — глобальні `extern "system"` без `self`. Стан (канал, набір
//! натиснутих клавіш для auto-repeat) живе у `thread_local` HOOK_STATE того ж
//! потоку, де крутиться pump — тож синхронізація не потрібна.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use typofix_platform::{InputEvent, KeyDir, KeyEvent};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, VK_CAPITAL, VK_CONTROL, VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN,
    VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
    UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::keystate::ModSnapshot;
use crate::scancode::{classify_vk, KeyKind};
use crate::window::window_info_for_hwnd;

thread_local! {
    /// Стан перехоплювача для callbacks поточного (hook) потоку.
    static HOOK_STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

/// Внутрішній стан, доступний callbacks через `thread_local`.
struct HookState {
    /// Канал у споживача (`WindowsPlatform::try_next_event`).
    sender: Sender<InputEvent>,
    /// VK-коди зараз натиснутих клавіш — для виявлення auto-repeat.
    pressed: HashSet<u32>,
    /// VK, для яких auto-repeat уже просигналено (щоб не флудити повторами).
    repeat_signaled: HashSet<u32>,
}

impl HookState {
    fn emit(&self, ev: InputEvent) {
        // Канал міг закритися (споживач помер) — тоді просто ігноруємо.
        let _ = self.sender.send(ev);
    }
}

/// Дескриптор працюючого хук-потоку. Drop коректно зупиняє pump і знімає хуки.
pub struct HookHandle {
    tid: Arc<AtomicU32>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HookHandle {
    /// Запустити хук-потік і дочекатися його готовності (ідентифікатор потоку
    /// опубліковано — отже хуки встановлені й pump стартував).
    pub fn start(sender: Sender<InputEvent>) -> Self {
        let tid = Arc::new(AtomicU32::new(0));
        let tid_for_thread = Arc::clone(&tid);
        let join = std::thread::Builder::new()
            .name("typofix-hook".into())
            .spawn(move || hook_thread_main(sender, tid_for_thread))
            .expect("spawn hook thread");

        // Чекаємо, поки потік опублікує свій id (зазвичай мікросекунди).
        while tid.load(Ordering::Acquire) == 0 {
            std::thread::yield_now();
        }
        Self {
            tid,
            join: Some(join),
        }
    }
}

impl Drop for HookHandle {
    fn drop(&mut self) {
        let tid = self.tid.load(Ordering::Acquire);
        if tid != 0 {
            // WM_QUIT валить GetMessageW (повертає 0) → pump виходить, хуки знімаються.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    tid,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    0,
                    0,
                );
            }
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Тіло хук-потоку: ставить хуки, публікує tid, крутить message-pump.
fn hook_thread_main(sender: Sender<InputEvent>, tid_out: Arc<AtomicU32>) {
    HOOK_STATE.with(|s| {
        *s.borrow_mut() = Some(HookState {
            sender,
            pressed: HashSet::new(),
            repeat_signaled: HashSet::new(),
        });
    });

    unsafe {
        let hmod = GetModuleHandleW(std::ptr::null());
        let kb_hook: HHOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0);
        let mouse_hook: HHOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
        // Зміна вікна на передньому плані — лише для емісії `FocusChange` (дешево).
        // Детекцію секретності тут НЕ робимо: вона на `crate::secure_thread`.
        let fg_hook: HWINEVENTHOOK = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(winevent_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );

        // Публікуємо tid лише після встановлення хуків — споживач певен, що готові.
        tid_out.store(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            Ordering::Release,
        );

        // Message-pump: тримає LL-хуки живими, доки не прийде WM_QUIT.
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !kb_hook.is_null() {
            UnhookWindowsHookEx(kb_hook);
        }
        if !mouse_hook.is_null() {
            UnhookWindowsHookEx(mouse_hook);
        }
        if !fg_hook.is_null() {
            UnhookWinEvent(fg_hook);
        }
    }

    HOOK_STATE.with(|s| *s.borrow_mut() = None);
}

/// Прочитати фізичний стан модифікаторів (через async/toggle key-state).
fn read_mod_snapshot() -> ModSnapshot {
    let down = |vk: i32| (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0;
    ModSnapshot {
        shift: down(VK_SHIFT as i32),
        ctrl: down(VK_CONTROL as i32),
        alt: down(VK_MENU as i32),
        meta: down(VK_LWIN as i32) || down(VK_RWIN as i32),
        caps: (unsafe { GetKeyState(VK_CAPITAL as i32) } & 1) != 0,
        // AltGr = фізично правий Alt (Windows додатково тримає Ctrl — це
        // розрулює ModSnapshot::to_modifiers).
        altgr: down(VK_RMENU as i32),
    }
}

/// Обробка однієї клавіатурної події у `thread_local`-стані.
fn handle_keyboard(wparam: WPARAM, kb: &KBDLLHOOKSTRUCT) {
    let is_down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
    let is_up = wparam as u32 == WM_KEYUP || wparam as u32 == WM_SYSKEYUP;
    let vk = kb.vkCode;

    HOOK_STATE.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(state) = guard.as_mut() else { return };

        if is_up {
            // Відпускання лише оновлює облік натиснутих; події вгору не емітимо.
            state.pressed.remove(&vk);
            state.repeat_signaled.remove(&vk);
            return;
        }
        if !is_down {
            return;
        }

        // Auto-repeat: клавіша вже натиснута й ще не відпущена.
        let is_repeat = state.pressed.contains(&vk);
        if is_repeat {
            // Сигналимо повтор лише раз (дедуплікація потоку повторів).
            if !state.repeat_signaled.insert(vk) {
                return;
            }
        } else {
            state.pressed.insert(vk);
        }

        let synthetic = (kb.flags & LLKHF_INJECTED) != 0;
        let modifiers = read_mod_snapshot().to_modifiers();

        // Навігація рве звʼязок буфера з текстом → окрема подія.
        if classify_vk(vk) == KeyKind::CaretMove {
            state.emit(InputEvent::CaretMove);
            return;
        }

        state.emit(InputEvent::Key(KeyEvent {
            scancode: kb.scanCode,
            vk,
            dir: KeyDir::Down,
            modifiers,
            timestamp_ms: kb.time as u64,
            is_synthetic: synthetic,
            is_autorepeat: is_repeat,
        }));
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        handle_keyboard(wparam, kb);
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        match wparam as u32 {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                HOOK_STATE.with(|s| {
                    if let Some(state) = s.borrow().as_ref() {
                        state.emit(InputEvent::MouseClick);
                    }
                });
            }
            _ => {}
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn winevent_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
    _id_object: i32,
    _id_child: i32,
    _id_thread: u32,
    _ms_event_time: u32,
) {
    // Зміна вікна на передньому плані: інвалідуємо буфер (FocusChange). Дешево —
    // лише `window_info_for_hwnd` + надсилання в канал; жодного UIA/COM тут (див.
    // модульний док). Детекція секретності — на `crate::secure_thread`.
    if event == EVENT_SYSTEM_FOREGROUND {
        let info = window_info_for_hwnd(hwnd);
        HOOK_STATE.with(|s| {
            if let Some(state) = s.borrow().as_ref() {
                state.emit(InputEvent::FocusChange(info));
            }
        });
    }
}
