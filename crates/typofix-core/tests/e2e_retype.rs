//! E2E-міра Фази 3: наскрізний перенабір через `VirtualPlatform`.
//!
//! Сценарій (як у реального користувача): розкладка лишилась **en**, людина
//! фізично набрала клавіші слова «привіт» → на екрані опинилось «ghbdsn». На
//! межі слова ядро має розпізнати українську, стерти крякозябри й перенабрати
//! правильний текст. Virtual не «друкує» клавіші сам — текст змінюють лише наші
//! [`Action`] (саме тому крякозябри ставимо через `set_text`, імітуючи те, що
//! ОС уже надрукувала). Деталі моделі — `typofix-platform-virtual/CLAUDE.md`.

use typofix_core::{
    step, Context, DetectorConfig, EngineState, ExclusionRules, LanguageProfile, LayoutId,
    WordRules,
};
use typofix_platform::{InputEvent, KeyDir, KeyEvent, Modifiers, Platform, WindowInfo};
use typofix_platform_virtual::{drive, VirtualPlatform};

static NO_EXCL: ExclusionRules = ExclusionRules::new();
static NO_RULES: WordRules = WordRules::new();

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

/// Зібрати профіль мови із вбудованих зразків `typofix-data` (розкладка + LM + словник).
fn profile(id: &str) -> LanguageProfile {
    LanguageProfile {
        id: LayoutId::new(id),
        layout: typofix_data::embedded_layout(id).expect("розкладка"),
        lm: typofix_data::sample_lm(id).expect("LM"),
        dict: typofix_data::sample_dict(id).expect("словник"),
    }
}

/// Прогнати чергу подій платформи через справжній `core::step` з даними мов.
fn run(platform: &mut VirtualPlatform, langs: &[LanguageProfile]) {
    run_with(platform, langs, &NO_EXCL);
}

/// Те саме, але із заданими виключеннями (для тесту bypass теки).
fn run_with(
    platform: &mut VirtualPlatform,
    langs: &[LanguageProfile],
    exclusions: &ExclusionRules,
) {
    let mut state = EngineState::default();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions,
            rules: &NO_RULES,
        };
        step(&mut state, ev, &ctx)
    });
}

// Фізичні позиції (scancode set 1) для слова «привіт» / «ghbdsn».
const G: u32 = 0x22;
const H: u32 = 0x23;
const B: u32 = 0x30;
const D: u32 = 0x20;
const S: u32 = 0x1F;
const N: u32 = 0x31;
const O: u32 = 0x18;
const SPACE: u32 = 0x39;
const BACKSPACE: u32 = 0x0E;

/// Прогнати чергу через `core::step`, тримаючи переданий `state` між викликами
/// (для багатоетапних сценаріїв самонавчання).
fn drive_state(platform: &mut VirtualPlatform, state: &mut EngineState, langs: &[LanguageProfile]) {
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        };
        step(state, ev, &ctx)
    });
}

fn typed_privit() -> [InputEvent; 7] {
    [key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]
}

#[test]
fn ghbdsn_in_en_becomes_privit() {
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en")); // користувач застряг у en
    platform.set_text("ghbdsn"); // те, що ОС уже надрукувала
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "привіт",
        "крякозябри мали перетворитись на привіт"
    );
    assert_eq!(
        platform.current_layout(),
        LayoutId::new("uk"),
        "розкладку мали перемкнути на uk для подальшого набору"
    );
}

#[test]
fn applied_actions_are_delete_switch_type() {
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);

    run(&mut platform, &langs);

    use typofix_platform::Action;
    assert_eq!(
        platform.applied_actions(),
        [
            Action::DeleteChars(6),
            Action::SwitchLayout(LayoutId::new("uk")),
            Action::TypeUnicode("привіт".into()),
        ]
    );
}

