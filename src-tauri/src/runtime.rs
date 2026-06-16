//! Рантайм-цикл рушія: з'єднує живу платформу (Windows-хук) із чистим ядром.
//!
//! ## Як це влаштовано
//! - Коли застосунок **увімкнено**, [`RuntimeManager`] піднімає окремий потік
//!   `typofix-engine`. Той створює `WindowsPlatform` (ставить системні хуки),
//!   у циклі тягне [`InputEvent`] (`try_next_event`), подає у
//!   `typofix_core::step(state, ev, ctx)` і застосовує отримані [`Action`] через
//!   `platform.apply` (Unicode-перенабір, switch layout тощо).
//! - **Пауза/вимкнення** = зупиняємо потік (прапорець + `join`); `Drop` для
//!   `WindowsPlatform` знімає хуки. Тобто на паузі ми взагалі НЕ перехоплюємо ввід.
//! - Зміна налаштувань у вікні → [`RuntimeManager::apply`] перезапускає потік із
//!   новим `Context` (виключення/детектор/мови перебудовуються).
//!
//! ## Маппінг конфіг → ядро
//! [`exclusion_rules_from`] / [`detector_config_from`] / [`load_language_profiles`]
//! — **чисті** (без хука), тож тестуються без живої системи.
//!
//! ## Самонавчання
//! Рушій емітить [`Action::CommitException(word)`] коли користувач відкинув
//! перенабір. App-шар дозаписує слово у `learned_exceptions.txt` (поряд із
//! settings.json), а на старті засіває ним `EngineState.learned`. Ядро саме
//! нічого не персистить (лишається чистим). Приватність: лише самі слова, локально.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use typofix_core::{DetectorConfig, ExclusionRules, LanguageProfile, LayoutId};

use crate::config::{AppSettings, LanguagePair};

/// Ім'я файлу навчених винятків (поряд із `settings.json`).
pub const LEARNED_FILE: &str = "learned_exceptions.txt";

// ===========================================================================
// Маппінг конфіг → ядро (чисте, тестоване без хука)
// ===========================================================================

/// Зібрати [`ExclusionRules`] зі списків виключень конфігу.
/// Нормалізацію шляхів (lowercase, `/`→`\`) робить саме `ExclusionRules`.
pub fn exclusion_rules_from(settings: &AppSettings) -> ExclusionRules {
    let mut rules = ExclusionRules::new();
    for name in &settings.exclusions.process_names {
        rules.exclude_process(name);
    }
    for exe in &settings.exclusions.exe_paths {
        rules.exclude_exe(exe);
    }
    for folder in &settings.exclusions.folders {
        rules.exclude_folder(folder);
    }
    rules
}

/// Зібрати [`DetectorConfig`] із advanced-порогів конфігу.
///
/// `min_word_len` → `min_switch_len` (прямий, змістовний маппінг).
/// `confidence_threshold` (0..1) масштабує `base_threshold` монотонно навколо
/// дефолту (0.75 = без зміни). Це **тимчасова** евристика: внутрішній поріг
/// детектора — це лог-ймовірнісний запас, а не 0..1-впевненість; справжня
/// калібровка — у фазі eval. Вищий конфіг → консервативніше (precision > recall).
pub fn detector_config_from(settings: &AppSettings) -> DetectorConfig {
    let base = DetectorConfig::default();
    let conf = settings.detection.confidence_threshold.clamp(0.0, 1.0);
    DetectorConfig {
        min_switch_len: usize::from(settings.detection.min_word_len.max(1)),
        base_threshold: base.base_threshold * (conf / 0.75),
        ..base
    }
}

/// Ідентифікатори мов для пари (поки фіксовано uk↔en).
pub fn langs_for(pair: LanguagePair) -> [&'static str; 2] {
    match pair {
        LanguagePair::UkEn => ["uk", "en"],
    }
}

/// Завантажити профілі мов (розкладка + LM + словник) для пари.
///
/// Без `override_dir` беруться вбудовані зразки (наскрізна робота «з коробки»);
/// реальні `.bin`/`.fst` підхопляться з `override_dir`, коли з'являться. Чисте
/// IO без хука → тестовно.
pub fn load_language_profiles(
    pair: LanguagePair,
    override_dir: Option<&Path>,
) -> Result<Vec<LanguageProfile>, String> {
    let mut profiles = Vec::new();
    for lang in langs_for(pair) {
        let layout = typofix_data::load_layout(lang, override_dir).map_err(|e| e.to_string())?;
        let lm = typofix_data::load_lm(lang, override_dir).map_err(|e| e.to_string())?;
        let dict = typofix_data::load_dict(lang, override_dir).map_err(|e| e.to_string())?;
        profiles.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout,
            lm,
            dict,
        });
    }
    Ok(profiles)
}

