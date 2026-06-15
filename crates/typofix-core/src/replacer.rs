//! Будує план перенабору з [`Decision`]: `DeleteChars(n) + SwitchLayout(best) +
//! TypeUnicode(corrected)`. Текст іде як **Unicode-рядок** (не повтор scancode),
//! щоб не залежати від моменту перемикання розкладки. §3.2.
//!
//! ## Збереження регістру/Caps/апострофа — «безкоштовне»
//! Виправлений текст ([`Decision::best_text`]) уже містить правильний регістр і
//! апострофи, бо буфер зберігає [`KeyStroke`] **разом з модифікаторами**
//! (SHIFT/CAPS/ALTGR), а [`Layout::interpret`] застосовує їх посимвольно. Тож
//! `replacer` не потребує окремої логіки регістру — він лише пакує результат у
//! дії.
//!
//! [`KeyStroke`]: crate::KeyStroke
//! [`Layout::interpret`]: crate::Layout::interpret

use crate::detector::Decision;
use typofix_platform::Action;

/// Зібрати план дій із рішення детектора.
///
/// Якщо перемикати не треба — порожній план. Інакше: стерти `n` символів, що
/// зараз на екрані (довжина поточної інтерпретації), перемкнути розкладку для
/// подальшого набору й набрати виправлений текст.
pub fn plan(decision: &Decision) -> Vec<Action> {
    if !decision.switch {
        return Vec::new();
    }

    let delete_count = decision.current_text.chars().count() as u32;
    let mut actions = Vec::with_capacity(3);
    if delete_count > 0 {
        actions.push(Action::DeleteChars(delete_count));
    }
    actions.push(Action::SwitchLayout(decision.best.clone()));
    actions.push(Action::TypeUnicode(decision.best_text.clone()));
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LayoutId;

    fn decision(switch: bool, current: &str, best_text: &str, best: &str) -> Decision {
        Decision {
            best: LayoutId::new(best),
            best_text: best_text.to_string(),
            current_text: current.to_string(),
            switch,
            confidence: 0.0,
        }
    }

    #[test]
    fn no_switch_yields_empty_plan() {
        assert!(plan(&decision(false, "ghbdsn", "привіт", "uk")).is_empty());
    }

    #[test]
    fn switch_builds_delete_switch_type_in_order() {
        let actions = plan(&decision(true, "ghbdsn", "привіт", "uk"));
        assert_eq!(
            actions,
            vec![
                Action::DeleteChars(6),
                Action::SwitchLayout(LayoutId::new("uk")),
                Action::TypeUnicode("привіт".into()),
            ]
        );
    }

    #[test]
    fn delete_count_is_in_chars_not_bytes() {
        // "привіт" — 6 символів, але 12 байтів у UTF-8.
        let actions = plan(&decision(true, "привіт", "ghbdsn", "en"));
        assert_eq!(actions[0], Action::DeleteChars(6));
    }

    #[test]
    fn preserves_case_from_best_text() {
        // best_text уже з великої (бо страйк мав SHIFT) — replacer лише пакує.
        let actions = plan(&decision(true, "Ghbdsn", "Привіт", "uk"));
        assert_eq!(actions[2], Action::TypeUnicode("Привіт".into()));
    }

    #[test]
    fn preserves_apostrophe() {
        let actions = plan(&decision(true, "g'znm", "п'ять", "uk"));
        assert_eq!(actions[2], Action::TypeUnicode("п'ять".into()));
    }
}
