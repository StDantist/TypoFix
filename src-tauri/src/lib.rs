//! TypoFix — Tauri-оболонка.
//!
//! GUI-шар: трей-іконка з меню + вікно налаштувань, що редагує й зберігає
//! конфіг (`config.rs`). Реальної логіки розпізнавання тут НЕМАЄ — місця
//! під'єднання ядра/платформи позначено `TODO` (Фаза 5).

// pub, щоб демо-бінар `src/bin/live_engine.rs` переюзав helper'и рантайму.
pub mod config;
mod feedback;
mod hotkeys;
pub mod runtime;

use std::sync::Mutex;

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent, Wry,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use config::AppSettings;
use runtime::RuntimeManager;

/// Глобальний стан застосунку в треї: повний конфіг у пам'яті.
/// Диск — джерело істини; ця копія тримається синхронізованою при save/toggle.
#[derive(Default)]
struct AppState {
    settings: Mutex<AppSettings>,
    /// Поточна розкладка активного вікна (`"uk"`/`"en"`), яку повідомив рушій
    /// (B2-індикатор у треї). `None` = ще невідомо / на паузі.
    current_lang: Mutex<Option<String>>,
}

/// Дві версії трей-іконки (B2-індикатор): звичайна (активний) і приглушена
/// (на паузі). Будуються один раз у `setup` з вшитої іконки застосунку.
struct TrayIcons {
    active: Image<'static>,
    paused: Image<'static>,
}

const TRAY_ID: &str = "main-tray";
const SETTINGS_WINDOW: &str = "settings";

/// Подія до фронтенду: конфіг змінився ззовні форми (напр. toggle у треї).
/// Вікно налаштувань слухає й оновлює перемикач «Увімкнено».
const EVENT_SETTINGS_CHANGED: &str = "settings:changed";

/// Подія до фронтенду: стан автозапуску змінився ззовні форми (toggle у треї).
/// Payload — `bool` (увімкнено). Вікно слухає й оновлює перемикач «Автозапуск».
/// Окрема від `settings:changed`, бо автозапуск НЕ в `settings.json` — джерело
/// істини сам плагін (реєстр/LaunchAgent), тож синхронізуємо його окремо.
const EVENT_AUTOSTART_CHANGED: &str = "autostart:changed";

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

    // Автозапуск: галочка = реальний стан плагіна (реєстр/LaunchAgent).
    // Помилка читання → знято (не валимо меню). По кліку — toggle_autostart.
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        MENU_AUTOSTART,
        "Запускати разом із Windows",
        true,
        autostart_on,
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
    if let Err(err) = guard.apply(app, settings, learned_path, data_dir) {
        eprintln!("TypoFix: рушій не стартував: {err}");
    }
}

/// Людська мітка розкладки для трею: `"uk"`→`"UK"`, `"en"`→`"EN"`, інше — в upper.
fn lang_label(lang: &str) -> String {
    lang.to_uppercase()
}

/// Оновити трей: меню, ІКОНКУ (активна/приглушена) і tooltip із поточною розкладкою.
/// Tooltip: «на паузі» коли вимкнено; «активний (UK/EN)» коли працює (розкладку
/// бере з `AppState.current_lang`, яку оновлює рушій — B2-індикатор).
fn refresh_tray(app: &AppHandle, enabled: bool) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = build_tray_menu(app, enabled) {
        let _ = tray.set_menu(Some(menu));
    }

    // Іконка: активна vs приглушена (чітко видно стан без наведення).
    if let Some(icons) = app.try_state::<TrayIcons>() {
        let icon = if enabled {
            icons.active.clone()
        } else {
            icons.paused.clone()
        };
        let _ = tray.set_icon(Some(icon));
    }

    // Tooltip: пауза / активний (+ розкладка, якщо відома).
    let tip = if !enabled {
        "TypoFix — на паузі".to_string()
    } else {
        let lang = app
            .state::<AppState>()
            .current_lang
            .lock()
            .expect("AppState отруєно")
            .clone();
        match lang {
            Some(l) => format!("TypoFix — активний ({})", lang_label(&l)),
            None => "TypoFix — активний".to_string(),
        }
    };
    let _ = tray.set_tooltip(Some(&tip));
}

