//! РУЧНИЙ live-харнес для `typofix-platform-windows`. **НЕ ЗАПУСКАТИ НАОСЛІП.**
//!
//! Цей бінарник ставить РЕАЛЬНІ системні хуки (перехоплює ВЕСЬ ввід) і за згоди
//! робить ОДИН контрольований перенабір через `SendInput` (друкує у вікно з
//! фокусом!). Тому його валідують лише вручну, у контрольованих умовах.
//!
//! ## Як запускати (вручну, оператором)
//! 1. `cargo build -p typofix-platform-windows --bin live_spike`
//! 2. Режим логування (безпечний, лише читає ввід):
//!    `cargo run -p typofix-platform-windows --bin live_spike`
//!    — кілька секунд друкує захоплені події (клавіші/кліки/зміна фокуса), тоді
//!    виходить. Натиски лишаються лише в RAM, нічого не пишеться на диск.
//! 3. Режим діагностики розкладки (безпечний, САМ перемикає): `... -- layoutprobe`
//!    — порівнює 4 методи визначення розкладки (M1..M4), сам перемикає
//!    розкладку на en і uk (через PostMessage), читає кожним методом і друкує
//!    підсумок: який метод СЛІДУЄ за реальною розкладкою. Відновлює початкову.
//! 4. Режим опитування (безпечний, БЕЗ друку, ручне перемикання): `... -- layout`
//!    — ~8 c кожні 500 мс друкує M1..M4 для активного вікна. Перемикай
//!    розкладку вручну (Ctrl+Shift) — дивись, який метод слідує.
//! 5. Режим перенабору (⚠️ ДРУКУЄ!): `... --bin live_spike -- type`
//!    — за ~3 c після старту переключись у порожнє тестове поле (Notepad); харнес
//!    зробить ОДИН перенабір: видалить 0 символів і набере "привіт".
//!    Нічого «чужого» не стирає (DeleteChars(0)).
//!
//! Очікуване в режимі логування: фізичні клавіші → `is_synthetic=false`; під час
//! кроку `type` наш власний ввід приходить назад уже `is_synthetic=true` (доказ
//! анти-циклу через `LLKHF_INJECTED`).

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};
    use typofix_platform::{Action, LayoutId, Platform};
    use typofix_platform_windows::{
        current_layout_id, foreground_window_info, installed_layout_ids, probe_layout_methods,
        LayoutProbe, WindowsPlatform,
    };

    let do_type = std::env::args().any(|a| a == "type");
    let do_probe = std::env::args().any(|a| a == "layoutprobe");
    let do_layout = std::env::args().any(|a| a == "layout") && !do_probe;

    // Один рядок: M1..M4 поряд + (опційно) цільова мова.
    let fmt_probe =
        |p: &LayoutProbe, target: Option<&str>| -> String {
            let t = target
                .map(|s| format!("цільова={s} | "))
                .unwrap_or_default();
            format!(
            "{t}M1={:?} M2={:?} M3={:?} M4={:?} | HKL(M1..M4)=0x{:08x},0x{:08x},0x{:08x},0x{:08x}",
            p.m1.id, p.m2.id, p.m3.id, p.m4.id, p.m1.hkl_bits, p.m2.hkl_bits, p.m3.hkl_bits,
            p.m4.hkl_bits
        )
        };

    // Режим 'layoutprobe' — САМ перемикає розкладку й порівнює 4 методи.
    if do_probe {
        println!("=== TypoFix live_spike: режим LAYOUTPROBE (САМ перемикає) ===");
        let installed = installed_layout_ids();
        println!("Встановлені розкладки: {installed:?}");
        let win = foreground_window_info();
        println!("Активне вікно: {}", win.process_name);

        let original = current_layout_id();
        println!("Початкова розкладка: {original:?}\n");

        // Тільки ті з {en, uk}, що реально встановлені (щоб нічого не інсталювати).
        let targets: Vec<LayoutId> = ["en", "uk"]
            .iter()
            .map(|s| LayoutId::new(*s))
            .filter(|id| installed.iter().any(|i| i.as_str() == id.as_str()))
            .collect();

        if targets.is_empty() {
            println!("⚠️  Серед встановлених немає ні en, ні uk — нема що перемикати.");
            return;
        }

        // WindowsPlatform.apply(SwitchLayout) робить PostMessage у переднє вікно.
        let mut platform = WindowsPlatform::new();
        let mut summary: Vec<(String, LayoutProbe)> = Vec::new();
        for target in &targets {
            platform.apply(&Action::SwitchLayout(target.clone()));
            std::thread::sleep(Duration::from_millis(400));
            let p = probe_layout_methods();
            println!("{}", fmt_probe(&p, Some(target.as_str())));
            summary.push((target.0.clone(), p));
        }

        // Відновлюємо початкову розкладку.
        platform.apply(&Action::SwitchLayout(original.clone()));
        std::thread::sleep(Duration::from_millis(200));
        println!("\nВідновлено розкладку: {:?}", current_layout_id());

        // Підсумок: який метод збігся з ЦІЛЬОВОЮ в УСІХ проходах = переможець.
        println!(
            "\n--- ПІДСУМОК (метод збігся з цільовою в усіх {} проходах) ---",
            summary.len()
        );
        for (label, get) in [
            (
                "M1",
                (|p: &LayoutProbe| &p.m1.id) as fn(&LayoutProbe) -> &LayoutId,
            ),
            ("M2", |p| &p.m2.id),
            ("M3", |p| &p.m3.id),
            ("M4", |p| &p.m4.id),
        ] {
            let all_match = summary
                .iter()
                .all(|(target, p)| get(p).as_str() == target.as_str());
            println!(
                "  {label}: {}",
                if all_match {
                    "✅ СЛІДУЄ за розкладкою"
                } else {
                    "❌ ні"
                }
            );
        }
        println!("=== Кінець LAYOUTPROBE. ===");
        return;
    }

    // Режим 'layout' — опитування БЕЗ самоперемикання (перемикай вручну Ctrl+Shift).
    if do_layout {
        println!("=== TypoFix live_spike: режим LAYOUT (опитування, без друку) ===");
        println!("~8 с, кожні 500 мс. Перемикай розкладку вручну (Ctrl+Shift).");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(8) {
            let win = foreground_window_info();
            let p = probe_layout_methods();
            println!("{} | вікно={}", fmt_probe(&p, None), win.process_name);
            std::thread::sleep(Duration::from_millis(500));
        }
        println!("=== Кінець режиму LAYOUT. ===");
        return;
    }

    println!("=== TypoFix live_spike (РУЧНИЙ) ===");
    println!("Логую ввід ~8 с. Ctrl+C для виходу.");
    if do_type {
        println!("⚠️  Режим 'type': за 3 с зроблю ОДИН перенабір -> 'привіт'.");
        println!("    Переключись у порожнє тестове поле ЗАРАЗ.");
    }

    let mut platform = WindowsPlatform::new();
    println!("Активне вікно на старті: {:?}", platform.active_window());
    println!("Активна розкладка: {:?}", platform.current_layout());

    let start = Instant::now();
    let mut typed = false;
    loop {
        while let Some(ev) = platform.try_next_event() {
            println!("{ev:?}");
        }
        if do_type && !typed && start.elapsed() >= Duration::from_secs(3) {
            println!(">>> Перенабір: SwitchLayout(uk) + TypeUnicode(\"привіт\")");
            platform.apply(&Action::SwitchLayout(typofix_platform::LayoutId::new("uk")));
            platform.apply(&Action::TypeUnicode("привіт".into()));
            typed = true;
        }
        if start.elapsed() >= Duration::from_secs(8) {
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    println!("=== Кінець. Хуки знято. ===");
}

#[cfg(not(windows))]
fn main() {
    eprintln!("live_spike доступний лише на Windows.");
}
