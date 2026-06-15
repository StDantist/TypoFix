//! Рішення про мову/розкладку слова: поєднує `layout_mapper` (інтерпретація
//! страйків), `lm` (правдоподібність) і `dict` (буст упевненості). §3.3.
//!
//! **Чисто й детерміновано.** Жодного завантаження даних: усі ресурси мов
//! ([`LanguageProfile`]) передаються ззовні через [`Context`] як позичені дані.
//!
//! ## Алгоритм
//! Для кожної ввімкненої мови інтерпретуємо ту саму послідовність фізичних
//! страйків у її розкладці й оцінюємо отриманий рядок:
//!
//! ```text
//! score(lang) = w1 · lm.score(text) + (dict.contains(text) ? bonus : 0)
//! ```
//!
//! Обираємо найкращу. Перемикаємо, лише якщо вона ≠ поточної, слово не надто
//! коротке, перевага над поточною інтерпретацією перевищує `threshold(len)` і
//! немає вето правил. **Принцип: за сумніву НЕ перемикати** (precision > recall).

use crate::{Context, Dictionary, KeyStroke, Layout, LayoutId, NgramModel};

/// Ресурси однієї мови, потрібні детектору. Власник — оркестратор/тест; у
/// `Context` потрапляє позиченим зрізом (core нічого не вантажить сам).
#[derive(Debug, Clone)]
pub struct LanguageProfile {
    /// Ідентифікатор мови/розкладки (`"uk"`, `"en"`).
    pub id: LayoutId,
    /// Розкладка для інтерпретації страйків.
    pub layout: Layout,
    /// Мовна n-gram модель.
    pub lm: NgramModel,
    /// Словник для бусту впевненості.
    pub dict: Dictionary,
}

impl LanguageProfile {
    /// Бал кандидата для заданого тексту (вже інтерпретованого в його розкладці).
    fn score(&self, text: &str, cfg: &DetectorConfig) -> f64 {
        if text.is_empty() {
            return f64::NEG_INFINITY;
        }
        let lm = self.lm.score(text);
        let bonus = if self.dict.contains(text) {
            cfg.dict_bonus
        } else {
            0.0
        };
        cfg.lm_weight * lm + bonus
    }
}

/// Налаштування детектора (ваги й крива порогу). Калібруватиметься на eval-датасеті
/// (Фаза 2/наступна задача); тут — розумні дефолти для доведення логіки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Вага лог-ймовірності LM (`w1`).
    pub lm_weight: f64,
    /// Бонус за наявність слова у словнику (`w2`-еквівалент).
    pub dict_bonus: f64,
    /// Базовий поріг переваги (для довгих слів).
    pub base_threshold: f64,
    /// Додаток до порогу, обернено пропорційний довжині (карає короткі слова).
    pub short_word_extra: f64,
    /// Мінімальна довжина слова, яке взагалі можна перемикати (коротші —
    /// неоднозначні в обох мовах, не чіпаємо).
    pub min_switch_len: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            lm_weight: 1.0,
            dict_bonus: 4.0,
            base_threshold: 1.0,
            short_word_extra: 6.0,
            min_switch_len: 2,
        }
    }
}

impl DetectorConfig {
    /// Поріг переваги залежно від довжини слова (символів).
    ///
    /// Короткі слова потребують значно більшої переваги (бо `по`/`gj`, `не`/`yt`
    /// валідні в обох мовах); довгі — меншої. Слова коротші за `min_switch_len`
    /// не перемикаються ніколи (поріг `+∞`).
    pub fn threshold(&self, len: usize) -> f64 {
        if len < self.min_switch_len {
            return f64::INFINITY;
        }
        self.base_threshold + self.short_word_extra / (len as f64)
    }
}

/// Результат розгляду слова детектором.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Найкраща мова за балом.
    pub best: LayoutId,
    /// Текст слова в найкращій розкладці (готовий для перенабору, з регістром).
    pub best_text: String,
    /// Текст слова в поточній розкладці (те, що зараз на екрані).
    pub current_text: String,
    /// Чи варто перемикати+перенабирати.
    pub switch: bool,
    /// Перевага найкращої над поточною (best.score − current.score); для дебагу/тестів.
    pub confidence: f64,
}

