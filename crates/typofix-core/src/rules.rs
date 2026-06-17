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
    /// **Особистий словник користувача** (`data/dicts/user.txt`) — слова, які
    /// користувач хоче, щоб апка ВИЗНАВАЛА як валідні й ПЕРЕМИКАЛА на них (жаргон/
    /// нікнейми поза стандартним словником, напр. `вжух`). Це ПОЗИТИВНИЙ сигнал
    /// (НЕ veto): такі слова дають dict-бонус, як звичайний член словника. Мова не
    /// тегується (двійник у чужій розкладці — біліберда, тож не сплутаєш).
    recognized: Vec<String>,
    /// **Активні ISO 4217 alphabetic-коди валют** (UPPERCASE), для розпізнавання
    /// валютних пар ([`is_currency_pair`]). Дані з `data/dicts/iso4217.txt`
    /// (loader `typofix-data`). Порожній → forex-сигнал вимкнено.
    ///
    /// [`is_currency_pair`]: WordRules::is_currency_pair
    currency_codes: Vec<String>,
    /// **Відомі файлові розширення** (lowercase, БЕЗ крапки), для позитивного
    /// сигналу «це латиниця» ([`is_known_extension`]). Дані з
    /// `data/dicts/extensions.txt` (loader `typofix-data`). Порожній → сигнал
    /// розширень вимкнено.
    ///
    /// [`is_known_extension`]: WordRules::is_known_extension
    extensions: Vec<String>,
}

use crate::LayoutId;

impl WordRules {
    /// Порожній набір (const → придатний для `static`).
    pub const fn new() -> Self {
        Self {
            veto: Vec::new(),
            force: Vec::new(),
            short_service: Vec::new(),
            recognized: Vec::new(),
            currency_codes: Vec::new(),
            extensions: Vec::new(),
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

    /// Додати слово в особистий словник «визнаних» (регістронезалежно). Таке
    /// слово дає dict-бонус, як звичайний член словника → апка перемикає на нього.
    pub fn recognize_word(&mut self, word: &str) -> &mut Self {
        self.recognized.push(word.to_lowercase());
        self
    }

    /// Чи `word` — у особистому словнику визнаних (регістронезалежно). Споживає
    /// детектор як ДОДАТКОВУ dict-членність (поряд із `LanguageProfile.dict`).
    pub fn recognizes(&self, word: &str) -> bool {
        if self.recognized.is_empty() {
            return false;
        }
        let w = word.to_lowercase();
        self.recognized.iter().any(|x| x == &w)
    }

    /// Додати alphabetic-код валюти ISO 4217 (нормалізується в UPPERCASE).
    pub fn add_currency_code(&mut self, code: &str) -> &mut Self {
        self.currency_codes.push(code.to_ascii_uppercase());
        self
    }

    /// Чи `token` — **валютна пара**: рівно 6 ASCII-літер, де ОБИДВІ половини
    /// (по 3) — валідні ISO 4217 коди (регістронезалежно). `EURUSD`/`gbpusd` →
    /// `true`; випадковий 6-літерний не-пара (`ABCDEF`) → `false`. Дзеркалить
    /// `typofix_data::is_currency_pair`, але без HashSet (core лишається чистим і
    /// без зайвих залежностей; перелік малий — лінійна перевірка дешева).
    pub fn is_currency_pair(&self, token: &str) -> bool {
        if self.currency_codes.is_empty() || token.len() != 6 || !token.is_ascii() {
            return false;
        }
        if !token.bytes().all(|b| b.is_ascii_alphabetic()) {
            return false;
        }
        let upper = token.to_ascii_uppercase();
        let (a, b) = upper.split_at(3);
        self.has_currency_code(a) && self.has_currency_code(b)
    }

    fn has_currency_code(&self, code: &str) -> bool {
        self.currency_codes.iter().any(|c| c == code)
    }

    /// Додати відоме файлове розширення (нормалізується: lowercase, без провідної крапки).
    pub fn add_extension(&mut self, ext: &str) -> &mut Self {
        self.extensions
            .push(ext.trim_start_matches('.').to_lowercase());
        self
    }

    /// Чи `token` — **відоме файлове розширення** (lowercase membership; провідну
    /// крапку, якщо є, ігноруємо). Дзеркалить `typofix_data::is_known_extension`,
    /// але без HashSet (core лишається чистим; перелік малий — лінійна перевірка).
    pub fn is_known_extension(&self, token: &str) -> bool {
        if self.extensions.is_empty() {
            return false;
        }
        let t = token.trim_start_matches('.').to_lowercase();
        !t.is_empty() && self.extensions.iter().any(|e| e == &t)
    }

    /// Чи набір порожній.
    pub fn is_empty(&self) -> bool {
        self.veto.is_empty()
            && self.force.is_empty()
            && self.short_service.is_empty()
            && self.recognized.is_empty()
            && self.currency_codes.is_empty()
            && self.extensions.is_empty()
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
    fn recognized_words_are_positive_and_case_insensitive() {
        let mut r = WordRules::new();
        assert!(!r.recognizes("вжух")); // порожній → нічого
        r.recognize_word("Вжух");
        assert!(r.recognizes("вжух")); // регістронезалежно
        assert!(r.recognizes("ВЖУХ"));
        assert!(!r.recognizes("світ"));
        assert!(!r.is_empty());
    }

    #[test]
    fn currency_pair_requires_both_halves_iso() {
        let mut r = WordRules::new();
        assert!(!r.is_currency_pair("EURUSD")); // порожній перелік → false
        for c in ["EUR", "USD", "GBP", "JPY"] {
            r.add_currency_code(c);
        }
        assert!(r.is_currency_pair("EURUSD"));
        assert!(r.is_currency_pair("gbpjpy")); // регістронезалежно
        assert!(r.is_currency_pair("USDGBP"));
        // Обидві половини мусять бути ISO:
        assert!(!r.is_currency_pair("EURXXX")); // XXX не в переліку
        assert!(!r.is_currency_pair("ABCDEF")); // випадковий 6-літерний не-пара
                                                // Не 6 ASCII-літер:
        assert!(!r.is_currency_pair("EUR")); // 3
        assert!(!r.is_currency_pair("EURUSDX")); // 7
        assert!(!r.is_currency_pair("EUR123")); // цифри
        assert!(!r.is_currency_pair("ЕУРУСД")); // кирилиця (не ASCII)
    }

    #[test]
    fn known_extension_membership_and_dot_stripping() {
        let mut r = WordRules::new();
        assert!(!r.is_known_extension("txt")); // порожній → false
        r.add_extension("txt");
        r.add_extension(".MD"); // провідна крапка ігнор., lowercase
        assert!(r.is_known_extension("txt"));
        assert!(r.is_known_extension("TXT")); // регістронезалежно
        assert!(r.is_known_extension(".txt")); // крапку ігнор.
        assert!(r.is_known_extension("md"));
        assert!(!r.is_known_extension("exe")); // не в переліку
        assert!(!r.is_known_extension("")); // порожній токен
        assert!(!r.is_empty());
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
