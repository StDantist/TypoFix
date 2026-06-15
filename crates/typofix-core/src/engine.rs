//! Оркестрація кроку рішення: зводить разом buffer → detector → rules →
//! replacer → undo. Поки заглушка — повертає порожній план.

use crate::{Context, EngineState};
use typofix_platform::{Action, InputEvent};

/// Внутрішня реалізація кроку (див. [`crate::step`]).
pub fn step(_state: &mut EngineState, _ev: InputEvent, _ctx: &Context) -> Vec<Action> {
    // TODO(phase-1): оновити буфер; на межі слова — викликати detector і,
    // за рішенням, зібрати план DeleteChars + SwitchLayout + TypeUnicode.
    Vec::new()
}
