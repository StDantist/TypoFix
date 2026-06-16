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
//! ## Друкований роздільник на межі слова (real-OS готча)
//! Перенабір майже завжди тригериться **друкованим** роздільником (пробіл/Enter/
//! таб/пунктуація). На реальній ОС наш хук пропускає натиск далі, тож роздільник
//! **уже на екрані** перед курсором у момент перенабору. Тому, якщо `separator`
//! заданий, стираємо `word_len + 1` (слово РАЗОМ із роздільником) і вписуємо
//! `corrected + separator` (повертаємо роздільник, щоб набір тривав далі).
//! Інакше (непридатний для друку тригер — F-клавіша/Delete) роздільника на
//! екрані немає → стара поведінка (`word_len`, без дописування).
//!
//! [`KeyStroke`]: crate::KeyStroke
//! [`Layout::interpret`]: crate::Layout::interpret

use crate::detector::Decision;
use typofix_platform::Action;

/// Зібрати план дій із рішення детектора.
///
/// Якщо перемикати не треба — порожній план. Інакше: стерти символи, що зараз на
/// екрані (поточна інтерпретація — і друкований `separator`, якщо він є),
/// перемкнути розкладку для подальшого набору й набрати виправлений текст
/// (з тим самим роздільником у кінці).
///
/// `separator` — символ роздільника, який УЖЕ надрукований на екрані ОС (напр.
/// `' '` для пробілу, `'\n'` для Enter). `None`, якщо тригер межі не друкований.
pub fn plan(decision: &Decision, separator: Option<char>) -> Vec<Action> {
    if !decision.switch {
        return Vec::new();
    }

    let word_len = decision.current_text.chars().count() as u32;
    // Хвостовий суфікс (пунктуація-роздільник, що йде дослівно після слова) уже на
    // екрані в поточній розкладці → теж стерти й повернути.
    let suffix_len = decision.suffix.chars().count() as u32;
    // Друкований роздільник-тригер уже на екрані → стерти його разом зі словом.
    let delete_count = word_len + suffix_len + u32::from(separator.is_some());

    let mut typed = decision.best_text.clone();
    typed.push_str(&decision.suffix); // дослівний хвостовий роздільник (напр. ",")
    if let Some(sep) = separator {
        typed.push(sep); // повертаємо роздільник-тригер за виправленим словом
    }

    let mut actions = Vec::with_capacity(3);
    if delete_count > 0 {
        actions.push(Action::DeleteChars(delete_count));
    }
    // Чиста корекція регістру (перетриманий Shift) НЕ міняє розкладку — слово вже
    // в правильній мові, треба лише перенабрати з виправленим регістром. Для
    // розкладко-перемикання `SwitchLayout` обов'язковий (подальший набір).
    if !decision.caps_only {
        actions.push(Action::SwitchLayout(decision.best.clone()));
    }
    actions.push(Action::TypeUnicode(typed));
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
            suffix: String::new(),
            caps_only: false,
        }
    }

    fn decision_with_suffix(
        switch: bool,
        current: &str,
        best_text: &str,
        best: &str,
        suffix: &str,
    ) -> Decision {
        Decision {
            suffix: suffix.to_string(),
            ..decision(switch, current, best_text, best)
        }
    }

    #[test]
    fn no_switch_yields_empty_plan() {
        assert!(plan(&decision(false, "ghbdsn", "привіт", "uk"), Some(' ')).is_empty());
    }

    #[test]
    fn switch_builds_delete_switch_type_in_order() {
        // Без друкованого роздільника (напр. недрукований тригер межі).
        let actions = plan(&decision(true, "ghbdsn", "привіт", "uk"), None);
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
    fn printable_separator_deletes_word_plus_one_and_retypes_it() {
        // Регрес off-by-one: пробіл уже на екрані → стерти слово+пробіл (7), а не
        // лише слово (6); виправлене слово вписати РАЗОМ із пробілом.
        let actions = plan(&decision(true, "ghbdsn", "привіт", "uk"), Some(' '));
        assert_eq!(
            actions,
            vec![
                Action::DeleteChars(7),
                Action::SwitchLayout(LayoutId::new("uk")),
                Action::TypeUnicode("привіт ".into()),
            ]
        );
    }

    #[test]
    fn newline_separator_is_preserved() {
        // Enter як межа: стерти слово+`\n`, вписати слово+`\n`.
        let actions = plan(&decision(true, "ghbdsn", "привіт", "uk"), Some('\n'));
        assert_eq!(actions[0], Action::DeleteChars(7));
        assert_eq!(actions[2], Action::TypeUnicode("привіт\n".into()));
    }

    #[test]
    fn delete_count_is_in_chars_not_bytes() {
        // "привіт" — 6 символів, але 12 байтів у UTF-8 (+1 за роздільник = 7).
        let actions = plan(&decision(true, "привіт", "ghbdsn", "en"), Some(' '));
        assert_eq!(actions[0], Action::DeleteChars(7));
    }

    #[test]
    fn preserves_case_from_best_text() {
        // best_text уже з великої (бо страйк мав SHIFT) — replacer лише пакує;
        // роздільник не псує регістр першої літери.
        let actions = plan(&decision(true, "Ghbdsn", "Привіт", "uk"), Some(' '));
        assert_eq!(actions[2], Action::TypeUnicode("Привіт ".into()));
    }

    #[test]
    fn preserves_apostrophe() {
        let actions = plan(&decision(true, "g'znm", "п'ять", "uk"), Some(' '));
        assert_eq!(actions[2], Action::TypeUnicode("п'ять ".into()));
    }

    #[test]
    fn caps_only_correction_omits_switch_layout() {
        // Перетриманий Shift: `ПРивіт`→`Привіт`, та сама розкладка → БЕЗ
        // SwitchLayout. Стираємо слово+роздільник і вписуємо виправлений регістр.
        let mut d = decision(true, "ПРивіт", "Привіт", "uk");
        d.caps_only = true;
        let actions = plan(&d, Some(' '));
        assert_eq!(
            actions,
            vec![
                Action::DeleteChars(7),
                Action::TypeUnicode("Привіт ".into()),
            ],
            "caps-корекція не має емітити SwitchLayout"
        );
    }

    #[test]
    fn trailing_punct_suffix_is_deleted_and_restored() {
        // `ghbdsn,` + пробіл: на екрані "ghbdsn, " (8). Гілка-роздільник трактує
        // кому як дослівний суфікс → стерти слово(6)+кому(1)+пробіл(1)=8, набрати
        // "привіт" + "," + " ". Кома НЕ з'їдається як «б».
        let d = decision_with_suffix(true, "ghbdsn", "привіт", "uk", ",");
        let actions = plan(&d, Some(' '));
        assert_eq!(
            actions,
            vec![
                Action::DeleteChars(8),
                Action::SwitchLayout(LayoutId::new("uk")),
                Action::TypeUnicode("привіт, ".into()),
            ]
        );
    }
}
