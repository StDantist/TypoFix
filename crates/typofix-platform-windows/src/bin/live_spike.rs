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
//! 3. Режим перенабору (⚠️ ДРУКУЄ!): `... --bin live_spike -- type`
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
    use typofix_platform::{Action, Platform};
    use typofix_platform_windows::WindowsPlatform;

    let do_type = std::env::args().any(|a| a == "type");

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
