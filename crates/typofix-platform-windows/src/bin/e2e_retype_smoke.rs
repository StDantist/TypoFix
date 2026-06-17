//! **Real-OS наскрізний e2e** ланцюга авто-виправлення TypoFix — АВТОНОМНИЙ.
//!
//! Доводить увесь стек НАЖИВО: реальна `WindowsPlatform` (LL-хуки) → реальні
//! мовні профілі з `typofix-data` → `typofix_core::step` → реальний перенабір
//! через `Action` у справжнє вікно Notepad. Той самий движковий стек, що в
//! `src-tauri::runtime::engine_loop`, але без Tauri.
//!
//! ## ⚠️ Фундаментальне обмеження (НЕ обхід, а коректна поведінка)
//! Хук визначає синтетичність ВИКЛЮЧНО за `LLKHF_INJECTED` (`hook.rs`), а Windows
//! ставить цей прапор на БУДЬ-який `SendInput`. Тож user-mode-«ввід» завжди
//! приходить у `step` як `is_synthetic=true`, і ядро його ІГНОРУЄ — це і є
//! анти-цикл (інакше наш перенабір перенабирав би себе). Симулювати «фізичний»
//! ввід, який хук вважає за людський, з user-mode НЕМОЖЛИВО (потрібен kernel-
//! драйвер). Тому харнес:
//!   1) РЕАЛЬНО інжектить фізичні scancode у Notepad (`SendInput`) — на екрані
//!      зʼявляється кирилична каша; хук їх ЗАХОПЛЮЄ → доводимо, що вони
//!      `is_synthetic=true` (capture-path працює, анти-цикл тримається);
//!   2) годує `step` ТИМИ Ж захопленими від ОС scancode, але `is_synthetic=false`
//!      (єдине, що підміняємо) з реальним `Context` (uk/en профілі);
//!   3) застосовує отримані `Action` РЕАЛЬНО через платформу (switch+retype);
//!   4) звіряє вміст Notepad (`WM_GETTEXT`) — каша стала правильним словом.
//!
//! Усе, крім походження `LLKHF_INJECTED`, — справжнє й наскрізне.
//!
//! ## Запуск (потребує GUI-сесії + встановлені uk і en розкладки)
//! `cargo run -p typofix-platform-windows --features e2e-harness --bin e2e_retype_smoke`
//! Дані: `TYPOFIX_DATA_DIR` або `data/` репозиторію (інакше слабші вбудовані зразки).
//! Код виходу: 0 — усі кейси зелені; 1 — провал / середовище без GUI чи розкладок.

#[cfg(all(windows, feature = "e2e-harness"))]
fn main() {
    std::process::exit(win::run());
}

#[cfg(all(windows, feature = "e2e-harness"))]
mod win {
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::time::{Duration, Instant};