/// Колбек від рушія (на головному потоці через `run_on_main_thread`): рушій
/// повідомив поточну розкладку активного вікна. Зберігаємо й оновлюємо трей.
/// `pub(crate)` — викликається з `runtime::engine_loop` (лише Windows; на інших
/// цілях рушій-потоку нема, тож тут — `allow(dead_code)`).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn on_engine_layout(app: &AppHandle, lang: &str) {
    {
        let state = app.state::<AppState>();
        let mut cur = state.current_lang.lock().expect("AppState отруєно");
        if cur.as_deref() == Some(lang) {
            return; // без змін — не смикаємо трей
        }
        *cur = Some(lang.to_string());
    }
    let enabled = app
        .state::<AppState>()
        .settings
        .lock()
        .expect("AppState отруєно")
        .enabled;
    refresh_tray(app, enabled);
}

/// Зробити «приглушену» (паузну) копію іконки: знебарвити (grayscale) і зробити
/// напівпрозорою — щоб стан паузи був візуально очевидним у треї.
fn make_paused_icon(active: &Image<'_>) -> Image<'static> {
    let (w, h) = (active.width(), active.height());
    let mut rgba = active.rgba().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        // Яскравість за Rec. 601; альфу прибираємо до ~43% (приглушений вигляд).
        let gray =
            ((u32::from(px[0]) * 30 + u32::from(px[1]) * 59 + u32::from(px[2]) * 11) / 100) as u8;
        px[0] = gray;
        px[1] = gray;
        px[2] = gray;
        px[3] = (u32::from(px[3]) * 110 / 255) as u8;
    }
    Image::new_owned(rgba, w, h)
}

/// Перемкнути «увімкнено» з трею: оновити стан, зберегти на диск,
/// оновити меню й сповістити вікно налаштувань.
/// `pub(crate)` — це ж дія для хоткея «Пауза/Відновити» (`hotkeys::route`).
pub(crate) fn toggle_enabled(app: &AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut settings = state.settings.lock().expect("AppState отруєно");
        settings.enabled = !settings.enabled;
        settings.clone()
    };

    // На паузі розкладка-індикатор більше не актуальна (рушій зупиниться) —
    // скидаємо, щоб трей не показував стару мову після відновлення до емісії.
    if !snapshot.enabled {
        *state.current_lang.lock().expect("AppState отруєно") = None;
    }

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

/// Поточний прапорець «увімкнено» (для оновлення трею, коли під рукою лише `app`).
fn current_enabled(app: &AppHandle) -> bool {
    app.state::<AppState>()
        .settings
        .lock()
        .expect("AppState отруєно")
        .enabled
}

/// Перемкнути автозапуск із трею: enable/disable через плагін, оновити галочку
/// в меню й сповістити вікно налаштувань. Стан читаємо назад із плагіна (реєстр —
/// джерело істини), тож меню/UI завжди показують реальний стан, а не «бажаний».
fn toggle_autostart(app: &AppHandle) {
    let mgr = app.autolaunch();
    let now = mgr.is_enabled().unwrap_or(false);
    let res = if now { mgr.disable() } else { mgr.enable() };
    if let Err(err) = res {
        eprintln!("TypoFix: не вдалося змінити автозапуск: {err}");
        return;
    }
    // Перечитуємо фактичний стан (а не припускаємо !now) — реєстр джерело істини.
    let actual = mgr.is_enabled().unwrap_or(!now);
    refresh_tray(app, current_enabled(app));
    let _ = app.emit(EVENT_AUTOSTART_CHANGED, actual);
}

