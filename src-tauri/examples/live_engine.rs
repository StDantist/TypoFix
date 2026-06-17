//! Демо/ДІАГНОСТИЧНИЙ харнес живого рушія (НЕ Tauri-GUI).
//!
//! ⚠️ Ставить системні хуки (перехоплює ВЕСЬ ввід) і друкує `SendInput` у вікно
//! з фокусом. **Запускати лише вручну й контрольовано** (не в CI, не наосліп).
//!
//! Два режими:
//! - звичайний — друкуй сам у порожній Notepad у НЕправильній розкладці;
//! - `-- self`  — через 2 c після старту харнес сам подає послідовність
//!   `g h b d s n` + пробіл (фізичні позиції), щоб відтворити сценарій без друку.
//!
//! ## Чому `self` подає події ВНУТРІШНЬОПРОЦЕСНО, а не через `SendInput`
//! Хук визначає `is_synthetic` ВИКЛЮЧНО за прапором `LLKHF_INJECTED`
//! (`hook.rs`), який ОС виставляє на БУДЬ-який `SendInput` — підпис
//! `INJECT_SIGNATURE` хук не читає. Тож інжектована через `SendInput` подія
//! завжди прийшла б `is_synthetic=true`, а `engine::step` синтетичні відкидає →
//! самотест не дійшов би до детектора. Тому `self` подає non-synthetic
//! `InputEvent` прямо в цикл (той самий шлях логування+`step`, реальний
//! `current_layout` від ОС) — це надійно відтворює сценарій для діагностики.
//!
//! Це приклад (`examples/`), а НЕ bin — навмисно, щоб `tauri build` його не бандлив
//! у реліз (демо-харнес у продукті не потрібен). Запуск (src-tauri — ВІДОКРЕМЛЕНИЙ
//! workspace, тож `-p typofix-app` із кореня НЕ працює) — з теки `src-tauri`:
//! ```text
//! $env:TYPOFIX_DATA_DIR = "d:\Projects\TypoFix\data"
//! cargo run --example live_engine            # звичайний
//! cargo run --example live_engine -- self    # самотест
//! ```
//! (з кореня репо — `cargo run --manifest-path src-tauri/Cargo.toml --example live_engine`).

