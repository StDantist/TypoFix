//! # typofix-platform
//!
//! Авторитетне джерело **спільних типів** TypoFix і абстракції ОС
//! ([`trait Platform`](Platform)).
//!
//! Цей крейт навмисно мінімальний і залежить лише від `std` (+ `bitflags`).
//! На нього спираються `typofix-core` (логіка) і всі реалізації платформи
//! (`windows` / `macos` / `virtual`). Типи тут — контракт між шарами, тож
//! будь-яка зміна тут зачіпає всю команду.
//!
//! Ключові інваріанти (див. `docs/ARCHITECTURE.md`):
//! - Час **передається ззовні** ([`KeyEvent::timestamp_ms`]) — ядро лишається
//!   детермінованим, без годинника.
//! - У ядро йдуть **усі** події ([`InputEvent`]), не лише клавіші, бо буфер
//!   слова інвалідується також кліком/навігацією/зміною фокуса.
//! - Перенабір тексту — через [`Action::TypeUnicode`] (готові символи), а не
//!   повтор scancode у новій розкладці.

use bitflags::bitflags;

/// Мовно-агностичний ідентифікатор розкладки/мови, напр. `"uk"`, `"en"`.
///
/// Це не локаль і не HKL — а наш внутрішній стабільний ключ, за яким
/// підбираються дані (розкладка/LM/словник) і який платформа мапить на
/// конкретний системний ідентифікатор розкладки.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayoutId(pub String);

impl LayoutId {
    /// Зручний конструктор з будь-чого рядкоподібного.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Рядкове представлення ідентифікатора.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Напрямок події клавіші.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyDir {
    /// Клавішу натиснуто.
    Down,
    /// Клавішу відпущено.
    Up,
}

bitflags! {
    /// Модифікатори, активні в момент події клавіші.
    ///
    /// `META` — це Win на Windows і Cmd на macOS. `ALTGR` тримаємо окремо від
    /// `ALT`, бо для розкладок це різні логічні стани (правий Alt дає третій
    /// рівень символів).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Modifiers: u16 {
        const CTRL  = 1 << 0;
        const ALT   = 1 << 1;
        const SHIFT = 1 << 2;
        /// Win (Windows) / Cmd (macOS).
        const META  = 1 << 3;
        /// Caps Lock увімкнено.
        const CAPS  = 1 << 4;
        /// AltGr (правий Alt) — третій рівень розкладки.
        const ALTGR = 1 << 5;
    }
}

/// Подія однієї фізичної клавіші.
///
/// Працюємо на рівні **фізичних** кодів (`scancode`), а не символів —
/// розпізнавання розкладки потребує читати одну й ту саму послідовність
/// натисків у різних розкладках.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// Апаратний scancode (set 1 на Windows / відповідник на macOS).
    pub scancode: u32,
    /// Віртуальна клавіша (VK на Windows / keycode на macOS).
    pub vk: u32,
    /// Натиск чи відпускання.
    pub dir: KeyDir,
    /// Активні модифікатори на момент події.
    pub modifiers: Modifiers,
    /// Час події в мілісекундах. **Передається ззовні** заради детермінізму
    /// ядра — ядро ніколи не читає системний годинник.
    pub timestamp_ms: u64,
    /// `true`, якщо подію згенерували ми самі (наш перенабір). Хук/ядро
    /// мають таке ігнорувати, щоб не утворився цикл.
    pub is_synthetic: bool,
    /// `true`, якщо це повтор від утримання клавіші (auto-repeat).
    pub is_autorepeat: bool,
}

/// Будь-яка подія вводу, що надходить у ядро.
///
/// У ядро йдуть **усі** події, не лише клавіші: клік, зміна фокуса й рух
/// курсора інвалідують буфер слова (інакше перенабір зітре не той текст).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputEvent {
    /// Подія клавіші.
    Key(KeyEvent),
    /// Клік мишею — позиція курсора могла змінитися.
    MouseClick,
    /// Зміна активного вікна/контролу — буфер ще й per-window.
    FocusChange(WindowInfo),
    /// Навігація курсором (стрілки / Home / End / виділення) — розриває
    /// зв'язок буфера з текстом перед курсором.
    CaretMove,
}

