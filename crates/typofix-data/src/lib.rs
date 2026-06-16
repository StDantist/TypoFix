//! # typofix-data
//!
//! Завантаження та вбудовування даних: розкладки (`data/layouts/*.toml`),
//! а згодом мовні моделі (`data/lm/*.bin`) і словники (`data/dicts/*.fst`).
//! Деталі — `docs/ARCHITECTURE.md` §6, `data/CLAUDE.md`.
//!
//! Це **єдиний** шар, де дозволено IO. `typofix-core` лишається чистим: він
//! отримує вже зібраний [`Layout`], не знаючи про файли.
//!
//! ## Розкладки
//! TOML — це **fallback/еталон** (у проді мапінг беремо з ОС). Тут TOML живить
//! mapper і тести. Кожен файл вбудовується в бінар через [`include_str!`]
//! ([`embedded_layout`]); за потреби можна підмінити версією з диска
//! ([`load_layout`] з `override_dir`).
//!
//! ### Формат TOML
//! ```toml
//! id = "uk"
//! name = "Українська"   # опційно, ігнорується завантажувачем
//!
//! [[key]]
//! scancode = 0x22       # Windows scancode set 1 (фізична клавіша G)
//! normal = "п"
//! shift = "П"           # опційно
//! altgr = "…"           # опційно
//! ```

pub mod eval;

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use typofix_core::{Dictionary, KeyCap, Layout, LayoutId, NgramModel};

/// Вбудовані TOML-джерела розкладок (компілюються в бінар).
const UK_TOML: &str = include_str!("../../../data/layouts/uk.toml");
const EN_TOML: &str = include_str!("../../../data/layouts/en.toml");

/// Помилки завантаження/парсингу розкладки.
#[derive(Debug, Error)]
pub enum LayoutError {
    /// Не вдалося прочитати файл з диска.
    #[error("не вдалося прочитати файл розкладки {path}: {source}")]
    Io {
        /// Шлях, який намагалися прочитати.
        path: PathBuf,
        /// Першопричина (помилка IO).
        source: std::io::Error,
    },
    /// TOML не розпарсився.
    #[error("не вдалося розпарсити TOML розкладки '{id}': {source}")]
    Parse {
        /// Ідентифікатор/підказка розкладки.
        id: String,
        /// Першопричина (помилка TOML).
        source: toml::de::Error,
    },
    /// Поле символу порожнє (немає жодного `char`).
    #[error("порожнє поле символу в розкладці '{id}', scancode {scancode:#04x}")]
    EmptyChar {
        /// Ідентифікатор розкладки.
        id: String,
        /// Проблемний scancode.
        scancode: u32,
    },
    /// Запит на невідому вбудовану розкладку.
    #[error("невідома вбудована розкладка: '{0}'")]
    UnknownEmbedded(String),
}

/// Сирий вигляд TOML-файлу розкладки.
#[derive(Debug, Deserialize)]
struct LayoutFile {
    id: String,
    #[serde(default)]
    key: Vec<KeyEntry>,
}

/// Один запис `[[key]]` у TOML.
#[derive(Debug, Deserialize)]
struct KeyEntry {
    scancode: u32,
    normal: String,
    #[serde(default)]
    shift: Option<String>,
    #[serde(default)]
    altgr: Option<String>,
}

fn first_char(s: &str) -> Option<char> {
    s.chars().next()
}

/// Розпарсити TOML-джерело розкладки у [`Layout`].
///
/// `id_hint` використовується лише в повідомленнях про помилку, якщо парсинг
/// зірветься до читання поля `id`.
fn parse_layout(id_hint: &str, src: &str) -> Result<Layout, LayoutError> {
    let file: LayoutFile = toml::from_str(src).map_err(|source| LayoutError::Parse {
        id: id_hint.to_string(),
        source,
    })?;

    let mut caps: Vec<(u32, KeyCap)> = Vec::with_capacity(file.key.len());
    for k in &file.key {
        let normal = first_char(&k.normal).ok_or_else(|| LayoutError::EmptyChar {
            id: file.id.clone(),
            scancode: k.scancode,
        })?;
        let shift = k.shift.as_deref().and_then(first_char);
        let altgr = k.altgr.as_deref().and_then(first_char);
        caps.push((
            k.scancode,
            KeyCap {
                normal,
                shift,
                altgr,
            },
        ));
    }

    Ok(Layout::new(LayoutId::new(file.id), caps))
}

