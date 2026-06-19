//! Регресія ЦІЛЬОВО-кейованого примусового перемикання (UI «always_switch» →
//! `WordRules::force_switch_word`): явні слова зі списку «завжди перемикати»
//! перемикаються НЕЗАЛЕЖНО від довжини (навіть 1–2 літери), в обхід рантаймового
//! `min_switch_len` (тут — реалістичний `=3`, як дефолт `src-tauri`).
//!
//! ⚠️ Поле `force` (`forces`) кейоване на ПОТОЧНИЙ (екранний) текст = кирилична
//! каша для набраного в чужій розкладці → для UI-винятків НЕ спрацьовує. Тому
//! окреме target-кейоване `force_switch`: список містить ЦІЛЬОВЕ слово («ad»), а
//! детектор порівнює з ним ІНТЕРПРЕТАЦІЮ кандидата.
//!
//! Реальні розкладки (`embedded_layout`) + реальні LM/словники з `data/` (SKIP,
//! якщо `.bin` нема). Механіку герметично стережуть юніти `force_switch_*` у
//! `rules.rs`/`detector.rs`.

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

fn strokes_in(layout: &Layout, word: &str) -> Vec<KeyStroke> {
    word.chars()
        .map(|c| layout.stroke_for(c).expect("символ має бути в розкладці"))
        .collect()
}

/// Контекст із РАНТАЙМОВИМ `min_switch_len=3` (дефолт `src-tauri` → `min_word_len=3`),
/// саме той режим, де гейти довжини ріжуть короткі слова без форсування.
fn ctx_min3<'a>(langs: &'a [LanguageProfile], current: &str, rules: &'a WordRules) -> Context<'a> {
    Context {
        active_window: Default::default(),
        current_layout: LayoutId::new(current),
        languages: langs,
        config: DetectorConfig {
            min_switch_len: 3,
            ..DetectorConfig::default()
        },
        exclusions: &NO_EXCL,
        rules,
        secure: false,
    }
}

#[test]
fn force_switch_two_letter_target_switches_with_runtime_min_len_3() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // ЦІЛЬ — валідне 2-літерне en-слово «ad»; набране у ВИПАДКОВО ввімкненій UK
    // розкладці → на екрані кирилична каша «фв».
    let mut rules = WordRules::new();
    rules.force_switch_word("ad");

    let d = detector::decide(&strokes_in(en, "ad"), &ctx_min3(&langs, "uk", &rules));
    assert_eq!(
        d.best,
        LayoutId::new("en"),
        "ціль 'ad' зі списку має перемкнутись на en (best={} conf={:.2})",
        d.best.as_str(),
        d.confidence
    );
    assert_eq!(d.best_text, "ad");
    assert!(
        d.switch,
        "2-літерна ціль 'ad' має перемкнутись попри min_switch_len=3 (conf={:.2})",
        d.confidence
    );
}

#[test]
fn force_switch_single_letter_target_switches_with_runtime_min_len_3() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // ОДНОЛІТЕРНА ціль «x» (поза курованим однолітерним whitelist) → набрана в UK
    // показує «ч»; має примусово перемкнутись.
    let mut rules = WordRules::new();
    rules.force_switch_word("x");

    let d = detector::decide(&strokes_in(en, "x"), &ctx_min3(&langs, "uk", &rules));
    assert_eq!(d.best, LayoutId::new("en"));
    assert_eq!(d.best_text, "x");
    assert!(
        d.switch,
        "1-літерна ціль 'x' зі списку має перемкнутись (conf={:.2})",
        d.confidence
    );
}

#[test]
fn two_letter_word_not_in_force_list_does_not_switch_at_min_3() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // PRECISION-КОНТРОЛЬ (а): те саме 2-літерне, але СПИСОК ПОРОЖНІЙ → при min=3
    // короткий шлях не повинен перемикати (нічого не регресуємо).
    let empty = WordRules::new();
    let d = detector::decide(&strokes_in(en, "ad"), &ctx_min3(&langs, "uk", &empty));
    assert!(
        !d.switch,
        "без force_switch 'ad' при min=3 НЕ перемикати (best={} conf={:.2})",
        d.best.as_str(),
        d.confidence
    );
}

#[test]
fn force_switch_target_also_vetoed_does_not_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // PRECISION-КОНТРОЛЬ (б): слово і в always_switch, і в never_switch → veto
    // переможе (veto має пріоритет над усіма forced-сигналами).
    let mut rules = WordRules::new();
    rules.force_switch_word("ad");
    rules.veto_word("ad");
    let d = detector::decide(&strokes_in(en, "ad"), &ctx_min3(&langs, "uk", &rules));
    assert!(
        !d.switch,
        "veto на 'ad' має перемогти force_switch (best={} conf={:.2})",
        d.best.as_str(),
        d.confidence
    );
}

#[test]
fn correctly_typed_target_in_own_layout_not_touched() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // PRECISION-КОНТРОЛЬ (в): ціль КОРЕКТНО набрана у ВЛАСНІЙ (en) розкладці →
    // best==current → не чіпаємо (фільтр `p.id != current_layout`).
    let mut rules = WordRules::new();
    rules.force_switch_word("ad");
    let d = detector::decide(&strokes_in(en, "ad"), &ctx_min3(&langs, "en", &rules));
    assert_eq!(d.current_text, "ad");
    assert!(
        !d.switch,
        "коректно набрану ціль 'ad' у власній розкладці не чіпати (best={} conf={:.2})",
        d.best.as_str(),
        d.confidence
    );
}
