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

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use fst::Map as FstMap;
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

// --- Частотний словник (fst::Map: слово → count) ---------------------------
//
// `{lang}.freq.fst` — `fst::Map`, що відображає слово (lowercase) у його розмовну
// частоту (count з OpenSubtitles, готує `data/fetch_freq.py`). На відміну від
// `Dictionary` (бінарне членство), даває core ГРАДУЙОВАНИЙ сигнал: часте слово
// (`the`,`you`) → НЕ чіпати; рідкісне/відсутнє (`ye`,`lox`) → можна перемкнути.
//
// ІНТЕРФЕЙС ДЛЯ CORE (узгоджено з Den): `freq(word) -> Option<u64>`. Відсутність
// запису (None) ≠ «не слово»: валідні рідкісні флексії довгого хвоста VESUM не
// мають freq-запису, але Є членами `Dictionary` → score() дає їм BASELINE-бонус,
// а freq лише ДОДАЄ зважування зверху. Власник score() — core (Den); тип-обгортку
// для `LanguageProfile` він визначає в core (як `Dictionary`), беручи сирі байти
// з `build_freq_map`/читаючи через `load_freq_map_file`.

/// Побудувати байти `fst::Map` (слово → count) зі списку пар. Дублі по слову
/// зливаються максимумом; вхід сортується/дедуплікується (вимога FST).
pub fn build_freq_map<I, S>(entries: I) -> Result<Vec<u8>, ModelError>
where
    I: IntoIterator<Item = (S, u64)>,
    S: AsRef<str>,
{
    let mut sorted: BTreeMap<String, u64> = BTreeMap::new();
    for (w, c) in entries {
        let key = w.as_ref().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let e = sorted.entry(key).or_insert(0);
        *e = (*e).max(c);
    }
    let map = FstMap::from_iter(sorted)?;
    Ok(map.as_fst().as_bytes().to_vec())
}

/// Записати частотний словник у файл `.freq.fst`.
pub fn save_freq_map(bytes: &[u8], path: &Path) -> Result<(), ModelError> {
    std::fs::write(path, bytes).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Завантажити частотний словник із файлу `.freq.fst`.
pub fn load_freq_map_file(path: &Path) -> Result<FstMap<Vec<u8>>, ModelError> {
    let bytes = std::fs::read(path).map_err(|source| ModelError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(FstMap::new(bytes)?)
}

/// Частота слова у словнику (регістронезалежно), або `None`, якщо запису немає.
pub fn freq_lookup(map: &FstMap<Vec<u8>>, word: &str) -> Option<u64> {
    map.get(word.to_lowercase())
}

// --- ISO 4217 коди валют (veto валютних пар) -------------------------------
//
// `data/dicts/iso4217.txt` — активні alphabetic-коди валют (один на рядок,
// UPPERCASE, `#` — коментар). Дані для core (Den): veto-правило живе в core, не
// тут. Патерн (у core): токен `^[A-Z]{6}$`, де `[0:3] ∈ ISO ∩ [3:6] ∈ ISO` —
// валютна пара (EURUSD, XAUUSD…) → не перемикати розкладку. Системно: множина
// кодів + патерн, не хардкод пар.
//
// ІНТЕРФЕЙС ДЛЯ CORE: `load_iso4217(&Path) -> HashSet<String>` (UPPERCASE-коди).
// Конструювати вето — у core: він тримає `HashSet`, перевіряє половинки токена.
// `is_currency_pair` — суто-даний (членство в множині) хелпер; КОЛИ його кликати
// (тобто власне veto в `step`) вирішує core.

/// Вбудований перелік ISO 4217 (committed, малий — кілька КБ).
const ISO4217_TXT: &str = include_str!("../../../data/dicts/iso4217.txt");

/// Розпарсити перелік ISO 4217 у множину кодів. Один рядок = один код,
/// `#` — коментар, порожні рядки ігноруються; коди нормалізуються в UPPERCASE.
pub fn parse_iso4217(src: &str) -> HashSet<String> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_uppercase())
        .collect()
}

/// Множина кодів із вбудованого переліку (детерміновано, без IO).
pub fn embedded_iso4217() -> HashSet<String> {
    parse_iso4217(ISO4217_TXT)
}

/// Завантажити перелік ISO 4217 із файлу на диску.
/// Файлу немає → вбудований перелік (щоб core завжди мав робочу множину).
pub fn load_iso4217(path: &Path) -> std::io::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(embedded_iso4217());
    }
    Ok(parse_iso4217(&std::fs::read_to_string(path)?))
}

