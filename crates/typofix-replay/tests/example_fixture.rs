//! Перевіряє, що синтетична приклад-фікстура у `fixtures/` парситься і що
//! окремо розмічений `*.expected.jsonl` читається коректно. Самого прогону тут
//! немає (драйвер на virtual-платформі будують інші тести) — лише формат.

use std::path::PathBuf;

use typofix_replay::{Expected, Session};

fn fixtures_dir() -> PathBuf {
    // crates/typofix-replay -> корінь репозиторію -> fixtures/
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

#[test]
fn example_session_parses() {
    let path = fixtures_dir().join("example_ghbdsn_uk.jsonl");
    let session = Session::from_file(&path).expect("фікстура має парситися");

    assert_eq!(session.setup.layout.as_str(), "en");
    assert_eq!(session.setup.window.process_name, "notepad.exe");
    // 6 літер + пробіл, кожна як Down+Up = 14 подій.
    assert_eq!(session.events.len(), 14);
    // replay() віддає всі події у порядку.
    assert_eq!(session.replay().count(), 14);
}

#[test]
fn example_expected_parses() {
    let path = fixtures_dir().join("example_ghbdsn_uk.expected.jsonl");
    let expected = Expected::from_file(&path).expect("expected має парситися");

    assert_eq!(expected.final_text.as_deref(), Some("привіт "));
    let actions = expected.actions.expect("дії задані");
    assert_eq!(actions.len(), 3);
}

#[test]
fn fixture_round_trips_through_serialization() {
    let path = fixtures_dir().join("example_ghbdsn_uk.jsonl");
    let session = Session::from_file(&path).unwrap();
    let reparsed = Session::from_jsonl(&session.to_jsonl().unwrap()).unwrap();
    assert_eq!(session, reparsed);
}
