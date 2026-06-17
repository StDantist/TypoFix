//! Регресія двох ПОЗИТИВНИХ сигналів перемикання на реальних розкладках:
//!  1. **Особистий словник** (`user.txt` → `WordRules::recognize_word`): слово,
//!     якого НЕМАЄ у стандартному словнику (`вжух`), користувач вписав як визнане
//!     → дістає dict-бонус → перемикається.
//!  2. **Forex-пари** (`iso4217.txt` → `WordRules::is_currency_pair`): валютна
//!     пара, набрана у ВИПАДКОВО ввімкненій укр. розкладці, впевнено перемикається
//!     на латиницю; коректно набрану латиницею пару НЕ ламаємо.
//!
//! Реальні розкладки (`embedded_layout`) + реальні LM/словники з `data/` (SKIP,
//! якщо `.bin` нема). ISO-перелік — вбудований fallback Bruno, тож forex-частина
//! працює навіть без `data/dicts/iso4217.txt` на диску. Механіку герметично
//! стережуть юніти `forex_*`/`user_word_*` у `detector.rs` і `rules.rs`.

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
fn personal_dictionary_word_switches() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let uk = &langs[0].layout;
    // Передумова тесту: «вжух» НЕ у стандартному словнику (інакше тест беззмістовний).
    if langs[0].dict.contains("вжух") {
        eprintln!("SKIP: 'вжух' уже у словнику — особистий словник не потрібен");
        return;
    }
    let mut rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    rules.recognize_word("вжух");

    // «вжух», набране в EN-розкладці → на екрані «d;e[». Має перемкнутись на uk.
    let d = detector::decide(&strokes_in(uk, "вжух"), &ctx(&langs, "en", &rules));
    assert_eq!(d.best, LayoutId::new("uk"));
    assert_eq!(d.best_text, "вжух");
    assert!(
        d.switch,
        "user-слово 'вжух' має перемкнутись (conf={:.2})",
        d.confidence
    );

    // Без особистого словника — НЕ ловиться (поза стандартним dict).
    let plain = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let d0 = detector::decide(&strokes_in(uk, "вжух"), &ctx(&langs, "en", &plain));
    assert!(
        !d0.switch,
        "без user.txt 'вжух' не перемикати (conf={:.2})",
        d0.confidence
    );
}

#[test]
fn forex_pair_switches_from_cyrillic_and_correct_one_is_kept() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // build_word_rules вантажить ISO 4217 (файл або вбудований fallback Bruno).
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);

    for pair in ["eurusd", "gbpusd", "usdjpy"] {
        // Набрано у ВИПАДКОВО ввімкненій UK-розкладці → кирилична каша на екрані.
        let d = detector::decide(&strokes_in(en, pair), &ctx(&langs, "uk", &rules));
        assert_eq!(
            d.best,
            LayoutId::new("en"),
            "пара '{pair}' з укр. розкладки має перемкнутись на en (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
        assert_eq!(d.best_text, pair);
        assert!(d.switch, "пара '{pair}' має перемкнутись");

        // КОНТРОЛЬ: та сама пара вже КОРЕКТНО в EN → не чіпати.
        let d_ok = detector::decide(&strokes_in(en, pair), &ctx(&langs, "en", &rules));
        assert_eq!(d_ok.current_text, pair);
        assert!(
            !d_ok.switch,
            "коректну латинську пару '{pair}' не ламати (best={} conf={:.2})",
            d_ok.best.as_str(),
            d_ok.confidence
        );
    }
}