/// Команда: чи увімкнено автозапуск (читає плагін — реєстр/LaunchAgent).
/// Вікно налаштувань кличе при відкритті, щоб показати реальний стан.
#[tauri::command]
fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// Команда: увімкнути/вимкнути автозапуск. Пише через плагін, оновлює галочку в
/// треї й повертає ФАКТИЧНИЙ стан (перечитаний із плагіна — джерела істини).
#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let mgr = app.autolaunch();
    if enabled { mgr.enable() } else { mgr.disable() }.map_err(|e| e.to_string())?;
    let actual = mgr.is_enabled().map_err(|e| e.to_string())?;
    refresh_tray(&app, current_enabled(&app));
    Ok(actual)
}

/// Шлях до файлу авто-навчених винятків (поряд із `settings.json`).
fn learned_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(config::config_dir(app)?.join(runtime::LEARNED_FILE))
}

/// Команда: список авто-навчених слів (дедуп + сортування) для перегляду в UI.
/// Це слова, які користувач відкинув (Backspace/revert) — движок їх ігнорує.
/// Приватність: лише читаємо вже наявний на диску файл, нічого не шлемо.
#[tauri::command]
fn list_learned(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(runtime::learned_for_display(&learned_path(&app)?))
}

/// Команда: прибрати одне слово зі списку навчених (атомарний перезапис файлу).
/// Повертає `true`, якщо слово було. Якщо движок активний — переконфігуровуємо
/// його (`sync_runtime`), бо `learned` засівається ЛИШЕ при старті потоку: інакше
/// слово й далі ігнорувалось би до перезапуску застосунку.
#[tauri::command]
fn remove_learned(
    app: AppHandle,
    state: State<'_, AppState>,
    word: String,
) -> Result<bool, String> {
    let removed =
        runtime::remove_learned(&learned_path(&app)?, &word).map_err(|e| e.to_string())?;
    if removed {
        let settings = state.settings.lock().expect("AppState отруєно").clone();
        sync_runtime(&app, &settings);
    }
    Ok(removed)
}

/// Команда: очистити весь список навчених слів (атомарний перезапис у порожній).
/// Як і `remove_learned`, переконфігуровує активний движок, щоб скинути seed.
#[tauri::command]
fn clear_learned(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    runtime::write_learned(&learned_path(&app)?, &[]).map_err(|e| e.to_string())?;
    let settings = state.settings.lock().expect("AppState отруєно").clone();
    sync_runtime(&app, &settings);
    Ok(())
}

/// Один запис у списку запущених процесів для пікера виключень.
/// `name` — exe-ім'я (напр. `chrome.exe`), `exe_path` — повний шлях, якщо доступний,
/// `icon` — іконка exe як base64 PNG data-URL (`data:image/png;base64,…`) або `None`,
/// `has_window` — чи має застосунок видиме верхньорівневе вікно (БУДЬ-ЯКИЙ його PID).
/// Приватність: лише імена/шляхи/іконки процесів, локально; нічого не зберігаємо й не шлемо.
#[derive(Debug, Clone, serde::Serialize)]
struct ProcessEntry {
    name: String,
    exe_path: Option<String>,
    icon: Option<String>,
    has_window: bool,
}

/// Кеш іконок за exe-шляхом (процес-глобальний). Витяг через shell повільнуватий
/// (~1–2 мс/exe), а застосунків десятки — тож «Оновити список» не перевитягує вже
/// відомі. Значення `None` теж кешуємо (негативний кеш: не довбати exe без іконки).
static ICON_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
> = std::sync::OnceLock::new();

/// Іконка exe як base64 PNG data-URL. Кешується за шляхом; помилка/нема іконки → `None`
/// (без падіння). Малий розмір (32px) — легкий payload.
fn icon_for_exe(path: &str) -> Option<String> {
    let cache = ICON_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(hit) = cache.lock().expect("ICON_CACHE отруєно").get(path) {
        return hit.clone();
    }
    let icon = extract_icon_data_url(path);
    cache
        .lock()
        .expect("ICON_CACHE отруєно")
        .insert(path.to_string(), icon.clone());
    icon
}

