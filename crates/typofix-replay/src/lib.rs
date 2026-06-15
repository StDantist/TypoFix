//! # typofix-replay
//!
//! Формат golden-фікстур та record/replay записаних сесій вводу для TypoFix
//! («баг як тест», рівень 2 у `docs/TESTING_STRATEGY.md`).
//!
//! ## Формат (два окремі `*.jsonl`-файли)
//!
//! **Сесія — `<name>.jsonl`** (вхід для прогону). JSONL: один JSON-обʼєкт на
//! рядок, кожен — тегований `Record`:
//! - рядок 1 — `{"type":"setup","window":{…},"layout":"uk"}` (рівно один,
//!   першим);
//! - далі — `{"type":"event","event":<InputEvent>}` у порядку надходження.
//!
//! Детермінізм: **час береться лише з подій** (`KeyEvent::timestamp_ms`) —
//! replay нічого не «годинникує», лише віддає записані події драйверу.
//!
//! **Очікуване — `<name>.expected.jsonl`** (розмічається НЕЗАЛЕЖНО, не
//! генерується движком — інакше тест циклічний, див. TESTING_STRATEGY §2).
//! JSONL тегованих записів:
//! - `{"type":"text","text":"…"}` — фінальний текст буфера (≤ 1 рядок);
//! - `{"type":"action","action":<Action>}` — очікувані дії у порядку.
//!
//! Можна задати лише текст, лише дії, або обидва. Якщо жодного `action`-рядка
//! немає — перелік дій вважається **незаданим** (не перевіряється), а не
//! «порожнім».
//!
//! ## Приватність
//!
//! Фікстури в репозиторії — **ТІЛЬКИ синтетичні або скрабовані** (нуль реальних
//! секретів): TypoFix бачить увесь ввід. Див. `fixtures/CLAUDE.md`.
//!
//! ## Межі
//!
//! Крейт — **джерело подій і чистий хелпер порівняння**; самого прогону він не
//! робить (драйвер на `typofix-platform-virtual` будують тести). Тому залежності
//! на virtual-платформу тут немає.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use typofix_platform::{Action, InputEvent, LayoutId, WindowInfo};

/// Помилки запису/читання фікстур.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// Помилка вводу-виводу.
    #[error("I/O помилка: {0}")]
    Io(#[from] std::io::Error),
    /// Невалідний JSON у конкретному рядку (1-based).
    #[error("JSON-помилка в рядку {line}: {source}")]
    Json {
        /// Номер рядка (1-based).
        line: usize,
        /// Вихідна помилка serde_json.
        source: serde_json::Error,
    },
    /// У сесії немає рядка `setup`.
    #[error("порожня сесія: відсутній рядок setup")]
    MissingSetup,
    /// Зустрівся другий `setup` (їх має бути рівно один, першим рядком).
    #[error("дублікат setup у рядку {0}: setup має бути рівно один, першим рядком")]
    DuplicateSetup(usize),
    /// Подія трапилася раніше за `setup`.
    #[error("подія в рядку {0} йде раніше за setup")]
    EventBeforeSetup(usize),
}

/// Початковий стан сесії: активне вікно та розкладка на старті.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Setup {
    /// Активне вікно на початку сесії.
    pub window: WindowInfo,
    /// Активна розкладка на початку сесії.
    pub layout: LayoutId,
}

impl Setup {
    /// Зручний конструктор.
    pub fn new(window: WindowInfo, layout: LayoutId) -> Self {
        Self { window, layout }
    }
}

/// Записана сесія: setup + послідовність подій вводу.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Початковий стан.
    pub setup: Setup,
    /// Події у порядку надходження.
    pub events: Vec<InputEvent>,
}

/// Один рядок файлу сесії (тегований за полем `type`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    Setup {
        window: WindowInfo,
        layout: LayoutId,
    },
    Event {
        event: InputEvent,
    },
}

/// Накопичувач сесії: збирає `InputEvent`-и поверх відомого setup.
///
/// Чистий і детермінований — лише складає події в порядку виклику; жодного
/// читання годинника (час уже всередині `KeyEvent::timestamp_ms`).
#[derive(Debug, Clone)]
pub struct Recorder {
    setup: Setup,
    events: Vec<InputEvent>,
}

impl Recorder {
    /// Новий накопичувач із заданим setup.
    pub fn new(setup: Setup) -> Self {
        Self {
            setup,
            events: Vec::new(),
        }
    }

    /// Додати одну подію.
    pub fn record(&mut self, event: InputEvent) {
        self.events.push(event);
    }

