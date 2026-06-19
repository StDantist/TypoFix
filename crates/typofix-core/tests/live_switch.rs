//! E2E перемикання розкладки **НА ЛЬОТУ** (mid-word live switch) через
//! `VirtualPlatform`. Етап 3: інтеграція `detector::live_decide` у `engine::step`
//! (гілка `Class::Word`) + мід-ворд перенабір + когерентність буфера.
//!
//! ⚠️ **Готча virtual:** екран змінюють ЛИШЕ наші `Action` — фізичні клавіші самі
//! НЕ друкуються. Тому стан екрана в момент тригерного страйка виставляємо вручну
//! (`set_text`), імітуючи те, що ОС уже надрукувала (хук пропускає натиск далі).
//!
//! Профілі — реальні embedded-розкладки (точне key→char) + sample LM, але
//! **кастровані словники** (`Dictionary::from_words`), щоб контрольовано задати,
//! що є живим префіксом, а що глухим кутом. Механіку герметично стережуть юніти
//! `detector::live_*` і `dict::has_prefix_*`.

use typofix_core::{
    step, Context, DetectorConfig, Dictionary, EngineState, ExclusionRules, LanguageProfile,
    LayoutId, WordRules,
};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers, Platform, WindowInfo};
use typofix_platform_virtual::{drive, VirtualPlatform};

static NO_EXCL: ExclusionRules = ExclusionRules::new();

fn key(scancode: u32) -> InputEvent {
    InputEvent::Key(KeyEvent {
        scancode,
        vk: 0,
        dir: KeyDir::Down,
        modifiers: Modifiers::empty(),
        timestamp_ms: 0,
        is_synthetic: false,
        is_autorepeat: false,
    })
}

/// Профіль із реальною embedded-розкладкою + sample LM + КАСТРОВАНИМ словником.
fn profile(id: &str, words: &[&str]) -> LanguageProfile {
    LanguageProfile {
        id: LayoutId::new(id),
        layout: typofix_data::embedded_layout(id).expect("розкладка"),
        lm: typofix_data::sample_lm(id).expect("LM"),
        dict: Dictionary::from_words(words.iter().copied()).expect("словник"),
        freq: None,
    }
}

/// uk + en із контрольованими словниками: en знає `ad*`/`advance`; uk знає
/// `світ`/`привіт`. `фв`/`cd`/`yxz`/`нчя` — глухі кути.
fn langs() -> [LanguageProfile; 2] {
    [
        profile("uk", &["привіт", "світ", "день", "вода"]),
        profile("en", &["advance", "add", "ad", "order", "world", "code"]),
    ]
}

/// Конфіг із увімкненим live-switch і `live_min_len=2` (тести оперують 2-літерними
/// прикладами; дефолт у проді — 3, OFF).
fn live_cfg() -> DetectorConfig {
    DetectorConfig {
        live_switch_enabled: true,
        live_min_len: 2,
        ..DetectorConfig::default()
    }
}

/// Прогнати чергу через `core::step`, тримаючи `state` між фазами; `secure`
/// знімається з платформи ОДИН раз (як у `runtime.rs`).
fn run(
    platform: &mut VirtualPlatform,
    state: &mut EngineState,
    langs: &[LanguageProfile],
    cfg: &DetectorConfig,
    rules: &WordRules,
    excl: &ExclusionRules,
) {
    let secure = platform.is_secure_field();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: *cfg,
            exclusions: excl,
            rules,
            secure,
        };
        step(state, ev, &ctx)
    });
}

// Фізичні позиції (scancode set 1). en: a=0x1E d=0x20 v=0x2F n=0x31 c=0x2E e=0x12
// y=0x15 x=0x2D z=0x2C. uk-двійники: 0x1E→ф 0x20→в 0x2E→с 0x1F→і 0x31→т.
const A: u32 = 0x1E;
const D: u32 = 0x20;
const V: u32 = 0x2F;
const N: u32 = 0x31;
const C: u32 = 0x2E;
const E: u32 = 0x12;
const Y: u32 = 0x15;
const X: u32 = 0x2D;
const Z: u32 = 0x2C;
const S: u32 = 0x1F;
const SPACE: u32 = 0x39;
const BACKSPACE: u32 = 0x0E;

/// Трійка дій мід-ворд live-switch uk→en для «фв»→«ad».
fn ad_switch() -> [Action; 3] {
    [
        Action::DeleteChars(2),
        Action::SwitchLayout(LayoutId::new("en")),
        Action::TypeUnicode("ad".into()),
    ]
}

