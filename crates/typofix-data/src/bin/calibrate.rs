//! Калібрувальний харнес детектора.
//!
//! Прогоняє розмічений eval-датасет (`data/eval/dataset.jsonl`) через
//! `typofix_core::detector::decide` на зразкових моделях і друкує метрики
//! (precision/recall/F1/accuracy — глобально й по категоріях) + список промахів.
//!
//! Запуск:
//! ```text
//! cargo run -p typofix-data --bin calibrate
//! ```
//!
//! ⚠️ Зразкові моделі дрібні → числа грубі/погані. Це ОЧІКУВАНО: мета — сам
//! харнес і baseline-знімок, не хороші числа (повна калібрація — на реальному
//! корпусі). Пороги/ваги в `typofix-core` тут НЕ змінюємо.

use typofix_data::eval;

fn main() {
    match eval::run_default() {
        Ok(report) => print!("{}", eval::format_report(&report)),
        Err(e) => {
            eprintln!("калібрування не вдалося: {e}");
            std::process::exit(1);
        }
    }
}