#[test]
fn real_english_word_is_left_untouched() {
    let langs = [profile("uk"), profile("en")];

    // w o r l d → en "world" (реальне слово); поточна розкладка en → не чіпати.
    const W: u32 = 0x11;
    const R: u32 = 0x13;
    const L: u32 = 0x26;

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("world");
    platform.enqueue_all([key(W), key(O), key(R), key(L), key(D), key(SPACE)]);

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "world",
        "реальне англ. слово чіпати не можна"
    );
    assert_eq!(platform.current_layout(), LayoutId::new("en"));
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn short_ambiguous_word_is_not_switched() {
    let langs = [profile("uk"), profile("en")];

    // Двосимвольне слово: занадто коротке/неоднозначне → за сумніву не перемикати.
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("go");
    platform.enqueue_all([key(G), key(O), key(SPACE)]);

    run(&mut platform, &langs);

    assert_eq!(platform.text(), "go", "коротке слово не перемикати");
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn mouse_click_invalidates_buffer_no_retype() {
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    // Усе слово набрано, але до межі стався клік — він рве зв'язок буфера з
    // текстом перед курсором, тож перенабору бути НЕ повинно (інакше стерли б
    // чужий текст). Без кліку цей самий ввід дав би "привіт".
    platform.enqueue_all([
        key(G),
        key(H),
        key(B),
        key(D),
        key(S),
        key(N),
        InputEvent::MouseClick,
        key(SPACE),
    ]);

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "ghbdsn",
        "після кліку перенабору бути не повинно"
    );
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn excluded_folder_bypasses_then_works_when_removed() {
    let langs = [profile("uk"), profile("en")];

    // Гра запущена з виключеної теки C:\Games → жодного перенабору.
    let game_window = WindowInfo {
        process_name: "game.exe".into(),
        exe_path: r"C:\Games\Cool\game.exe".into(),
        is_fullscreen: false,
    };

    let mut excl = ExclusionRules::new();
    excl.exclude_folder(r"C:\Games");

    let mut platform = VirtualPlatform::new();
    platform.set_window(game_window.clone());
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);

    run_with(&mut platform, &langs, &excl);

    assert_eq!(
        platform.text(),
        "ghbdsn",
        "у виключеній теці чіпати не можна"
    );
    assert!(platform.applied_actions().is_empty());

    // Те саме вікно, але БЕЗ виключення → перенабір спрацьовує.
    let mut platform = VirtualPlatform::new();
    platform.set_window(game_window);
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "привіт",
        "без виключення — звичайний перенабір"
    );
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}

// --- Самонавчання (undo) ---------------------------------------------------

#[test]
fn rejection_emits_commit_exception_and_learns_word() {
    use typofix_platform::Action;
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all(typed_privit());
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(platform.text(), "привіт", "перенабір мав статися");

    // Користувач негайно тисне Backspace → відкидання перенабору.
    platform.enqueue(key(BACKSPACE));
    drive_state(&mut platform, &mut state, &langs);

    assert!(
        state.learned.contains("ghbdsn"),
        "слово мало стати навченим"
    );
    assert_eq!(
        platform.applied_actions().last(),
        Some(&Action::CommitException("ghbdsn".into())),
        "мала емітитись CommitException з оригінальним словом"
    );
}

#[test]
fn learned_word_is_not_switched() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();
    state.learned.learn("ghbdsn"); // ніби завантажено з персистентного сховища

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all(typed_privit());
    drive_state(&mut platform, &mut state, &langs);

    assert_eq!(platform.text(), "ghbdsn", "навчене слово не перемикати");
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn rejected_word_is_not_reswitched_on_second_appearance() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    // 1) Перша поява → перенабір.
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all(typed_privit());
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(platform.text(), "привіт");

    // 2) Негайне відкидання.
    platform.enqueue(key(BACKSPACE));
    drive_state(&mut platform, &mut state, &langs);
    assert!(state.learned.contains("ghbdsn"));

    // 3) Друга поява того самого слова (користувач знову в en) → НЕ чіпати.
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all(typed_privit());
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(
        platform.text(),
        "ghbdsn",
        "після відкидання друга поява лишається незмінною"
    );
}

#[test]
fn accepting_retype_does_not_learn() {
    use typofix_platform::Action;
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn");
    platform.enqueue_all(typed_privit());
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(platform.text(), "привіт");

    // Замість Backspace користувач просто продовжує набір (приймає перенабір).
    platform.enqueue(key(0x1E)); // будь-яка літера
    drive_state(&mut platform, &mut state, &langs);

    assert!(state.learned.is_empty(), "прийнятий перенабір не вчиться");
    assert!(
        !platform
            .applied_actions()
            .iter()
            .any(|a| matches!(a, Action::CommitException(_))),
        "CommitException не має емітитись при прийнятті"
    );
}
