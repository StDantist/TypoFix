//! Integration-обгортка калібрувального харнеса.
//!
//! Перевіряє, що харнес запускається наскрізно на реальному датасеті:
//! завантаження → побудова профілів → прогін → метрики. НЕ перевіряє якість
//! чисел (моделі зразкові й дрібні — числа грубі); лише цілісність конвеєра й
//! консистентність матриці помилок.

use typofix_data::eval::{self, Outcome};

#[test]
fn dataset_loads_and_has_both_classes() {
    let examples = eval::load_dataset(&eval::default_dataset_path()).expect("датасет має читатися");
    assert!(examples.len() > 100, "очікуємо солідний датасет");
    let pos = examples.iter().filter(|e| e.should_switch).count();
    let neg = examples.len() - pos;
    assert!(
        pos > 0 && neg > 0,
        "мають бути обидва класи (pos={pos} neg={neg})"
    );
    // Інваріанти схеми.
    for e in &examples {
        if e.should_switch {
            assert_ne!(
                e.typed_layout, e.intended_layout,
                "позитив: typed != intended"
            );
        } else {
            assert_eq!(
                e.typed_layout, e.intended_layout,
                "негатив: typed == intended"
            );
        }
    }
}

#[test]
fn harness_runs_end_to_end_and_matrix_is_consistent() {
    let report = eval::run_default().expect("харнес має відпрацювати");

    // Кожен приклад потрапив рівно в одну клітинку матриці.
    let o = &report.overall;
    assert_eq!(o.total(), report.rows.len(), "сума матриці = к-сть рядків");

    // Сума по категоріях = глобальна сума.
    let cat_total: usize = report.by_category.values().map(|c| c.total()).sum();
    assert_eq!(cat_total, o.total(), "категорії покривають усі рядки");

    // FP-рядків стільки ж, скільки FP у матриці (звіт консистентний).
    let fp_rows = report
        .rows
        .iter()
        .filter(|r| r.outcome == Outcome::Fp)
        .count();
    assert_eq!(fp_rows, o.fp);

    // Метрики не панікують і дають скінченні числа або NaN (для порожніх знаменників).
    let acc = o.accuracy();
    assert!(acc.is_finite() || acc.is_nan());

    // format_report не панікує і щось друкує.
    let text = eval::format_report(&report);
    assert!(text.contains("ГЛОБАЛЬНО"));
    assert!(text.contains("ПО КАТЕГОРІЯХ"));
}
