//! Регресія частотно-зваженого dict-бонусу (`detector::score`, `FrequencyMap`).
//!
//! **⚠️ КОНТРАКТ `ну`↔`ye` ПЕРЕВЕРНУТО precision-аудитом.** Раніше частота
//! перемикала `ну` над архаїчним `ye` (обидва — реальні слова). АЛЕ для КОРОТКОГО
//! слова (len≤2), коли поточний текст — РЕАЛЬНЕ слово своєї мови (`ye`/`db`/`lt`
//! у словнику), `current_is_dict` тепер БЛОКУЄ перемикання незалежно від частоти
//! (precision-first: користувач міг мати на увазі англ. `ye`/database/less-than).
//! Частота веде data-driven перемикання лише коли поточний двійник — НЕ слово
//! (`та`/`що`/`от`). Частотний ШАР як такий (градуйований бал) лишається — він
//! живить `best_score.freq` для data-driven гейта й precision-гард `us`↔`гі`.
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
        secure: false,
    }
}

#[test]
fn nu_does_not_switch_because_ye_is_real_en() {
    // КОНТРАКТ ПЕРЕВЕРНУТО (precision-аудит): `ye` — РЕАЛЬНЕ (архаїчне) англ. слово
    // у en.fst, тож для короткого слова `current_is_dict` блокує перемикання
    // незалежно від частоти. `ye`→`ну` був у списку FP власника (як db/bp/lt/nt).
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    let strokes = strokes_in(uk, "ну");
    let d = detector::decide(&strokes, &ctx(&langs, "en", &rules));
    assert_eq!(d.current_text, "ye", "двійник на екрані має бути 'ye'");
    assert!(
        !d.switch,
        "реальне англ. 'ye' (у словнику) НЕ сміє перемикатись на 'ну' (conf={:.2})",
        d.confidence
    );
}

#[test]
fn frequent_uk_twins_switch_when_en_twin_not_a_word() {
    // Позитивний бік: часті укр. слова, чий en-двійник — НЕ реальне слово
    // (`от`→`jn`, `то`→`nj`, `що`→`oj` — жодного нема в en.fst), перемикаються
    // по ЧАСТОТІ (data-driven). Контраст із `ну`(ye, реальне) вище.
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["от", "то", "що"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "часте укр. '{w}' (en-двійник НЕ слово) має перемкнутись (switch={} best={} '{}' conf={:.2})",
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
