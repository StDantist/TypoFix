//! # typofix-platform-windows
//!
//! Реалізація [`typofix_platform::Platform`] для Windows поверх живого WinAPI:
//! `WH_KEYBOARD_LL`/`WH_MOUSE_LL` + WinEvent для подій, `SendInput` /
//! `ToUnicodeEx` / `WM_INPUTLANGCHANGEREQUEST` для дій і запитів.
//!
//! Архітектура: фоновий хук-потік із власним message-pump постачає
//! [`InputEvent`] у канал; `WindowsPlatform` на потоці движка тягне їх
//! ([`Platform::try_next_event`]) і виконує [`Action`] прямими викликами ОС.
//! Деталі й готчі — у локальному `CLAUDE.md` і `docs/ARCHITECTURE.md` §4.
//!
//! На не-Windows цілях крейт компілюється як тонка заглушка (щоб
//! `cargo build --workspace` лишався зеленим у CI), але **робить нічого**.

// Чисті, кросплатформні хелпери — компілюються й тестуються всюди.
mod keystate;
mod scancode;

#[cfg(windows)]
mod hook;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod layout;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use windows_impl::WindowsPlatform;

// Публічні запити без побічних ефектів (безпечні для автотестів і app-шару):
// розкладка з ОС (`ToUnicodeEx`) та активне вікно.
#[cfg(windows)]
pub use layout::{
    char_for_active_layout, char_for_layout, current_hkl_bits, current_layout_id,
    installed_layout_ids, probe_layout_methods, LayoutProbe, MethodResult,
};
#[cfg(windows)]
pub use window::foreground_window_info;

#[cfg(not(windows))]
pub use stub::WindowsPlatform;

#[cfg(windows)]
mod windows_impl {
    use std::sync::mpsc::{channel, Receiver};

    use typofix_platform::{Action, InputEvent, LayoutId, Platform, WindowInfo};

    use crate::hook::HookHandle;
    use crate::{inject, layout, window};

    /// Жива реалізація [`Platform`] для Windows.
    ///
    /// Створення ставить глобальні хуки (перехоплюють **увесь** фізичний ввід) —
    /// тримай рівно один екземпляр на процес. Drop знімає хуки й глушить потік.
    pub struct WindowsPlatform {
        /// Споживацький кінець каналу подій від хук-потоку.
        rx: Receiver<InputEvent>,
        /// Живий хук-потік; Drop зупиняє його коректно. Поле тримає його живим.
        _hook: HookHandle,
    }

    impl WindowsPlatform {
        /// Встановити хуки й почати приймати події.
        ///
        /// ⚠️ Побічний ефект для всієї системи: ставить `WH_KEYBOARD_LL` /
        /// `WH_MOUSE_LL`. Для автотестів використовуй чисті модулі
        /// (`keystate`/`scancode`) або запити без хука (`layout`/`window`).
        pub fn new() -> Self {
            let (tx, rx) = channel();
            let hook = HookHandle::start(tx);
            Self { rx, _hook: hook }
        }
    }

    impl Default for WindowsPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Platform for WindowsPlatform {
        fn try_next_event(&mut self) -> Option<InputEvent> {
            // Неблокуюче: повертаємо None, коли черга порожня (контракт trait).
            self.rx.try_recv().ok()
        }

        fn apply(&mut self, action: &Action) {
            match action {
                Action::None | Action::CommitException(_) => {}
                Action::TypeUnicode(text) => inject::type_unicode(text),
                Action::DeleteChars(n) => inject::delete_chars(*n),
                Action::SwitchLayout(id) => {
                    // ЛИШЕ серед уже встановлених розкладок — НІКОЛИ не інсталюємо
                    // (інакше засмічуємо систему дублями). Немає такої мови →
                    // тихо не перемикаємо (precision > recall; явний опціон — згодом).
                    if let Some(hkl) = layout::installed_hkl_for_layout_id(id) {
                        inject::switch_layout(hkl);
                    }
                }
            }
        }

        fn active_window(&self) -> WindowInfo {
            window::foreground_window_info()
        }

        fn current_layout(&self) -> LayoutId {
            layout::current_layout_id()
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use typofix_platform::{Action, InputEvent, LayoutId, Platform, WindowInfo};

    /// Заглушка для не-Windows цілей: компілюється, але нічого не робить.
    #[derive(Debug, Default)]
    pub struct WindowsPlatform;

    impl WindowsPlatform {
        pub fn new() -> Self {
            Self
        }
    }

    impl Platform for WindowsPlatform {
        fn try_next_event(&mut self) -> Option<InputEvent> {
            None
        }
        fn apply(&mut self, _action: &Action) {}
        fn active_window(&self) -> WindowInfo {
            WindowInfo::default()
        }
        fn current_layout(&self) -> LayoutId {
            LayoutId::new("en")
        }
    }
}
