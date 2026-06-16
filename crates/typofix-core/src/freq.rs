//! Частотний шар: `FrequencyMap` — обгортка над `fst::Map` (слово → count), що
//! дає детектору **ГРАДУЙОВАНИЙ** сигнал поверх бінарного членства [`Dictionary`].
//!
//! Чисто й детерміновано. Байти `.freq.fst` читає `typofix-data`
//! ([`load_freq_map_file`]); тут лише обчислення над уже завантаженою мапою.
//!
//! ## Чому log-ймовірність, а не сирий count (критично!)
//! Корпуси різних мов мають РІЗНИЙ масштаб counts: EN `the`≈22.7M, а UK `що`≈65k
//! (EN-корпус на ~2 порядки більший). Тож порівнювати сирі counts (чи навіть
//! `ln(count)`) між мовами не можна — це систематично підіграє EN. Натомість
//! беремо **log-ймовірність** `lp(w) = ln(count) − ln(total)`, нормалізовану на
//! суму всіх counts мапи: це частка слова в корпусі, зіставна між корпусами.
//! Тоді UK `ну`(lp≈−5.85) ≫ EN `ye`(lp≈−11.1), хоча сирі counts близькі (12k/10k).
//!
//! [`load_freq_map_file`]: ../../typofix_data/fn.load_freq_map_file.html
//! [`Dictionary`]: crate::Dictionary

use fst::{Map as FstMap, Streamer};

/// Незмінна частотна мапа однієї мови (слово lowercase → count), з попередньо
/// порахованим `ln(total)` для нормалізації в log-ймовірність.
///
/// **Семантика `None` ≠ «не слово».** Відсутність запису означає лише «немає
/// частотних даних» (рідкісна флексія довгого хвоста VESUM, якої немає в розмовному
/// корпусі). Такі слова Є валідними членами [`Dictionary`] і мусять зберігати
/// baseline-бонус; частота лише ДОДАЄ зважування зверху для слів, що Є в мапі.
#[derive(Debug, Clone)]
pub struct FrequencyMap {
    map: FstMap<Vec<u8>>,
    /// `ln(сума всіх counts)` — знаменник нормалізації log-ймовірності.
    ln_total: f64,
}

impl FrequencyMap {
    /// Обгорнути вже завантажену `fst::Map` (напр. з [`load_freq_map_file`]).
    ///
    /// Підсумовує всі counts один раз, щоб порахувати `ln_total` для нормалізації.
    ///
    /// [`load_freq_map_file`]: ../../typofix_data/fn.load_freq_map_file.html
    pub fn from_fst_map(map: FstMap<Vec<u8>>) -> Self {
        let mut total: u64 = 0;
        let mut stream = map.stream();
        while let Some((_, v)) = stream.next() {
            total = total.saturating_add(v);
        }
        let ln_total = if total == 0 { 0.0 } else { (total as f64).ln() };
        Self { map, ln_total }
    }

    /// Зібрати з готових байтів `fst::Map` (для тестів/раундтрипу).
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, fst::Error> {
        Ok(Self::from_fst_map(FstMap::new(bytes)?))
    }

    /// Сирий count слова (регістронезалежно), або `None`, якщо запису немає.
    pub fn count(&self, word: &str) -> Option<u64> {
        self.map.get(word.to_lowercase())
    }

    /// **Log-ймовірність** слова: `ln(count) − ln(total)` (нормалізована частка в
    /// корпусі, зіставна між мовами), або `None`, якщо запису немає.
    ///
    /// Значення завжди ≤ 0 (count ≤ total). Часте слово → ближче до 0; рідкісне →
    /// сильно від'ємне. Порожня мапа → завжди `None`.
    pub fn log_prob(&self, word: &str) -> Option<f64> {
        self.count(word)
            .filter(|&c| c > 0)
            .map(|c| (c as f64).ln() - self.ln_total)
    }

    /// Серіалізовані байти (для запису/раундтрипу).
    pub fn as_bytes(&self) -> &[u8] {
        self.map.as_fst().as_bytes()
    }

    /// Кількість записів у мапі.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Чи мапа порожня.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Зібрати мапу зі списку пар (слово, count) — мінімальна репліка
    /// `typofix_data::build_freq_map` для герметичних тестів core.
    fn freq_map(entries: &[(&str, u64)]) -> FrequencyMap {
        use std::collections::BTreeMap;
        let sorted: BTreeMap<String, u64> = entries
            .iter()
            .map(|(w, c)| (w.to_lowercase(), *c))
            .collect();
        let map = FstMap::from_iter(sorted).unwrap();
        FrequencyMap::from_fst_map(map)
    }

    #[test]
    fn count_is_case_insensitive_and_absent_is_none() {
        let m = freq_map(&[("the", 1000), ("ye", 5)]);
        assert_eq!(m.count("the"), Some(1000));
        assert_eq!(m.count("THE"), Some(1000));
        assert_eq!(m.count("ye"), Some(5));
        assert_eq!(m.count("lox"), None); // відсутнє ≠ нульова частота
    }

    #[test]
    fn log_prob_ranks_frequent_above_rare() {
        let m = freq_map(&[("frequent", 9000), ("rare", 1000)]);
        // total = 10000; lp(frequent) = ln(0.9), lp(rare) = ln(0.1).
        let lf = m.log_prob("frequent").unwrap();
        let lr = m.log_prob("rare").unwrap();
        assert!(lf > lr);
        assert!((lf - (0.9f64).ln()).abs() < 1e-9);
        assert!((lr - (0.1f64).ln()).abs() < 1e-9);
        assert_eq!(m.log_prob("absent"), None);
    }

    #[test]
    fn cross_corpus_normalization_favors_relatively_common() {
        // Імітуємо реальний масштаб: EN-корпус на 2 порядки більший за UK.
        // Сирий count «ye»(10k) ≈ «ну»(12k), АЛЕ нормалізована частка «ну» у
        // маленькому корпусі ≫ частки «ye» у великому. Саме це й розв'язує ну→ye.
        let uk = freq_map(&[("ну", 12_729), ("filler", 4_400_000)]);
        let en = freq_map(&[("ye", 10_344), ("filler", 688_000_000)]);
        let lp_nu = uk.log_prob("ну").unwrap();
        let lp_ye = en.log_prob("ye").unwrap();
        assert!(
            lp_nu > lp_ye + 3.0,
            "ну ({lp_nu}) має бути помітно ймовірнішим за ye ({lp_ye})"
        );
    }

    #[test]
    fn empty_map_yields_none() {
        let m = freq_map(&[]);
        assert!(m.is_empty());
        assert_eq!(m.log_prob("anything"), None);
        assert_eq!(m.count("anything"), None);
    }

    #[test]
    fn bytes_roundtrip_preserves_log_prob() {
        let m = freq_map(&[("a", 100), ("b", 300)]);
        let restored = FrequencyMap::from_bytes(m.as_bytes().to_vec()).unwrap();
        assert_eq!(restored.len(), m.len());
        assert_eq!(restored.log_prob("a"), m.log_prob("a"));
        assert_eq!(restored.log_prob("b"), m.log_prob("b"));
    }
}