    /// Додати потік подій.
    pub fn record_all(&mut self, events: impl IntoIterator<Item = InputEvent>) {
        self.events.extend(events);
    }

    /// Уже зібрані події (для інтроспекції під час запису).
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Завершити запис і отримати сесію.
    pub fn finish(self) -> Session {
        Session {
            setup: self.setup,
            events: self.events,
        }
    }
}

impl Session {
    /// Ітератор подій для прогону через драйвер/движок (replay-джерело).
    pub fn replay(&self) -> impl Iterator<Item = &InputEvent> {
        self.events.iter()
    }

    /// Серіалізувати у JSONL-рядок (setup першим, далі події; з кінцевим `\n`).
    pub fn to_jsonl(&self) -> Result<String, ReplayError> {
        let mut out = Vec::new();
        self.write_jsonl(&mut out)?;
        // write_jsonl пише валідний UTF-8 (JSON), тож конверсія безпечна.
        Ok(String::from_utf8(out).expect("serde_json завжди дає валідний UTF-8"))
    }

    /// Записати сесію у будь-який `Write` (один JSON-обʼєкт на рядок).
    pub fn write_jsonl<W: Write>(&self, mut writer: W) -> Result<(), ReplayError> {
        let setup = Record::Setup {
            window: self.setup.window.clone(),
            layout: self.setup.layout.clone(),
        };
        writeln!(writer, "{}", to_line(&setup)?)?;
        for event in &self.events {
            let rec = Record::Event {
                event: event.clone(),
            };
            writeln!(writer, "{}", to_line(&rec)?)?;
        }
        Ok(())
    }

    /// Записати сесію у файл.
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), ReplayError> {
        let file = std::fs::File::create(path)?;
        self.write_jsonl(std::io::BufWriter::new(file))
    }

    /// Розпарсити сесію з JSONL-рядка.
    pub fn from_jsonl(text: &str) -> Result<Session, ReplayError> {
        Self::read_jsonl(text.as_bytes())
    }

    /// Прочитати сесію з будь-якого `Read` (через `BufRead`).
    pub fn read_jsonl<R: BufRead>(reader: R) -> Result<Session, ReplayError> {
        let mut setup: Option<Setup> = None;
        let mut events = Vec::new();

        for (idx, line) in reader.lines().enumerate() {
            let line_no = idx + 1;
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue; // допускаємо порожні рядки між записами
            }
            let rec: Record =
                serde_json::from_str(trimmed).map_err(|source| ReplayError::Json {
                    line: line_no,
                    source,
                })?;
            match rec {
                Record::Setup { window, layout } => {
                    if setup.is_some() {
                        return Err(ReplayError::DuplicateSetup(line_no));
                    }
                    setup = Some(Setup { window, layout });
                }
                Record::Event { event } => {
                    if setup.is_none() {
                        return Err(ReplayError::EventBeforeSetup(line_no));
                    }
                    events.push(event);
                }
            }
        }

        let setup = setup.ok_or(ReplayError::MissingSetup)?;
        Ok(Session { setup, events })
    }

    /// Прочитати сесію з файлу.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Session, ReplayError> {
        let file = std::fs::File::open(path)?;
        Self::read_jsonl(BufReader::new(file))
    }
}

/// Очікуваний результат прогону сесії.
///
/// **Розмічається незалежно** від движка (TESTING_STRATEGY §2). `None`-поле
/// означає «не перевіряти», `Some` — перевіряти на точну рівність.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expected {
    /// Очікуваний фінальний текст буфера (якщо задано).
    pub final_text: Option<String>,
    /// Очікувана послідовність дій (якщо задано).
    pub actions: Option<Vec<Action>>,
}

/// Один рядок файлу `*.expected.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ExpectedRecord {
    Text { text: String },
    Action { action: Action },
}

impl Expected {
    /// Серіалізувати очікуване у JSONL (рядок `text`, далі рядки `action`).
    pub fn to_jsonl(&self) -> Result<String, ReplayError> {
        let mut out = Vec::new();
        self.write_jsonl(&mut out)?;
        Ok(String::from_utf8(out).expect("serde_json завжди дає валідний UTF-8"))
    }

    /// Записати очікуване у будь-який `Write`.
    pub fn write_jsonl<W: Write>(&self, mut writer: W) -> Result<(), ReplayError> {
        if let Some(text) = &self.final_text {
            let rec = ExpectedRecord::Text { text: text.clone() };
            writeln!(writer, "{}", to_line(&rec)?)?;
        }
        if let Some(actions) = &self.actions {
            for action in actions {
                let rec = ExpectedRecord::Action {
                    action: action.clone(),
                };
                writeln!(writer, "{}", to_line(&rec)?)?;
            }
        }
        Ok(())
    }

