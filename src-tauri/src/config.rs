//! Конфіг застосунку: DTO + читання/запис на диск.
//!
//! ## Приватність (залізне правило проєкту)
//! У файл конфігу йдуть **ЛИШЕ налаштування** — списки виключень, прапорці,
//! мова, пороги. НІКОЛИ не натиски, буфер чи будь-який набраний текст (вони
//! живуть тільки в RAM ядра й не серіалізуються).
//!
//! ## Чому власний DTO, а не типи `typofix-core`
//! Це app-шар у відокремленому workspace; він НЕ залежить від `typofix-core`.
//! DTO дзеркалить форму `ExclusionRules` (process/exe/folder) суто для
//! редагування+збереження. Маппінг DTO → core-правила зробиться при живій
//! проводці двигуна (Фаза 5).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Ім'я файлу конфігу в каталозі застосунку (Tauri app config dir).
pub const SETTINGS_FILE: &str = "settings.json";

/// Поточна версія схеми конфігу (для майбутніх міграцій).
pub const SCHEMA_VERSION: u32 = 1;

/// Мовна пара. Поки фіксовано uk↔en, але закладено в модель як enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LanguagePair {
    /// Українська ↔ англійська.
    #[default]
    #[serde(rename = "uk-en")]
    UkEn,
}

/// Списки виключень — дзеркало форми `core::ExclusionRules`.
/// Усі рядки зберігаються як ввів користувач; нормалізацію (lowercase, `/`→`\`)
/// робить core при матчингу — тут лише тримаємо й валідуємо непорожність.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExclusionsDto {
    /// Імена процесів, напр. `game.exe`.
    pub process_names: Vec<String>,
    /// Повні exe-шляхи.
    pub exe_paths: Vec<String>,
    /// Теки (exe-prefix, рекурсивно).
    pub folders: Vec<String>,
}

/// Винятки рівня СЛОВА (як у Punto Switcher) — особистий словник.
/// Дзеркало `core::WordRules` (позитив + veto). На відміну від `ExclusionsDto`
/// тут нормалізуємо регістр (lowercase): матчинг у ядрі регістронезалежний,
/// тож зберігання в нижньому регістрі прибирає дублі-варіанти регістру.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WordsDto {
    /// «Завжди перемикати» — позитивний особистий словник: слова, які апка має
    /// ВИЗНАВАТИ й перемикати на них (жаргон/нікнейми/forex поза стандартним
    /// словником, напр. `лох`). Об'єднується з `data/dicts/user.txt`.
    pub always_switch: Vec<String>,
    /// «Ніколи не перемикати» — per-word veto: слова, які лишати недоторканими.
    pub never_switch: Vec<String>,
}

/// Пороги детектора (advanced). Дзеркало майбутнього `DetectorConfig`.
/// Значення за замовч. — консервативні (precision > recall).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetectionDto {
    /// Мінімальна довжина слова, яке взагалі розглядаємо.
    pub min_word_len: u8,
    /// Поріг впевненості (0.0–1.0), вище якого перенабираємо.
    pub confidence_threshold: f64,
}

impl Default for DetectionDto {
    fn default() -> Self {
        Self {
            min_word_len: 3,
            confidence_threshold: 0.75,
        }
    }
}

/// Кореневий DTO налаштувань, який серіалізується у `settings.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Версія схеми (для міграцій).
    pub version: u32,
    /// `true` = активний, `false` = пауза. Синхронізовано з треєм.
    pub enabled: bool,
    /// Мовна пара.
    pub language: LanguagePair,
    /// Виключення застосунків/папок.
    pub exclusions: ExclusionsDto,
    /// Винятки рівня слова (особистий словник: always/never switch).
    pub words: WordsDto,
    /// Пороги детектора (advanced).
    pub detection: DetectionDto,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            enabled: true,
            language: LanguagePair::UkEn,
            exclusions: ExclusionsDto::default(),
            words: WordsDto::default(),
            detection: DetectionDto::default(),
        }
    }
}

impl AppSettings {
    /// Прибрати порожні/дубльовані рядки у списках виключень (валідація вводу).
    /// Зберігаємо порядок; дублі визначаємо після `trim` (без зміни регістру —
    /// нормалізацію регістру робить core при матчингу).
    pub fn sanitized(mut self) -> Self {
        self.exclusions.process_names = dedup_nonempty(self.exclusions.process_names);
        self.exclusions.exe_paths = dedup_nonempty(self.exclusions.exe_paths);
        self.exclusions.folders = dedup_nonempty(self.exclusions.folders);
        // Слова: trim + нормалізація регістру (lowercase) + дедуп. Регістр
        // нормалізуємо, бо матчинг у ядрі регістронезалежний (на відміну від
        // шляхів виключень, де регістр зберігаємо як ввів користувач).
        self.words.always_switch = dedup_nonempty_lower(self.words.always_switch);
        self.words.never_switch = dedup_nonempty_lower(self.words.never_switch);
        // Тримаємо version у відомому діапазоні (на випадок підробленого файлу).
        if self.version == 0 {
            self.version = SCHEMA_VERSION;
        }
        self
    }
}

/// Викинути порожні після trim і дублі (стабільний порядок першої появи).
fn dedup_nonempty(items: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(items.len());
    for raw in items {
        let trimmed = raw.trim().to_string();
        if !trimmed.is_empty() && !seen.contains(&trimmed) {
            seen.push(trimmed);
        }
    }
    seen
}