    use typofix_core::{
        step, Action, Context, DetectorConfig, EngineState, ExclusionRules, FrequencyMap,
        InputEvent, KeyDir, KeyEvent, LanguageProfile, LayoutId, Modifiers, WordRules,
    };
    use typofix_platform::Platform;
    use typofix_platform_windows::{
        current_layout_id, foreground_window_info, installed_layout_ids, WindowsPlatform,
    };

    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, WPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        VK_CONTROL, VK_DELETE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetGUIThreadInfo, GetWindowTextW,
        GetWindowThreadProcessId, IsWindowVisible, SendMessageW, SetForegroundWindow, ShowWindow,
        GUITHREADINFO, SW_RESTORE, WM_GETTEXT, WM_GETTEXTLENGTH,
    };

    // Set-1 scancode фізичних QWERTY-клавіш (те, що дає реальна клавіатура).
    const SC_SPACE: u16 = 0x39;
    const VK_A: u16 = 0x41;

    /// Один e2e-кейс: фізичні клавіші у вихідній розкладці → очікуваний результат.
    struct Case {
        title: &'static str,
        /// Розкладка ОС під час «набору» (в ній фізичні клавіші → каша).
        source_lang: &'static str,
        /// Очікувана розкладка ПІСЛЯ авто-перемикання.
        target_lang: &'static str,
        /// Фізичні scancode слова (без пробілу — пробіл додаємо як межу).
        scancodes: &'static [u16],
        /// Що має опинитися у вікні після перенабору (зі завершальним пробілом).
        expected: &'static str,
    }

    pub fn run() -> i32 {
        println!("=== TypoFix e2e_retype_smoke (real-OS наскрізний ланцюг) ===\n");

        // (1) Профілі мов із реальних даних (як runtime::load_language_profiles).
        let data_root = resolve_data_root();
        match &data_root {
            Some(d) => println!("[setup] дані: {}", d.display()),
            None => println!("[setup] дані: вбудовані зразки (data/ не знайдено)"),
        }
        let profiles = match load_profiles(data_root.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("СЕРЕДОВИЩЕ: не вдалося завантажити профілі мов: {e}");
                return 1;
            }
        };
        println!(
            "[setup] профілі: {:?}",
            profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>()
        );

        // (2) Перевірити, що ВСТАНОВЛЕНІ і uk, і en (інакше тест неможливий).
        let installed = installed_layout_ids();
        let has = |lang: &str| installed.iter().any(|i| i.as_str() == lang);
        println!(
            "[setup] встановлені розкладки: {:?}",
            installed.iter().map(|i| i.as_str()).collect::<Vec<_>>()
        );
        if !has("uk") || !has("en") {
            eprintln!(
                "СЕРЕДОВИЩЕ без потрібних розкладок (треба uk І en серед встановлених) — \
                 тест неможливий. Не фейкую результат."
            );
            return 1;
        }

        // (3) Запустити Notepad і вивести foreground (AttachThreadInput-обхід).
        let mut child = match std::process::Command::new("notepad.exe").spawn() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("СЕРЕДОВИЩЕ: не вдалося запустити notepad.exe: {e}");
                return 1;
            }
        };
        let forced = force_foreground_notepad(Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(400));
        let fg = foreground_window_info().process_name;
        println!("[setup] foreground: {fg:?} (примусово={forced})");
        if !fg.to_lowercase().contains("notepad") {
            eprintln!(
                "СЕРЕДОВИЩЕ: Notepad не активний (foreground={fg:?}) — немає інтерактивного \
                 робочого столу? Перериваю чесно."
            );
            let _ = child.kill();
            kill_all_notepad();
            return 1;
        }

        // Запамʼятати вихідну розкладку, щоб відновити наприкінці.
        let original_layout = current_layout_id();
        println!(
            "[setup] вихідна розкладка: {:?}\n",
            original_layout.as_str()
        );

        // ⚠️ Ставить системні LL-хуки на весь час життя (як engine_loop).
        let mut platform = WindowsPlatform::new();
        std::thread::sleep(Duration::from_millis(300));

        let cases = [
            Case {
                title: "EN-слово у UK-розкладці (каша → hello)",
                source_lang: "uk",
                target_lang: "en",
                // h e l l o
                scancodes: &[0x23, 0x12, 0x26, 0x26, 0x18],
                expected: "hello ",
            },
            Case {
                title: "UK-слово в EN-розкладці (ghbdsn → привіт)",
                source_lang: "en",
                target_lang: "uk",
                // g h b d s n
                scancodes: &[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31],
                expected: "привіт ",
            },
        ];

        let mut all_ok = true;
        for case in &cases {
            let ok = run_case(&mut platform, &profiles, case);
            all_ok &= ok;
        }

        // (8) Прибирання: відновити вихідну розкладку, зняти хуки, вбити Notepad.
        switch_layout_and_wait(
            &mut platform,
            original_layout.as_str(),
            Duration::from_secs(2),
        );
        drop(platform); // зняти хуки
        let _ = child.kill();
        kill_all_notepad();
        println!("\n[cleanup] розкладку відновлено, хуки знято, Notepad вбито.");

        println!(
            "\n=== ПІДСУМОК: {} ===",
            if all_ok {
                "✅ усі кейси пройшли наскрізно"
            } else {
                "❌ є провали (див. вище)"
            }
        );
        i32::from(!all_ok)
    }

    /// Прогнати один кейс; повернути `true`, якщо перенабір дав очікуване.
    fn run_case(platform: &mut WindowsPlatform, profiles: &[LanguageProfile], case: &Case) -> bool {
        println!("--- Кейс: {} ---", case.title);

        // Вивести ОС у вихідну розкладку (серед встановлених, НЕ інсталюємо).
        if !switch_layout_and_wait(platform, case.source_lang, Duration::from_secs(3)) {
            eprintln!(
                "  ❌ не вдалося перемкнути ОС на {} (поточна {:?})",
                case.source_lang,
                current_layout_id().as_str()
            );
            return false;
        }
        // Чистимо вікно (Ctrl+A + Delete), щоб не накопичувати з минулого кейса.
        clear_notepad();
        std::thread::sleep(Duration::from_millis(200));

        // Спорожнити канал хука перед набором — лишити тільки наші майбутні події.
        drain_events(platform);

        // (5) РЕАЛЬНО інжектимо фізичні scancode + пробіл (НЕ підписані; хук
        // побачить LLKHF_INJECTED, але це найближче до фізичної клавіатури).
        let mut sc: Vec<u16> = case.scancodes.to_vec();
        sc.push(SC_SPACE);
        type_scancodes_physical(&sc);
        std::thread::sleep(Duration::from_millis(350));

        // Зібрати, що захопив хук. Доводимо: усі — is_synthetic=true (анти-цикл).
        let captured = collect_key_events(platform);
        let synth_all = !captured.is_empty() && captured.iter().all(|k| k.is_synthetic);
        let captured_sc: Vec<u16> = captured.iter().map(|k| k.scancode as u16).collect();
        println!(
            "  хук захопив {} key-down, scancodes={:02x?}, усі synthetic={}",
            captured.len(),
            captured_sc,
            synth_all
        );
        if captured.len() != sc.len() || !synth_all || captured_sc != sc {
            eprintln!(
                "  ❌ хук не захопив очікувані події (очікував {:02x?}) — перериваю кейс",
                sc
            );
            return false;
        }

        // (6) Годуємо ядро ТИМИ Ж scancode від ОС, але is_synthetic=false.
        // Context — реальний: активне вікно + поточна розкладка + профілі.
        let excl = ExclusionRules::new();
        let rules = WordRules::new();
        let active_window = platform.active_window();
        let mut state = EngineState::default();
        let mut actions: Vec<Action> = Vec::new();
        let mut core_time = Duration::ZERO;

        for (i, k) in captured.iter().enumerate() {
            let ev = InputEvent::Key(KeyEvent {
                scancode: k.scancode,
                vk: k.vk,
                dir: KeyDir::Down,
                modifiers: Modifiers::empty(),
                timestamp_ms: (i as u64) * 30,
                is_synthetic: false, // ← єдина підміна (див. обмеження у шапці)
                is_autorepeat: false,
            });
            let ctx = Context {
                active_window: active_window.clone(),
                current_layout: LayoutId::new(case.source_lang),
                languages: profiles,
                config: DetectorConfig::default(),
                exclusions: &excl,
                rules: &rules,
            };
            let t0 = Instant::now();
            let out = step(&mut state, ev, &ctx);
            core_time += t0.elapsed();
            if !out.is_empty() {
                actions = out;
            }
        }

        println!(
            "  core::step видав {} дій за {:.3} мс: {}",
            actions.len(),
            core_time.as_secs_f64() * 1000.0,
            describe_actions(&actions)
        );
        if actions.is_empty() {
            eprintln!("  ❌ ядро не вирішило перемикати — детекція не спрацювала");
            return false;
        }

        // (7) Застосувати дії РЕАЛЬНО (switch layout + Unicode-перенабір у Notepad).
        let apply_start = Instant::now();
        for a in &actions {
            platform.apply(a);
        }
        // Дати ОС домалювати перенабір + асинхронний switch.
        std::thread::sleep(Duration::from_millis(500));
        let apply_time = apply_start.elapsed();

        // (6/READBACK) Звірити вміст edit-контрола Notepad через WM_GETTEXT.
        let content = focus_window_text().unwrap_or_default();
        let text_ok = content == case.expected;
        println!(
            "  Notepad WM_GETTEXT = {content:?} (очікував {:?}) → {}",
            case.expected,
            verdict(text_ok)
        );

        // Опційно: перевірити, що поточна розкладка стала цільовою.
        std::thread::sleep(Duration::from_millis(200));
        let now_lang = current_layout_id();
        let layout_ok = now_lang.as_str() == case.target_lang;
        println!(
            "  розкладка після перенабору = {:?} (очікував {:?}) → {}",
            now_lang.as_str(),
            case.target_lang,
            verdict(layout_ok)
        );
        println!(
            "  ⏱ реакція ядра {:.3} мс, застосування (switch+retype+settle) {} мс\n",
            core_time.as_secs_f64() * 1000.0,
            apply_time.as_millis()
        );

        // Головний критерій — текст у вікні. Розкладка — додатковий (не блокер,
        // бо M2-зчитування для деяких вікон буває запізнілим), але звітуємо.
        text_ok
    }

    // ===================== Завантаження профілів (як runtime) =================

    fn load_profiles(data_root: Option<&Path>) -> Result<Vec<LanguageProfile>, String> {
        let layout_dir = data_root.map(|d| d.join("layouts"));
        let lm_dir = data_root.map(|d| d.join("lm"));
        let dict_dir = data_root.map(|d| d.join("dicts"));
        let mut out = Vec::new();
        for lang in ["uk", "en"] {
            let layout = typofix_data::load_layout(lang, layout_dir.as_deref())
                .map_err(|e| e.to_string())?;
            let lm = typofix_data::load_lm(lang, lm_dir.as_deref()).map_err(|e| e.to_string())?;
            let dict =
                typofix_data::load_dict(lang, dict_dir.as_deref()).map_err(|e| e.to_string())?;
            let freq = dict_dir
                .as_deref()
                .map(|d| d.join(format!("{lang}.freq.fst")))
                .filter(|p| p.exists())
                .and_then(|p| typofix_data::load_freq_map_file(&p).ok())
                .map(FrequencyMap::from_fst_map);
            out.push(LanguageProfile {
                id: LayoutId::new(lang),
                layout,
                lm,
                dict,
                freq,
            });
        }
        Ok(out)
    }

    /// Корінь `data/`: `TYPOFIX_DATA_DIR` → інакше вгору по предках від cwd.
    fn resolve_data_root() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("TYPOFIX_DATA_DIR") {
            let d = PathBuf::from(p);
            if d.join("layouts").is_dir() {
                return Some(d);
            }
        }
        let mut cur = std::env::current_dir().ok();
        while let Some(d) = cur {
            let cand = d.join("data");
            if cand.join("layouts").is_dir() {
                return Some(cand);
            }
            cur = d.parent().map(Path::to_path_buf);
        }
        None
    }

    // ===================== Платформні утиліти харнеса ==========================

    fn verdict(ok: bool) -> &'static str {
        if ok {
            "✅ PASS"
        } else {
            "❌ FAIL"
        }
    }

    fn describe_actions(actions: &[Action]) -> String {
        actions
            .iter()
            .map(|a| match a {
                Action::None => "None".to_string(),
                Action::SwitchLayout(id) => format!("SwitchLayout({})", id.as_str()),
                Action::DeleteChars(n) => format!("DeleteChars({n})"),
                Action::TypeUnicode(s) => format!("TypeUnicode({s:?})"),
                Action::CommitException(s) => format!("CommitException({s:?})"),
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Спорожнити канал подій платформи (відкинути все накопичене).
    fn drain_events(platform: &mut WindowsPlatform) {
        while platform.try_next_event().is_some() {}
    }

    /// Зібрати key-down події з каналу хука (фільтр від focus/caret-подій).
    fn collect_key_events(platform: &mut WindowsPlatform) -> Vec<KeyEvent> {
        let mut keys = Vec::new();
        while let Some(ev) = platform.try_next_event() {
            if let InputEvent::Key(k) = ev {
                keys.push(k);
            }
        }
        keys
    }

    /// Перемкнути ОС на `lang` (серед встановлених) і дочекатися підтвердження.
    fn switch_layout_and_wait(
        platform: &mut WindowsPlatform,
        lang: &str,
        timeout: Duration,
    ) -> bool {
        platform.apply(&Action::SwitchLayout(LayoutId::new(lang)));
        let deadline = Instant::now() + timeout;
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if current_layout_id().as_str() == lang {
                return true;
            }
            if Instant::now() >= deadline {
                return current_layout_id().as_str() == lang;
            }
        }
    }

    /// Зібрати keyboard-INPUT у scancode-режимі (фізична клавіша, dwExtraInfo=0 —
    /// НЕ підписуємо: хай хук бачить як «зовнішній» ввід).
    fn kbd_scan(scancode: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: scancode,
                    dwFlags: flags | KEYEVENTF_SCANCODE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    /// Keyboard-INPUT у VK-режимі (для Ctrl+A / Delete).
    fn kbd_vk(vk: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_inputs(inputs: &[INPUT]) {
        if inputs.is_empty() {
            return;
        }
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
    }

    /// Реально «натиснути» послідовність фізичних scancode (down+up на кожен).
    fn type_scancodes_physical(scancodes: &[u16]) {
        let mut inputs = Vec::with_capacity(scancodes.len() * 2);
        for &sc in scancodes {
            inputs.push(kbd_scan(sc, 0));
            inputs.push(kbd_scan(sc, KEYEVENTF_KEYUP));
        }
        send_inputs(&inputs);
    }

    /// Очистити Notepad: Ctrl+A (виділити все) → Delete.
    fn clear_notepad() {
        send_inputs(&[
            kbd_vk(VK_CONTROL, 0),
            kbd_vk(VK_A, 0),
            kbd_vk(VK_A, KEYEVENTF_KEYUP),
            kbd_vk(VK_CONTROL, KEYEVENTF_KEYUP),
        ]);
        std::thread::sleep(Duration::from_millis(80));
        send_inputs(&[kbd_vk(VK_DELETE, 0), kbd_vk(VK_DELETE, KEYEVENTF_KEYUP)]);
    }

    /// Прочитати текст фокусного edit-контрола активного вікна (`WM_GETTEXT`).
    /// Через `GetGUIThreadInfo(fgTid).hwndFocus` (справжнє вікно вводу, навіть у
    /// UWP-хості), fallback — саме foreground-вікно.
    fn focus_window_text() -> Option<String> {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return None;
            }
            let tid = GetWindowThreadProcessId(fg, ptr::null_mut());
            let mut gti: GUITHREADINFO = std::mem::zeroed();
            gti.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            let focus = if GetGUIThreadInfo(tid, &mut gti) != 0 && !gti.hwndFocus.is_null() {
                gti.hwndFocus
            } else {
                fg
            };
            let len = SendMessageW(focus, WM_GETTEXTLENGTH, 0, 0);
            if len <= 0 {
                return Some(String::new());
            }
            let cap = len as usize + 1;
            let mut buf = vec![0u16; cap];
            let got = SendMessageW(focus, WM_GETTEXT, cap as WPARAM, buf.as_mut_ptr() as LPARAM);
            if got <= 0 {
                return Some(String::new());
            }
            Some(String::from_utf16_lossy(&buf[..got as usize]))
        }
    }

    // ----- foreground Notepad (як у selection_smoke) -----

    fn force_foreground_notepad(timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(hwnd) = find_notepad_window() {
                unsafe {
                    let fg = GetForegroundWindow();
                    let target_tid = GetWindowThreadProcessId(fg, ptr::null_mut());
                    let self_tid = GetCurrentThreadId();
                    AttachThreadInput(self_tid, target_tid, 1);
                    ShowWindow(hwnd, SW_RESTORE);
                    BringWindowToTop(hwnd);
                    SetForegroundWindow(hwnd);
                    AttachThreadInput(self_tid, target_tid, 0);
                }
                std::thread::sleep(Duration::from_millis(250));
                if foreground_window_info()
                    .process_name
                    .to_lowercase()
                    .contains("notepad")
                {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    fn find_notepad_window() -> Option<HWND> {
        let mut found: Option<HWND> = None;
        unsafe {
            EnumWindows(Some(enum_proc), &mut found as *mut Option<HWND> as LPARAM);
        }
        found
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            let title = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
            if title.contains("notepad") || title.contains("блокнот") {
                let out = &mut *(lparam as *mut Option<HWND>);
                *out = Some(hwnd);
                return 0;
            }
        }
        1
    }

    fn kill_all_notepad() {
        let _ = std::process::Command::new("taskkill")
            .args(["/f", "/im", "notepad.exe"])
            .output();
    }
}

#[cfg(all(windows, not(feature = "e2e-harness")))]
fn main() {
    eprintln!(
        "e2e_retype_smoke потребує feature `e2e-harness`. Запуск: \
         cargo run -p typofix-platform-windows --features e2e-harness --bin e2e_retype_smoke"
    );
    std::process::exit(2);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("e2e_retype_smoke доступний лише на Windows.");
    std::process::exit(1);
}