    /// Записати очікуване у файл.
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), ReplayError> {
        let file = std::fs::File::create(path)?;
        self.write_jsonl(std::io::BufWriter::new(file))
    }

    /// Розпарсити очікуване з JSONL-рядка.
    pub fn from_jsonl(text: &str) -> Result<Expected, ReplayError> {
        Self::read_jsonl(text.as_bytes())
    }

    /// Прочитати очікуване з будь-якого `Read`.
    ///
    /// Якщо у файлі є хоч один `action`-рядок — `actions` стає `Some` (порядок
    /// збережено); якщо жодного — лишається `None` («не перевіряти дії»).
    pub fn read_jsonl<R: BufRead>(reader: R) -> Result<Expected, ReplayError> {
        let mut final_text: Option<String> = None;
        let mut actions: Option<Vec<Action>> = None;

        for (idx, line) in reader.lines().enumerate() {
            let line_no = idx + 1;
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let rec: ExpectedRecord =
                serde_json::from_str(trimmed).map_err(|source| ReplayError::Json {
                    line: line_no,
                    source,
                })?;
            match rec {
                ExpectedRecord::Text { text } => final_text = Some(text),
                ExpectedRecord::Action { action } => {
                    actions.get_or_insert_with(Vec::new).push(action)
                }
            }
        }

        Ok(Expected {
            final_text,
            actions,
        })
    }

    /// Прочитати очікуване з файлу.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Expected, ReplayError> {
        let file = std::fs::File::open(path)?;
        Self::read_jsonl(BufReader::new(file))
    }
}

/// Розбіжність тексту: очікуване проти фактичного.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    /// Очікуваний текст.
    pub expected: String,
    /// Фактичний текст (порожній рядок, якщо движок його не дав).
    pub actual: String,
}

/// Розбіжність послідовності дій.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionsDiff {
    /// Очікувані дії.
    pub expected: Vec<Action>,
    /// Фактичні дії.
    pub actual: Vec<Action>,
}

/// Структурований результат порівняння очікуваного з фактичним.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayDiff {
    /// Розбіжність тексту (якщо текст перевірявся й не збігся).
    pub text: Option<TextDiff>,
    /// Розбіжність дій (якщо дії перевірялися й не збіглися).
    pub actions: Option<ActionsDiff>,
}

impl ReplayDiff {
    /// `true`, якщо все, що перевірялося, збіглося.
    pub fn is_match(&self) -> bool {
        self.text.is_none() && self.actions.is_none()
    }
}

/// Чисте порівняння очікуваного з фактичним результатом прогону.
///
/// Перевіряються лише задані (`Some`) поля `Expected`. `actual_text` — `None`,
/// якщо движок не виробив тексту (тоді при заданому `final_text` фіксуємо
/// розбіжність із фактичним порожнім рядком).
pub fn compare(
    expected: &Expected,
    actual_text: Option<&str>,
    actual_actions: &[Action],
) -> ReplayDiff {
    let text = expected.final_text.as_ref().and_then(|want| {
        let got = actual_text.unwrap_or("");
        if got == want {
            None
        } else {
            Some(TextDiff {
                expected: want.clone(),
                actual: got.to_string(),
            })
        }
    });

    let actions = expected.actions.as_ref().and_then(|want| {
        if want.as_slice() == actual_actions {
            None
        } else {
            Some(ActionsDiff {
                expected: want.clone(),
                actual: actual_actions.to_vec(),
            })
        }
    });

    ReplayDiff { text, actions }
}

