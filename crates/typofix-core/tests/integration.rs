//! Integration-тест **плумбінгу** (не детекції): прогоняє потік подій через
//! `drive` + справжній `typofix_core::step` поверх `VirtualPlatform`.
//!
//! ⚠️ `step` зараз — заглушка (повертає порожній план), тож тут ми перевіряємо
//! проводку: події тягнуться з платформи, контекст будується з її стану, дії
//! застосовуються назад. Реальний сценарій «ghbdsn → привіт» зʼявиться у
//! Фазі 2-3, коли ядро навчиться розпізнавати.

use typofix_core::{step, Context, DetectorConfig, EngineState};
use typofix_platform::{InputEvent, KeyDir, KeyEvent, LayoutId, Modifiers, Platform, WindowInfo};
use typofix_platform_virtual::{drive, VirtualPlatform};

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

/// Обгортка, що повертає замикання-крок поверх `core::step`, тримаючи
/// `EngineState` між викликами і будуючи `Context` зі знімка платформи.
fn run_engine(platform: &mut VirtualPlatform) {
    let mut state = EngineState::default();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: &[],
            config: DetectorConfig::default(),
        };
        step(&mut state, ev, &ctx)
    });
}

#[test]
fn skeleton_engine_drains_events_without_mutating_text() {
    let mut platform = VirtualPlatform::new();
    platform.set_text("заздалегідь");
    platform.enqueue_all([
        key(0x1E),
        key(0x30),
        InputEvent::MouseClick,
        InputEvent::CaretMove,
    ]);

    run_engine(&mut platform);

    // Скелетний step нічого не робить: текст недоторканий, черга вичерпана,
    // жодної дії не застосовано.
    assert_eq!(platform.text(), "заздалегідь");
    assert_eq!(platform.pending_events(), 0);
    assert!(platform.applied_actions().is_empty());
}

#[test]
fn focus_change_carries_window_into_context() {
    let mut platform = VirtualPlatform::new();
    let editor = WindowInfo {
        process_name: "editor.exe".into(),
        exe_path: r"C:\editor.exe".into(),
        is_fullscreen: false,
    };
    platform.set_window(editor.clone());
    platform.set_layout(LayoutId::new("uk"));
    platform.enqueue(InputEvent::FocusChange(editor.clone()));

    run_engine(&mut platform);

    // Перевіряємо, що проводка дійшла до кінця і стан платформи стабільний.
    assert_eq!(platform.current_window(), editor);
    assert_eq!(platform.current_layout(), LayoutId::new("uk"));
    assert_eq!(platform.pending_events(), 0);
}

#[test]
fn driver_is_reusable_across_runs() {
    let mut platform = VirtualPlatform::new();
    platform.enqueue(key(0x1E));
    run_engine(&mut platform);
    assert_eq!(platform.pending_events(), 0);

    // Другий прогін на тій самій платформі також коректно вичерпує чергу.
    platform.enqueue_all([key(0x30), key(0x2E)]);
    run_engine(&mut platform);
    assert_eq!(platform.pending_events(), 0);
}
