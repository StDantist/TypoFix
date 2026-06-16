//! TypoFix — Tauri-оболонка.
//!
//! GUI-шар: трей-іконка з меню + вікно налаштувань, що редагує й зберігає
//! конфіг (`config.rs`). Реальної логіки розпізнавання тут НЕМАЄ — місця
//! під'єднання ядра/платформи позначено `TODO` (Фаза 5).

// pub, щоб демо-бінар `src/bin/live_engine.rs` переюзав helper'и рантайму.
pub mod config;
pub mod runtime;

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent, Wry,
};

use config::AppSettings;
use runtime::RuntimeManager;

/// Глобальний стан застосунку в треї: повний конфіг у пам'яті.
/// Диск — джерело істини; ця копія тримається синхронізованою при save/toggle.
#[derive(Default)]
struct AppState {
    settings: Mutex<AppSettings>,
}

const TRAY_ID: &str = "main-tray";
const SETTINGS_WINDOW: &str = "settings";

/// Подія до фронтенду: конфіг змінився ззовні форми (напр. toggle у треї).
/// Вікно налаштувань слухає й оновлює перемикач «Увімкнено».
const EVENT_SETTINGS_CHANGED: &str = "settings:changed";

// Ідентифікатори пунктів меню.
const MENU_STATUS: &str = "status";
const MENU_TOGGLE: &str = "toggle_enabled";
const MENU_SETTINGS: &str = "open_settings";
const MENU_AUTOSTART: &str = "toggle_autostart";
const MENU_QUIT: &str = "quit";

/// Показати (і сфокусувати) вікно налаштувань. Воно завжди існує — лише ховається.
fn show_settings(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(SETTINGS_WINDOW) {
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Зібрати трей-меню для поточного стану (увімкнено/пауза).
/// Перебудовуємо повністю при кожній зміні — меню маленьке, це дешево.
fn build_tray_menu(app: &AppHandle, enabled: bool) -> tauri::Result<Menu<Wry>> {
    let status_label = if enabled {
        "● Статус: активний"
    } else {
        "● Статус: на паузі"
    };
    // Рядок статусу неактивний (disabled) — це індикатор, не кнопка.
    let status = MenuItem::with_id(app, MENU_STATUS, status_label, false, None::<&str>)?;

    let toggle_label = if enabled {
        "Пауза"
    } else {
        "Відновити"
    };
    let toggle = MenuItem::with_id(app, MENU_TOGGLE, toggle_label, true, None::<&str>)?;

    let settings = MenuItem::with_id(
        app,
        MENU_SETTINGS,
        "Відкрити налаштування…",
        true,
        None::<&str>,
    )?;

    // TODO(autostart): реальний toggle через tauri-plugin-autostart.
    let autostart = MenuItem::with_id(
        app,
        MENU_AUTOSTART,
        "Автозапуск при вході (TODO)",
        true,
        None::<&str>,
    )?;

    let quit = MenuItem::with_id(app, MENU_QUIT, "Вихід", true, None::<&str>)?;

    Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &settings,
            &autostart,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )
}

/// Знайти корінь каталогу даних (`layouts/`, `lm/`, `dicts/`) для standalone-запуску,
/// щоб застосунок працював подвійним кліком БЕЗ `TYPOFIX_DATA_DIR`. Порядок кандидатів
/// = пріоритет:
/// 1. `TYPOFIX_DATA_DIR` — явний override (dev/демо).
/// 2. `resource_dir()/data` — ресурси бандла (`cargo tauri build`, `bundle.resources`).
/// 3. `data` поряд з `.exe` + у предків шляху — портативний режим і dev-білд
///    `cargo build --release` (exe у `src-tauri/target/release/` → предок-репо має `data/`).
///
/// Жодного кандидата → `None` → вбудовані зразки (працює «з коробки», але слабше).
fn resolve_data_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    // 1) Явний env-override має найвищий пріоритет.
    if let Some(dir) = runtime::resolved_data_dir() {
        return Some(dir);
    }

    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // 2) Ресурси бандла (Tauri копіює сюди `data/` при `tauri build`).
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join("data"));
    }

    // 3) Поряд з .exe і вгору по предках (портативний zip / dev release-білд).
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1) {
            candidates.push(ancestor.join("data"));
        }
    }

    runtime::find_data_dir(candidates)
}

