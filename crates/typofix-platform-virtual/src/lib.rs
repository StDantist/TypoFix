//! # typofix-platform-virtual
//!
//! Віртуальна реалізація [`Platform`] повністю в пам'яті (віртуальна
//! клавіатура + текстовий буфер + «активне вікно») для автономних E2E-тестів.
//!
//! Симулює те, як ОС *відреагувала б* на наші [`Action`]: вставка тексту,
//! видалення символів перед курсором, перемикання розкладки. Тести задають
//! події через [`VirtualPlatform::enqueue`] і читають результат через
//! [`VirtualPlatform::text`] / [`VirtualPlatform::caret`] — точно як побачив би
//! користувач у полі вводу.
//!
//! **Готча:** virtual моделює *наше уявлення* про ОС, а не саму ОС. Він не
//! ловить платформних багів (гонка перемикання розкладки, autocomplete-поля,
//! UIPI тощо) — це робота real-OS харнесу. Деталі в локальному `CLAUDE.md`.

use std::collections::VecDeque;

use typofix_platform::{Action, InputEvent, LayoutId, Platform, WindowInfo};

/// Симулятор ОС у пам'яті: текстове поле + активне вікно + розкладка + черга
/// вхідних подій.
///
/// Текст зберігаємо як [`Vec<char>`], а курсор — як індекс у *символах* (не
/// байтах). Так усі операції коректні для Unicode (наприклад `ї`, `’`) без
/// ризику розрубати кодову точку посередині.
#[derive(Debug, Clone)]
pub struct VirtualPlatform {
    /// Вміст віртуального текстового поля, посимвольно.
    text: Vec<char>,
    /// Позиція курсора як кількість символів *перед* ним (`0..=text.len()`).
    caret: usize,
    /// Поточне активне вікно.
    window: WindowInfo,
    /// Поточна активна розкладка.
    layout: LayoutId,
    /// Черга подій, які повертає [`Platform::try_next_event`] (FIFO).
    events: VecDeque<InputEvent>,
    /// Журнал застосованих дій — зручно для перевірок плумбінгу в тестах.
    applied: Vec<Action>,
}

impl Default for VirtualPlatform {
    fn default() -> Self {
        Self {
            text: Vec::new(),
            caret: 0,
            window: WindowInfo::default(),
            layout: LayoutId::new("en"),
            events: VecDeque::new(),
            applied: Vec::new(),
        }
    }
}

impl VirtualPlatform {
    /// Створити порожній віртуальний симулятор (розкладка за умовчанням `"en"`).
    pub fn new() -> Self {
        Self::default()
    }

    // --- Тестове API: налаштування входу -----------------------------------

    /// Поставити одну подію в кінець черги.
    pub fn enqueue(&mut self, ev: InputEvent) {
        self.events.push_back(ev);
    }

    /// Поставити кілька подій у чергу, зберігаючи порядок.
    pub fn enqueue_all(&mut self, evs: impl IntoIterator<Item = InputEvent>) {
        self.events.extend(evs);
    }

    /// Задати активне вікно (симуляція фокуса на іншому застосунку).
    ///
    /// Це лише змінює стан, який повертає [`Platform::active_window`]; щоб ядро
    /// дізналося про зміну, надішли ще й [`InputEvent::FocusChange`].
    pub fn set_window(&mut self, window: WindowInfo) {
        self.window = window;
    }

    /// Задати поточну розкладку напряму (поза [`Action::SwitchLayout`]).
    pub fn set_layout(&mut self, layout: LayoutId) {
        self.layout = layout;
    }

    /// Попередньо заповнити текст і поставити курсор у кінець (зручно для
    /// сценаріїв «користувач уже щось набрав»).
    pub fn set_text(&mut self, text: &str) {
        self.text = text.chars().collect();
        self.caret = self.text.len();
    }

    // --- Тестове API: інспекція стану --------------------------------------

    /// Поточний вміст віртуального текстового поля.
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// Позиція курсора в *символах* від початку тексту.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Поточне активне вікно (синонім до [`Platform::active_window`] для тестів).
    pub fn current_window(&self) -> WindowInfo {
        self.window.clone()
    }