#[cfg(windows)]
fn main() {
    use std::sync::mpsc::{channel, TryRecvError};
    use std::time::{Duration, Instant};

    use typofix_app_lib::config::AppSettings;
    use typofix_app_lib::runtime::{
        detector_config_from, exclusion_rules_from, load_language_profiles, resolved_data_dir,
    };
    use typofix_core::{step, Context, EngineState, InputEvent, KeyDir, KeyStroke, WordRules};
    use typofix_platform::Platform;
    use typofix_platform_windows::WindowsPlatform;

    /// Скільки секунд тримати рушій живим.
    const RUN_SECS: u64 = 20;
    /// VK Esc — ранній вихід.
    const VK_ESCAPE: u32 = 0x1B;
    // Структурні scancode (set 1) — межа слова незалежно від розкладки.
    const SC_SPACE: u32 = 0x39;
    const SC_ENTER: u32 = 0x1C;
    const SC_TAB: u32 = 0x0F;
    const SC_BACKSPACE: u32 = 0x0E;

    let self_test = std::env::args().any(|a| a == "self");

    let settings = AppSettings::default(); // enabled = true
    let data_dir = resolved_data_dir();
    match &data_dir {
        Some(d) => println!("Дані: {} (реальні моделі)", d.display()),
        None => println!("Дані: вбудовані зразки (TYPOFIX_DATA_DIR не задано — точність нижча)"),
    }

    let profiles = match load_language_profiles(settings.language, data_dir.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Не вдалося завантажити моделі: {e}");
            std::process::exit(1);
        }
    };
    let exclusions = exclusion_rules_from(&settings);
    let config = detector_config_from(&settings);
    let rules = WordRules::new();

    // ⚠️ Хуки ставляться тут.
    let mut platform = WindowsPlatform::new();
    let mut state = EngineState::default();

    let win = platform.active_window();
    let proc = if win.process_name.is_empty() {
        "<невідомо>".to_string()
    } else {
        win.process_name.clone()
    };
    println!(
        "Активне вікно: {proc} | розкладка(ОС): {} | мови у Context: {}",
        platform.current_layout().as_str(),
        profiles
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    if self_test {
        println!("▶ Режим SELF: через 2 c подам 'g h b d s n' + пробіл (внутрішньопроцесно).");
    } else {
        println!(
            "▶ Рушій активний {RUN_SECS}s. Esc — вихід. Перемкнись у порожній Notepad \
             і друкуй у НЕправильній розкладці (напр. 'ghbdsn' замість 'привіт')."
        );
    }

    // Канал самотесту: окремий потік подає non-synthetic події в основний цикл.
    let (test_tx, test_rx) = channel::<InputEvent>();
    if self_test {
        std::thread::Builder::new()
            .name("typofix-selftest".into())
            .spawn(move || self_inject(test_tx))
            .expect("spawn selftest thread");
    }

    // Діагностичний дзеркальний буфер слова (лише для логів межі).
    let mut diag_word: Vec<KeyStroke> = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(RUN_SECS);
    while Instant::now() < deadline {
        // Спершу самотестові події, далі — реальні з платформи.
        let event = match test_rx.try_recv() {
            Ok(ev) => Some(ev),
            Err(TryRecvError::Empty) => platform.try_next_event(),
            Err(TryRecvError::Disconnected) => platform.try_next_event(),
        };
        let Some(event) = event else {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };

        // Esc (фізичний) → ранній вихід.
        if let InputEvent::Key(k) = &event {
            if k.dir == KeyDir::Down && k.vk == VK_ESCAPE && !k.is_synthetic {
                println!("Esc — виходжу.");
                break;
            }
        }

        let os_layout = platform.current_layout();
        let ctx = Context {
            active_window: platform.active_window(),
            current_layout: os_layout.clone(),
            languages: &profiles,
            config,
            exclusions: &exclusions,
            rules: &rules,
        };

        // --- Діагностичне логування події ---
        if let InputEvent::Key(k) = &event {
            if !k.is_synthetic && k.dir == KeyDir::Down {
                let stroke = KeyStroke::from(k);
                let ch = ctx
                    .current_profile()
                    .and_then(|p| p.layout.char_at(stroke.scancode, stroke.modifiers));
                let ch_str = ch.map(|c| c.to_string()).unwrap_or_else(|| {
                    if ctx.current_profile().is_none() {
                        "∅(розкладки ОС немає серед мов!)".into()
                    } else {
                        "∅".into()
                    }
                });
                println!(
                    "key sc={:#04x} vk={:#04x} synthetic={} | поточна розкладка(ОС)={} | символ-у-поточній='{}'",
                    k.scancode,
                    k.vk,
                    k.is_synthetic,
                    os_layout.as_str(),
                    ch_str
                );

                // Дзеркалимо накопичення слова й ловимо межу для логу Decision.
                let is_structural = matches!(stroke.scancode, SC_SPACE | SC_ENTER | SC_TAB);
                if stroke.scancode == SC_BACKSPACE {
                    diag_word.pop();
                } else if is_structural {
                    log_boundary(&diag_word, &ctx, &profiles, &os_layout);
                    diag_word.clear();
                } else {
                    match ctx
                        .current_profile()
                        .and_then(|p| p.layout.char_at(stroke.scancode, stroke.modifiers))
                    {
                        Some(c) if c.is_alphabetic() || c == '\'' || c == '’' => {
                            diag_word.push(stroke);
                        }
                        // Пунктуація/цифра або немає профілю → межа слова.
                        _ => {
                            log_boundary(&diag_word, &ctx, &profiles, &os_layout);
                            diag_word.clear();
                        }
                    }
                }
            }
        } else {
            // Неклавішні події рвуть буфер.
            diag_word.clear();
        }

        // --- Реальний крок рушія + лог застосованих дій ---
        let actions = step(&mut state, event, &ctx);
        for action in &actions {
            println!("apply: {action:?}");
        }
        log_correction(&actions, &os_layout, &profiles);
        for action in &actions {
            platform.apply(action);
        }
    }

    println!("⏹ Завершено (хуки знято).");
}