/// Трійка дій live-switch en→uk для «cd»→«св».
fn sv_switch() -> [Action; 3] {
    [
        Action::DeleteChars(2),
        Action::SwitchLayout(LayoutId::new("uk")),
        Action::TypeUnicode("св".into()),
    ]
}

#[test]
fn early_switch_mid_word_uk_to_en() {
    // uk активна, "фв" на екрані (ОС надрукувала в uk); "ad" — живий en-префікс,
    // "фв" — глухий кут uk → після `d` ранній перенабір + перемикання.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert_eq!(
        platform.applied_actions(),
        [
            Action::DeleteChars(2),
            Action::SwitchLayout(LayoutId::new("en")),
            Action::TypeUnicode("ad".into()),
        ],
        "мід-ворд: стерти 2 (фв), перемкнути на en, набрати «ad»"
    );
    assert_eq!(platform.text(), "ad");
    assert_eq!(platform.current_layout(), LayoutId::new("en"));
}

#[test]
fn word_continues_coherently_after_live_switch() {
    // Після свічу користувач ДОДРУКОВУЄ слово; ОС (вже в en) друкує решту фізичних
    // клавіш правильно. Буфер НЕ скидали → продовження когерентне, на межі — ЖОДНОГО
    // повторного перенабору (live_locked коротко замикає boundary).
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    // Фаза 2: ОС у en допечатала «vance» + пробіл (моделюємо станом екрана).
    platform.set_text("advance ");
    platform.enqueue_all([key(V), key(A), key(N), key(C), key(E), key(SPACE)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert_eq!(platform.text(), "advance ", "слово завершилось коректно");
    assert_eq!(
        platform.current_layout(),
        LayoutId::new("en"),
        "розкладка лишилась en"
    );
    // applied_actions = ЛИШЕ початкова трійка; жодних повторних Delete/Switch.
    assert_eq!(
        platform.applied_actions(),
        [
            Action::DeleteChars(2),
            Action::SwitchLayout(LayoutId::new("en")),
            Action::TypeUnicode("ad".into()),
        ],
        "після межі НЕ має бути повторного перенабору (boundary коротко замкнено)"
    );
}

#[test]
fn both_languages_dead_end_no_action() {
    // «yxz» (en) / «нчя» (uk) — не префікс у жодній мові (код/випадковість) →
    // нічого не робимо (чекати межу, як зараз).
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("yxz");
    platform.enqueue_all([key(Y), key(X), key(Z)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "обидва глухі кути → жодної дії (actions={:?})",
        platform.applied_actions()
    );
    assert_eq!(platform.text(), "yxz");
}

#[test]
fn correct_word_in_own_language_not_jerked_mid_word() {
    // uk активна, користувач набирає РЕАЛЬНЕ укр. слово «привіт» фізичними
    // клавішами: кожен префікс живий у uk → жодного live-switch до межі.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    // привіт: п=0x22 р=0x23 и=0x30 в=0x20 і=0x1F т=0x31
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("привіт");
    platform.enqueue_all([key(0x22), key(0x23), key(0x30), key(D), key(S), key(N)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "коректне укр. слово не смикати мід-ворд (actions={:?})",
        platform.applied_actions()
    );
    assert_eq!(platform.text(), "привіт");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn secure_field_no_live_switch() {
    // Поле пароля: повний bypass у `step` ДО гілки Word → live недосяжний.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_secure(true);
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "у secure-полі — нічого"
    );
    assert_eq!(platform.text(), "фв");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn excluded_window_no_live_switch() {
    // Виключене вікно: повний bypass у `step` → live недосяжний.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();
    let mut excl = ExclusionRules::new();
    excl.exclude_folder(r"C:\Games");

    let mut platform = VirtualPlatform::new();
    platform.set_window(WindowInfo {
        process_name: "game.exe".into(),
        exe_path: r"C:\Games\Cool\game.exe".into(),
        is_fullscreen: false,
    });
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &excl,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "у виключеному вікні — нічого"
    );
    assert_eq!(platform.text(), "фв");
}

#[test]
fn veto_blocks_live_switch() {
    // veto на цільове «ad» → live НЕ перемикає (precision-замок тримає detector).
    let langs = langs();
    let mut state = EngineState::default();
    let mut rules = WordRules::new();
    rules.veto_word("ad");

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &rules,
        &NO_EXCL,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "veto блокує live-switch"
    );
    assert_eq!(platform.text(), "фв");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn symmetry_en_to_uk() {
    // Поточна en, користувач набирає укр. слово «світ» (фіз. c,d,…): «cd» — глухий
    // кут en, «св» — живий префікс uk → перемикання en→uk. Симетрія напряму.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("cd"); // на екрані en-двійник перших двох клавіш
    platform.enqueue_all([key(C), key(D)]); // c,d → uk «св»

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert_eq!(
        platform.applied_actions(),
        [
            Action::DeleteChars(2),
            Action::SwitchLayout(LayoutId::new("uk")),
            Action::TypeUnicode("св".into()),
        ],
        "симетрія EN→UK: стерти «cd», перемкнути на uk, набрати «св»"
    );
    assert_eq!(platform.text(), "св");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn pin_cleared_after_backspace_to_empty_next_word_processed() {
    // ДЕФЕКТ 1 (regression). uk, «фв»→live-switch «ad» (live_locked=true) →
    // Backspace×2 СПОРОЖНЮЄ буфер (pop-до-порожнього МУСИТЬ зняти пін) → НОВЕ слово
    // має отримати нормальну обробку. Якби пін витік — кожен страйк/межа другого
    // слова придушувались би (ЧЕРВОНИЙ до фіксу: другий switch відсутній).
    //
    // Друге слово тут — свіжий live-switch en→uk («cd»→«св»): детермінований у
    // virtual (свіч на ОСТАННЬОМУ страйку, без розбіжності розкладки мід-фазою, на
    // відміну від ghbdsn-boundary, де pass-through друк залежав би від того, чи
    // спрацював свіч). Суть дефекту та сама: пін не сміє пережити слово.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );
    assert_eq!(
        platform.applied_actions(),
        ad_switch(),
        "передумова: word1 свіч"
    );

    // Backspace×2 → буфер [a,d]→[a]→[] (pop-до-порожнього знімає пін).
    platform.enqueue_all([key(BACKSPACE), key(BACKSPACE)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    // НОВЕ слово в en-розкладці (лишилась після word1): «cd»→живий uk-префікс «св».
    platform.set_text("cd");
    platform.enqueue_all([key(C), key(D)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    let expected: Vec<Action> = ad_switch().into_iter().chain(sv_switch()).collect();
    assert_eq!(
        platform.applied_actions(),
        expected.as_slice(),
        "після backspace-до-порожнього пін знято → друге слово отримало свіч (не придушено)"
    );
    assert_eq!(platform.text(), "св");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

#[test]
fn pin_cleared_after_boundary_next_word_processed() {
    // Друге слово ПІСЛЯ успішного live-switch (word1 завершено МЕЖЕЮ, не backspace)
    // отримує нормальну обробку: межа робить `reset` → пін знято → word2 свіч.
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    // word1: «фв»→«ad», далі ОС допечатала пробіл (межа) → reset знімає пін.
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text("фв");
    platform.enqueue_all([key(A), key(D)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );
    platform.set_text("ad "); // ОС у en допечатала пробіл
    platform.enqueue_all([key(SPACE)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    // word2: «cd»→«св» (свіжий live-switch — пін word1 не пережив межу).
    platform.set_text("ad cd");
    platform.enqueue_all([key(C), key(D)]);
    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    let expected: Vec<Action> = ad_switch().into_iter().chain(sv_switch()).collect();
    assert_eq!(
        platform.applied_actions(),
        expected.as_slice(),
        "після межі word1 пін знято → word2 отримав нормальну обробку (свіч)"
    );
    assert_eq!(platform.text(), "ad св");
}

#[test]
fn both_languages_dead_end_longer_word_no_action() {
    // Обидва-глухий-кут на ДОВШОМУ слові (len 5): «yxzyx» (en) / «нчянч» (uk) —
    // не префікс у жодній мові → жодної дії на всю довжину (не лише на len 3).
    let langs = langs();
    let mut state = EngineState::default();
    let no_rules = WordRules::new();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("yxzyx");
    platform.enqueue_all([key(Y), key(X), key(Z), key(Y), key(X)]);

    run(
        &mut platform,
        &mut state,
        &langs,
        &live_cfg(),
        &no_rules,
        &NO_EXCL,
    );

    assert!(
        platform.applied_actions().is_empty(),
        "довгий обопільний глухий кут → жодної дії (actions={:?})",
        platform.applied_actions()
    );
    assert_eq!(platform.text(), "yxzyx");
}
