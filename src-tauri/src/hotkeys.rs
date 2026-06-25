//! Глобальні гарячі клавіші (B1): реєстрація з конфіга + роутинг у дії застосунку.
//!
//! ## Як це влаштовано
//! - Плагін `tauri-plugin-global-shortcut` реєструється з єдиним handler'ом
//!   ([`plugin`]). Handler спрацьовує на КОЖНИЙ зареєстрований хоткей; за фактичним
//!   `Shortcut` шукаємо дію у [`HotkeyRegistry`] і роутимо ([`route`]).
//! - [`apply`] перереєстровує хоткеї під поточний конфіг: знімає ВСІ попередні
//!   (`unregister_all`) і ставить заново лише `enabled` прив'язки з валідним
//!   акселератором. Викликається в `setup` і після кожного `save_settings`.
//! - Хоткеї живуть незалежно від `enabled` (пауза/активний): інакше не можна було б
//!   ВІДНОВИТИ роботу з клавіатури. Пауза — це окрема дія двигуна, не зняття хоткеїв.
//!
//! ## Роутинг дій
//! - `PauseResume` → [`crate::toggle_enabled`] (інверсія `enabled`; пауза/відновлення
//!   не потребують активного рушія, тож ідуть прямо, не через канал).
//! - `RevertLast` / `ManualSwitch` / `CaseUpper`/`Lower`/`Sentence` → команда в
//!   потік рушія через [`RuntimeManager::send_command`] (`EngineCommand`). Рушій
//!   виконує її на своїх `EngineState`+платформі (хендлер не має до них доступу).
//!   Якщо рушій НЕ активний (пауза/`enabled=false`) — `send_command` повертає
//!   `false`, дія тихо ігнорується (revert/manual/case без движка не мають сенсу).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use typofix_core::CaseMode;

use crate::config::{AppSettings, HotkeyAction};
use crate::runtime::{EngineCommand, RuntimeManager};

/// Реєстр активних хоткеїв: відображення зареєстрованого `Shortcut` → дія.
/// Тримається в Tauri-стані за `Mutex`; handler плагіна шукає тут дію за тим
/// `Shortcut`, що спрацював.
#[derive(Default)]
pub struct HotkeyRegistry {
    map: Mutex<HashMap<Shortcut, HotkeyAction>>,
}

/// Плагін глобальних хоткеїв із роутинг-handler'ом. Підключається у `run()`.
/// Сам нічого не реєструє — конкретні хоткеї ставить [`apply`] із конфіга.
pub fn plugin() -> tauri::plugin::TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            // Реагуємо лише на натиск (handler кличеться і на відпускання).
            if event.state != ShortcutState::Pressed {
                return;
            }
            let action = {
                let registry = app.state::<HotkeyRegistry>();
                let map = registry.map.lock().expect("HotkeyRegistry отруєно");
                map.get(shortcut).copied()
            };
            if let Some(action) = action {
                route(app, action);
            }
        })
        .build()
}

/// Перереєструвати хоткеї під поточний конфіг: зняти всі попередні й поставити
/// заново лише `enabled` прив'язки з валідним акселератором. Помилки не валять
/// застосунок — лише лог (невалідний/зайнятий акселератор просто не активується).
pub fn apply(app: &AppHandle, settings: &AppSettings) {
    let gs = app.global_shortcut();
    // Знімаємо все й будуємо набір з нуля — простіше й надійніше, ніж діфати.
    if let Err(err) = gs.unregister_all() {
        eprintln!("TypoFix: не вдалося зняти попередні хоткеї: {err}");
    }
    let registry = app.state::<HotkeyRegistry>();
    let mut map = registry.map.lock().expect("HotkeyRegistry отруєно");
    map.clear();

    for action in HotkeyAction::ALL {
        let binding = settings.hotkeys.binding(action);
        if !binding.enabled {
            continue;
        }
        let accel = binding.accelerator.trim();
        if accel.is_empty() {
            continue;
        }
        let shortcut = match Shortcut::from_str(accel) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("TypoFix: некоректний акселератор {accel:?} ({action:?}): {err}");
                continue;
            }
        };
        if let Err(err) = gs.register(shortcut) {
            eprintln!("TypoFix: не вдалося зареєструвати {accel:?} ({action:?}): {err}");
            continue;
        }
        map.insert(shortcut, action);
    }
}

/// Виконати дію, прив'язану до хоткея.
fn route(app: &AppHandle, action: HotkeyAction) {
    match action {
        // Та сама логіка, що й трей-пункт «Пауза/Відновити»: інвертує `enabled`,
        // пише на диск, оновлює трей і емітить `settings:changed`. Не через канал —
        // пауза має працювати й коли рушій зупинено (щоб ВІДНОВИТИ роботу).
        HotkeyAction::PauseResume => crate::toggle_enabled(app),
        // Решта — команда в потік рушія (він володіє state+платформою).
        HotkeyAction::RevertLast => send(app, EngineCommand::RevertLast),
        HotkeyAction::ManualSwitch => send(app, EngineCommand::ManualSwitch),
        HotkeyAction::CaseUpper => send(app, EngineCommand::ApplyCase(CaseMode::Upper)),
        HotkeyAction::CaseLower => send(app, EngineCommand::ApplyCase(CaseMode::Lower)),
        HotkeyAction::CaseSentence => send(app, EngineCommand::ApplyCase(CaseMode::Sentence)),
        // Додати виділене слово у словник правил: резолюція (виділення + flip для
        // «always») — у потоці рушія, бо там platform+languages; персист — на app-боці.
        HotkeyAction::AlwaysSwitchSelection => {
            send(app, EngineCommand::AddSelectionWord { to_always: true })
        }
        HotkeyAction::NeverSwitchSelection => {
            send(app, EngineCommand::AddSelectionWord { to_always: false })
        }
    }
}

/// Надіслати команду рушієві. Якщо рушій не активний (пауза/вимкнено) —
/// `send_command` повертає `false`, дію тихо ігноруємо (лише лог у stdout).
fn send(app: &AppHandle, cmd: EngineCommand) {
    let manager = app.state::<Mutex<RuntimeManager>>();
    let sent = manager
        .lock()
        .expect("RuntimeManager отруєно")
        .send_command(cmd);
    if !sent {
        println!("TypoFix: {cmd:?} проігноровано — рушій не активний (пауза/вимкнено)");
    }
}
