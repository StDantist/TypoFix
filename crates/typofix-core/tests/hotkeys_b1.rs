//! E2E гарячих клавіш B1 через `VirtualPlatform`: ручний відкат останнього
//! перенабору (`revert_last`) і примусове перемикання останнього слова в обхід
//! порогу впевненості (`force_switch_last`). Як і в `e2e_retype`, текст міняють
//! ЛИШЕ наші [`Action`]; крякозябри ставимо через `set_text`, імітуючи ОС.

use typofix_core::{
    force_switch_last, revert_last, step, Context, DetectorConfig, EngineState, ExclusionRules,
    LanguageProfile, LayoutId, WordRules,
};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers, Platform, WindowInfo};
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

fn profile(id: &str) -> LanguageProfile {
    LanguageProfile {
        id: LayoutId::new(id),
        layout: typofix_data::embedded_layout(id).expect("розкладка"),
        lm: typofix_data::sample_lm(id).expect("LM"),
        dict: typofix_data::sample_dict(id).expect("словник"),
        freq: None,
    }
}

/// Прогнати чергу подій платформи через `core::step`, тримаючи `state` між
/// викликами.
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

/// Побудувати `Context` зі знімка платформи (для прямого виклику API гарячих
/// клавіш поза циклом подій).
fn ctx_from<'a>(platform: &VirtualPlatform, langs: &'a [LanguageProfile]) -> Context<'a> {
    Context {
        active_window: platform.current_window(),
        current_layout: platform.current_layout(),
        languages: langs,
        config: DetectorConfig::default(),
        exclusions: &NO_EXCL,
        rules: &NO_RULES,
    }
}

/// Застосувати план дій до платформи (як це робить движок поза `drive`).
fn apply_all(platform: &mut VirtualPlatform, actions: &[Action]) {
    for a in actions {
        platform.apply(a);
    }
}

// Фізичні позиції (scancode set 1).
const G: u32 = 0x22;
const H: u32 = 0x23;
const B: u32 = 0x30;
const D: u32 = 0x20;
const S: u32 = 0x1F;
const N: u32 = 0x31;
const O: u32 = 0x18;
const SPACE: u32 = 0x39;

// --- revert_last -----------------------------------------------------------

#[test]
fn revert_restores_exact_original_and_learns_word() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    // Спершу нормальний авто-перенабір: "ghbdsn " → "привіт " (uk).
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn ");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(platform.text(), "привіт ", "перенабір мав статися");
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));

    // Користувач тисне гарячу клавішу відкату.
    let actions = revert_last(&mut state);
    apply_all(&mut platform, &actions);

    // Екран повернувся РІВНО до оригіналу (слово + пробіл), розкладка — стара en.
    assert_eq!(
        platform.text(),
        "ghbdsn ",
        "відкат має відновити точний оригінал"
    );
    assert_eq!(
        platform.current_layout(),
        LayoutId::new("en"),
        "відкат повертає стару розкладку"
    );
    // Дії відкату: стерти 7, повернути en, надрукувати оригінал, завчити слово.
    assert_eq!(
        actions,
        vec![
            Action::DeleteChars(7),
            Action::SwitchLayout(LayoutId::new("en")),
            Action::TypeUnicode("ghbdsn ".into()),
            Action::CommitException("ghbdsn".into()),
        ]
    );
    // Слово завчене → апка більше його не чіпатиме.
    assert!(
        state.learned.contains("ghbdsn"),
        "слово мало стати навченим"
    );
    // Вікно очікування закрите (повторний відкат — порожній).
    assert!(
        revert_last(&mut state).is_empty(),
        "повторний відкат — no-op"
    );
}

#[test]
fn revert_with_nothing_pending_is_noop() {
    let mut state = EngineState::default();
    assert!(
        revert_last(&mut state).is_empty(),
        "немає чого відкочувати → порожній план"
    );
    assert!(state.learned.is_empty());
}

#[test]
fn reverted_word_is_not_reswitched_on_next_appearance() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    // Перенабір, потім ручний відкат → слово завчене.
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn ");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);
    drive_state(&mut platform, &mut state, &langs);
    apply_all(&mut platform, &revert_last(&mut state));
    assert!(state.learned.contains("ghbdsn"));

    // Друга поява того самого слова → НЕ чіпати (learned-veto).
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("ghbdsn ");
    platform.enqueue_all([key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]);
    drive_state(&mut platform, &mut state, &langs);
    assert_eq!(
        platform.text(),
        "ghbdsn ",
        "після відкату слово лишається незмінним"
    );
}

