//! Демо-харнес ЖИВОГО рушія (НЕ Tauri-GUI) для контрольованої перевірки.
//!
//! ⚠️ Ставить системні хуки (перехоплює ВЕСЬ ввід) і друкує `SendInput` у вікно
//! з фокусом. **Запускати лише вручну й контрольовано** (не в CI, не наосліп).
//!
//! Що робить: будує `Context` із РЕАЛЬНИМИ моделями (uk+en, через
//! `TYPOFIX_DATA_DIR`), піднімає `WindowsPlatform`, ганяє цикл рушія ~20 c
//! (вихід раніше по Esc), логуючи кожну корекцію. По завершенні `Drop` платформи
//! знімає хуки.
//!
//! Запуск (з кореня репо):
//! ```text
//! $env:TYPOFIX_DATA_DIR = "d:\Projects\TypoFix\data"
//! cargo run -p typofix-app --bin live_engine
//! ```
//! Без `TYPOFIX_DATA_DIR` — fallback на слабкі вбудовані зразки.

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};

    use typofix_app_lib::config::AppSettings;
    use typofix_app_lib::runtime::{
        detector_config_from, exclusion_rules_from, load_language_profiles, resolved_data_dir,
    };
    use typofix_core::{step, Context, EngineState, InputEvent, KeyDir, WordRules};
    use typofix_platform::Platform;
    use typofix_platform_windows::WindowsPlatform;

    /// Скільки секунд тримати рушій живим.
    const RUN_SECS: u64 = 20;
    /// VK Esc — ранній вихід.
    const VK_ESCAPE: u32 = 0x1B;

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
        "Активне вікно: {proc} | розкладка: {}",
        platform.current_layout().as_str()
    );
    println!(
        "▶ Рушій активний {RUN_SECS}s. Esc — вихід. Перемкнись у порожній Notepad \
         і друкуй у НЕправильній розкладці (напр. 'ghbdsn' замість 'привіт')."
    );

    let deadline = Instant::now() + Duration::from_secs(RUN_SECS);
    while Instant::now() < deadline {
        let Some(event) = platform.try_next_event() else {
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

        let source_layout = platform.current_layout();
        let ctx = Context {
            active_window: platform.active_window(),
            current_layout: source_layout.clone(),
            languages: &profiles,
            config,
            exclusions: &exclusions,
            rules: &rules,
        };

        let actions = step(&mut state, event, &ctx);
        log_correction(&actions, &source_layout, &profiles);

        for action in &actions {
            platform.apply(action);
        }
    }

    println!("⏹ Завершено (хуки знято).");
}

/// Розпізнати в плані дій перенабір і надрукувати зрозумілий рядок корекції.
#[cfg(windows)]
fn log_correction(
    actions: &[typofix_core::Action],
    source: &typofix_core::LayoutId,
    profiles: &[typofix_core::LanguageProfile],
) {
    use typofix_core::Action;

    // Корекція = є TypeUnicode (готовий перенабраний текст).
    let Some(corrected) = actions.iter().find_map(|a| match a {
        Action::TypeUnicode(t) => Some(t.clone()),
        _ => None,
    }) else {
        return;
    };

    // Цільова мова — зі SwitchLayout, якщо є; інакше лишається поточна.
    let target = actions
        .iter()
        .find_map(|a| match a {
            Action::SwitchLayout(id) => Some(id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| source.clone());

    // «Що було» реконструюємо: символи перенабору → їхні фізичні страйки в
    // ЦІЛЬОВІЙ розкладці → інтерпретація тих самих страйків у ВИХІДНІЙ.
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
/// `None`, якщо якийсь символ не має страйка в цільовій розкладці.
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