/// Завантажити **вбудовану** розкладку за ідентифікатором (`"uk"`, `"en"`).
pub fn embedded_layout(id: &str) -> Result<Layout, LayoutError> {
    let src = match id {
        "uk" => UK_TOML,
        "en" => EN_TOML,
        other => return Err(LayoutError::UnknownEmbedded(other.to_string())),
    };
    parse_layout(id, src)
}

/// Завантажити розкладку з конкретного TOML-файлу на диску.
pub fn load_layout_file(path: &Path) -> Result<Layout, LayoutError> {
    let src = std::fs::read_to_string(path).map_err(|source| LayoutError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let id_hint = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    parse_layout(&id_hint, &src)
}

/// Завантажити розкладку з override-каталогу, якщо там є `{id}.toml`, інакше —
/// вбудовану. Так користувач може підмінити/додати розкладку з диска, не
/// перезбираючи бінар.
pub fn load_layout(id: &str, override_dir: Option<&Path>) -> Result<Layout, LayoutError> {
    if let Some(dir) = override_dir {
        let candidate = dir.join(format!("{id}.toml"));
        if candidate.exists() {
            return load_layout_file(&candidate);
        }
    }
    embedded_layout(id)
}

// ===========================================================================
// LM (n-gram) і словник (FST): тренування/побудова, серіалізація, завантаження.
// ===========================================================================

/// Вбудовані **зразки** для розробки й тестів (committed, малі — кілька КБ).
/// Реальні великі моделі/словники генеруються окремо й кладуться у
/// `data/lm/*.bin` / `data/dicts/*.fst` (gitignored). Див. follow-up у
/// `data/CLAUDE.md`.
const UK_SAMPLE_CORPUS: &str = include_str!("../../../data/samples/uk.corpus.txt");
const EN_SAMPLE_CORPUS: &str = include_str!("../../../data/samples/en.corpus.txt");
const UK_SAMPLE_WORDS: &str = include_str!("../../../data/samples/uk.words.txt");
const EN_SAMPLE_WORDS: &str = include_str!("../../../data/samples/en.words.txt");

/// Помилки роботи з LM/словниками.
#[derive(Debug, Error)]
pub enum ModelError {
    /// Не вдалося прочитати/записати файл.
    #[error("IO {path}: {source}")]
    Io {
        /// Шлях.
        path: PathBuf,
        /// Першопричина.
        source: std::io::Error,
    },
    /// Помилка (де)серіалізації LM (bincode).
    #[error("bincode (де)серіалізація LM: {0}")]
    Bincode(#[from] bincode::Error),
    /// Помилка FST (побудова/читання словника).
    #[error("FST: {0}")]
    Fst(#[from] fst::Error),
    /// Запит на невідому вбудовану мову зразка.
    #[error("невідомий вбудований зразок для мови: '{0}'")]
    UnknownSample(String),
}

// --- LM --------------------------------------------------------------------

/// Натренувати n-gram модель із сирого тексту (тонка обгортка над core).
///
/// Цей самий шлях з'їсть і великий корпус — достатньо передати більший `corpus`.
pub fn train_lm(corpus: &str, order: usize, k: f64) -> NgramModel {
    NgramModel::train(corpus, order, k)
}

/// Серіалізувати модель у компактні байти (`.bin`).
pub fn serialize_lm(model: &NgramModel) -> Result<Vec<u8>, ModelError> {
    Ok(bincode::serialize(model)?)
}

/// Десеріалізувати модель із байтів `.bin`.
pub fn deserialize_lm(bytes: &[u8]) -> Result<NgramModel, ModelError> {
    Ok(bincode::deserialize(bytes)?)
}

/// Записати модель у файл `.bin`.
pub fn save_lm(model: &NgramModel, path: &Path) -> Result<(), ModelError> {
    let bytes = serialize_lm(model)?;
    std::fs::write(path, bytes).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Завантажити модель із конкретного файлу `.bin`.
pub fn load_lm_file(path: &Path) -> Result<NgramModel, ModelError> {
    let bytes = std::fs::read(path).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    deserialize_lm(&bytes)
}

/// Натренувати модель із вбудованого зразка корпусу (детерміновано).
pub fn sample_lm(lang: &str) -> Result<NgramModel, ModelError> {
    let corpus = match lang {
        "uk" => UK_SAMPLE_CORPUS,
        "en" => EN_SAMPLE_CORPUS,
        other => return Err(ModelError::UnknownSample(other.to_string())),
    };
    Ok(NgramModel::train(
        corpus,
        typofix_core::lm::DEFAULT_ORDER,
        typofix_core::lm::DEFAULT_K,
    ))
}

/// Завантажити LM: з `override_dir/{lang}.bin`, якщо є, інакше — з вбудованого
/// зразка. (Доки немає реальних `.bin`, зразок забезпечує наскрізну роботу.)
pub fn load_lm(lang: &str, override_dir: Option<&Path>) -> Result<NgramModel, ModelError> {
    if let Some(dir) = override_dir {
        let candidate = dir.join(format!("{lang}.bin"));
        if candidate.exists() {
            return load_lm_file(&candidate);
        }
    }
    sample_lm(lang)
}

// --- Словник (FST) ---------------------------------------------------------

/// Побудувати FST-словник зі списку слів (тонка обгортка над core).
pub fn build_dict<I, S>(words: I) -> Result<Dictionary, ModelError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    Ok(Dictionary::from_words(words)?)
}

/// Записати словник у файл `.fst`.
pub fn save_dict(dict: &Dictionary, path: &Path) -> Result<(), ModelError> {
    std::fs::write(path, dict.as_bytes()).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Завантажити словник із конкретного файлу `.fst`.
pub fn load_dict_file(path: &Path) -> Result<Dictionary, ModelError> {
    let bytes = std::fs::read(path).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Dictionary::from_bytes(bytes)?)
}

/// Побудувати словник із вбудованого зразка списку слів (одне слово на рядок).
pub fn sample_dict(lang: &str) -> Result<Dictionary, ModelError> {
    let list = match lang {
        "uk" => UK_SAMPLE_WORDS,
        "en" => EN_SAMPLE_WORDS,
        other => return Err(ModelError::UnknownSample(other.to_string())),
    };
    build_dict(list.lines().map(str::trim).filter(|l| !l.is_empty()))
}

/// Завантажити словник: з `override_dir/{lang}.fst`, якщо є, інакше — з
/// вбудованого зразка слів.
pub fn load_dict(lang: &str, override_dir: Option<&Path>) -> Result<Dictionary, ModelError> {
    if let Some(dir) = override_dir {
        let candidate = dir.join(format!("{lang}.fst"));
        if candidate.exists() {
            return load_dict_file(&candidate);
        }
    }
    sample_dict(lang)
}

// --- Whitelist коротких службових слів (`{lang}.short.txt`) ----------------

/// Розпарсити whitelist коротких службових слів: один lowercase-рядок = слово,
/// `#` = коментар, порожні рядки ігноруються. Формат — `data/dicts/{lang}.short.txt`
/// (деталі: `data/CLAUDE.md`). Повертає слова як є (без додаткового lowercase —
/// файл уже lowercase; матчинг у `WordRules` усе одно регістронезалежний).
pub fn parse_short_words(src: &str) -> Vec<String> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// Завантажити whitelist коротких службових слів мови з `dir/{lang}.short.txt`.
/// Файлу немає → порожній список (дзеркальна релаксація для мови просто вимкнена).
pub fn load_short_words(lang: &str, dir: &Path) -> std::io::Result<Vec<String>> {
    let path = dir.join(format!("{lang}.short.txt"));
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(parse_short_words(&std::fs::read_to_string(path)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use typofix_core::{KeyStroke, Modifiers};

    fn strokes(scancodes: &[u32]) -> Vec<KeyStroke> {
        scancodes
            .iter()
            .map(|&sc| KeyStroke::new(sc, Modifiers::empty()))
            .collect()
    }

    #[test]
    fn embedded_en_parses_and_maps_qwerty() {
        let en = embedded_layout("en").expect("en має парситися");
        assert_eq!(en.id().as_str(), "en");
        // A = 0x1E, G = 0x22 (set 1).
        assert_eq!(en.char_at(0x1E, Modifiers::empty()), Some('a'));
        assert_eq!(en.char_at(0x1E, Modifiers::SHIFT), Some('A'));
        assert_eq!(en.char_at(0x22, Modifiers::empty()), Some('g'));
    }

    #[test]
    fn embedded_uk_has_required_letters() {
        let uk = embedded_layout("uk").expect("uk має парситися");
        assert_eq!(uk.id().as_str(), "uk");
        // Ключові українські символи з брифу.
        assert_eq!(uk.char_at(0x22, Modifiers::empty()), Some('п')); // G→п
        assert_eq!(uk.char_at(0x1F, Modifiers::empty()), Some('і')); // S→і
        assert_eq!(uk.char_at(0x1B, Modifiers::empty()), Some('ї')); // ]→ї
        assert_eq!(uk.char_at(0x28, Modifiers::empty()), Some('є')); // '→є
        assert_eq!(uk.char_at(0x2B, Modifiers::empty()), Some('ґ')); // \→ґ
                                                                     // Апостроф ’ (U+2019), не ASCII.
        assert!(uk.stroke_for('\u{2019}').is_some());
    }

    #[test]
    fn loaded_uk_interprets_ghbdsn_as_privit() {
        let en = embedded_layout("en").unwrap();
        let uk = embedded_layout("uk").unwrap();
        let seq = strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]); // g h b d s n
        assert_eq!(en.interpret(&seq), "ghbdsn");
        assert_eq!(uk.interpret(&seq), "привіт");
    }

    #[test]
    fn unknown_embedded_layout_errors() {
        assert!(matches!(
            embedded_layout("xx"),
            Err(LayoutError::UnknownEmbedded(_))
        ));
    }

    #[test]
    fn override_dir_missing_file_falls_back_to_embedded() {
        // Неіснуючий каталог → fallback на вбудовану без помилки.
        let uk = load_layout("uk", Some(Path::new("definitely/missing/dir"))).unwrap();
        assert_eq!(uk.id().as_str(), "uk");
    }

    // --- LM ----------------------------------------------------------------

    #[test]
    fn sample_lm_trains_and_distinguishes_languages() {
        let uk = sample_lm("uk").unwrap();
        let en = sample_lm("en").unwrap();
        assert!(!uk.is_empty() && !en.is_empty());
        // Зразок дає консистентну модель: реальні слова правдоподібніші за шум
        // у відповідній мові.
        assert!(uk.score("привіт") > uk.score("ghbdsn"));
        assert!(en.score("hello") > en.score("привіт"));
    }

    #[test]
    fn lm_bincode_roundtrip_preserves_scores() {
        let uk = sample_lm("uk").unwrap();
        let bytes = serialize_lm(&uk).unwrap();
        let restored = deserialize_lm(&bytes).unwrap();
        // Повна рівність моделі + однакові бали.
        assert_eq!(restored, uk);
        assert_eq!(restored.score("привіт"), uk.score("привіт"));
    }

    #[test]
    fn unknown_sample_lang_errors() {
        assert!(matches!(sample_lm("xx"), Err(ModelError::UnknownSample(_))));
        assert!(matches!(
            sample_dict("xx"),
            Err(ModelError::UnknownSample(_))
        ));
    }

    // --- Словник (FST) -----------------------------------------------------

    #[test]
    fn sample_dict_contains_expected_words() {
        let uk = sample_dict("uk").unwrap();
        assert!(uk.contains("привіт"));
        assert!(uk.contains("світ"));
        assert!(uk.contains("сім'я"));
        assert!(!uk.contains("ghbdsn"));

        let en = sample_dict("en").unwrap();
        assert!(en.contains("hello"));
        assert!(!en.contains("привіт"));
    }

    #[test]
    fn dict_fst_bytes_roundtrip() {
        let uk = sample_dict("uk").unwrap();
        let restored = Dictionary::from_bytes(uk.as_bytes().to_vec()).unwrap();
        assert!(restored.contains("привіт"));
        assert_eq!(restored.len(), uk.len());
    }
}
