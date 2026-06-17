//! Приватність (залізне правило №4): у **секретному** (пароль) полі ядро НЕ сміє
//! ні буферити натиски, ні перемикати розкладку. Репро власника: введення пароля
//! до RAR-архіву (діалог WinRAR) перемикало розкладку — бо детекція секретних
//! полів була не реалізована.
//!
//! Тут — герметичний E2E через `VirtualPlatform`: те саме слово, що В НОРМІ
//! перемкнулося б (`ghbdsn`→`привіт`), у секретному полі лишається недоторканим
//! і НІЧОГО не накопичується в буфері; контроль (поле НЕ секретне) доводить, що
//! саме гейт `secure` — причина, а не якась інша умова.

use typofix_core::{
    step, Context, DetectorConfig, EngineState, ExclusionRules, LanguageProfile, LayoutId,
    WordRules,
};
use typofix_platform::{InputEvent, KeyDir, KeyEvent, Modifiers, Platform};
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

// Фізичні позиції (scancode set 1) для слова «привіт»/«ghbdsn».
const G: u32 = 0x22;
const H: u32 = 0x23;
const B: u32 = 0x30;
const D: u32 = 0x20;
const S: u32 = 0x1F;
const N: u32 = 0x31;
const SPACE: u32 = 0x39;

/// Прогнати чергу через `core::step`, беручи `secure` зі стану платформи
/// (`is_secure_field`) — так само, як це робить `runtime.rs` щокроку.
fn run(platform: &mut VirtualPlatform, langs: &[LanguageProfile]) -> EngineState {
    let mut state = EngineState::default();
    // `secure` сталий на час сценарію → знімаємо ОДИН раз перед циклом (усередині
    // замикання `drive` платформа вже позичена мутабельно).
    let secure = platform.is_secure_field();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
            secure,
        };
        step(&mut state, ev, &ctx)
    });
    state
}

fn typed_privit() -> [InputEvent; 7] {
    [key(G), key(H), key(B), key(D), key(S), key(N), key(SPACE)]
}

#[test]
fn secure_field_never_switches_and_buffers_nothing() {
    let langs = [profile("uk"), profile("en")];

    // Поле пароля: користувач набирає те, що зазвичай дало б «привіт».
    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_secure(true);
    platform.set_text("ghbdsn "); // на екрані вже надруковане (хук пропускає далі)
    platform.enqueue_all(typed_privit());

    let state = run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "ghbdsn ",
        "у секретному полі перенабору бути НЕ повинно"
    );
    assert_eq!(
        platform.current_layout(),
        LayoutId::new("en"),
        "розкладку не чіпаємо в полі пароля"
    );
    assert!(
        platform.applied_actions().is_empty(),
        "жодної дії: {:?}",
        platform.applied_actions()
    );
    // Приватність: у пам'яті ядра нічого не лишилось про набране в полі пароля.
    assert!(
        EngineState::default() == state,
        "стан ядра має лишатись порожнім (буфер не накопичено)"
    );
}

#[test]
fn same_word_switches_when_field_is_not_secure() {
    // Контроль причинності: те саме слово при `secure=false` ПЕРЕМИКАЄТЬСЯ —
    // отже саме гейт `secure`, а не інша умова, блокує перенабір вище.
    let langs = [profile("uk"), profile("en")];

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_secure(false);
    platform.set_text("ghbdsn ");
    platform.enqueue_all(typed_privit());

    run(&mut platform, &langs);

    assert_eq!(
        platform.text(),
        "привіт ",
        "поза секретним полем — звичайний перенабір"
    );
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
}
