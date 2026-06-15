//! # typofix-platform-virtual
//!
//! Віртуальна реалізація [`Platform`] повністю в пам'яті (віртуальна
//! клавіатура + текстовий буфер + «активне вікно») для автономних E2E-тестів.
//!
//! **Це лише каркас.** Повну реалізацію (черга подій, застосування дій до
//! віртуального тексту, керування фокусом/розкладкою) робить окрема задача —
//! тут навмисно чисте місце. Не додавати сюди логіку.

use typofix_platform::{Action, InputEvent, LayoutId, Platform, WindowInfo};

/// Симулятор ОС у пам'яті. Поки порожній каркас.
#[derive(Debug, Default)]
pub struct VirtualPlatform {
    // TODO: черга InputEvent, віртуальний текстовий буфер, активне вікно,
    //       поточна розкладка, лог застосованих дій для перевірок у тестах.
}

impl VirtualPlatform {
    /// Створити порожній віртуальний симулятор.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Platform for VirtualPlatform {
    fn try_next_event(&mut self) -> Option<InputEvent> {
        todo!("virtual: дістати подію з in-memory черги")
    }

    fn apply(&mut self, _action: &Action) {
        todo!("virtual: застосувати дію до in-memory тексту/розкладки")
    }

    fn active_window(&self) -> WindowInfo {
        todo!("virtual: повернути поточне віртуальне вікно")
    }

    fn current_layout(&self) -> LayoutId {
        todo!("virtual: повернути поточну віртуальну розкладку")
    }
}
