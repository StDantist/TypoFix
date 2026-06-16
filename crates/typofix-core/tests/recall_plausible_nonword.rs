//! Регресія кейсу B (калібрування `short_word_extra=4.0`): правдоподібні укр.
//! НЕ-словникові слова мають перемикатись на char-LM перевагу (без dict-hit),
//! а тонко-маржинальні негативи (англ. слова/код) — НІ. Лочить recall-виграш і
//! precision-запас одночасно.
//!
//! Вантажить РЕАЛЬНІ моделі з `data/lm`,`data/dicts` (бо margin — властивість
//! реального LM). Якщо їх нема (CI, gitignored `.bin`/`.fst`) — тест SKIP'иться;
//! конфіг-інваріант усе одно стереже калібрування в `detector.rs` (hermetic).

use std::path::PathBuf;

use typofix_core::{
    detector, Context, DetectorConfig, ExclusionRules, KeyStroke, LanguageProfile, LayoutId,
    Modifiers, WordRules,
};

static NO_EXCL: ExclusionRules = ExclusionRules::new();
static NO_RULES: WordRules = WordRules::new();

fn real_profiles() -> Option<Vec<LanguageProfile>> {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
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
        });
    }
    Some(v)
}

fn strokes(scs: &[u32]) -> Vec<KeyStroke> {
    scs.iter()
        .map(|&s| KeyStroke::new(s, Modifiers::empty()))
        .collect()
}

fn ctx<'a>(langs: &'a [LanguageProfile], current: &str) -> Context<'a> {
    Context {
        active_window: Default::default(),
        current_layout: LayoutId::new(current),
        languages: langs,
        config: DetectorConfig::default(),
        exclusions: &NO_EXCL,
        rules: &NO_RULES,
    }
}

#[test]
fn plausible_nonword_rjk_switches_to_kol() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    // r j k (en) → к о л (uk). «кол» НЕМАЄ у словнику, але char-LM сильно за uk
    // (фонотактично валідне) — має перемкнутись завдяки калібруванню кейсу B.
    let d = detector::decide(&strokes(&[0x13, 0x24, 0x25]), &ctx(&langs, "en"));
    assert_eq!(d.current_text, "rjk");
    assert_eq!(d.best_text, "кол");
    assert!(
        d.switch,
        "правдоподібне не-слово має перемкнутись (conf={:.2}, thr(3)={:.2})",
        d.confidence,
        DetectorConfig::default().threshold(3)
    );
    assert_eq!(d.best, LayoutId::new("uk"));
}

#[test]
fn thin_margin_negatives_do_not_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    // Найтонший негатив із свіпу: `vec` (код) — uk-інтерпретація має conf≈1.15,
    // нижче thr(3)=2.33 → НЕ перемикати. Плюс звичайні англ. слова (lm_adv<0).
    let cases: &[(&str, &[u32])] = &[
        ("vec", &[0x2F, 0x12, 0x2E]), // v e c
        ("the", &[0x14, 0x23, 0x12]), // t h e
        ("for", &[0x21, 0x18, 0x13]), // f o r
        ("cat", &[0x2E, 0x1E, 0x14]), // c a t
        ("str", &[0x1F, 0x14, 0x13]), // s t r (код)
    ];
    for (name, scs) in cases {
        let d = detector::decide(&strokes(scs), &ctx(&langs, "en"));
        assert!(
            !d.switch,
            "негатив '{name}' НЕ має перемикатись (best={} conf={:.2}, thr(3)={:.2})",
            d.best.as_str(),
            d.confidence,
            DetectorConfig::default().threshold(3)
        );
    }
}
