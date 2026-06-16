//! Швидка перевірка наявності слова через FST (`fst::Set`) — буст упевненості
//! детектора (слово зі словника майже напевно валідне в цій мові).
//!
//! Чисто й детерміновано. Побудова FST у пам'яті ([`Dictionary::from_words`]) —
//! це обчислення, не IO; читання/запис байтів `.fst` робить `typofix-data`.
//! `fst` усередині використовує `unsafe`, але це сторонній крейт — наше правило
//! `#![forbid(unsafe_code)]` стосується лише нашого коду.

use std::collections::BTreeSet;

use fst::{Set, Streamer};

/// Звести всі апострофоподібні символи до канонічного `canon`.
///
/// **Готча — апостроф у двох виглядах.** Український апостроф приходить у двох
/// кодпойнтах: ASCII `'` (U+0027, яким записаний морфословник VESUM,
/// `data/dicts/uk.full.txt`) і типографський `’` (U+2019, що його генерує
/// розкладка з `uk.toml` для тієї ж клавіші). Для dict-lookup їх ТРЕБА вважати
/// одним символом — інакше `сім'я` (зі словника) і `сім’я` (з розкладки) стають
/// різними ключами, і апострофні слова (тисячі у VESUM) промахуються повз
/// словник → не ловляться. Апостроф — не літера, тож регістр його не зачіпає.
fn normalize_apostrophes(s: &str, canon: char) -> String {
    s.chars()
        .map(|c| match c {
            // ASCII ', right single quote, modifier letter apostrophe, left single quote.
            '\u{0027}' | '\u{2019}' | '\u{02BC}' | '\u{2018}' => canon,
            other => other,
        })
        .collect()
}

/// Незмінна множина слів однієї мови, побудована на FST.
///
/// Слова зберігаються у нижньому регістрі; [`contains`] теж зводить запит до
/// нижнього регістру, тож перевірка регістронезалежна.
///
/// [`contains`]: Dictionary::contains
#[derive(Debug, Clone)]
pub struct Dictionary {
    set: Set<Vec<u8>>,
}

impl Dictionary {
    /// Зібрати словник зі списку слів (у будь-якому порядку, з дублікатами).
    ///
    /// Слова зводяться до нижнього регістру, сортуються й дедуплікуються
    /// (вимога FST — впорядкований унікальний вхід; `BTreeSet<String>` дає байтовий
    /// порядок, що збігається з порядком кодпойнтів у UTF-8).
    pub fn from_words<I, S>(words: I) -> Result<Self, fst::Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        // Зводимо до нижнього регістру І канонізуємо апостроф до ASCII U+0027,
        // щоб новозбудовані словники були апостроф-консистентні (див.
        // [`normalize_apostrophes`]). Запит у `contains` канонізується дзеркально.
        let sorted: BTreeSet<String> = words
            .into_iter()
            .map(|w| normalize_apostrophes(&w.as_ref().to_lowercase(), '\u{0027}'))
            .filter(|w| !w.is_empty())
            .collect();
        let set = Set::from_iter(sorted)?;
        Ok(Self { set })
    }

    /// Завантажити словник із готових байтів FST (напр. з диска через `typofix-data`).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, fst::Error> {
        Ok(Self {
            set: Set::new(bytes)?,
        })
    }

    /// Серіалізовані байти FST (для запису `.fst` у `typofix-data`).
    pub fn as_bytes(&self) -> &[u8] {
        self.set.as_fst().as_bytes()
    }

    /// Кількість слів у словнику.
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Чи словник порожній.
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Чи є слово у словнику (регістронезалежно, апостроф-нормалізовано).
    ///
    /// Готовий `.fst` із диска може зберігати апостроф у будь-якому вигляді
    /// (VESUM-частина — ASCII `'` U+0027, корпусна — типографський `’` U+2019),
    /// а сам запит приходить із розкладки як U+2019. Тож пробуємо обидва канони:
    /// слово в обидвох виглядах апострофа — той самий ключ (див.
    /// [`normalize_apostrophes`]). Без апострофа — рівно один lookup.
    pub fn contains(&self, word: &str) -> bool {
        let lower = word.to_lowercase();
        if self.set.contains(&lower) {
            return true;
        }
        let ascii = normalize_apostrophes(&lower, '\u{0027}');
        if ascii != lower && self.set.contains(&ascii) {
            return true;
        }
        let typo = normalize_apostrophes(&lower, '\u{2019}');
        typo != lower && self.set.contains(&typo)
    }

    /// Усі слова словника у відсортованому порядку (для тестів/дебагу).
    pub fn words(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.set.len());
        let mut stream = self.set.stream();
        while let Some(bytes) = stream.next() {
            if let Ok(s) = std::str::from_utf8(bytes) {
                out.push(s.to_string());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict() -> Dictionary {
        Dictionary::from_words(["привіт", "світ", "друже", "Привіт", "hello", "world"])
            .expect("словник має побудуватися")
    }

    #[test]
    fn contains_present_and_absent() {
        let d = dict();
        assert!(d.contains("привіт"));
        assert!(d.contains("світ"));
        assert!(d.contains("hello"));
        assert!(!d.contains("ghbdsn"));
        assert!(!d.contains("qwerty"));
    }

    #[test]
    fn apostrophe_variants_match_either_storage() {
        // Словник зі словом, записаним ASCII-апострофом U+0027 (як у VESUM):
        // запит типографським U+2019 (як генерує розкладка) має знаходити.
        let ascii = Dictionary::from_words(["сім'я"]).unwrap();
        assert!(ascii.contains("сім'я"), "U+0027 запит");
        assert!(
            ascii.contains("сім’я"),
            "U+2019 запит має матчити U+0027 запис"
        );

        // Слово, ПОДАНЕ при побудові з U+2019: `from_words` канонізує його до
        // U+0027, тож обидва варіанти запиту все одно знаходять.
        let typo_built = Dictionary::from_words(["комп’ютер"]).unwrap();
        assert!(typo_built.contains("комп'ютер"), "U+0027 запит");
        assert!(typo_built.contains("комп’ютер"), "U+2019 запит");

        // Модифікаторний апостроф U+02BC теж канонізується.
        assert!(ascii.contains("сім\u{02BC}я"), "U+02BC запит");

        // Без апострофа поведінка незмінна.
        assert!(!ascii.contains("сімя"));
    }

    #[test]
    fn case_insensitive_and_deduplicated() {
        let d = dict();
        // "Привіт" і "привіт" — той самий запис.
        assert!(d.contains("ПРИВІТ"));
        // 6 входів, але "Привіт"/"привіт" злилися → 5 унікальних.
        assert_eq!(d.len(), 5);
    }

    #[test]
    fn bytes_roundtrip() {
        let d = dict();
        let bytes = d.as_bytes().to_vec();
        let restored = Dictionary::from_bytes(bytes).unwrap();
        assert!(restored.contains("привіт"));
        assert_eq!(restored.len(), d.len());
        assert_eq!(restored.words(), d.words());
    }

    #[test]
    fn empty_dictionary_is_safe() {
        let d = Dictionary::from_words(Vec::<String>::new()).unwrap();
        assert!(d.is_empty());
        assert!(!d.contains("будь-що"));
    }
}
