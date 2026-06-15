//! # typofix-core
//!
//! Чисте детерміноване ядро рішень TypoFix. **Жодного I/O, годинника чи
//! випадковості** — час приходить у подіях, дані — у [`Context`].
//!
//! Вхід — потік [`InputEvent`] + [`Context`]; вихід — послідовність
//! [`Action`]. Уся логіка живе у підмодулях нижче; зараз вони — заглушки, а
//! [`step`] повертає порожній план (нічого не робити).
//!
//! Опис підмодулів і алгоритму — у `docs/ARCHITECTURE.md` §3.

#![forbid(unsafe_code)]

pub mod buffer;
pub mod detector;
pub mod dict;
pub mod engine;
pub mod exceptions;
pub mod layout_mapper;
pub mod lm;
pub mod replacer;
pub mod rules;
pub mod undo;

pub use buffer::{BufferStore, WordBuffer};
pub use detector::{Decision, DetectorConfig, LanguageProfile};
pub use dict::Dictionary;
pub use exceptions::{ExclusionRules, LearnedExceptions};
pub use layout_mapper::{KeyCap, KeyStroke, Layout};
pub use lm::NgramModel;
pub use rules::WordRules;
pub use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, LayoutId, Modifiers, WindowInfo};

/// Увесь змінний стан ядра між викликами [`step`].
///
/// Тримає per-window буфери слів. Далі сюди додадуться стек undo й кеш
/// динамічних винятків.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineState {
    /// Буфери натискань поточного слова, окремо для кожного вікна.
    pub buffers: BufferStore,
    /// Самонавчені виключення слів (поповнюються на відкидання перенабору).
    /// App-шар може заповнити їх на старті з персистентного сховища.
    pub learned: LearnedExceptions,
    /// Внутрішнє: останній перенабір, що очікує можливого негайного відкидання.
    pending_retype: Option<engine::PendingRetype>,
}

/// Незмінний контекст одного кроку рішення.
///
/// Постачається платформою/оркестратором; ядро лише **читає** його. Несе
/// позичені ресурси мов ([`LanguageProfile`]) — core нічого не вантажить сам, а
/// важкі моделі не клонуються щокроку.
#[derive(Debug, Clone)]
pub struct Context<'a> {
    /// Активне вікно/застосунок на момент події.
    pub active_window: WindowInfo,
    /// Поточна активна розкладка системи.
    pub current_layout: LayoutId,
    /// Увімкнені мови-кандидати (включно з поточною).
    pub languages: &'a [LanguageProfile],
    /// Налаштування детектора (ваги, крива порогу).
    pub config: DetectorConfig,
    /// Виключення застосунків/папок (де TypoFix узагалі не діє).
    pub exclusions: &'a ExclusionRules,
    /// Правила рівня слова (veto/force).
    pub rules: &'a WordRules,
}

impl Context<'_> {
    /// Профіль поточної розкладки серед увімкнених мов, якщо є.
    pub fn current_profile(&self) -> Option<&LanguageProfile> {
        self.languages.iter().find(|p| p.id == self.current_layout)
    }

    /// Чи активне вікно повністю виключене (детектор має бути обійдений).
    pub fn is_window_excluded(&self) -> bool {
        self.exclusions.excludes(&self.active_window)
    }
}

/// Один крок машини рішень: обробити подію й повернути план дій.
///
/// Детермінований за побудовою: результат залежить лише від `state`, `ev` і
/// `ctx` (жодного годинника/IO/випадковості).
pub fn step(state: &mut EngineState, ev: InputEvent, ctx: &Context) -> Vec<Action> {
    engine::step(state, ev, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    static NO_EXCL: ExclusionRules = ExclusionRules::new();
    static NO_RULES: WordRules = WordRules::new();

    fn sample_context() -> Context<'static> {
        Context {
            active_window: WindowInfo::default(),
            current_layout: LayoutId::new("en"),
            languages: &[],
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        }
    }

    fn sample_key_event() -> InputEvent {
        InputEvent::Key(KeyEvent {
            scancode: 0x1E, // 'a' у scancode set 1
            vk: 0x41,
            dir: KeyDir::Down,
            modifiers: Modifiers::empty(),
            timestamp_ms: 0,
            is_synthetic: false,
            is_autorepeat: false,
        })
    }

    #[test]
    fn step_on_fresh_state_does_nothing() {
        let mut state = EngineState::default();
        let ctx = sample_context();
        let actions = step(&mut state, sample_key_event(), &ctx);
        assert!(
            actions.is_empty(),
            "свіже ядро не повинно діяти: {actions:?}"
        );
    }
}
