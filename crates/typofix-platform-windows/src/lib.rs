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
mod secure;
#[cfg(windows)]
mod selection;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use windows_impl::WindowsPlatform;

// Публічні запити без побічних ефектів (безпечні для автотестів і app-шару):
// розкладка з ОС (`ToUnicodeEx`) та активне вікно.
#[cfg(windows)]
pub use layout::{
    char_for_active_layout, char_for_layout, current_hkl_bits, current_layout_id,
    installed_layout_ids, installed_layouts, probe_layout_methods, InstalledLayout, LayoutProbe,
    MethodResult,
};
#[cfg(windows)]
pub use secure::debug_uia_focus_is_password;
#[cfg(windows)]
pub use selection::get_selection_text;
#[cfg(windows)]
pub use window::{foreground_focus_is_secure, foreground_window_info};

#[cfg(not(windows))]
pub use stub::{installed_layouts, InstalledLayout, WindowsPlatform};

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

        fn is_secure_field(&self) -> bool {
            // Hot-path: лише читаємо кеш (дешево, без блокувань). Перерахунок
            // (нативна перевірка + UIA) робить хук-потік на зміні фокуса —
            // див. `secure`/`hook`. Покриває нативні ES_PASSWORD/passwordchar
            // поля + UIA IsPassword (WinRAR-combo, веб/Electron).
            crate::secure::cached_is_secure()
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use typofix_platform::{Action, InputEvent, LayoutId, Platform, WindowInfo};

    /// Заглушка для не-Windows цілей: компілюється, але нічого не робить.
    #[derive(Debug, Default)]
    pub struct WindowsPlatform;

    /// Дзеркало `layout::InstalledLayout` для не-Windows (порт розкладок — згодом).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct InstalledLayout {
        pub name: String,
        pub primary_langid: u16,
        pub is_active: bool,
    }

    /// На не-Windows розкладок ОС не перелічуємо — порожньо.
    pub fn installed_layouts() -> Vec<InstalledLayout> {
        Vec::new()
    }

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
