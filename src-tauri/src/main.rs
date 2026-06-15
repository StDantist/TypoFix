// У release ховаємо консольне вікно на Windows (інакше блимає консоль).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    typofix_app_lib::run();
}