/// Серіалізувати запис у компактний однорядковий JSON.
fn to_line<T: Serialize>(value: &T) -> Result<String, ReplayError> {
    serde_json::to_string(value).map_err(|source| ReplayError::Json { line: 0, source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use typofix_platform::{KeyDir, KeyEvent, Modifiers};

    fn key(scancode: u32, vk: u32, dir: KeyDir, ts: u64) -> InputEvent {
        InputEvent::Key(KeyEvent {
            scancode,
            vk,
            dir,
            modifiers: Modifiers::empty(),
            timestamp_ms: ts,
            is_synthetic: false,
            is_autorepeat: false,
        })
    }

    fn sample_session() -> Session {
        let setup = Setup::new(
            WindowInfo {
                process_name: "notepad.exe".into(),
                exe_path: r"C:\Windows\System32\notepad.exe".into(),
                is_fullscreen: false,
            },
            LayoutId::new("en"),
        );
        let mut rec = Recorder::new(setup);
        rec.record(key(0x22, 0x47, KeyDir::Down, 100));
        rec.record(key(0x22, 0x47, KeyDir::Up, 140));
        rec.record(InputEvent::CaretMove);
        rec.record(InputEvent::FocusChange(WindowInfo {
            process_name: "code.exe".into(),
            exe_path: r"C:\code.exe".into(),
            is_fullscreen: false,
        }));
        rec.record(InputEvent::MouseClick);
        rec.finish()
    }

    #[test]
    fn session_round_trip_is_identical() {
        let session = sample_session();
        let text = session.to_jsonl().unwrap();
        let back = Session::from_jsonl(&text).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn jsonl_is_one_object_per_line() {
        let session = sample_session();
        let text = session.to_jsonl().unwrap();
        let lines: Vec<&str> = text.lines().collect();
        // setup + 5 подій
        assert_eq!(lines.len(), 6);
        // перший рядок — setup
        assert!(lines[0].contains("\"type\":\"setup\""));
        // кожен рядок — самодостатній валідний JSON
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).unwrap();
        }
    }

    #[test]
    fn modifiers_survive_round_trip() {
        let setup = Setup::new(WindowInfo::default(), LayoutId::new("uk"));
        let mut rec = Recorder::new(setup);
        rec.record(InputEvent::Key(KeyEvent {
            scancode: 0x1E,
            vk: 0x41,
            dir: KeyDir::Down,
            modifiers: Modifiers::CTRL | Modifiers::SHIFT | Modifiers::ALTGR,
            timestamp_ms: 7,
            is_synthetic: true,
            is_autorepeat: false,
        }));
        let session = rec.finish();
        let back = Session::from_jsonl(&session.to_jsonl().unwrap()).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn missing_setup_is_error() {
        let line = r#"{"type":"event","event":"MouseClick"}"#;
        assert!(matches!(
            Session::from_jsonl(line),
            Err(ReplayError::EventBeforeSetup(1))
        ));
        assert!(matches!(
            Session::from_jsonl(""),
            Err(ReplayError::MissingSetup)
        ));
    }

    #[test]
    fn duplicate_setup_is_error() {
        let s = sample_session();
        let mut text = s.to_jsonl().unwrap();
        // додаємо другий setup
        let first = text.lines().next().unwrap().to_owned();
        text.push_str(&first);
        assert!(matches!(
            Session::from_jsonl(&text),
            Err(ReplayError::DuplicateSetup(_))
        ));
    }

    #[test]
    fn blank_lines_are_skipped() {
        let session = sample_session();
        let text = session.to_jsonl().unwrap();
        let spaced = text.replace('\n', "\n\n");
        let back = Session::from_jsonl(&spaced).unwrap();
        assert_eq!(session, back);
    }

    #[test]
    fn expected_round_trip() {
        let expected = Expected {
            final_text: Some("привіт ".into()),
            actions: Some(vec![
                Action::SwitchLayout(LayoutId::new("uk")),
                Action::DeleteChars(6),
                Action::TypeUnicode("привіт".into()),
            ]),
        };
        let back = Expected::from_jsonl(&expected.to_jsonl().unwrap()).unwrap();
        assert_eq!(expected, back);
    }

    #[test]
    fn expected_without_actions_leaves_none() {
        let expected = Expected {
            final_text: Some("abc".into()),
            actions: None,
        };
        let back = Expected::from_jsonl(&expected.to_jsonl().unwrap()).unwrap();
        assert_eq!(back.actions, None);
        assert_eq!(back.final_text.as_deref(), Some("abc"));
    }

    #[test]
    fn compare_matches_and_detects_mismatch() {
        let expected = Expected {
            final_text: Some("привіт".into()),
            actions: Some(vec![Action::TypeUnicode("привіт".into())]),
        };
        let ok = compare(
            &expected,
            Some("привіт"),
            &[Action::TypeUnicode("привіт".into())],
        );
        assert!(ok.is_match());

        let bad = compare(&expected, Some("hello"), &[Action::None]);
        assert!(!bad.is_match());
        assert_eq!(bad.text.unwrap().actual, "hello");
        assert_eq!(bad.actions.unwrap().expected.len(), 1);
    }

    #[test]
    fn compare_ignores_unset_fields() {
        // Нічого не задано — завжди збіг.
        let empty = Expected::default();
        assert!(compare(&empty, None, &[Action::None]).is_match());

        // Задано лише дії — текст не звіряємо.
        let only_actions = Expected {
            final_text: None,
            actions: Some(vec![]),
        };
        assert!(compare(&only_actions, Some("будь-що"), &[]).is_match());
    }
}