/// Витягти іконку exe й закодувати в PNG data-URL. Будь-яка помилка → `None`.
/// На не-Windows — заглушка (витяг іконок поки лише на Windows; macOS — згодом).
#[cfg(not(windows))]
fn extract_icon_data_url(_path: &str) -> Option<String> {
    None
}

#[cfg(windows)]
fn extract_icon_data_url(path: &str) -> Option<String> {
    use base64::Engine as _;

    let png = win_icon::exe_icon_png(path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    Some(format!("data:image/png;base64,{b64}"))
}

/// Витяг іконки exe напряму через Win32. Уся unsafe-FFI ізольована тут.
///
/// Конвеєр: `SHGetFileInfoW` (асоційована іконка файлу) → `HICON` →
/// `GetIconInfo` → `GetDIBits` (32bpp top-down BGRA) → RGBA (з відновленням альфи
/// з AND-маски, якщо колірний bitmap без альфи) → PNG (`image`). Будь-яка
/// невдача/нема доступу → `None` (без падіння).
#[cfg(windows)]
mod win_icon {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    /// PNG-байти іконки exe `path` (32×32 або скільки віддасть shell) або `None`.
    pub fn exe_icon_png(path: &str) -> Option<Vec<u8>> {
        let wide: Vec<u16> = std::ffi::OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: shfi занулено; передаємо валідний NUL-термінований шлях і розмір.
        let mut shfi: SHFILEINFOW = unsafe { zeroed() };
        let ok = unsafe {
            SHGetFileInfoW(
                wide.as_ptr(),
                0,
                &mut shfi,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_SMALLICON,
            )
        };
        if ok == 0 || shfi.hIcon.is_null() {
            return None;
        }
        let png = icon_to_png(shfi.hIcon);
        // SAFETY: hIcon отримано від SHGetFileInfoW і ще валідний.
        unsafe { DestroyIcon(shfi.hIcon) };
        png
    }

    /// HICON → RGBA → PNG. Звільняє bitmap'и маски/кольору.
    fn icon_to_png(hicon: *mut c_void) -> Option<Vec<u8>> {
        // SAFETY: hicon валідний; ii занулено перед заповненням.
        let mut ii: ICONINFO = unsafe { zeroed() };
        if unsafe { GetIconInfo(hicon, &mut ii) } == 0 {
            return None;
        }
        let result = render(ii.hbmColor, ii.hbmMask);
        // SAFETY: обидва bitmap'и створено GetIconInfo — наша відповідальність звільнити.
        unsafe {
            if !ii.hbmColor.is_null() {
                DeleteObject(ii.hbmColor);
            }
            if !ii.hbmMask.is_null() {
                DeleteObject(ii.hbmMask);
            }
        }
        result
    }

    fn render(hbm_color: *mut c_void, hbm_mask: *mut c_void) -> Option<Vec<u8>> {
        if hbm_color.is_null() {
            return None; // монохромні (лише маска) не показуємо — рідкість.
        }
        // Розміри з колірного bitmap.
        let mut bmp: BITMAP = unsafe { zeroed() };
        let got = unsafe {
            GetObjectW(
                hbm_color,
                size_of::<BITMAP>() as i32,
                (&mut bmp as *mut BITMAP).cast(),
            )
        };
        if got == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
            return None;
        }
        let w = bmp.bmWidth;
        let h = bmp.bmHeight;
        let px = (w as usize).checked_mul(h as usize)?;

        // SAFETY: екранний DC; звільняємо нижче.
        let hdc = unsafe { GetDC(std::ptr::null_mut()) };
        if hdc.is_null() {
            return None;
        }
        let color = dib_32(hdc, hbm_color, w, h, px);
        // Альфу беремо з колірного bitmap; якщо вся 0 — відновлюємо з AND-маски.
        let mask = dib_32(hdc, hbm_mask, w, h, px);
        // SAFETY: hdc отримано GetDC(null).
        unsafe { ReleaseDC(std::ptr::null_mut(), hdc) };

        let color = color?;
        let mut rgba = vec![0u8; px * 4];
        let mut any_alpha = false;
        for i in 0..px {
            let b = color[i * 4];
            let g = color[i * 4 + 1];
            let r = color[i * 4 + 2];
            let a = color[i * 4 + 3];
            if a != 0 {
                any_alpha = true;
            }
            rgba[i * 4] = r;
            rgba[i * 4 + 1] = g;
            rgba[i * 4 + 2] = b;
            rgba[i * 4 + 3] = a;
        }
        if !any_alpha {
            // Класична іконка без альфи: AND-маска (ненульовий піксель = прозоро).
            match &mask {
                Some(m) if !hbm_mask.is_null() => {
                    for i in 0..px {
                        let transparent = m[i * 4] != 0 || m[i * 4 + 1] != 0 || m[i * 4 + 2] != 0;
                        rgba[i * 4 + 3] = if transparent { 0 } else { 255 };
                    }
                }
                _ => {
                    for i in 0..px {
                        rgba[i * 4 + 3] = 255;
                    }
                }
            }
        }

        encode_png(w as u32, h as u32, rgba)
    }

    /// Прочитати bitmap у 32bpp top-down BGRA-буфер через GetDIBits.
    fn dib_32(hdc: *mut c_void, hbm: *mut c_void, w: i32, h: i32, px: usize) -> Option<Vec<u8>> {
        if hbm.is_null() {
            return None;
        }
        let mut bi: BITMAPINFO = unsafe { zeroed() };
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // від'ємна → top-down (рядки зверху вниз)
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        };
        let mut buf = vec![0u8; px * 4];
        // SAFETY: buf достатнього розміру (px*4); bi коректно ініціалізовано.
        let lines = unsafe {
            GetDIBits(
                hdc,
                hbm,
                0,
                h as u32,
                buf.as_mut_ptr().cast::<c_void>(),
                &mut bi,
                DIB_RGB_COLORS,
            )
        };
        (lines != 0).then_some(buf)
    }

    fn encode_png(w: u32, h: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let mut png: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        Some(png)
    }
}