// ===========================================================================
// Персистенція навчених винятків (чисте, path-based, тестоване)
// ===========================================================================

/// Прочитати навчені слова з диска (по слову на рядок). Файлу немає → порожньо.
pub fn load_learned(path: &Path) -> Vec<String> {
    match fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Дозаписати одне навчене слово. Дублі нешкідливі: засів через `learn()` їх
/// дедуплікує в пам'яті. Створює каталог за потреби.
pub fn append_learned(path: &Path, word: &str) -> std::io::Result<()> {
    let w = word.trim();
    if w.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{w}")
}

// ===========================================================================
// Менеджер рантайму: старт/стоп потоку рушія
// ===========================================================================

/// Керує життєвим циклом потоку рушія. Зберігається у Tauri-стані за `Mutex`.
#[derive(Default)]
pub struct RuntimeManager {
    #[cfg(windows)]
    engine: Option<EngineHandle>,
}

impl RuntimeManager {
    /// Привести рантайм у відповідність до налаштувань: увімкнено → (пере)запуск
    /// рушія з актуальним конфігом; пауза/вимкнено → зупинка.
    pub fn apply(
        &mut self,
        settings: &AppSettings,
        learned_path: PathBuf,
        data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        self.stop_engine();
        if settings.enabled {
            self.start_engine(settings, learned_path, data_dir)?;
        }
        Ok(())
    }

    /// Зупинити рушій (при виході із застосунку).
    pub fn shutdown(&mut self) {
        self.stop_engine();
    }

    #[cfg(windows)]
    fn stop_engine(&mut self) {
        if let Some(handle) = self.engine.take() {
            handle.stop();
        }
    }

    #[cfg(not(windows))]
    fn stop_engine(&mut self) {}

    #[cfg(windows)]
    fn start_engine(
        &mut self,
        settings: &AppSettings,
        learned_path: PathBuf,
        data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        let exclusions = exclusion_rules_from(settings);
        let config = detector_config_from(settings);
        let languages = load_language_profiles(settings.language, data_dir.as_deref())?;
        let seed = load_learned(&learned_path);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("typofix-engine".to_string())
            .spawn(move || {
                engine_loop(
                    stop_for_thread,
                    exclusions,
                    config,
                    languages,
                    seed,
                    learned_path,
                );
            })
            .map_err(|e| format!("не вдалося запустити потік рушія: {e}"))?;

        self.engine = Some(EngineHandle { stop, thread });
        Ok(())
    }

    #[cfg(not(windows))]
    fn start_engine(
        &mut self,
        _settings: &AppSettings,
        _learned_path: PathBuf,
        _data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        // Жива платформа лише на Windows; на інших цілях рушій — no-op (порт згодом).
        Ok(())
    }
}

/// Хендл живого потоку рушія: прапорець зупинки + сам потік.
#[cfg(windows)]
struct EngineHandle {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
}

#[cfg(windows)]
impl EngineHandle {
    /// Просигналити зупинку й дочекатися завершення (Drop знімає хуки).
    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.thread.join();
    }
}

