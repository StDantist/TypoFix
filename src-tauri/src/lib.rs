//! TypoFix — Tauri-оболонка (скелет).
//!
//! Це лише GUI-каркас: трей-іконка з меню + приховуване вікно налаштувань.
//! Реальної логіки розпізнавання тут НЕМАЄ — її під'єднають пізніше
//! (`typofix-core` + платформні крейти) у місцях, позначених `TODO`.

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent, Wry,
};

/// Глобальний стан застосунку в треї. Поки що — лише прапорець паузи.
#[derive(Default)]
struct AppState {
    /// `true` → розпізнавання на паузі (ще не під'єднано до ядра).
    paused: Mutex<bool>,
}

const TRAY_ID: &str = "main-tray";
const SETTINGS_WINDOW: &str = "settings";

// Ідентифікатори пунктів меню.
const MENU_STATUS: &str = "status";
const MENU_TOGGLE: &str = "toggle_pause";
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

/// Зібрати трей-меню для поточного стану паузи.
/// Перебудовуємо повністю при кожній зміні стану — меню маленьке, це дешево.
fn build_tray_menu(app: &AppHandle, paused: bool) -> tauri::Result<Menu<Wry>> {
    let status_label = if paused {
        "● Статус: на паузі"
    } else {
        "● Статус: активний"
    };
    // Рядок статусу неактивний (disabled) — це індикатор, не кнопка.
    let status = MenuItem::with_id(app, MENU_STATUS, status_label, false, None::<&str>)?;

    let toggle_label = if paused {
        "Відновити"
    } else {
        "Пауза"
    };
    let toggle = MenuItem::with_id(app, MENU_TOGGLE, toggle_label, true, None::<&str>)?;

    let settings = MenuItem::with_id(
        app,
        MENU_SETTINGS,
        "Відкрити налаштування…",
        true,
        None::<&str>,
    )?;

    // TODO(autostart): зробити реальний toggle через tauri-plugin-autostart.
    // Поки що пункт лише показує намір; стан не зберігається.
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

/// Перемкнути паузу й оновити меню/підказку трею.
fn toggle_pause(app: &AppHandle) {
    let state = app.state::<AppState>();
    let now = {
        let mut paused = state.paused.lock().expect("AppState.paused отруєно");
        *paused = !*paused;
        *paused
    };

    // TODO: тут під'єднати реальну паузу/відновлення hot-path хука (платформа).

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_tray_menu(app, now) {
            let _ = tray.set_menu(Some(menu));
        }
        let tip = if now {
            "TypoFix — на паузі"
        } else {
            "TypoFix — активний"
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

/// Точка входу застосунку. Викликається з `main.rs`.
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            let menu = build_tray_menu(&handle, false)?;

            // Іконку трею беремо з вшитої іконки застосунку (bundle.icon).
            let icon = app
                .default_window_icon()
                .cloned()
                .expect("default window icon має бути вшита через bundle.icon");

            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .tooltip("TypoFix — активний")
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
            MENU_TOGGLE => toggle_pause(app),
            MENU_SETTINGS => show_settings(app),
            MENU_AUTOSTART => {
                // TODO(autostart): під'єднати tauri-plugin-autostart enable/disable.
            }
            MENU_QUIT => app.exit(0),
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