/// Множина PID-ів, що мають **видиме верхньорівневе вікно з заголовком** (не
/// tool-window). Через Win32 `EnumWindows`. Дає змогу пікеру ховати фонові/системні
/// процеси без вікон. На не-Windows — порожня множина (фільтр просто нічого не ховає).
#[cfg(not(windows))]
fn window_pids() -> std::collections::HashSet<u32> {
    std::collections::HashSet::new()
}

#[cfg(windows)]
fn window_pids() -> std::collections::HashSet<u32> {
    use std::collections::HashSet;

    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId,
        IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    // Колбек EnumWindows: збирає PID-и видимих вікон із заголовком (не tool-window).
    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam — вказівник на наш HashSet<u32> (переданий нижче); живий
        // увесь час синхронного EnumWindows.
        let set = unsafe { &mut *(lparam as *mut HashSet<u32>) };
        // Видиме + має заголовок (відсіює фонові host-вікна без тексту).
        if unsafe { IsWindowVisible(hwnd) } != 0 && unsafe { GetWindowTextLengthW(hwnd) } > 0 {
            // Відсіюємо tool-windows (службові палітри/тултіпи — не «застосунки»).
            let ex = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
            if ex & WS_EX_TOOLWINDOW == 0 {
                let mut pid: u32 = 0;
                unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                if pid != 0 {
                    set.insert(pid);
                }
            }
        }
        1 // TRUE — продовжувати перелік
    }

    let mut set: HashSet<u32> = HashSet::new();
    // SAFETY: collect валідний; передаємо &mut set вказівником, він живе до повернення.
    unsafe {
        EnumWindows(Some(collect), (&mut set as *mut HashSet<u32>) as LPARAM);
    }
    set
}