/// Інформація про активне вікно/застосунок.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct WindowInfo {
    /// Ім'я процесу (напр. `"notepad.exe"`).
    pub process_name: String,
    /// Повний шлях до виконуваного файлу (для виключень за папкою).
    pub exe_path: String,
    /// Чи вікно у повноекранному / exclusive-input режимі (авто-пауза).
    pub is_fullscreen: bool,
}

/// Дія, яку ядро просить платформу виконати.
///
/// Перенабір розбито на окремі дії свідомо: [`SwitchLayout`](Action::SwitchLayout)
/// потрібен лише щоб **подальший** ручний набір був у правильній розкладці, а сам
/// перенабраний текст іде через [`TypeUnicode`](Action::TypeUnicode) як готові
/// символи — це усуває гонку з асинхронним перемиканням розкладки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Нічого не робити.
    None,
    /// Перемкнути активну розкладку (для подальшого набору користувача).
    SwitchLayout(LayoutId),
    /// Видалити `n` символів перед курсором (Backspace × n).
    DeleteChars(u32),
    /// Набрати готовий Unicode-текст (перенабір), не повторюючи scancode.
    TypeUnicode(String),
    /// Самонавчання: додати слово у винятки (після скасування користувачем).
    CommitException(String),
}

/// Абстракція операційної системи для TypoFix.
///
/// Один і той самий trait реалізують `typofix-platform-windows`,
/// `typofix-platform-macos` і `typofix-platform-virtual`. Контракт навмисно
/// вузький: платформа лише **постачає події**, **виконує дії** й **відповідає
/// на запити стану** — уся логіка рішень лишається в `typofix-core`.
///
/// ## Потокова модель
/// Реалізації для реальних ОС працюють у фоновому хук-потоці з мінімальним
/// обсягом роботи в callback (див. `docs/ARCHITECTURE.md` §5). Метод
/// [`try_next_event`](Platform::try_next_event) тут — споживацький бік цього
/// каналу: він **не блокує** і повертає `None`, коли подій немає.
pub trait Platform {
    /// Неблокуюче отримання наступної події вводу.
    ///
    /// Повертає `Some(ev)`, якщо подія готова, інакше `None`. Викликається
    /// движковим потоком у циклі. Реалізація відповідає за дедуплікацію
    /// auto-repeat і позначення синтетичних подій (`is_synthetic`).
    fn try_next_event(&mut self) -> Option<InputEvent>;

    /// Застосувати дію, обчислену ядром.
    ///
    /// Синтетичний ввід, породжений тут (напр. [`Action::TypeUnicode`] /
    /// [`Action::DeleteChars`]), **мусить** позначатися як `is_synthetic`, щоб
    /// власний хук його ігнорував і не виник цикл. [`Action::None`] — no-op.
    fn apply(&mut self, action: &Action);

    /// Поточне активне вікно (процес, шлях до exe, fullscreen).
    fn active_window(&self) -> WindowInfo;

    /// Поточна активна розкладка системи.
    fn current_layout(&self) -> LayoutId;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_combine_and_separate_altgr_from_alt() {
        let m = Modifiers::CTRL | Modifiers::SHIFT;
        assert!(m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::ALT));
        // ALTGR — це окремий біт, не ALT.
        assert!(!Modifiers::ALTGR.contains(Modifiers::ALT));
    }

    #[test]
    fn layout_id_roundtrips() {
        let id = LayoutId::new("uk");
        assert_eq!(id.as_str(), "uk");
        assert_eq!(id, LayoutId("uk".to_string()));
    }
}