/// Потік самотесту: через 2 c подає послідовність 'g h b d s n' + пробіл як
/// **non-synthetic** події (scancode set 1, фізичні позиції q-w-e-r-t-y).
#[cfg(windows)]
fn self_inject(tx: std::sync::mpsc::Sender<typofix_core::InputEvent>) {
    use std::time::Duration;
    use typofix_core::{InputEvent, KeyDir, KeyEvent, Modifiers};

    std::thread::sleep(Duration::from_secs(2));

    // (scancode, vk) для g,h,b,d,s,n та пробілу.
    let seq: [(u32, u32); 7] = [
        (0x22, 0x47), // g
        (0x23, 0x48), // h
        (0x30, 0x42), // b
        (0x20, 0x44), // d
        (0x1F, 0x53), // s
        (0x31, 0x4E), // n
        (0x39, 0x20), // space
    ];

    for (i, (scancode, vk)) in seq.into_iter().enumerate() {
        let ev = InputEvent::Key(KeyEvent {
            scancode,
            vk,
            dir: KeyDir::Down,
            modifiers: Modifiers::empty(),
            timestamp_ms: 1000 + (i as u64) * 50,
            is_synthetic: false,
            is_autorepeat: false,
        });
        if tx.send(ev).is_err() {
            return; // основний цикл завершився
        }
        std::thread::sleep(Duration::from_millis(60));
    }
}

/// Лог межі слова: інтерпретація буфера в кожній мові + рішення детектора.
#[cfg(windows)]
fn log_boundary(
    strokes: &[typofix_core::KeyStroke],
    ctx: &typofix_core::Context,
    profiles: &[typofix_core::LanguageProfile],
    os_layout: &typofix_core::LayoutId,
) {
    if strokes.is_empty() {
        return;
    }
    let interp: String = profiles
        .iter()
        .map(|p| format!("[{}:'{}']", p.id.as_str(), p.layout.interpret(strokes)))
        .collect::<Vec<_>>()
        .join(" ");

    let d = typofix_core::detector::decide(strokes, ctx);
    println!(
        "межа: буфер інтерпретовано → {interp} | поточна(ОС)={} | Decision{{ switch={}, best={}, current='{}', best_text='{}', confidence={:.3} }}",
        os_layout.as_str(),
        d.switch,
        d.best.as_str(),
        d.current_text,
        d.best_text,
        d.confidence
    );
}

/// Дружній підсумок корекції, коли план містить перенабір.
#[cfg(windows)]
fn log_correction(
    actions: &[typofix_core::Action],
    source: &typofix_core::LayoutId,
    profiles: &[typofix_core::LanguageProfile],
) {
    use typofix_core::Action;

    let Some(corrected) = actions.iter().find_map(|a| match a {
        Action::TypeUnicode(t) => Some(t.clone()),
        _ => None,
    }) else {
        return;
    };
    let target = actions
        .iter()
        .find_map(|a| match a {
            Action::SwitchLayout(id) => Some(id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| source.clone());

    let target_layout = profiles.iter().find(|p| p.id == target).map(|p| &p.layout);
    let source_layout = profiles.iter().find(|p| p.id == *source).map(|p| &p.layout);
    let original = match (target_layout, source_layout) {
        (Some(tl), Some(sl)) => back_translate(&corrected, tl, sl).unwrap_or_else(|| "<?>".into()),
        _ => "<?>".into(),
    };

    println!(
        "[ВИПРАВЛЕНО] '{original}' → '{corrected}' (мова {}→{})",
        source.as_str(),
        target.as_str()
    );
}

/// Перекласти текст із цільової розкладки назад у вихідну через фізичні страйки.
#[cfg(windows)]
fn back_translate(
    corrected: &str,
    target: &typofix_core::Layout,
    source: &typofix_core::Layout,
) -> Option<String> {
    let mut strokes = Vec::with_capacity(corrected.chars().count());
    for ch in corrected.chars() {
        strokes.push(target.stroke_for(ch)?);
    }
    Some(source.interpret(&strokes))
}

#[cfg(not(windows))]
fn main() {
    eprintln!("live_engine доступний лише на Windows.");
}