    /// Журнал усіх дій, застосованих через [`Platform::apply`] (для перевірок
    /// плумбінгу: яку саме послідовність дій згенерувало ядро).
    pub fn applied_actions(&self) -> &[Action] {
        &self.applied
    }

    /// Скільки подій ще чекає в черзі.
    pub fn pending_events(&self) -> usize {
        self.events.len()
    }
}

impl Platform for VirtualPlatform {
    fn try_next_event(&mut self) -> Option<InputEvent> {
        self.events.pop_front()
    }

    fn apply(&mut self, action: &Action) {
        self.applied.push(action.clone());
        match action {
            // Вставити готовий текст у позицію курсора й посунути курсор за ним.
            Action::TypeUnicode(s) => {
                let inserted: Vec<char> = s.chars().collect();
                let n = inserted.len();
                self.text.splice(self.caret..self.caret, inserted);
                self.caret += n;
            }
            // Стерти n символів ПЕРЕД курсором (Backspace × n), не виходячи за
            // межі початку буфера.
            Action::DeleteChars(n) => {
                let n = (*n as usize).min(self.caret);
                let start = self.caret - n;
                self.text.drain(start..self.caret);
                self.caret = start;
            }
            // Лише змінює активну розкладку — на текст не впливає.
            Action::SwitchLayout(id) => {
                self.layout = id.clone();
            }
            // Не впливають на віртуальне текстове поле.
            Action::None | Action::CommitException(_) => {}
        }
    }

    fn active_window(&self) -> WindowInfo {
        self.window.clone()
    }

    fn current_layout(&self) -> LayoutId {
        self.layout.clone()
    }
}

