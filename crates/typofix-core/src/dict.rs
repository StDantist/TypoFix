//! Швидка перевірка наявності слова через FST (`fst::Set`) — буст упевненості
//! детектора (слово зі словника майже напевно валідне в цій мові).
//!
//! Чисто й детерміновано. Побудова FST у пам'яті ([`Dictionary::from_words`]) —
//! це обчислення, не IO; читання/запис байтів `.fst` робить `typofix-data`.
//! `fst` усередині використовує `unsafe`, але це сторонній крейт — наше правило
//! `#![forbid(unsafe_code)]` стосується лише нашого коду.

use std::collections::BTreeSet;

use fst::{Set, Streamer};

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
        let sorted: BTreeSet<String> = words
            .into_iter()
            .map(|w| w.as_ref().to_lowercase())
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

    /// Чи є слово у словнику (регістронезалежно).
    pub fn contains(&self, word: &str) -> bool {
        self.set.contains(word.to_lowercase())
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