/// Команда: перелік ЗАРАЗ ЗАПУЩЕНИХ процесів, **дедуплікований за exe-іменем**
/// (один запис на застосунок, не на кожен PID), відсортований за іменем.
/// Працює в межах `core:default` (як `load_settings`) — нового permission не треба.
#[tauri::command]
fn list_running_processes() -> Result<Vec<ProcessEntry>, String> {
    use std::collections::HashMap;

    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};

    // Оновлюємо ЛИШЕ процеси (без CPU/RAM/дисків) — дешевше й точно під задачу.
    // `with_exe` обов'язковий: без нього sysinfo НЕ заповнює exe-шлях (а отже й
    // канонічне ім'я з file_name, і витяг іконки).
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
    );

    // PID-и з видимими вікнами — збираємо один раз, далі позначаємо записи.
    let win_pids = window_pids();

    // Дедуп за нормалізованим (lowercase) exe-іменем. Якщо в одного імені кілька
    // PID-ів — лишаємо перший, але «доповнюємо» шляхом, якщо раніше його не було.
    let mut by_name: HashMap<String, ProcessEntry> = HashMap::new();
    for proc in sys.processes().values() {
        // Канонічне exe-ім'я: беремо з file_name шляху (повне, із розширенням),
        // інакше — name() (на Windows це вже повне ім'я процесу).
        let exe_path = proc.exe().map(|p| p.to_string_lossy().into_owned());
        let name = proc
            .exe()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| proc.name().to_string_lossy().into_owned());
        if name.is_empty() {
            continue;
        }

        let has_window = win_pids.contains(&proc.pid().as_u32());

        let key = name.to_lowercase();
        by_name
            .entry(key)
            .and_modify(|e| {
                // Доповнюємо шлях/іконку, якщо раніше не було (інший PID того ж exe).
                if e.exe_path.is_none() {
                    e.exe_path = exe_path.clone();
                }
                if e.icon.is_none() {
                    e.icon = exe_path.as_deref().and_then(icon_for_exe);
                }
                // Достатньо, щоб БУДЬ-ЯКИЙ PID цього exe мав вікно.
                e.has_window |= has_window;
            })
            .or_insert_with(|| {
                let icon = exe_path.as_deref().and_then(icon_for_exe);
                ProcessEntry {
                    name,
                    exe_path,
                    icon,
                    has_window,
                }
            });
    }

    let mut entries: Vec<ProcessEntry> = by_name.into_values().collect();
    // Стабільний, передбачуваний порядок для UI (регістронезалежно за іменем).
    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(entries)
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
    // Перереєструвати хоткеї під нові прив'язки (увімк./вимк./зміна акселератора).
    hotkeys::apply(&app, &cleaned);
    Ok(cleaned)
}

