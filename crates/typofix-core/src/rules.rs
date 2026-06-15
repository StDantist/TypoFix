//! Правила рівня слова: veto (ніколи не перемикати) і force (перемикати завжди).
//!
//! Чисто й детерміновано — лише дані + матчинг, передаються в
//! [`Context`](crate::Context) позиченими. Підключається у veto-хук
//! [`detector`](crate::detector). **Принцип лишається:** за сумніву не
//! перемикати; veto захищає precision (нікнейми, терміни, код), force —
//! рідкісний навмисний перемикач нижче порогу.
//!
//! Зараз матчинг — **точний збіг слова** (регістронезалежно). Багатші патерни
//! (регекси/гліоби) — окремий follow-up, щоб тут лишалося просто й без залежностей.

/// Набір правил рівня слова.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordRules {
    /// Слова, які НЕ перемикати (звірка з поточним АБО виправленим текстом).
    veto: Vec<String>,
    /// Слова (поточний текст), які перемикати завжди — в обхід порогу/довжини.
    force: Vec<String>,
}

impl WordRules {
    /// Порожній набір (const → придатний для `static`).
    pub const fn new() -> Self {
        Self {
            veto: Vec::new(),
            force: Vec::new(),
        }
    }

    /// Додати слово у veto-список (регістронезалежно).
    pub fn veto_word(&mut self, word: &str) -> &mut Self {
        self.veto.push(word.to_lowercase());
        self
    }

    /// Додати слово у force-список (регістронезалежно).
    pub fn force_word(&mut self, word: &str) -> &mut Self {
        self.force.push(word.to_lowercase());
        self
    }

    /// Чи слово під забороною перемикання. Збіг або з тим, що на екрані
    /// (`current_text`), або з кандидатом-виправленням (`best_text`) — щоб
    /// захистити обидва боки (і навмисний ввід, і небажане виправлення).
    pub fn vetoes(&self, current_text: &str, best_text: &str) -> bool {
        if self.veto.is_empty() {
            return false;
        }
        let cur = current_text.to_lowercase();
        let best = best_text.to_lowercase();
        self.veto.iter().any(|w| w == &cur || w == &best)
    }

    /// Чи поточний текст у force-списку (перемикати в обхід порогу).
    pub fn forces(&self, current_text: &str) -> bool {
        if self.force.is_empty() {
            return false;
        }
        let cur = current_text.to_lowercase();
        self.force.iter().any(|w| w == &cur)
    }

    /// Чи набір порожній.
    pub fn is_empty(&self) -> bool {
        self.veto.is_empty() && self.force.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_rules_veto_and_force_nothing() {
        let r = WordRules::new();
        assert!(!r.vetoes("ghbdsn", "привіт"));
        assert!(!r.forces("ghbdsn"));
    }

    #[test]
    fn veto_matches_either_side_case_insensitive() {
        let mut r = WordRules::new();
        r.veto_word("Привіт");
        assert!(r.vetoes("ghbdsn", "привіт")); // збіг із best_text
        assert!(r.vetoes("ПРИВІТ", "щось")); // збіг із current_text
        assert!(!r.vetoes("ghbdsn", "світ"));
    }

    #[test]
    fn force_matches_current_text() {
        let mut r = WordRules::new();
        r.force_word("ghbdsn");
        assert!(r.forces("GHBDSN"));
        assert!(!r.forces("world"));
    }
}
