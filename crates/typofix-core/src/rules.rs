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
    /// **Курований whitelist коротких СЛУЖБОВИХ слів**, per-language
    /// (`(LayoutId, lowercase-слово)`). Це НЕ veto/force, а МЕМБЕРШИП-сигнал
    /// «цей короткий кандидат — справжнє службове слово мови» для дзеркальної
    /// релаксації порога в детекторі (`detector::decide`): на відміну від
    /// довільного збігу в повному словнику (де `ат`/`ді` — шум від корпусу),
    /// whitelist розрізняє `от`/`ти`/`чи` (службові → можна перемкнути на
    /// одиночний dict-hit) від код-токенів `fn`→`ат`. Дані приходять ззовні
    /// (`typofix-data` читає `data/dicts/{lang}.short.txt`) — core нічого не
    /// вантажить. Порожній за замовчуванням → дзеркальна релаксація вимкнена
    /// (стара поведінка, нуль нового перемикання).
    short_service: Vec<(LayoutId, String)>,
}

use crate::LayoutId;

impl WordRules {
    /// Порожній набір (const → придатний для `static`).
    pub const fn new() -> Self {
        Self {
            veto: Vec::new(),
            force: Vec::new(),
            short_service: Vec::new(),
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

    /// Додати слово у whitelist коротких службових слів мови `lang`
    /// (регістронезалежно). Дублікати нешкідливі (мембершип-перевірка).
    pub fn allow_short_service(&mut self, lang: &LayoutId, word: &str) -> &mut Self {
        self.short_service.push((lang.clone(), word.to_lowercase()));
        self
    }

    /// Чи `word` — куроване коротке СЛУЖБОВЕ слово мови `lang` (регістронезалежно).
    /// Використовується дзеркальною релаксацією порога коротких слів у детекторі.
    pub fn is_short_service(&self, lang: &LayoutId, word: &str) -> bool {
        if self.short_service.is_empty() {
            return false;
        }
        let w = word.to_lowercase();
        self.short_service
            .iter()
            .any(|(l, sw)| l == lang && sw == &w)
    }

    /// Чи набір порожній.
    pub fn is_empty(&self) -> bool {
        self.veto.is_empty() && self.force.is_empty() && self.short_service.is_empty()
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

    #[test]
    fn short_service_is_language_scoped_and_case_insensitive() {
        let uk = LayoutId::new("uk");
        let en = LayoutId::new("en");
        let mut r = WordRules::new();
        assert!(!r.is_short_service(&uk, "от")); // порожній → вимкнено
        r.allow_short_service(&uk, "От");
        r.allow_short_service(&en, "we");
        assert!(r.is_short_service(&uk, "ОТ")); // регістронезалежно
        assert!(r.is_short_service(&en, "we"));
        assert!(!r.is_short_service(&en, "от")); // не та мова
        assert!(!r.is_short_service(&uk, "ат")); // не в whitelist
        assert!(!r.is_empty());
    }
}