/// Чи є `token` валютною парою щодо множини кодів `iso`: рівно 6 ASCII-літер,
/// перша й друга половини (по 3) — обидві коди ISO. Регістронезалежно.
/// Суто перевірка членства в множині (дані); власне veto-рішення — у core.
pub fn is_currency_pair(iso: &HashSet<String>, token: &str) -> bool {
    if token.len() != 6 || !token.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    let up = token.to_uppercase();
    iso.contains(&up[0..3]) && iso.contains(&up[3..6])
}

// --- Особистий словник користувача (veto «ніколи не чіпати») ---------------
//
// `data/dicts/user.txt` — персональний whitelist (жаргон, нікнейми, бренди),
// один термін на рядок, `#` — коментар. Loader читає у рантаймі; core тримає як
// veto-набір (збіг → не перемикати). Регістронезалежність — на боці core
// (матчинг), тут повертаємо терміни як є (trim), без зміни регістру.

/// Розпарсити особистий словник: один рядок = один термін, `#` — коментар,
/// порожні рядки ігноруються, краї обрізаються.
pub fn parse_user_words(src: &str) -> Vec<String> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// Завантажити особистий словник користувача з файлу.
/// Файлу немає → порожній список (veto-набір просто порожній).
pub fn load_user_words(path: &Path) -> std::io::Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(parse_user_words(&std::fs::read_to_string(path)?))
}

// --- Файлові розширення (switch: безглузде укр. читання → латиниця) ---------
//
// `data/dicts/extensions.txt` — добре відомі розширення БЕЗ крапки (один на
// рядок, lowercase, `#` — коментар). Сценарій: англ. розширення, набране в укр.
// розкладці, виходить безглуздям (`txt`→`еche`) → перемкнути на латиницю.
// Loader дає лише МНОЖИНУ; гейт «не чіпати, якщо укр. читання — реальне слово»
// (для ризикових `doc`/`log`/`go`…) робить core (Den).
//
// ІНТЕРФЕЙС ДЛЯ CORE: `load_extensions(&Path) -> HashSet<String>` (lowercase),
// `is_known_extension(&set, token) -> bool` (lowercase membership). Список
// найризиковіших (короткі / схожі на слова) — `data/CLAUDE.md`.

/// Вбудований перелік розширень (committed, малий — кілька КБ).
const EXTENSIONS_TXT: &str = include_str!("../../../data/dicts/extensions.txt");

/// Розпарсити перелік розширень у множину. Один рядок = один токен, `#` —
/// коментар, порожні рядки ігноруються; токени нормалізуються в lowercase.
pub fn parse_extensions(src: &str) -> HashSet<String> {
    src.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_lowercase())
        .collect()
}

/// Множина розширень із вбудованого переліку (детерміновано, без IO).
pub fn embedded_extensions() -> HashSet<String> {
    parse_extensions(EXTENSIONS_TXT)
}

/// Завантажити перелік розширень із файлу на диску.
/// Файлу немає → вбудований перелік (щоб core завжди мав робочу множину).
pub fn load_extensions(path: &Path) -> std::io::Result<HashSet<String>> {
    if !path.exists() {
        return Ok(embedded_extensions());
    }
    Ok(parse_extensions(&std::fs::read_to_string(path)?))
}

