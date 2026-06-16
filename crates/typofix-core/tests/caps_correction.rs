//! E2E корекції регістру (помилка перетриманого Shift) через `VirtualPlatform`.
//!
//! Сценарій: користувач перетримав Shift і набрав `ПРивіт` (зайва 2-га велика) —
//! слово вже в правильній МОВІ/розкладці (uk), помилка лише в РЕГІСТРІ. На межі
//! слова ядро має нормалізувати регістр → `Привіт`, **БЕЗ зміни розкладки**
//! (`SwitchLayout` не емітиться). Virtual не «друкує» сам — крякозябри/каси ставимо
//! через `set_text`, імітуючи те, що ОС уже надрукувала; міняють текст лише наші
//! [`Action`]. Деталі — `typofix-platform-virtual/CLAUDE.md`.

use typofix_core::{
    step, Context, DetectorConfig, EngineState, ExclusionRules, LanguageProfile, LayoutId,
    WordRules,
};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers, Platform};
use typofix_platform_virtual::{drive, VirtualPlatform};

static NO_EXCL: ExclusionRules = ExclusionRules::new();
static NO_RULES: WordRules = WordRules::new();

/// Натиск із заданими модифікаторами (SHIFT → велика літера в розкладці).
fn key_mod(scancode: u32, modifiers: Modifiers) -> InputEvent {
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

fn key(scancode: u32) -> InputEvent {
    key_mod(scancode, Modifiers::empty())
}

fn profile(id: &str) -> LanguageProfile {
    LanguageProfile {
        id: LayoutId::new(id),
        layout: typofix_data::embedded_layout(id).expect("розкладка"),
        lm: typofix_data::sample_lm(id).expect("LM"),
        dict: typofix_data::sample_dict(id).expect("словник"),
        freq: None,
    }
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

// Фізичні позиції (scancode set 1) для «привіт».
const G: u32 = 0x22; // п
const H: u32 = 0x23; // р
const B: u32 = 0x30; // и
const D: u32 = 0x20; // в
const S: u32 = 0x1F; // і
const N: u32 = 0x31; // т
const SPACE: u32 = 0x39;
const SHIFT: Modifiers = Modifiers::SHIFT;

#[test]
fn overheld_shift_two_caps_is_normalized() {
    // `ПРивіт`→`Привіт`: 2 великі на початку, реальне укр. слово. Розкладка uk
    // (слово вже правильною мовою) → лише корекція регістру, без перемикання.
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("ПРивіт "); // на екрані разом із надрукованим пробілом
    platform.enqueue_all([
        key_mod(G, SHIFT),
        key_mod(H, SHIFT),
        key(B),
        key(D),
        key(S),
        key(N),
        key(SPACE),
    ]);

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "Привіт ",
        "зайві великі літери префікса мали стати малими"
    );
    assert_eq!(
        platform.current_layout(),
        LayoutId::new("uk"),
        "розкладка НЕ має змінюватись (чиста caps-корекція)"
    );
    // Стерти слово+пробіл (7), вписати «Привіт »; SwitchLayout відсутній.
    assert_eq!(
        platform.applied_actions(),
        [
            Action::DeleteChars(7),
            Action::TypeUnicode("Привіт ".into()),
        ],
        "caps-корекція не має емітити SwitchLayout"
    );
}

#[test]
fn overheld_shift_three_caps_is_normalized() {
    // `ПРИвіт`→`Привіт`: 3 великі на початку.
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("ПРИвіт ");
    platform.enqueue_all([
        key_mod(G, SHIFT),
        key_mod(H, SHIFT),
        key_mod(B, SHIFT),
        key(D),
        key(S),
        key(N),
        key(SPACE),
    ]);

    run(&mut platform, &langs);

    assert_eq!(platform.text(), "Привіт ");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn all_caps_word_is_left_untouched() {
    // `ПРИВІТ` — повністю велике (навмисний капс/акронім) → не чіпати.
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("ПРИВІТ ");
    platform.enqueue_all([
        key_mod(G, SHIFT),
        key_mod(H, SHIFT),
        key_mod(B, SHIFT),
        key_mod(D, SHIFT),
        key_mod(S, SHIFT),
        key_mod(N, SHIFT),
        key(SPACE),
    ]);

    run(&mut platform, &langs);

    assert_eq!(platform.text(), "ПРИВІТ ", "ALL-CAPS чіпати не можна");
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn already_correct_capitalized_word_is_left_untouched() {
    // `Привіт` — одна велика + малі → вже коректно, не чіпати.
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("Привіт ");
    platform.enqueue_all([
        key_mod(G, SHIFT),
        key(H),
        key(B),
        key(D),
        key(S),
        key(N),
        key(SPACE),
    ]);

    run(&mut platform, &langs);

    assert_eq!(platform.text(), "Привіт ", "коректний регістр не чіпати");
    assert!(platform.applied_actions().is_empty());
}
