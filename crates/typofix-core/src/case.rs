//! Перетворення РЕГІСТРУ виділеного тексту (гаряча клавіша B1).
//!
//! **Чисте й детерміноване.** Текст приходить ЗЗОВНІ (виділення з ОС через
//! платформний шар), а НЕ з буфера натискань — це окрема ручна команда, не
//! розкладко-перемикання. Юнікод-коректно (укр. літери теж).

/// Режим перетворення регістру.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// УСЕ ВЕЛИКИМИ.
    Upper,
    /// усе малими.
    Lower,
    /// Перша літера велика, решта малі (Як речення).
    Sentence,
}

/// Перетворити регістр `text` за обраним режимом. Юнікод-коректно (працює і для
/// української, і для англійської).
///
/// - [`CaseMode::Upper`] — усе у верхній регістр;
/// - [`CaseMode::Lower`] — усе в нижній;
/// - [`CaseMode::Sentence`] — ПЕРША літера у верхній, решта в нижній. Велику
///   отримує перший АЛФАВІТНИЙ символ (провідні пробіли/лапки лишаються як є).
pub fn transform_case(text: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Upper => text.to_uppercase(),
        CaseMode::Lower => text.to_lowercase(),
        CaseMode::Sentence => {
            let mut out = String::with_capacity(text.len());
            let mut capitalized = false;
            for ch in text.chars() {
                if !capitalized && ch.is_alphabetic() {
                    out.extend(ch.to_uppercase());
                    capitalized = true;
                } else {
                    out.extend(ch.to_lowercase());
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_uppercases_uk_and_en() {
        assert_eq!(
            transform_case("Привіт world", CaseMode::Upper),
            "ПРИВІТ WORLD"
        );
        assert_eq!(transform_case("вже ВЕЛИКІ", CaseMode::Upper), "ВЖЕ ВЕЛИКІ");
    }

    #[test]
    fn lower_lowercases_uk_and_en() {
        assert_eq!(
            transform_case("ПРИВІТ World", CaseMode::Lower),
            "привіт world"
        );
        assert_eq!(transform_case("HeLLo Світ", CaseMode::Lower), "hello світ");
    }

    #[test]
    fn sentence_capitalizes_first_letter_only() {
        // Укр.: перша велика, решта малі (попри мішаний вхід).
        assert_eq!(
            transform_case("привІТ всім", CaseMode::Sentence),
            "Привіт всім"
        );
        // Англ.: те саме.
        assert_eq!(
            transform_case("hELLo WORLD", CaseMode::Sentence),
            "Hello world"
        );
    }

    #[test]
    fn sentence_skips_leading_non_letters() {
        // Провідні пробіли/лапки не «з'їдають» капіталізацію — велику дістає
        // перший алфавітний символ.
        assert_eq!(transform_case("  привіт", CaseMode::Sentence), "  Привіт");
        assert_eq!(transform_case("«слово»", CaseMode::Sentence), "«Слово»");
    }

    #[test]
    fn empty_and_non_letters_are_stable() {
        assert_eq!(transform_case("", CaseMode::Sentence), "");
        assert_eq!(transform_case("123 !?", CaseMode::Sentence), "123 !?");
        assert_eq!(transform_case("123", CaseMode::Upper), "123");
    }
}