/// Розглянути буферизоване слово й вирішити, чи перемикати.
///
/// `strokes` — фізичні натискання слова (layout-незалежні). Якщо поточної
/// розкладки немає серед `ctx.languages`, безпечно не перемикаємо (не знаємо,
/// що саме на екрані → не можна коректно стерти).
pub fn decide(strokes: &[KeyStroke], ctx: &Context) -> Decision {
    let cfg = &ctx.config;

    let current = ctx.current_profile();
    let current_text = current
        .map(|p| p.layout.interpret(strokes))
        .unwrap_or_default();
    let current_score = current
        .map(|p| p.score(&current_text, cfg))
        .unwrap_or(f64::NEG_INFINITY);

    // Початково найкраща — поточна (щоб за відсутності переваги нічого не міняти).
    let mut best = ctx.current_layout.clone();
    let mut best_text = current_text.clone();
    let mut best_score = current_score;

    for p in ctx.languages {
        let text = p.layout.interpret(strokes);
        let sc = p.score(&text, cfg);
        if sc > best_score {
            best_score = sc;
            best = p.id.clone();
            best_text = text;
        }
    }

    let len = current_text.chars().count();
    let confidence = best_score - current_score;

    // Правила рівня слова: veto (захист precision) має пріоритет; force дозволяє
    // перемкнути в обхід порогу/довжини (але не в обхід veto чи best≠current).
    let vetoed = ctx.rules.vetoes(&current_text, &best_text);
    let forced = ctx.rules.forces(&current_text);

    let switch = current.is_some()
        && best != ctx.current_layout
        && !vetoed
        && (forced || (len >= cfg.min_switch_len && confidence > cfg.threshold(len)));

    Decision {
        best,
        best_text,
        current_text,
        switch,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyCap, Modifiers};

    // Маленькі ручні профілі (без IO): достатньо клавіш для тестових слів.
    // Фізичні позиції (set 1): G=0x22 H=0x23 B=0x30 D=0x20 S=0x1F N=0x31,
    // плюс кілька для коротких слів: O=0x18, T=0x14.
    fn en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x22, KeyCap::letter('g', 'G')),
                (0x23, KeyCap::letter('h', 'H')),
                (0x30, KeyCap::letter('b', 'B')),
                (0x20, KeyCap::letter('d', 'D')),
                (0x1F, KeyCap::letter('s', 'S')),
                (0x31, KeyCap::letter('n', 'N')),
                (0x18, KeyCap::letter('o', 'O')),
                (0x14, KeyCap::letter('t', 'T')),
            ],
        );
        let lm = NgramModel::train("hello world good night not to be on go", 3, 0.5);
        let dict =
            Dictionary::from_words(["hello", "world", "good", "night", "not", "to", "on", "go"])
                .unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
        }
    }

    fn uk_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x22, KeyCap::letter('п', 'П')),
                (0x23, KeyCap::letter('р', 'Р')),
                (0x30, KeyCap::letter('и', 'И')),
                (0x20, KeyCap::letter('в', 'В')),
                (0x1F, KeyCap::letter('і', 'І')),
                (0x31, KeyCap::letter('т', 'Т')),
                (0x18, KeyCap::letter('щ', 'Щ')),
                (0x14, KeyCap::letter('е', 'Е')),
            ],
        );
        let lm = NgramModel::train(
            "привіт світ як справи добрий день привіт друже все добре привіт",
            3,
            0.5,
        );
        let dict =
            Dictionary::from_words(["привіт", "світ", "друже", "добре", "день", "п"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
        }
    }

    fn strokes(scancodes: &[u32]) -> Vec<KeyStroke> {
        scancodes
            .iter()
            .map(|&sc| KeyStroke::new(sc, Modifiers::empty()))
            .collect()
    }

    use crate::{ExclusionRules, WordRules};

    static NO_EXCL: ExclusionRules = ExclusionRules::new();
    static NO_RULES: WordRules = WordRules::new();

    fn ctx_with<'a>(langs: &'a [LanguageProfile], current: &str) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: LayoutId::new(current),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        }
    }

    fn ctx_with_rules<'a>(
        langs: &'a [LanguageProfile],
        current: &str,
        rules: &'a WordRules,
    ) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: LayoutId::new(current),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules,
        }
    }

    #[test]
    fn switches_long_gibberish_to_real_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // g h b d s n → en "ghbdsn", uk "привіт".
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(d.switch, "мало перемкнути (confidence={})", d.confidence);
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "привіт");
        assert_eq!(d.current_text, "ghbdsn");
    }

    #[test]
    fn does_not_switch_when_current_is_already_a_real_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // h e then... "hello" need l/o; use "good": g o o d → en "good" (у словнику).
        let d = decide(&strokes(&[0x22, 0x18, 0x18, 0x20]), &ctx);
        assert!(
            !d.switch,
            "реальне англ. слово не чіпати (best={:?})",
            d.best
        );
        assert_eq!(d.current_text, "good");
    }

    #[test]
    fn does_not_switch_short_ambiguous_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // 2 страйки: en "to" / uk "пе"? → коротке, поріг дуже високий → не чіпати.
        let d = decide(&strokes(&[0x14, 0x18]), &ctx);
        assert!(
            !d.switch,
            "коротке слово не перемикати (confidence={})",
            d.confidence
        );
    }

    #[test]
    fn threshold_is_stricter_for_short_words() {
        let cfg = DetectorConfig::default();
        assert!(cfg.threshold(2) > cfg.threshold(6));
        assert!(cfg.threshold(6) > cfg.threshold(12));
        // Коротше за min_switch_len → нескінченність (ніколи).
        assert_eq!(cfg.threshold(0), f64::INFINITY);
    }

    #[test]
    fn no_current_profile_means_no_switch() {
        let langs = [uk_profile()];
        // Поточна "en" відсутня серед профілів → не знаємо, що на екрані.
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(!d.switch);
    }

    #[test]
    fn veto_word_blocks_high_score_switch() {
        let langs = [en_profile(), uk_profile()];
        let mut rules = WordRules::new();
        rules.veto_word("привіт"); // навіть із високим балом — не чіпати
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(!d.switch, "veto має заблокувати перемикання");
        assert_eq!(d.best_text, "привіт"); // детектор усе одно бачить кандидата
    }

    #[test]
    fn force_word_switches_below_min_length() {
        let langs = [en_profile(), uk_profile()];
        // 1 символ: g → en "g", uk "п" (є у словнику uk → bonus робить best=uk).
        // Коротше за min_switch_len(2) → БЕЗ force не перемикається ніколи.
        let plain = ctx_with(&langs, "en");
        let d0 = decide(&strokes(&[0x22]), &plain);
        assert!(!d0.switch, "коротке (1 символ) без force не чіпати");

        let mut rules = WordRules::new();
        rules.force_word("g");
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x22]), &ctx);
        assert_eq!(d.best, LayoutId::new("uk"));
        assert!(d.switch, "force має перемкнути попри min length");
    }
}