/// Точка входу застосунку. Викликається з `main.rs`.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(hotkeys::plugin())
        // Автозапуск (B5). Args=None: застосунок і так стартує прихованим у трей,
        // окремих launch-аргументів не потребує. macOS — через LaunchAgent.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .manage(AppState::default())
        .manage(Mutex::new(RuntimeManager::default()))
        .manage(hotkeys::HotkeyRegistry::default())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            list_running_processes,
            get_autostart,
            set_autostart,
            list_learned,
            remove_learned,
            clear_learned
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            // Завантажуємо конфіг із диска в стан (перший запуск → дефолти).
            let initial = config::load_from_disk(&handle).unwrap_or_default();
            let enabled = initial.enabled;
            *app.state::<AppState>()
                .settings
                .lock()
                .expect("AppState отруєно") = initial.clone();

            // Дві версії трей-іконки (B2-індикатор): активна = вшита іконка
            // застосунку; паузна = її приглушена (grayscale+прозоріша) копія.
            // Будуємо ВЛАСНІ (`new_owned`) → `Image<'static>` (інакше клон лишається
            // позиченим від `App` і не може зберігатись у стані).
            let active_icon = {
                let src = app
                    .default_window_icon()
                    .expect("default window icon має бути вшита через bundle.icon");
                Image::new_owned(src.rgba().to_vec(), src.width(), src.height())
            };
            let paused_icon = make_paused_icon(&active_icon);
            app.manage(TrayIcons {
                active: active_icon.clone(),
                paused: paused_icon.clone(),
            });

            // Піднімаємо рушій, якщо застосунок увімкнено (на паузі — нічого).
            // Рушій одразу запланує (через головний потік) емісію поточної
            // розкладки → on_engine_layout оновить трей ПІСЛЯ його побудови нижче.
            sync_runtime(&handle, &initial);

            // Реєструємо глобальні хоткеї з конфіга (незалежно від enabled —
            // інакше неможливо було б відновити роботу з клавіатури).
            hotkeys::apply(&handle, &initial);

            let menu = build_tray_menu(&handle, enabled)?;

            // Початкові іконка/тултип за станом enabled; розкладку додасть рушій.
            let icon = if enabled { active_icon } else { paused_icon };
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
            MENU_AUTOSTART => toggle_autostart(app),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_running_processes_returns_deduped_sorted_nonempty() {
        // Не потребує GUI/хуків — лише читає таблицю процесів ОС.
        let list = list_running_processes().expect("перелік процесів");
        // Цей тест-процес сам запущений → список не порожній.
        assert!(!list.is_empty(), "очікували хоча б один процес");

        // Дедуп за exe-іменем (регістронезалежно): без повторів.
        let mut keys: Vec<String> = list.iter().map(|p| p.name.to_lowercase()).collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(before, keys.len(), "імена мають бути унікальні (дедуп)");

        // Відсортовано за іменем (регістронезалежно).
        let names: Vec<String> = list.iter().map(|p| p.name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "список має бути відсортований за іменем");

        // Жодного порожнього імені.
        assert!(list.iter().all(|p| !p.name.is_empty()));

        // Іконки: ті, що є — валідні PNG data-URL.
        assert!(list
            .iter()
            .filter_map(|p| p.icon.as_deref())
            .all(|d| d.starts_with("data:image/png;base64,")));
    }

    #[test]
    fn icons_are_extracted_for_most_processes_and_are_fast() {
        use std::time::Instant;

        // Холодний прогін (кеш порожній) — це верхня межа за часом.
        let t0 = Instant::now();
        let list = list_running_processes().expect("перелік процесів");
        let cold = t0.elapsed();

        let with_path = list.iter().filter(|p| p.exe_path.is_some()).count();
        let with_icon = list.iter().filter(|p| p.icon.is_some()).count();
        println!(
            "процесів={} з_шляхом={} з_іконкою={} холодний_витяг={:?}",
            list.len(),
            with_path,
            with_icon,
            cold
        );

        // Теплий прогін (кеш заповнено) має бути помітно швидшим.
        let t1 = Instant::now();
        let _ = list_running_processes().expect("перелік процесів (теплий)");
        let warm = t1.elapsed();
        println!("теплий_витяг={warm:?}");

        // Більшість процесів із доступним exe-шляхом мають віддати іконку.
        if with_path > 0 {
            assert!(
                with_icon * 2 >= with_path,
                "очікували іконки хоча б у половини процесів зі шляхом: {with_icon}/{with_path}"
            );
        }
    }

    #[test]
    fn window_pids_consistent_with_has_window_flag() {
        let wp = window_pids();
        let list = list_running_processes().expect("перелік процесів");
        let has_win = list.iter().filter(|p| p.has_window).count();
        println!("window_pids={} записів_has_window={}", wp.len(), has_win);

        // У headless-середовищі вікон може не бути (0) — тоді нічого не стверджуємо.
        // Якщо ж вікна Є, то хоча б один застосунок має позначитись has_window.
        if !wp.is_empty() {
            assert!(
                has_win >= 1,
                "є window-PID-и, але жодного запису з has_window"
            );
        }
        // has_window-записів не може бути більше, ніж усіх записів.
        assert!(has_win <= list.len());
    }
}
