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

pub use layout_mapper::{KeyCap, KeyStroke, Layout};
pub use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, LayoutId, Modifiers, WindowInfo};

/// Увесь змінний стан ядра між викликами [`step`].
///
/// Поки порожній — наповнюватиметься per-window буфером, стеком undo,
/// динамічними винятками тощо у наступних фазах.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineState {
    // TODO(phase-1): buffer (per-window), undo-стек, кеш винятків.
}

/// Незмінний контекст для одного кроку рішення.
///
/// Постачається платформою/оркестратором; ядро лише читає його.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// Активне вікно/застосунок на момент події.
    pub active_window: WindowInfo,
    /// Поточна активна розкладка системи.
    pub current_layout: LayoutId,
    // TODO(phase-1): settings (мовні пари, поріг, правила, виключення).
}

/// Один крок машини рішень: обробити подію й повернути план дій.
///
/// Детермінований за побудовою: результат залежить лише від `state`, `ev` і
/// `ctx`. Зараз — заглушка, що нічого не робить.
pub fn step(state: &mut EngineState, ev: InputEvent, ctx: &Context) -> Vec<Action> {
    // Поки делегуємо в engine-заглушку, щоб місце склейки було видиме.
    engine::step(state, ev, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_context() -> Context {
        Context {
            active_window: WindowInfo::default(),
            current_layout: LayoutId::new("en"),
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
