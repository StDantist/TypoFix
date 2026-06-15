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

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use typofix_core::{KeyCap, Layout, LayoutId};

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
}
