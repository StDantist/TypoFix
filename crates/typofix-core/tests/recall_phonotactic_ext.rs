//! Регресія двох сигналів розпізнавання A1 на реальних розкладках:
//!  1. **Фонотактика** — укр. читання, що починається з «ь» (U+044C), НЕМОЖЛИВЕ
//!     (нема укр. слів на «ь») → перемкнути на латиницю.
//!  2. **Файлові розширення** — EN-двійник кандидата — відоме розширення
//!     (`txt`/`md`/`pdf`), а укр. читання НЕ слово → перемкнути на латиницю;
//!     ризикові розширення-слова (`doc`/`log`/`go`), коректно набрані в EN, НЕ
//!     ламаємо.
//!
//! Реальні розкладки (`embedded_layout`) + реальні LM/словники з `data/` (SKIP,
//! якщо `.bin` нема). Перелік розширень — вбудований fallback Bruno. Механіку
//! герметично стережуть юніти `phonotactic_*`/`extension_*` у `detector.rs`.

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
        secure: false,
    }
}

#[test]
fn phonotactic_soft_sign_start_switches_on_real_models() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // «mb» (не розширення, не валютна пара) у UK-розкладці → «ьи» (старт із «ь»).
    // Лише фонотактика може це перемкнути.
    let rules = WordRules::new();
    let d = detector::decide(&strokes_in(en, "mb"), &ctx(&langs, "uk", &rules));
    assert_eq!(d.current_text, "ьи", "укр. читання має починатися з «ь»");
    assert_eq!(d.best, LayoutId::new("en"));
    assert_eq!(d.best_text, "mb");
    assert!(
        d.switch,
        "старт із «ь» неможливий → перемкнути на латиницю (conf={:.2})",
        d.confidence
    );
}

#[test]
fn no_real_uk_word_starts_with_soft_sign() {
    // Інваріант, на якому стоїть фонотактичне правило: у реальному словнику НЕМАЄ
    // жодного слова на «ь». Якщо колись з'явиться — правило треба переглянути.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let count = langs[0]
        .dict
        .words()
        .iter()
        .filter(|w| w.starts_with('ь'))
        .count();
    assert_eq!(count, 0, "укр. словник не має містити слів на «ь»");
}

#[test]
fn extension_switches_from_cyrillic_and_correct_one_kept() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    // build_word_rules вантажить перелік розширень (файл або вбудований fallback).
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);

    for ext in ["txt", "pdf", "json", "html"] {
        // Розширення, набране у ввімкненій UK-розкладці → кирилична каша.
        let d = detector::decide(&strokes_in(en, ext), &ctx(&langs, "uk", &rules));
        assert_eq!(
            d.best,
            LayoutId::new("en"),
            "розширення '{ext}' з укр. розкладки → перемкнути на en (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
        assert_eq!(d.best_text, ext);
        assert!(d.switch, "розширення '{ext}' має перемкнутись");
    }
}

#[test]
fn risky_extension_words_typed_in_en_are_not_touched() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let en = &langs[1].layout;
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    // Ризикові розширення-слова, КОРЕКТНО набрані в EN → НЕ чіпати (best==current,
    // фільтр `p.id != current_layout` лишає їх; гейт «укр. читання — слово» теж
    // захищає інший напрям). Це і є precision-гард для `doc`/`log`/`go`.
    for w in ["doc", "log", "go", "md"] {
        let d = detector::decide(&strokes_in(en, w), &ctx(&langs, "en", &rules));
        assert_eq!(d.current_text, w);
        assert!(
            !d.switch,
            "коректне англ. '{w}' не ламати (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
    }
}