/// Чи є `token` відомим файловим розширенням щодо множини `ext` (lowercase
/// membership; провідну крапку, якщо є, ігноруємо). Суто перевірка членства;
/// гейт «укр. читання — реальне слово» для ризикових токенів робить core.
pub fn is_known_extension(ext: &HashSet<String>, token: &str) -> bool {
    let t = token.trim_start_matches('.').to_lowercase();
    !t.is_empty() && ext.contains(&t)
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
    fn freq_map_builds_and_looks_up() {
        let bytes = build_freq_map([("the", 1000u64), ("ye", 5u64), ("THE", 200u64)]).unwrap();
        let map = FstMap::new(bytes).unwrap();
        // Регістронезалежно + дублі злиті максимумом.
        assert_eq!(freq_lookup(&map, "the"), Some(1000));
        assert_eq!(freq_lookup(&map, "The"), Some(1000));
        assert_eq!(freq_lookup(&map, "ye"), Some(5));
        // Відсутнє слово → None (≠ нульова частота).
        assert_eq!(freq_lookup(&map, "lox"), None);
    }

    // --- ISO 4217 + user.txt -----------------------------------------------

    #[test]
    fn iso4217_parses_majors_and_metals() {
        let iso = embedded_iso4217();
        for c in ["EUR", "USD", "GBP", "JPY", "CHF", "AUD", "CAD", "NZD"] {
            assert!(iso.contains(c), "очікувано {c} у ISO-множині");
        }
        // Дорогоцінні метали — реальні Forex-інструменти, лишені у переліку.
        assert!(iso.contains("XAU") && iso.contains("XAG"));
        // Тестові/службові свідомо виключені (зменшують хибні veto).
        assert!(!iso.contains("XXX") && !iso.contains("XTS"));
        // Розумний обсяг активного набору.
        assert!(iso.len() > 140, "надто мало кодів: {}", iso.len());
    }

    #[test]
    fn iso4217_ignores_comments_and_normalizes_case() {
        let set = parse_iso4217("# коментар\n eur \nUSD\n\n# ще\ngbp");
        assert_eq!(set.len(), 3);
        assert!(set.contains("EUR") && set.contains("USD") && set.contains("GBP"));
    }

    #[test]
    fn currency_pair_detection() {
        let iso = embedded_iso4217();
        assert!(is_currency_pair(&iso, "EURUSD"));
        assert!(is_currency_pair(&iso, "GBPUSD"));
        assert!(is_currency_pair(&iso, "XAUUSD")); // золото/долар
        assert!(is_currency_pair(&iso, "eurusd")); // регістронезалежно
                                                   // Не пара:
        assert!(!is_currency_pair(&iso, "HELLOO")); // обидві половини — не коди
        assert!(!is_currency_pair(&iso, "EURGHB")); // друга половина не код
        assert!(!is_currency_pair(&iso, "EUR")); // не 6 літер
        assert!(!is_currency_pair(&iso, "EURUS1")); // не лише літери
    }

    #[test]
    fn load_iso4217_falls_back_to_embedded() {
        let iso = load_iso4217(Path::new("definitely/missing/iso.txt")).unwrap();
        assert!(iso.contains("EUR"));
    }

    #[test]
    fn user_words_parses_and_skips_comments() {
        let words = parse_user_words("# хедер\nвжух\n  крякозябри  \n\n# коментар\nEURUSD");
        assert_eq!(words, vec!["вжух", "крякозябри", "EURUSD"]);
    }

    #[test]
    fn load_user_words_missing_file_is_empty() {
        let words = load_user_words(Path::new("definitely/missing/user.txt")).unwrap();
        assert!(words.is_empty());
    }

    // --- Файлові розширення ------------------------------------------------

    #[test]
    fn extensions_parse_known_set() {
        let ext = embedded_extensions();
        for e in ["txt", "md", "pdf", "rs", "json", "png", "mp4", "docx"] {
            assert!(ext.contains(e), "очікувано {e} у множині розширень");
        }
        // Усе lowercase, без крапок, без коментарів.
        assert!(ext
            .iter()
            .all(|e| !e.starts_with('#') && !e.starts_with('.')));
        assert!(ext.len() > 60, "надто мало розширень: {}", ext.len());
    }

    #[test]
    fn extensions_normalize_and_ignore_comments() {
        let set = parse_extensions("# хедер\n TXT \nMd\n\n# ще\n.pdf");
        // Lowercase; `.pdf` лишається з крапкою (parse не чистить крапку — це
        // робить is_known_extension), тож тут перевіряємо саме нормалізацію.
        assert!(set.contains("txt") && set.contains("md"));
    }

    #[test]
    fn known_extension_membership() {
        let ext = embedded_extensions();
        assert!(is_known_extension(&ext, "txt"));
        assert!(is_known_extension(&ext, "TXT")); // регістронезалежно
        assert!(is_known_extension(&ext, ".pdf")); // провідна крапка ок
        assert!(!is_known_extension(&ext, "zzz")); // невідоме
        assert!(!is_known_extension(&ext, "")); // порожнє
        assert!(!is_known_extension(&ext, ".")); // лише крапка
    }

    #[test]
    fn load_extensions_falls_back_to_embedded() {
        let ext = load_extensions(Path::new("definitely/missing/ext.txt")).unwrap();
        assert!(ext.contains("txt"));
    }

    #[test]
    fn dict_fst_bytes_roundtrip() {
        let uk = sample_dict("uk").unwrap();
        let restored = Dictionary::from_bytes(uk.as_bytes().to_vec()).unwrap();
        assert!(restored.contains("привіт"));
        assert_eq!(restored.len(), uk.len());
    }
}