/// Привести рантайм-цикл рушія у відповідність до налаштувань (старт/стоп/рестарт).
/// Помилки не валять застосунок — лише лог; GUI лишається живим.
fn sync_runtime(app: &AppHandle, settings: &AppSettings) {
    let learned_path = match config::config_dir(app) {
        Ok(dir) => dir.join(runtime::LEARNED_FILE),
        Err(err) => {
            eprintln!("TypoFix: немає каталогу для навчених винятків: {err}");
            return;
        }
    };
    let manager = app.state::<Mutex<RuntimeManager>>();
    let mut guard = manager.lock().expect("RuntimeManager отруєно");
    // Самостійний пошук моделей (env → ресурси бандла → поряд з .exe); інакше зразки.
    let data_dir = resolve_data_dir(app);
    if let Err(err) = guard.apply(settings, learned_path, data_dir) {
        eprintln!("TypoFix: рушій не стартував: {err}");
    }
}

/// Оновити трей-меню й tooltip під поточний `enabled`.
fn refresh_tray(app: &AppHandle, enabled: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, enabled) {
            let _ = tray.set_menu(Some(menu));
        }
        let tip = if enabled {
            "TypoFix — активний"
        } else {
            "TypoFix — на паузі"
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

/// Перемкнути «увімкнено» з трею: оновити стан, зберегти на диск,
/// оновити меню й сповістити вікно налаштувань.
fn toggle_enabled(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut settings = state.settings.lock().expect("AppState отруєно");
        settings.enabled = !settings.enabled;
        settings.clone()
    };

    // TODO: тут під'єднати реальну паузу/відновлення hot-path хука (платформа).

    if let Err(err) = config::save_to_disk(app, &snapshot) {
        // Не валимо застосунок через помилку диска — лише лог у stderr.
        eprintln!("TypoFix: не вдалося зберегти конфіг із трею: {err}");
    }
    refresh_tray(app, snapshot.enabled);
    // Старт/стоп рушія під новий стан (пауза знімає хуки).
    sync_runtime(app, &snapshot);
    // Сповіщаємо фронтенд (повним конфігом — вікно вирішить, що оновити).
    let _ = app.emit(EVENT_SETTINGS_CHANGED, snapshot);
}

/// Команда: прочитати конфіг із диска (джерело істини) й оновити in-memory.
#[tauri::command]
fn load_settings(app: AppHandle, state: State<'_, AppState>) -> Result<AppSettings, String> {
    let settings = config::load_from_disk(&app)?;
    *state.settings.lock().expect("AppState отруєно") = settings.clone();
    Ok(settings)
}

/// Команда: зберегти конфіг із форми. Валідуємо, пишемо на диск, оновлюємо
/// in-memory й трей. Повертаємо очищену версію (форма синхронізується).
#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let cleaned = settings.sanitized();
    config::save_to_disk(&app, &cleaned)?;
    *state.settings.lock().expect("AppState отруєно") = cleaned.clone();
    refresh_tray(&app, cleaned.enabled);
    // Перебудувати рантайм-цикл під нові виключення/детектор/мову.
    sync_runtime(&app, &cleaned);
    Ok(cleaned)
}

/// Точка входу застосунку. Викликається з `main.rs`.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .manage(Mutex::new(RuntimeManager::default()))
        .invoke_handler(tauri::generate_handler![load_settings, save_settings])
        .setup(|app| {
            let handle = app.handle().clone();

            // Завантажуємо конфіг із диска в стан (перший запуск → дефолти).
            let initial = config::load_from_disk(&handle).unwrap_or_default();
            let enabled = initial.enabled;
            *app.state::<AppState>()
                .settings
                .lock()
                .expect("AppState отруєно") = initial.clone();

            // Піднімаємо рушій, якщо застосунок увімкнено (на паузі — нічого).
            sync_runtime(&handle, &initial);

            let menu = build_tray_menu(&handle, enabled)?;

            // Іконку трею беремо з вшитої іконки застосунку (bundle.icon).
            let icon = app
                .default_window_icon()
                .cloned()
                .expect("default window icon має бути вшита через bundle.icon");

            let tooltip = if enabled {
                "TypoFix — активний"
            } else {
                "TypoFix — на паузі"
            };

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .tooltip(tooltip)
                .menu(&menu)
                // Меню — лише за правим кліком; лівий клік відкриває налаштування.
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_settings(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_TOGGLE => toggle_enabled(app),
            MENU_SETTINGS => show_settings(app),
            MENU_AUTOSTART => {
                // TODO(autostart): під'єднати tauri-plugin-autostart enable/disable.
            }
            MENU_QUIT => {
                // Коректно знімаємо хуки перед виходом.
                app.state::<Mutex<RuntimeManager>>()
                    .lock()
                    .expect("RuntimeManager отруєно")
                    .shutdown();
                app.exit(0);
            }
            _ => {}
        })
        .on_window_event(|window, event| {
            // Закриття вікна налаштувань = приховати його, а не виходити.
            // Застосунок живе у треї.
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == SETTINGS_WINDOW {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("помилка під час запуску TypoFix");
}