// --- force_switch_last -----------------------------------------------------

#[test]
fn force_switch_ignores_threshold_on_short_uncertain_word() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    // "go" — коротке/неоднозначне: АВТОМАТИЧНО не перемикається (контроль нижче).
    // Набираємо БЕЗ роздільника → слово ще в буфері (поточне).
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("go");
    platform.enqueue_all([key(G), key(O)]);
    drive_state(&mut platform, &mut state, &langs);
    // Авто нічого не зробив (межі не було) — текст недоторканий.
    assert_eq!(platform.text(), "go");

    // Ручне примусове перемикання — в обхід порогу.
    let ctx = ctx_from(&platform, &langs);
    let actions = force_switch_last(&mut state, &ctx);
    apply_all(&mut platform, &actions);

    // g→п, o→щ у uk-розкладці; перемкнулось попри коротке/невпевнене слово.
    assert_eq!(
        platform.text(),
        "пщ",
        "force має перемкнути навіть коротке/невпевнене слово"
    );
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
    assert_eq!(
        actions,
        vec![
            Action::DeleteChars(2),
            Action::SwitchLayout(LayoutId::new("uk")),
            Action::TypeUnicode("пщ".into()),
        ]
    );

    // Контроль: те саме слово з роздільником АВТОМАТИЧНО не перемикається.
    let mut state2 = EngineState::default();
    let mut p2 = VirtualPlatform::new();
    p2.set_layout(LayoutId::new("en"));
    p2.set_text("go ");
    p2.enqueue_all([key(G), key(O), key(SPACE)]);
    drive_state(&mut p2, &mut state2, &langs);
    assert_eq!(p2.text(), "go ", "контроль: авто не чіпає коротке слово");
}

#[test]
fn force_switch_can_be_reverted() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("go");
    platform.enqueue_all([key(G), key(O)]);
    drive_state(&mut platform, &mut state, &langs);

    let ctx = ctx_from(&platform, &langs);
    apply_all(&mut platform, &force_switch_last(&mut state, &ctx));
    assert_eq!(platform.text(), "пщ");

    // Ручне перемикання теж відкочується (pending виставлено).
    apply_all(&mut platform, &revert_last(&mut state));
    assert_eq!(platform.text(), "go", "відкат ручного перемикання");
    assert_eq!(platform.current_layout(), LayoutId::new("en"));
    assert!(state.learned.contains("go"));
}

#[test]
fn force_switch_with_empty_buffer_is_noop() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    // Жодного набору → буфер порожній.
    let ctx = ctx_from(&platform, &langs);
    let actions = force_switch_last(&mut state, &ctx);
    assert!(actions.is_empty(), "порожній буфер → нема що перемикати");
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn force_switch_with_only_current_language_is_noop() {
    // Лише поточна мова серед профілів → немає іншої → no-op.
    let langs = [profile("en")];
    let mut state = EngineState::default();

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("go");
    platform.enqueue_all([key(G), key(O)]);
    drive_state(&mut platform, &mut state, &langs);

    let ctx = ctx_from(&platform, &langs);
    assert!(
        force_switch_last(&mut state, &ctx).is_empty(),
        "немає іншої мови → нема куди перемикати"
    );
}

#[test]
fn force_switch_in_excluded_window_is_noop() {
    let langs = [profile("uk"), profile("en")];
    let mut state = EngineState::default();

    let game = WindowInfo {
        process_name: "game.exe".into(),
        exe_path: r"C:\Games\game.exe".into(),
        is_fullscreen: false,
    };
    let mut excl = ExclusionRules::new();
    excl.exclude_folder(r"C:\Games");

    let mut platform = VirtualPlatform::new();
    platform.set_window(game);
    platform.set_layout(LayoutId::new("en"));
    platform.set_text("go");
    platform.enqueue_all([key(G), key(O)]);
    // Прокрутимо буфер напряму (вікно виключене — step його б не буферив, тож
    // будуємо ctx із виключенням і перевіряємо bypass самого force_switch).
    drive(&mut platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: &langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL, // буфер наповнюємо без виключення
            rules: &NO_RULES,
        };
        step(&mut state, ev, &ctx)
    });

    let ctx = Context {
        active_window: platform.current_window(),
        current_layout: platform.current_layout(),
        languages: &langs,
        config: DetectorConfig::default(),
        exclusions: &excl, // тепер вікно виключене
        rules: &NO_RULES,
    };
    assert!(
        force_switch_last(&mut state, &ctx).is_empty(),
        "у виключеному вікні force не діє"
    );
}
