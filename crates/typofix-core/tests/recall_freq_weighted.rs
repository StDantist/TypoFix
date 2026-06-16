//! Регресія частотно-зваженого dict-бонусу (`detector::score`, `FrequencyMap`).
//!
//! **Цільовий баг — `ну`↔`ye`:** обидва двійники — РЕАЛЬНІ слова (укр. вигук `ну`
//! і архаїчне англ. `ye`), обидва у своїх словниках. Бінарний dict-бонус
//! скасовується, LM майже рівні → детектор НЕ перемикав. Частотний шар розрізняє:
//! нормалізована log-ймовірність `ну`(≈−5.85) ≫ `ye`(≈−11.1), тож коротко-словний
//! гейт відкривається по ЧАСТОТНІЙ маржі й перемикання відбувається.
//!
//! **Precision-бік:** часте англ. слово, чий укр. двійник теж є в словнику, але
//! РІДКІСНИЙ (`us`↔`гі`), лишається англійським — частота захищає легітимний ввід.
//!
//! Вантажить РЕАЛЬНІ моделі+частоти з `data/` (`{lang}.bin/.fst/.freq.fst`). Нема
//! їх (CI, gitignored) → SKIP; герметичне покриття механіки — юніти `freq_*` у
//! `detector.rs` і `freq.rs`.

use std::path::PathBuf;

use typofix_core::{
    detector, Context, DetectorConfig, ExclusionRules, FrequencyMap, KeyStroke, LanguageProfile,
    Layout, LayoutId, WordRules,
};

static NO_EXCL: ExclusionRules = ExclusionRules::new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

/// Профілі з УВІМКНЕНИМ частотним шаром (як у проді/eval). `None`, якщо реальних
/// артефактів немає.
fn real_profiles_with_freq() -> Option<Vec<LanguageProfile>> {
    let data = data_dir();
    let lm_dir = data.join("lm");
    let dict_dir = data.join("dicts");
    if !lm_dir.join("uk.bin").exists() || !dict_dir.join("uk.freq.fst").exists() {
        return None;
    }
    let mut v = Vec::new();
    for lang in ["uk", "en"] {
        let freq_path = dict_dir.join(format!("{lang}.freq.fst"));
        let freq = typofix_data::load_freq_map_file(&freq_path)
            .ok()
            .map(FrequencyMap::from_fst_map);
        v.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout: typofix_data::embedded_layout(lang).unwrap(),
            lm: typofix_data::load_lm(lang, Some(&lm_dir)).unwrap(),
            dict: typofix_data::load_dict(lang, Some(&dict_dir)).unwrap(),
            freq,
        });
    }
    Some(v)
}

/// Те саме, але БЕЗ частотного шару (`freq: None`) — контроль причинності.
fn real_profiles_no_freq() -> Option<Vec<LanguageProfile>> {
    let mut v = real_profiles_with_freq()?;
    for p in &mut v {
        p.freq = None;
    }
    Some(v)
}

/// Фізичні страйки слова, як його НАБИРАЮТЬ у заданій розкладці (зворотний індекс).
fn strokes_in(layout: &Layout, word: &str) -> Vec<KeyStroke> {
    word.chars()
        .map(|c| layout.stroke_for(c).expect("символ має бути в розкладці"))
        .collect()
}

fn ctx<'a>(langs: &'a [LanguageProfile], current: &str, rules: &'a WordRules) -> Context<'a> {
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
fn nu_switches_over_archaic_ye() {
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    // «ну», набране в EN-розкладці → на екрані «ye» (архаїчне англ. слово у словнику).
    let strokes = strokes_in(uk, "ну");
    let d = detector::decide(&strokes, &ctx(&langs, "en", &rules));
    assert_eq!(d.current_text, "ye", "двійник на екрані має бути 'ye'");
    assert!(
        d.switch && d.best == LayoutId::new("uk") && d.best_text == "ну",
        "часте 'ну' має перемкнутись над рідкісним 'ye' (switch={} best={} '{}' conf={:.2})",
        d.switch,
        d.best.as_str(),
        d.best_text,
        d.confidence
    );
}

#[test]
fn nu_does_not_switch_without_freq_layer() {
    // Контроль причинності: БЕЗ частотного шару (freq=None) той самий «ну»↔«ye»
    // НЕ перемикається — саме частота розв'язує кейс, не щось інше.
    let Some(langs) = real_profiles_no_freq() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    let d = detector::decide(&strokes_in(uk, "ну"), &ctx(&langs, "en", &rules));
    assert_eq!(d.current_text, "ye");
    assert!(
        !d.switch,
        "без частотного шару 'ну'↔'ye' не мав перемикатись (conf={:.2}) — це й був баг",
        d.confidence
    );
}

#[test]
fn frequent_uk_twins_switch_over_real_but_rare_en() {
    // Інші «обидва-реальні-слова» кейси, де укр. набагато частіше за en-двійник.
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["ну", "от"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "часте укр. '{w}' має перемкнутись (switch={} best={} '{}' conf={:.2})",
            d.switch,
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn common_english_word_stays_when_uk_twin_is_rare() {
    // PRECISION-ГАРД: «us» — часте англ. слово; його укр. двійник «гі» Є у словнику,
    // але РІДКІСНИЙ. Частота захищає легітимний англ. ввід → НЕ перемикати.
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let en = &langs[1].layout;
    for w in ["us", "is", "to", "we"] {
        let d = detector::decide(&strokes_in(en, w), &ctx(&langs, "en", &rules));
        assert!(
            !d.switch,
            "часте англ. '{w}' (двійник рідкісний/не-слово) НЕ чіпати (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}
