//! Символьна **n-gram** мовна модель: `score(word) -> logprob`.
//!
//! Чисто й детерміновано — жодного IO. Модель тренується з тексту
//! ([`NgramModel::train`]) і серіалізується в `typofix-data` (`data/lm/*.bin`).
//! Детектор використовуватиме [`NgramModel::score`], щоб порівнювати, у якій
//! мові послідовність символів виглядає правдоподібніше.
//!
//! ## Згладжування — add-k (адитивне)
//! Для кожного контексту `c₁..cₙ₋₁` оцінка наступного символа `x`:
//!
//! ```text
//! P(x | ctx) = (count(ctx·x) + k) / (count(ctx) + k · V)
//! ```
//!
//! де `V` — розмір словника символів (включно з кінцевим маркером). Це дає
//! ненульову ймовірність невідомим n-грамам, а для **повністю невідомого
//! контексту** (нашого «крякозябра» в чужій мові) формула вироджується в
//! `≈ 1/V` на символ — тобто тексти не тієї мови природно отримують низький
//! бал. Add-k обрано замість backoff за простоту, детермінованість і
//! компактність (немає потреби зберігати ваги backoff). `k` конфігуровний.
//!
//! ## Межі слова
//! Слово доповнюється `(n-1)` стартовими маркерами й одним кінцевим, тож модель
//! вчить і ймовірність початку/кінця слова. Маркери — приватні sentinel-символи
//! (U+0002/U+0003), яких не буває в тексті.
//!
//! ## Нормалізація
//! [`NgramModel::score`] ділить суму лог-ймовірностей на кількість передбачень
//! (≈ довжину слова), щоб слова різної довжини були порівнянні. Більший
//! (менш від'ємний) бал = правдоподібніше.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Початок слова (sentinel у контексті, ніколи не ціль).
const START: char = '\u{2}';
/// Кінець слова (sentinel-ціль: модель вчить ймовірність завершення слова).
const END: char = '\u{3}';

/// Порядок n-grams за замовчуванням (триграми).
pub const DEFAULT_ORDER: usize = 3;
/// Коефіцієнт add-k згладжування за замовчуванням.
pub const DEFAULT_K: f64 = 0.5;

/// Натренована символьна n-gram модель однієї мови.
///
/// Зберігаємо **лічильники** (а не попередньо обчислені лог-ймовірності): так
/// add-k лишається прозорим, модель компактна, а серіалізація через `BTreeMap`
/// — детермінована (відтворювані `.bin`). Лог-ймовірності рахуються в [`score`]
/// «на льоту».
///
/// [`score`]: NgramModel::score
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NgramModel {
    /// Порядок n (кількість символів у n-грамі).
    order: usize,
    /// Коефіцієнт add-k згладжування.
    k: f64,
    /// Розмір символьного словника `V` (унікальні цілі, включно з END).
    vocab_size: usize,
    /// `контекст·символ → лічильник` (рядки довжини `order`).
    ngram_counts: BTreeMap<String, u64>,
    /// `контекст → лічильник` (рядки довжини `order-1`).
    context_counts: BTreeMap<String, u64>,
}

impl NgramModel {
    /// Натренувати модель із сирого тексту.
    ///
    /// Текст розбивається на слова ([`tokenize`]), кожне слово доповнюється
    /// маркерами меж і розкладається на n-грами. `order >= 1`, `k > 0`.
    ///
    /// # Panics
    /// Якщо `order == 0`.
    pub fn train(text: &str, order: usize, k: f64) -> Self {
        assert!(order >= 1, "order має бути >= 1");
        let mut ngram_counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut context_counts: BTreeMap<String, u64> = BTreeMap::new();
        let mut vocab: BTreeSet<char> = BTreeSet::new();

        for word in tokenize(text) {
            let chars = padded(&word, order);
            for i in (order - 1)..chars.len() {
                let ctx: String = chars[i - (order - 1)..i].iter().collect();
                let tgt = chars[i];
                let mut ngram = ctx.clone();
                ngram.push(tgt);
                *ngram_counts.entry(ngram).or_insert(0) += 1;
                *context_counts.entry(ctx).or_insert(0) += 1;
                vocab.insert(tgt);
            }
        }

        Self {
            order,
            k,
            vocab_size: vocab.len(),
            ngram_counts,
            context_counts,
        }
    }

    /// Порядок n моделі.
    pub fn order(&self) -> usize {
        self.order
    }

    /// Розмір символьного словника `V`.
    pub fn vocab_size(&self) -> usize {
        self.vocab_size
    }

    /// Чи модель порожня (не натренована).
    pub fn is_empty(&self) -> bool {
        self.vocab_size == 0
    }

    /// Сумарна лог-ймовірність слова (НЕ нормалізована за довжиною).
    ///
    /// Повертає `f64::NEG_INFINITY` для порожньої моделі.
    pub fn log_prob(&self, word: &str) -> f64 {
        self.sum_and_count(word).0
    }

    /// Лог-ймовірність слова, **нормалізована за довжиною** (середня лог-ймовірність
    /// на символ). Саме це порівнює детектор між мовами/розкладками.
    ///
    /// Більший (менш від'ємний) бал = правдоподібніше. Порожнє слово/модель →
    /// `f64::NEG_INFINITY`.
    pub fn score(&self, word: &str) -> f64 {
        let (sum, n) = self.sum_and_count(word);
        if n == 0 {
            f64::NEG_INFINITY
        } else {
            sum / n as f64
        }
    }