/// Як [`dedup_nonempty`], але ще нормалізує регістр у lowercase (для слів-винятків,
/// де матчинг у ядрі регістронезалежний → дублі-варіанти регістру схлопуються).
fn dedup_nonempty_lower(items: Vec<String>) -> Vec<String> {
    let mut seen: Vec<String> = Vec::with_capacity(items.len());
    for raw in items {
        let norm = raw.trim().to_lowercase();
        if !norm.is_empty() && !seen.contains(&norm) {
            seen.push(norm);
        }
    }
    seen
}

/// Каталог конфігу застосунку (Tauri app config dir). Тут живуть `settings.json`
/// і файл навчених винятків.
pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map_err(|e| format!("немає каталогу конфігу застосунку: {e}"))
}

/// Повний шлях до файлу конфігу.
fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join(SETTINGS_FILE))
}

/// Прочитати конфіг із диска. Файлу немає → дефолти (перший запуск).
/// Файл є, але пошкоджений → помилка (UI попередить, не затираємо мовчки).
pub fn load_from_disk(app: &AppHandle) -> Result<AppSettings, String> {
    read_from_path(&settings_path(app)?)
}

/// Записати конфіг на диск атомарно, створивши каталог.
pub fn save_to_disk(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    write_to_path(&settings_path(app)?, settings)
}

/// Прочитати конфіг із конкретного шляху (чиста IO без `AppHandle` → тестовно).
fn read_from_path(path: &Path) -> Result<AppSettings, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<AppSettings>(&text)
            .map(AppSettings::sanitized)
            .map_err(|e| format!("пошкоджений конфіг {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(format!("не вдалося прочитати {}: {e}", path.display())),
    }
}

/// Записати конфіг атомарно (tmp → rename), створивши каталог.
fn write_to_path(path: &Path, settings: &AppSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("не вдалося створити {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(settings).map_err(|e| format!("серіалізація конфігу: {e}"))?;
    // Атомарність: пишемо в тимчасовий файл, потім rename поверх цілі —
    // так напівзаписаний файл ніколи не стане «справжнім» конфігом.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("запис {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename у {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roundtrips_through_json() {
        let s = AppSettings::default();
        let json = serde_json::to_string_pretty(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn language_pair_serializes_as_kebab() {
        let json = serde_json::to_string(&LanguagePair::UkEn).unwrap();
        assert_eq!(json, "\"uk-en\"");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Лише enabled задано — решта з #[serde(default)].
        let partial = r#"{ "enabled": false }"#;
        let s: AppSettings = serde_json::from_str(partial).unwrap();
        assert!(!s.enabled);
        assert_eq!(s.language, LanguagePair::UkEn);
        assert_eq!(s.detection, DetectionDto::default());
        assert!(s.exclusions.process_names.is_empty());
    }

    #[test]
    fn missing_file_yields_defaults() {
        let path =
            std::env::temp_dir().join(format!("typofix-missing-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);
        assert_eq!(read_from_path(&path).unwrap(), AppSettings::default());
    }

    #[test]
    fn corrupt_file_is_an_error_not_silent_default() {
        let path =
            std::env::temp_dir().join(format!("typofix-corrupt-{}.json", std::process::id()));
        fs::write(&path, b"{ not json").unwrap();
        assert!(read_from_path(&path).is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn write_then_read_roundtrips_through_disk() {
        let path =
            std::env::temp_dir().join(format!("typofix-roundtrip-{}.json", std::process::id()));
        let _ = fs::remove_file(&path);

        let original = AppSettings {
            enabled: false,
            exclusions: ExclusionsDto {
                process_names: vec!["game.exe".into()],
                folders: vec![r"C:\Games".into()],
                ..Default::default()
            },
            detection: DetectionDto {
                confidence_threshold: 0.9,
                ..Default::default()
            },
            ..Default::default()
        };

        write_to_path(&path, &original).unwrap();
        let loaded = read_from_path(&path).unwrap();
        assert_eq!(loaded, original);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn sanitize_drops_empty_and_duplicate_entries() {
        let mut s = AppSettings::default();
        s.exclusions.process_names = vec![
            "game.exe".into(),
            "  game.exe  ".into(), // дубль після trim
            "   ".into(),          // порожній
            "other.exe".into(),
        ];
        let s = s.sanitized();
        assert_eq!(s.exclusions.process_names, vec!["game.exe", "other.exe"]);
    }

    #[test]
    fn sanitize_words_trims_lowercases_and_dedups() {
        let mut s = AppSettings::default();
        s.words.always_switch = vec![
            "Лох".into(),
            "  лох  ".into(), // дубль після trim+lowercase
            "ЛОХ".into(),     // дубль за регістром
            "   ".into(),     // порожній
            "EURUSD".into(),
        ];
        s.words.never_switch = vec!["Vec".into(), "vec".into()];
        let s = s.sanitized();
        assert_eq!(s.words.always_switch, vec!["лох", "eurusd"]);
        assert_eq!(s.words.never_switch, vec!["vec"]);
    }

    #[test]
    fn words_missing_field_falls_back_to_default() {
        // Старий settings.json без секції `words` читається без падіння.
        let partial = r#"{ "enabled": true }"#;
        let s: AppSettings = serde_json::from_str(partial).unwrap();
        assert!(s.words.always_switch.is_empty());
        assert!(s.words.never_switch.is_empty());
    }
}