/// Драйвер E2E-тестів: у циклі тягне події з платформи, прогоняє кожну через
/// `step` і застосовує повернені [`Action`] назад до платформи.
///
/// `step` отримує подію плюс знімок контексту (активне вікно + розкладка) на
/// момент події. Цей підпис дозволяє обгорнути `typofix_core::step` замиканням,
/// що тримає `EngineState` і будує `Context` (приклад — у integration-тестах
/// `typofix-core`). Так virtual-крейт не залежить від `core` (уникаємо циклу).
///
/// Працює, доки в черзі є події. Якщо `step` поставить нові події в чергу
/// (через захоплену `&mut VirtualPlatform`), вони теж оброблятимуться — але
/// типовий тест просто заповнює чергу заздалегідь.
pub fn drive<F>(platform: &mut VirtualPlatform, mut step: F)
where
    F: FnMut(InputEvent, &WindowInfo, &LayoutId) -> Vec<Action>,
{
    while let Some(ev) = platform.try_next_event() {
        let window = platform.active_window();
        let layout = platform.current_layout();
        for action in step(ev, &window, &layout) {
            platform.apply(&action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typofix_platform::{KeyDir, KeyEvent, Modifiers};

    fn key(scancode: u32) -> InputEvent {
        InputEvent::Key(KeyEvent {
            scancode,
            vk: 0,
            dir: KeyDir::Down,
            modifiers: Modifiers::empty(),
            timestamp_ms: 0,
            is_synthetic: false,
            is_autorepeat: false,
        })
    }

    #[test]
    fn type_unicode_inserts_at_caret_and_advances() {
        let mut p = VirtualPlatform::new();
        p.apply(&Action::TypeUnicode("hi".into()));
        assert_eq!(p.text(), "hi");
        assert_eq!(p.caret(), 2);
    }

    #[test]
    fn type_unicode_inserts_into_middle() {
        let mut p = VirtualPlatform::new();
        p.set_text("ac");
        // Поставимо курсор між a і c, стерши c і знову не вийде — зробимо інакше:
        // видалимо 1 символ (c), курсор стане 1, потім вставимо "b" і "c".
        p.apply(&Action::DeleteChars(1)); // "a", caret=1
        p.apply(&Action::TypeUnicode("b".into())); // "ab", caret=2
        assert_eq!(p.text(), "ab");
        assert_eq!(p.caret(), 2);
    }

    #[test]
    fn delete_chars_removes_before_caret() {
        let mut p = VirtualPlatform::new();
        p.set_text("hello");
        p.apply(&Action::DeleteChars(2));
        assert_eq!(p.text(), "hel");
        assert_eq!(p.caret(), 3);
    }

    #[test]
    fn delete_chars_clamps_to_buffer_start() {
        let mut p = VirtualPlatform::new();
        p.set_text("ab");
        p.apply(&Action::DeleteChars(10));
        assert_eq!(p.text(), "");
        assert_eq!(p.caret(), 0);
    }

    #[test]
    fn delete_on_empty_buffer_is_noop() {
        let mut p = VirtualPlatform::new();
        p.apply(&Action::DeleteChars(3));
        assert_eq!(p.text(), "");
        assert_eq!(p.caret(), 0);
    }

    #[test]
    fn unicode_chars_are_handled_per_char_not_byte() {
        let mut p = VirtualPlatform::new();
        // 'ї' і '’' — багатобайтові в UTF-8; працюємо по символах.
        p.apply(&Action::TypeUnicode("приї’".into()));
        assert_eq!(p.caret(), 5, "5 символів попри більшу к-сть байтів");
        p.apply(&Action::DeleteChars(2)); // стерти ї’
        assert_eq!(p.text(), "при");
        assert_eq!(p.caret(), 3);
    }

    #[test]
    fn switch_layout_changes_layout_not_text() {
        let mut p = VirtualPlatform::new();
        p.set_text("abc");
        p.apply(&Action::SwitchLayout(LayoutId::new("uk")));
        assert_eq!(p.current_layout(), LayoutId::new("uk"));
        assert_eq!(p.text(), "abc");
        assert_eq!(p.caret(), 3);
    }

    #[test]
    fn none_and_commit_exception_do_not_touch_buffer() {
        let mut p = VirtualPlatform::new();
        p.set_text("abc");
        p.apply(&Action::None);
        p.apply(&Action::CommitException("abc".into()));
        assert_eq!(p.text(), "abc");
        assert_eq!(p.caret(), 3);
    }

    #[test]
    fn retype_sequence_delete_then_type_simulates_correction() {
        let mut p = VirtualPlatform::new();
        p.set_text("ghbdsn"); // нібито «привіт» у неправильній розкладці
                              // Перенабір: стерти 6 символів, перемкнути розкладку, набрати правильне.
        p.apply(&Action::DeleteChars(6));
        p.apply(&Action::SwitchLayout(LayoutId::new("uk")));
        p.apply(&Action::TypeUnicode("привіт".into()));
        assert_eq!(p.text(), "привіт");
        assert_eq!(p.caret(), 6);
        assert_eq!(p.current_layout(), LayoutId::new("uk"));
    }

    #[test]
    fn try_next_event_pops_fifo() {
        let mut p = VirtualPlatform::new();
        p.enqueue(key(0x1E));
        p.enqueue_all([key(0x30), InputEvent::MouseClick]);
        assert_eq!(p.pending_events(), 3);
        assert_eq!(p.try_next_event(), Some(key(0x1E)));
        assert_eq!(p.try_next_event(), Some(key(0x30)));
        assert_eq!(p.try_next_event(), Some(InputEvent::MouseClick));
        assert_eq!(p.try_next_event(), None);
    }

    #[test]
    fn drive_pumps_all_events_through_step() {
        let mut p = VirtualPlatform::new();
        p.enqueue_all([key(0x1E), key(0x30), key(0x2E)]);
        // Тривіальний «step»: на кожну Key-подію набирає 'x'.
        let mut count = 0;
        drive(&mut p, |ev, _win, _layout| {
            if let InputEvent::Key(_) = ev {
                count += 1;
                vec![Action::TypeUnicode("x".into())]
            } else {
                vec![]
            }
        });
        assert_eq!(count, 3);
        assert_eq!(p.text(), "xxx");
        assert_eq!(p.pending_events(), 0);
    }

    #[test]
    fn drive_sees_window_and_layout_snapshot() {
        let mut p = VirtualPlatform::new();
        p.set_layout(LayoutId::new("uk"));
        p.set_window(WindowInfo {
            process_name: "notepad.exe".into(),
            ..Default::default()
        });
        p.enqueue(InputEvent::MouseClick);
        let mut seen: Option<(String, String)> = None;
        drive(&mut p, |_ev, win, layout| {
            seen = Some((win.process_name.clone(), layout.as_str().to_string()));
            vec![]
        });
        assert_eq!(seen, Some(("notepad.exe".into(), "uk".into())));
    }
}