/// Тіло потоку рушія: тягне події з платформи, проганяє через ядро, застосовує дії.
#[cfg(windows)]
fn engine_loop(
    stop: Arc<AtomicBool>,
    exclusions: ExclusionRules,
    config: DetectorConfig,
    languages: Vec<LanguageProfile>,
    seed: Vec<String>,
    learned_path: PathBuf,
) {
    use std::time::Duration;

    use typofix_core::{step, Action, Context, EngineState, WordRules};
    use typofix_platform::Platform;
    use typofix_platform_windows::WindowsPlatform;

    // ⚠️ Ставить системні хуки на весь час життя потоку.
    let mut platform = WindowsPlatform::new();

    let mut state = EngineState::default();
    for word in &seed {
        state.learned.learn(word);
    }
    // Word-level veto/force ще не в конфігу — поки порожньо (форма Фази 6 їх не має).
    let rules = WordRules::new();

    while !stop.load(Ordering::SeqCst) {
        let Some(event) = platform.try_next_event() else {
            // Канал порожній — коротка пауза, щоб не крутити CPU вхолосту.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };

        let ctx = Context {
            active_window: platform.active_window(),
            current_layout: platform.current_layout(),
            languages: &languages,
            config,
            exclusions: &exclusions,
            rules: &rules,
        };

        let actions = step(&mut state, event, &ctx);
        for action in &actions {
            // Самонавчання персистимо тут (ядро лишається чистим).
            if let Action::CommitException(word) = action {
                if let Err(e) = append_learned(&learned_path, word) {
                    eprintln!("TypoFix: не вдалося зберегти навчене слово: {e}");
                }
            }
            platform.apply(action);
        }
    }
    // Вихід із циклу → drop(platform) знімає хуки.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DetectionDto, ExclusionsDto};
    use typofix_core::WindowInfo;

    fn settings_with(exclusions: ExclusionsDto, detection: DetectionDto) -> AppSettings {
        AppSettings {
            exclusions,
            detection,
            ..Default::default()
        }
    }

    #[test]
    fn exclusion_mapping_covers_process_exe_and_folder() {
        let settings = settings_with(
            ExclusionsDto {
                process_names: vec!["game.exe".into()],
                exe_paths: vec![r"C:\Apps\tool.exe".into()],
                folders: vec![r"C:\Games".into()],
            },
            DetectionDto::default(),
        );
        let rules = exclusion_rules_from(&settings);

        let by_process = WindowInfo {
            process_name: "game.exe".into(),
            ..Default::default()
        };
        let by_exe = WindowInfo {
            process_name: "tool.exe".into(),
            exe_path: r"C:\Apps\tool.exe".into(),
            ..Default::default()
        };
        let by_folder = WindowInfo {
            process_name: "x.exe".into(),
            exe_path: r"C:\Games\sub\x.exe".into(),
            ..Default::default()
        };
        let allowed = WindowInfo {
            process_name: "editor.exe".into(),
            exe_path: r"C:\Work\editor.exe".into(),
            ..Default::default()
        };

        assert!(rules.excludes(&by_process));
        assert!(rules.excludes(&by_exe));
        assert!(rules.excludes(&by_folder));
        assert!(!rules.excludes(&allowed));
    }

    #[test]
    fn detector_mapping_sets_min_switch_len() {
        let settings = settings_with(
            ExclusionsDto::default(),
            DetectionDto {
                min_word_len: 4,
                confidence_threshold: 0.75,
            },
        );
        let cfg = detector_config_from(&settings);
        assert_eq!(cfg.min_switch_len, 4);
        // 0.75 → база незмінна.
        assert_eq!(cfg.base_threshold, DetectorConfig::default().base_threshold);
    }

    #[test]
    fn detector_threshold_is_monotonic_in_confidence() {
        let low = detector_config_from(&settings_with(
            ExclusionsDto::default(),
            DetectionDto {
                min_word_len: 3,
                confidence_threshold: 0.3,
            },
        ));
        let high = detector_config_from(&settings_with(
            ExclusionsDto::default(),
            DetectionDto {
                min_word_len: 3,
                confidence_threshold: 0.95,
            },
        ));
        assert!(high.base_threshold > low.base_threshold);
    }

    #[test]
    fn language_profiles_load_uk_and_en_from_embedded_samples() {
        let profiles = load_language_profiles(LanguagePair::UkEn, None).unwrap();
        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["uk", "en"]);
        // Профілі реально завантажились (непорожні моделі/розкладки).
        assert!(!profiles[0].layout.is_empty());
        assert!(!profiles[0].lm.is_empty());
    }

    #[test]
    fn learned_roundtrips_and_skips_blank() {
        let path = std::env::temp_dir().join(format!("typofix-learned-{}.txt", std::process::id()));
        let _ = fs::remove_file(&path);

        append_learned(&path, "привіт").unwrap();
        append_learned(&path, "  світ  ").unwrap();
        append_learned(&path, "   ").unwrap(); // порожнє — ігнорується

        let words = load_learned(&path);
        assert_eq!(words, vec!["привіт", "світ"]);

        let _ = fs::remove_file(&path);
    }
}