    /// Спільне ядро [`log_prob`]/[`score`]: повертає `(сума_логів, к-сть_передбачень)`.
    ///
    /// [`log_prob`]: NgramModel::log_prob
    /// [`score`]: NgramModel::score
    fn sum_and_count(&self, word: &str) -> (f64, usize) {
        if self.vocab_size == 0 {
            return (f64::NEG_INFINITY, 0);
        }
        let lowered = word.to_lowercase();
        let chars = padded(&lowered, self.order);
        let denom_vocab = self.k * self.vocab_size as f64;
        let mut sum = 0.0;
        let mut count = 0usize;
        for i in (self.order - 1)..chars.len() {
            let ctx: String = chars[i - (self.order - 1)..i].iter().collect();
            let tgt = chars[i];
            let mut ngram = ctx.clone();
            ngram.push(tgt);
            let c_ngram = *self.ngram_counts.get(&ngram).unwrap_or(&0) as f64;
            let c_ctx = *self.context_counts.get(&ctx).unwrap_or(&0) as f64;
            let p = (c_ngram + self.k) / (c_ctx + denom_vocab);
            sum += p.ln();
            count += 1;
        }
        (sum, count)
    }
}

/// Розбити текст на слова для тренування/скорингу.
///
/// Слово — максимальний пробіг літер (Unicode `is_alphabetic`) разом з
/// апострофами (`'` та `’`, бо `п'ять`/`м'який` — одне слово). Усе зводиться до
/// нижнього регістру. Пробіли цифри й пунктуація — роздільники; «слова» без
/// жодної літери відкидаються.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut has_alpha = false;
    for ch in text.chars() {
        if ch.is_alphabetic() {
            cur.extend(ch.to_lowercase());
            has_alpha = true;
        } else if ch == '\'' || ch == '’' {
            cur.push(ch);
        } else {
            if has_alpha {
                words.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            has_alpha = false;
        }
    }
    if has_alpha {
        words.push(cur);
    }
    words
}

/// Доповнити слово маркерами меж: `(order-1)` × START спереду й один END ззаду.
fn padded(word: &str, order: usize) -> Vec<char> {
    let mut v = Vec::with_capacity(order + word.len());
    v.resize(order.saturating_sub(1), START);
    v.extend(word.chars());
    v.push(END);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    // Мінікорпус: достатньо для демонстрації детермінованих властивостей.
    const UK: &str = "привіт як справи привіт світ привіт друже добрий день \
        дякую будь ласка все добре привіт ще раз привіт усім";
    const EN: &str = "hello how are you hello world hello there good morning \
        thank you please all good hello again hello everyone";

    #[test]
    fn ukrainian_word_scores_higher_than_latin_gibberish() {
        let uk = NgramModel::train(UK, DEFAULT_ORDER, DEFAULT_K);
        // Реальне українське слово проти латинських «крякозябрів» у тій самій
        // (українській) моделі.
        let good = uk.score("привіт");
        let gibberish = uk.score("ghbdsn");
        assert!(
            good > gibberish,
            "привіт ({good}) має бути правдоподібніший за ghbdsn ({gibberish})"
        );
    }

    #[test]
    fn english_model_prefers_english() {
        let en = NgramModel::train(EN, DEFAULT_ORDER, DEFAULT_K);
        assert!(en.score("hello") > en.score("привіт"));
    }

    #[test]
    fn cross_model_disambiguates_language() {
        let uk = NgramModel::train(UK, DEFAULT_ORDER, DEFAULT_K);
        let en = NgramModel::train(EN, DEFAULT_ORDER, DEFAULT_K);
        // "привіт" правдоподібніший в uk, "hello" — в en.
        assert!(uk.score("привіт") > en.score("привіт"));
        assert!(en.score("hello") > uk.score("hello"));
    }

    #[test]
    fn score_is_length_normalized_and_deterministic() {
        let uk = NgramModel::train(UK, DEFAULT_ORDER, DEFAULT_K);
        // Детермінізм: той самий вхід → той самий бал.
        assert_eq!(uk.score("привіт"), uk.score("привіт"));
        // Нормалізація: score = log_prob / к-сть передбачень. Для "привіт"
        // (6 літер) передбачень 7 (6 + END).
        let lp = uk.log_prob("привіт");
        let s = uk.score("привіт");
        assert!((s - lp / 7.0).abs() < 1e-12);
    }

    #[test]
    fn empty_model_and_empty_word_are_safe() {
        let empty = NgramModel::train("", DEFAULT_ORDER, DEFAULT_K);
        assert!(empty.is_empty());
        assert_eq!(empty.score("привіт"), f64::NEG_INFINITY);

        let uk = NgramModel::train(UK, DEFAULT_ORDER, DEFAULT_K);
        // Порожнє слово все одно дає одне передбачення (END), не падає.
        assert!(uk.score("").is_finite());
    }

    #[test]
    fn tokenize_keeps_apostrophes_and_drops_punctuation() {
        let w = tokenize("П'ять, шість! м’який");
        assert_eq!(w, vec!["п'ять", "шість", "м’який"]);
    }

    #[test]
    fn configurable_order() {
        let uk = NgramModel::train(UK, 2, DEFAULT_K);
        assert_eq!(uk.order(), 2);
        assert!(uk.score("привіт") > uk.score("zzzzzz"));
    }
}
