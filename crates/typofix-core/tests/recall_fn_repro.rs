//! ТИМЧАСОВИЙ репро-харнес для двох реальних FN-кейсів (recall).
//! Вантажить РЕАЛЬНІ моделі з `data/lm`,`data/dicts`, якщо вони є.

use std::path::PathBuf;

use typofix_core::{
    step, Context, DetectorConfig, EngineState, ExclusionRules, LanguageProfile, LayoutId,
    WordRules,
};
use typofix_platform::{InputEvent, KeyDir, KeyEvent, Modifiers};
use typofix_platform_virtual::{drive, VirtualPlatform};

static NO_EXCL: ExclusionRules = ExclusionRules::new();
static NO_RULES: WordRules = WordRules::new();

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

fn key(scancode: u32, modifiers: Modifiers) -> InputEvent {
    InputEvent::Key(KeyEvent {
        scancode,
        vk: 0,
        dir: KeyDir::Down,
        modifiers,
        timestamp_ms: 0,
        is_synthetic: false,
        is_autorepeat: false,
    })
}

fn run(platform: &mut VirtualPlatform, langs: &[LanguageProfile]) {
    let mut state = EngineState::default();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        };
        step(&mut state, ev, &ctx)
    });
}

const SPACE: u32 = 0x39;
const E: Modifiers = Modifiers::empty();
const SH: Modifiers = Modifiers::SHIFT;

#[test]
fn repro_case1_dobre_with_comma_key() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("en"));
    p.set_text("lj,ht "); // на екрані: те, що ОС надрукувала (з пробілом-тригером)
                          // l j , h t  =  д о б р е
    p.enqueue_all([
        key(0x26, E), // l → д
        key(0x24, E), // j → о
        key(0x33, E), // , → б
        key(0x23, E), // h → р
        key(0x14, E), // t → е
        key(SPACE, E),
    ]);
    run(&mut p, &langs);
    eprintln!("CASE1 result text = {:?}", p.text());
    eprintln!("CASE1 actions = {:?}", p.applied_actions());
    assert_eq!(p.text(), "добре ", "мало стати «добре »");
}

#[ignore = "FN-баг recall на не-словах (ALL-CAPS тікер): окрема задача — див. звіт"]
#[test]
fn repro_case2_eurusd_allcaps() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("uk"));
    p.set_text("УГКГІВ ");
    // E U R U S D (з Shift) у uk-розкладці = У Г К Г І В
    p.enqueue_all([
        key(0x12, SH), // E → У
        key(0x16, SH), // U → Г
        key(0x13, SH), // R → К
        key(0x16, SH), // U → Г
        key(0x1F, SH), // S → І
        key(0x20, SH), // D → В
        key(SPACE, E),
    ]);
    run(&mut p, &langs);
    eprintln!("CASE2 result text = {:?}", p.text());
    eprintln!("CASE2 actions = {:?}", p.applied_actions());
    assert_eq!(p.text(), "EURUSD ", "мало стати «EURUSD »");
}
