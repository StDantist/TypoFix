//! Регресії на повному словнику VESUM (`data/dicts/uk.fst`, коміт cdb467a):
//! 1. розмовна лексика (`рашка`) тепер IN → en-двійник `hfirf` має ловитись;
//! 2. **апостроф-нормалізація:** словник VESUM використовує ASCII `'` (U+0027),
//!    а розкладка генерує типографський `’` (U+2019) → без зведення до канону
//!    апострофні слова (`сім'я`, `комп'ютер`) промахуються повз dict-lookup.
//!
//! Наскрізний E2E через `VirtualPlatform` з РЕАЛЬНИМИ моделями з `data/`. Якщо
//! їх нема (CI, gitignored `.bin`/`.fst`) — тест SKIP'иться.

use std::path::PathBuf;

use typofix_core::{
    step, Context, DetectorConfig, EngineState, ExclusionRules, KeyStroke, LanguageProfile, Layout,
    LayoutId, WordRules,
};
use typofix_platform::{InputEvent, KeyDir, KeyEvent, Modifiers, Platform};
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
        });
    }
    Some(v)
}

/// Натиск зі scancode + модифікаторами.
fn ev(stroke: KeyStroke) -> InputEvent {
    InputEvent::Key(KeyEvent {
        scancode: stroke.scancode,
        vk: 0,
        dir: KeyDir::Down,
        modifiers: stroke.modifiers,
        timestamp_ms: 0,
        is_synthetic: false,
        is_autorepeat: false,
    })
}

const SPACE: u32 = 0x39;

fn space() -> InputEvent {
    InputEvent::Key(KeyEvent {
        scancode: SPACE,
        vk: 0,
        dir: KeyDir::Down,
        modifiers: Modifiers::empty(),
        timestamp_ms: 0,
        is_synthetic: false,
        is_autorepeat: false,
    })
}

/// Фізичні страйки для слова, як його НАБИРАЮТЬ у заданій розкладці.
fn strokes_in(layout: &Layout, word: &str) -> Vec<KeyStroke> {
    word.chars()
        .map(|c| layout.stroke_for(c).expect("символ має бути в розкладці"))
        .collect()
}

/// Прогнати: користувач у EN фізично набрав укр. слово `uk_word` + пробіл.
/// На екрані вже en-інтерпретація (хук пропускає натиски) → перевіряємо, що
/// ядро перенабрало рівно `uk_word ` (з пробілом).
fn type_uk_word_in_en(langs: &[LanguageProfile], uk_word: &str) -> (String, LayoutId) {
    let uk = &langs[0].layout;
    let en = &langs[1].layout;
    let strokes = strokes_in(uk, uk_word);
    let on_screen = format!("{} ", en.interpret(&strokes)); // en-двійник + пробіл

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text(&on_screen);
    let mut events: Vec<InputEvent> = strokes.iter().map(|&s| ev(s)).collect();
    events.push(space());
    platform.enqueue_all(events);

    let mut state = EngineState::default();
    drive(&mut platform, |e, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        };
        step(&mut state, e, &ctx)
    });
    (platform.text(), platform.current_layout())
}

#[test]
fn vesum_apostrophe_words_are_in_dict_under_layout_codepoint() {
    // Прямий guard баг-репорту: VESUM-слова записані ASCII `'` (U+0027), а
    // розкладка дає типографський `’` (U+2019). Беремо РЕАЛЬНІ U+0027-слова зі
    // словника й перевіряємо, що запит у вигляді U+2019 (як прийде з розкладки)
    // їх знаходить. Без апостроф-нормалізації в `Dictionary::contains` — промах.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let dict = &langs[0].dict;
    let mut checked = 0;
    for w in dict.words() {
        if w.contains('\u{0027}') {
            let as_typed: String = w
                .chars()
                .map(|c| if c == '\u{0027}' { '\u{2019}' } else { c })
                .collect();
            assert!(
                dict.contains(&as_typed),
                "U+0027-слово '{w}' має знаходитись за запитом U+2019 '{as_typed}'"
            );
            checked += 1;
            if checked >= 200 {
                break;
            }
        }
    }
    assert!(checked > 0, "у VESUM мали бути апострофні слова");
}

#[test]
fn rashka_slang_is_caught() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    // `рашка` (VESUM має розмовну лексику) набране в EN → "hfirf" → має ловитись.
    let (text, layout) = type_uk_word_in_en(&langs, "рашка");
    assert_eq!(text, "рашка ", "розмовне 'рашка' має перенабратись");
    assert_eq!(layout, LayoutId::new("uk"));
}

#[test]
fn apostrophe_words_are_caught() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    // Апострофні слова: розкладка дає U+2019, словник має U+0027. Без
    // нормалізації — промах повз dict → не ловиться. Слова беремо з U+2019
    // (як їх знає розкладка); на екран/перенабір іде той самий U+2019.
    for w in ["сім’я", "комп’ютер", "п’ять", "об’єкт"] {
        let (text, layout) = type_uk_word_in_en(&langs, w);
        assert_eq!(text, format!("{w} "), "апострофне '{w}' має перенабратись");
        assert_eq!(layout, LayoutId::new("uk"), "'{w}' → uk");
    }
}
