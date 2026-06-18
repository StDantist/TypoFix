//! Регресія дзеркальної релаксації порога для КОРОТКИХ службових слів
//! (`detector::decide`, `WordRules::is_short_service`). Принцип «справжнє слово
//! ↔ біліберда»: коротке (len=2, ОДИНОЧНІ не чіпаємо) перемикається на dict-hit, якщо
//! кандидат — куроване службове слово (whitelist `data/dicts/{lang}.short.txt`)
//! І джерельний двійник НЕ справжнє слово. Реальні короткі англ. (`is`/`to`) —
//! НЕ чіпати (їхній двійник у поточній en — теж справжнє слово).
//!
//! Вантажить РЕАЛЬНІ моделі/словники + whitelist із `data/`. Якщо їх нема (CI,
//! gitignored `.bin`/`.fst`) — тест SKIP'иться; герметичне покриття механіки —
//! в `detector.rs` (юніт-тести `mirror_*`).

use std::path::PathBuf;

use typofix_core::{
    detector, Context, DetectorConfig, ExclusionRules, KeyStroke, LanguageProfile, Layout,
    LayoutId, WordRules,
};

static NO_EXCL: ExclusionRules = ExclusionRules::new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

fn real_profiles() -> Option<Vec<LanguageProfile>> {
    let data = data_dir();
    let lm_dir = data.join("lm");
    let dict_dir = data.join("dicts");
    if !lm_dir.join("uk.bin").exists() {
        return None;
    }
    let mut v = Vec::new();
    for lang in ["uk", "en"] {
        v.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout: typofix_data::embedded_layout(lang).unwrap(),
            lm: typofix_data::load_lm(lang, Some(&lm_dir)).unwrap(),
            dict: typofix_data::load_dict(lang, Some(&dict_dir)).unwrap(),
            freq: None,
        });
    }
    Some(v)
}

/// Фізичні страйки для слова, як його НАБИРАЮТЬ у заданій розкладці (зворотний
/// індекс): ті самі клавіші дають це слово в `layout` і біліберду в іншій.
fn strokes_in(layout: &Layout, word: &str) -> Vec<KeyStroke> {
    word.chars()
        .map(|c| layout.stroke_for(c).expect("символ має бути в розкладці"))
        .collect()
}

fn ctx<'a>(langs: &'a [LanguageProfile], current: &str, rules: &'a WordRules) -> Context<'a> {
    ctx_cfg(langs, current, rules, DetectorConfig::default())
}

fn ctx_cfg<'a>(
    langs: &'a [LanguageProfile],
    current: &str,
    rules: &'a WordRules,
    config: DetectorConfig,
) -> Context<'a> {
    Context {
        active_window: Default::default(),
        current_layout: LayoutId::new(current),
        languages: langs,
        config,
        exclusions: &NO_EXCL,
        rules,
        secure: false,
    }
}

#[test]
fn one_letter_tokens_never_switch() {
    // КОНТРАКТ ЗМІНЕНО (раніше assert був ЗВОРОТНИЙ). Одиночний токен (len=1)
    // НІКОЛИ не перемикається — навіть якщо його укр.-двійник у whitelist коротких
    // службових слів. Причина: репро власника — кома `,` (en) сидить на клавіші
    // `б`(uk, whitelist) → дзеркало хибно робило з коми «б». Самотня літера
    // практично ніколи не є самостійним словом, вартим перемикання; precision >
    // recall. Межа дзеркала — ЛІТЕРАЛ `len >= 2` (не cfg). Свідомий FN на одиночних.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["і", "й", "в", "у", "з"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            !d.switch,
            "1-літерний токен '{w}' НЕ має перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn two_letter_service_words_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["ти", "чи", "ми", "до", "по"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "2-літерне службове '{w}' має перемкнутись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn que_and_to_switch_with_runtime_min_len_3() {
    // РЕПРО власника на РЕАЛЬНИХ моделях ІЗ ЖИВИМ КОНФІГОМ: рантайм `src-tauri`
    // дефолтить `min_word_len=3` → `min_switch_len=3`. «oj»(en)→«що», «nj»(en)→«то»
    // (o→щ, j→о, n→т на ЙЦУКЕН). Поки межа дзеркала була `cfg.min_switch_len`, тут
    // обидва НЕ перемикалися (2>=3 false) — саме цей сценарій тести на default()=2
    // НЕ ловили. Після фікса (літерал 2) перемикаються попри min_switch_len=3.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let cfg = DetectorConfig {
        min_switch_len: 3,
        ..DetectorConfig::default()
    };
    let uk = &langs[0].layout;
    for w in ["що", "то"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx_cfg(&langs, "en", &rules, cfg));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "'{w}' (репро, min_switch_len=3) має перемкнутись (switch={} best={} '{}' conf={:.2})",
            d.switch,
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn real_english_short_words_do_not_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let en = &langs[1].layout;
    // Реальні короткі англ., набрані в EN → двійник у вихідній en — справжнє
    // слово → дзеркало НЕ спрацьовує, нічого не чіпаємо (precision-замок).
    for w in ["a", "i", "is", "to", "it"] {
        let d = detector::decide(&strokes_in(en, w), &ctx(&langs, "en", &rules));
        assert!(
            !d.switch,
            "реальне англ. '{w}' НЕ має перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}
