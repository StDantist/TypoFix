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

use tauri::AppHandle;
use typofix_core::{
    CaseMode, DetectorConfig, ExclusionRules, FrequencyMap, LanguageProfile, LayoutId, WordRules,
};

use crate::config::{AppSettings, LanguagePair, WordsDto};

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

/// Зібрати [`DetectorConfig`] із порогів і перемикачів поведінки конфігу.
///
/// `min_word_len` → `min_switch_len` (прямий, змістовний маппінг).
/// `confidence_threshold` (0..1) масштабує `base_threshold` монотонно навколо
/// дефолту (0.75 = без зміни). Це **тимчасова** евристика: внутрішній поріг
/// детектора — це лог-ймовірнісний запас, а не 0..1-впевненість; справжня
/// калібровка — у фазі eval. Вищий конфіг → консервативніше (precision > recall).
///
/// **Перемикачі поведінки (B4)** — `BehaviorDto` → `*_enabled`-прапорці 1:1:
/// `fix_case`→`case_fix_enabled`, `forex`→`forex_enabled`,
/// `recognize_extensions`→`extensions_enabled`, `phonotactics`→`phonotactics_enabled`,
/// `fix_capslock`→`capslock_fix_enabled`. Решта полів — з `DetectorConfig::default`.
pub fn detector_config_from(settings: &AppSettings) -> DetectorConfig {
    let base = DetectorConfig::default();
    let conf = settings.detection.confidence_threshold.clamp(0.0, 1.0);
    let b = &settings.behavior;
    DetectorConfig {
        min_switch_len: usize::from(settings.detection.min_word_len.max(1)),
        base_threshold: base.base_threshold * (conf / 0.75),
        case_fix_enabled: b.fix_case,
        forex_enabled: b.forex,
        extensions_enabled: b.recognize_extensions,
        phonotactics_enabled: b.phonotactics,
        capslock_fix_enabled: b.fix_capslock,
        ..base
    }
}

/// Ідентифікатори мов для пари. Тонка обгортка над [`LanguagePair::langs`] —
/// зв'язка «пара → мови» живе в одному місці (config.rs), поряд із варіантом enum,
/// тож тут НЕМАЄ захардкодженого uk/en. Усі лоадери нижче ітерують саме це.
pub fn langs_for(pair: LanguagePair) -> [&'static str; 2] {
    pair.langs()
}

/// Завантажити профілі мов (розкладка + LM + словник) для пари.
///
/// `data_dir` — **корінь** каталогу даних репозиторію (із піддиректоріями
/// `layouts/`, `lm/`, `dicts/`). `typofix-data` шукає файли як
/// `{piддир}/{lang}.{toml,bin,fst}`; відсутній файл/каталог → fallback на
/// вбудований зразок (наскрізна робота «з коробки»). Чисте IO без хука → тестовно.
pub fn load_language_profiles(
    pair: LanguagePair,
    data_dir: Option<&Path>,
) -> Result<Vec<LanguageProfile>, String> {
    // Кожен вид даних має власну піддиректорію — їх і передаємо як override.
    let layout_dir = data_dir.map(|d| d.join("layouts"));
    let lm_dir = data_dir.map(|d| d.join("lm"));
    let dict_dir = data_dir.map(|d| d.join("dicts"));

    let mut profiles = Vec::new();
    for lang in langs_for(pair) {
        let layout =
            typofix_data::load_layout(lang, layout_dir.as_deref()).map_err(|e| e.to_string())?;
        let lm = typofix_data::load_lm(lang, lm_dir.as_deref()).map_err(|e| e.to_string())?;
        let dict = typofix_data::load_dict(lang, dict_dir.as_deref()).map_err(|e| e.to_string())?;
        // Частотна мапа — опційна (м'яка деградація): є `{lang}.freq.fst` →
        // градуйований сигнал; нема/помилка → лише baseline dict-бонус.
        let freq = dict_dir
            .as_deref()
            .map(|d| d.join(format!("{lang}.freq.fst")))
            .filter(|p| p.exists())
            .and_then(|p| typofix_data::load_freq_map_file(&p).ok())
            .map(FrequencyMap::from_fst_map);
        profiles.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout,
            lm,
            dict,
            freq,
        });
    }
    Ok(profiles)
}

/// Завантажити правила рівня слова в [`WordRules`] для мовної пари, **об'єднавши**
/// джерела з файлів `data/` з особистими словами-винятками з налаштувань (`words`):
/// - whitelist коротких СЛУЖБОВИХ слів — `data/dicts/{lang}.short.txt`;
/// - **recognized** (позитив, перемикати) = `user.txt` ∪ `words.always_switch`;
/// - **veto** (ніколи не перемикати) = `words.never_switch` (per-word veto з UI);
/// - ISO 4217 коди валют — `data/dicts/iso4217.txt` (forex-сигнал).
///
/// `data_dir` — **корінь** `data/` (функція сама додає піддиректорію `dicts/`).
/// Whitelist вмикає дзеркальну релаксацію порога коротких слів у детекторі (`от`/
/// `ти`/`we`...). Немає каталогу даних / файлів → файлові джерела порожні (фіча
/// просто вимкнена) — **м'яка деградація, не паніка**; але слова з `words` усе одно
/// застосовуються (вони не залежать від `data/`). Чисте path-based IO → тестовно.
pub fn load_word_rules(pair: LanguagePair, data_dir: Option<&Path>, words: &WordsDto) -> WordRules {
    let mut rules = WordRules::new();

    if let Some(root) = data_dir {
        let dict_dir = root.join("dicts");
        for lang in langs_for(pair) {
            // Помилка читання трактуємо як «немає whitelist» — не валимо рушій.
            let short = typofix_data::load_short_words(lang, &dict_dir).unwrap_or_default();
            let id = LayoutId::new(lang);
            for w in &short {
                rules.allow_short_service(&id, w);
            }
        }

        // Особистий словник (`user.txt`) — ПОЗИТИВНІ визнані слова (жаргон/нікнейми
        // поза стандартним словником): дають dict-бонус → апка перемикає на них.
        // М'яка деградація: нема файлу / помилка → порожньо.
        for w in typofix_data::load_user_words(&dict_dir.join("user.txt")).unwrap_or_default() {
            rules.recognize_word(&w);
        }

        // ISO 4217 коди валют — для розпізнавання валютних пар (forex-сигнал
        // перемикання на латиницю). Нема файлу → вбудований перелік (loader Bruno).
        if let Ok(codes) = typofix_data::load_iso4217(&dict_dir.join("iso4217.txt")) {
            for c in &codes {
                rules.add_currency_code(c);
            }
        }

        // Відомі файлові розширення — позитивний сигнал «це латиниця» (`txt`/`md`…);
        // гейт «укр. читання — реальне слово» робить core. Нема файлу → вбудований
        // перелік (loader Bruno).
        if let Ok(exts) = typofix_data::load_extensions(&dict_dir.join("extensions.txt")) {
            for e in &exts {
                rules.add_extension(e);
            }
        }
    }

    // Особисті слова з налаштувань (UI) — ПОВЕРХ файлових джерел, не замість них:
    // `always_switch` додаються до recognized (як user.txt), `never_switch` —
    // per-word veto. Регістр уже нормалізовано в `AppSettings::sanitized`, але
    // `WordRules` усе одно матчить регістронезалежно.
    for w in &words.always_switch {
        rules.recognize_word(w);
        // Цільово-кейоване ПРИМУСОВЕ перемикання: на відміну від recognize_word
        // (лише dict-бонус, який гейти довжини ріжуть для 1–2-літерних), force_switch
        // форсує перемикання НЕЗАЛЕЖНО від довжини/порогу — явні слова зі списку
        // «always_switch» перемикаються навіть короткими. Кейовано на ЦІЛЬ (саме
        // слово), тож працює при набраному в чужій розкладці. Деталі —
        // crates/typofix-core/CLAUDE.md (секція user_forced).
        rules.force_switch_word(w);
    }
    for w in &words.never_switch {
        rules.veto_word(w);
    }

    rules
}

/// Каталог даних для override-моделей: змінна `TYPOFIX_DATA_DIR`, якщо вказує на
/// наявну теку. Інакше `None` → вбудовані зразки. (Зручно для демо/розробки;
/// у проді шлях резолвить GUI — див. [`find_data_dir`].)
pub fn resolved_data_dir() -> Option<PathBuf> {
    let raw = std::env::var_os("TYPOFIX_DATA_DIR")?;
    let dir = PathBuf::from(raw);
    dir.is_dir().then_some(dir)
}

/// Чи виглядає `dir` як корінь каталогу даних? Критерій — є піддиректорія
/// `layouts/` (її завжди очікує [`load_language_profiles`]). Захищає від випадку,
/// коли поряд з `.exe` лежить чужа тека `data` без наших моделей.
pub fn data_dir_is_valid(dir: &Path) -> bool {
    dir.join("layouts").is_dir()
}

/// Знайти корінь каталогу даних серед кандидатів (порядок = пріоритет).
/// Повертає перший, що проходить [`data_dir_is_valid`]; інакше `None`
/// (→ вбудовані зразки). Чисте: жодного IO крім перевірки існування.
pub fn find_data_dir<I>(candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    candidates.into_iter().find(|d| data_dir_is_valid(d))
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

/// Навчені слова, готові для показу в UI: дедуплікація (регістронезалежно, лишаємо
/// перший варіант) + сортування (регістронезалежно). Файлу немає → порожньо.
pub fn learned_for_display(path: &Path) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for w in load_learned(path) {
        let key = w.to_lowercase();
        if !seen.iter().any(|s| s.to_lowercase() == key) {
            seen.push(w);
        }
    }
    seen.sort_by_key(|w| w.to_lowercase());
    seen
}

/// Перезаписати файл навчених слів АТОМАРНО (tmp → rename), по слову на рядок
/// (як `config::save_to_disk`). Порожній список → файл стає порожнім (шлях лишається
/// валідним для подальшого `append_learned`). Створює каталог за потреби.
pub fn write_learned(path: &Path, words: &[String]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = String::new();
    for w in words {
        let w = w.trim();
        if !w.is_empty() {
            body.push_str(w);
            body.push('\n');
        }
    }
    let tmp = path.with_extension("txt.tmp");
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path)
}

/// Прибрати одне слово з файлу навчених (регістронезалежно), зберігши решту.
/// Повертає `true`, якщо слово було й файл перезаписано без нього; `false` — якщо
/// слова не було (файл не чіпаємо). Атомарний перезапис через [`write_learned`].
pub fn remove_learned(path: &Path, word: &str) -> std::io::Result<bool> {
    let target = word.trim().to_lowercase();
    if target.is_empty() {
        return Ok(false);
    }
    let current = load_learned(path);
    let kept: Vec<String> = current
        .iter()
        .filter(|w| w.trim().to_lowercase() != target)
        .cloned()
        .collect();
    if kept.len() == current.len() {
        return Ok(false); // нічого не видалили
    }
    write_learned(path, &kept)?;
    Ok(true)
}

// ===========================================================================
// Менеджер рантайму: старт/стоп потоку рушія
// ===========================================================================

/// Чи означає набір дій РЕАЛЬНИЙ авто-перенабір (для звукового сигналу B2)?
/// Справжнє виправлення = крок ядра видав І `SwitchLayout`, І `TypeUnicode` (саме
/// перенабір), а не пропуск клавіші (`None`) чи самонавчання (`CommitException`).
/// Чисте → тестовно без хука.
pub fn is_real_switch(actions: &[typofix_core::Action]) -> bool {
    use typofix_core::Action;
    let has_switch = actions.iter().any(|a| matches!(a, Action::SwitchLayout(_)));
    let has_type = actions.iter().any(|a| matches!(a, Action::TypeUnicode(_)));
    has_switch && has_type
}

/// Команда від хоткей-хендлера (потік Tauri) до потоку рушія (B1).
///
/// **Чому канал, а не прямий виклик:** рушій крутиться в ОКРЕМОМУ потоці
/// ([`engine_loop`]) і ВОЛОДІЄ `EngineState` + платформою (хуки/ввід). Хоткей-
/// хендлер у потоці Tauri не має до них доступу й не сміє їх шарити. Тож він
/// лише шле команду каналом, а виконує її рушій на СВОЇХ state+платформі —
/// поряд із input-подіями, тим самим серіалізуючи доступ до стану.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineCommand {
    /// Скасувати останнє авто-перемикання (повернути оригінал + завчити слово).
    RevertLast,
    /// Примусово перемкнути розкладку останнього слова (ігнорує поріг).
    ManualSwitch,
    /// Застосувати регістр до поточного виділення ОС.
    ApplyCase(CaseMode),
}

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
        app: &AppHandle,
        settings: &AppSettings,
        learned_path: PathBuf,
        data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        self.stop_engine();
        if settings.enabled {
            self.start_engine(app, settings, learned_path, data_dir)?;
        }
        Ok(())
    }

    /// Зупинити рушій (при виході із застосунку).
    pub fn shutdown(&mut self) {
        self.stop_engine();
    }

    /// Надіслати команду активному рушієві. Повертає `false`, якщо рушій НЕ
    /// запущено (пауза/`enabled=false`) — тоді хоткей-дія просто ігнорується
    /// (revert/manual/case не мають сенсу без активного движка). Пауза-toggle
    /// іде окремим шляхом ([`crate::toggle_enabled`]), не через цей канал.
    pub fn send_command(&self, cmd: EngineCommand) -> bool {
        #[cfg(windows)]
        {
            if let Some(handle) = &self.engine {
                return handle.tx.send(cmd).is_ok();
            }
            false
        }
        #[cfg(not(windows))]
        {
            let _ = cmd;
            false
        }
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
        app: &AppHandle,
        settings: &AppSettings,
        learned_path: PathBuf,
        data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        let exclusions = exclusion_rules_from(settings);
        let config = detector_config_from(settings);
        let languages = load_language_profiles(settings.language, data_dir.as_deref())?;
        // Правила рівня слова: whitelist коротких службових слів + особистий
        // словник (user.txt ∪ always_switch) + veto (never_switch) + forex-коди.
        let rules = load_word_rules(settings.language, data_dir.as_deref(), &settings.words);
        let seed = load_learned(&learned_path);
        // Прапорець звуку (B2) і клон AppHandle (для емісії зміни розкладки в трей).
        let sound_on_switch = settings.feedback.sound_on_switch;
        let app_for_thread = app.clone();

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        // Канал команд від хоткеїв: хендлер шле, рушій поллить у своєму циклі.
        let (tx, cmd_rx) = std::sync::mpsc::channel::<EngineCommand>();
        let thread = std::thread::Builder::new()
            .name("typofix-engine".to_string())
            .spawn(move || {
                engine_loop(
                    app_for_thread,
                    stop_for_thread,
                    cmd_rx,
                    exclusions,
                    config,
                    languages,
                    rules,
                    seed,
                    learned_path,
                    sound_on_switch,
                );
            })
            .map_err(|e| format!("не вдалося запустити потік рушія: {e}"))?;

        self.engine = Some(EngineHandle { stop, tx, thread });
        Ok(())
    }

    #[cfg(not(windows))]
    fn start_engine(
        &mut self,
        _app: &AppHandle,
        _settings: &AppSettings,
        _learned_path: PathBuf,
        _data_dir: Option<PathBuf>,
    ) -> Result<(), String> {
        // Жива платформа лише на Windows; на інших цілях рушій — no-op (порт згодом).
        Ok(())
    }
}

/// Хендл живого потоку рушія: прапорець зупинки + канал команд + сам потік.
#[cfg(windows)]
struct EngineHandle {
    stop: Arc<AtomicBool>,
    tx: std::sync::mpsc::Sender<EngineCommand>,
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

/// Тіло потоку рушія: тягне події з платформи й команди з хоткей-каналу,
/// проганяє через ядро, застосовує дії на власних `state`+платформі.
// Приватна точка входу потоку: усі аргументи — owned-дані, передані один раз при
// старті (group у struct лише задля ліку аргументів не вартий зайвого типу).
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn engine_loop(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    cmd_rx: std::sync::mpsc::Receiver<EngineCommand>,
    exclusions: ExclusionRules,
    config: DetectorConfig,
    languages: Vec<LanguageProfile>,
    rules: WordRules,
    seed: Vec<String>,
    learned_path: PathBuf,
    sound_on_switch: bool,
) {
    use std::sync::mpsc::TryRecvError;
    use std::time::Duration;

    use typofix_core::{
        force_switch_last, revert_last, step, transform_case, Action, Context, EngineState,
    };
    use typofix_platform::Platform;
    use typofix_platform_windows::{get_selection_text, WindowsPlatform};

    // ⚠️ Ставить системні хуки на весь час життя потоку.
    let mut platform = WindowsPlatform::new();

    let mut state = EngineState::default();
    for word in &seed {
        state.learned.learn(word);
    }
    // `rules` несе whitelist коротких службових слів, особистий словник
    // (recognized) і per-word veto з налаштувань.

    // Остання повідомлена в трей розкладка (debounce: емітимо лише на зміну).
    // Перший прохід зі стартовим значенням `None` гарантовано емітне поточну.
    let mut last_lang: Option<String> = None;
    // Повідомити трей про поточну розкладку, якщо змінилась (на головному потоці —
    // tray-операції Win32 мають іти звідти). `run_on_main_thread` не блокує цикл.
    let notify_layout = |app: &AppHandle, lang: &str, last: &mut Option<String>| {
        if last.as_deref() == Some(lang) {
            return;
        }
        *last = Some(lang.to_string());
        let app2 = app.clone();
        let lang2 = lang.to_string();
        let _ = app.run_on_main_thread(move || crate::on_engine_layout(&app2, &lang2));
    };
    // Емітимо стартову розкладку одразу (трей покаже UK/EN, а не просто «активний»).
    notify_layout(&app, platform.current_layout().as_str(), &mut last_lang);

    // Застосувати набір дій + персист самонавчання (спільне для step і команд).
    // Платформа приходить параметром (не захоплюється) → без конфлікту борроу
    // з основним циклом, що теж позичає `platform`.
    let apply_actions = |platform: &mut WindowsPlatform, actions: &[Action]| {
        for action in actions {
            // Самонавчання персистимо тут (ядро лишається чистим).
            if let Action::CommitException(word) = action {
                if let Err(e) = append_learned(&learned_path, word) {
                    eprintln!("TypoFix: не вдалося зберегти навчене слово: {e}");
                }
            }
            platform.apply(action);
        }
    };

    while !stop.load(Ordering::SeqCst) {
        // 1) Команди від хоткеїв мають пріоритет і обробляються неблокуюче.
        match cmd_rx.try_recv() {
            Ok(EngineCommand::RevertLast) => {
                let actions = revert_last(&mut state);
                apply_actions(&mut platform, &actions);
                continue;
            }
            Ok(EngineCommand::ManualSwitch) => {
                let ctx = Context {
                    active_window: platform.active_window(),
                    current_layout: platform.current_layout(),
                    languages: &languages,
                    config,
                    exclusions: &exclusions,
                    rules: &rules,
                    secure: platform.is_secure_field(),
                };
                let actions = force_switch_last(&mut state, &ctx);
                apply_actions(&mut platform, &actions);
                continue;
            }
            Ok(EngineCommand::ApplyCase(mode)) => {
                // Виділення приходить ЗЗОВНІ (синтет. Ctrl+C), не з буфера натисків.
                if let Some(text) = get_selection_text() {
                    let out = transform_case(&text, mode);
                    if out != text {
                        // Друк поверх виділення замінює його (поле затирає виділене
                        // вводом) — `DeleteChars` не потрібен. Якщо в якомусь полі
                        // виділення не затирається вводом — ловитимемо живим прогоном.
                        platform.apply(&Action::TypeUnicode(out));
                    }
                }
                continue;
            }
            Err(TryRecvError::Empty) => {}
            // Усі sender'и впали (EngineHandle дропнуто без stop) — виходимо.
            Err(TryRecvError::Disconnected) => break,
        }

        // 2) Вхідні події.
        let Some(event) = platform.try_next_event() else {
            // Канал порожній — коротка пауза, щоб не крутити CPU вхолосту.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };

        // Поточна розкладка (до можливого нашого перемикання) — для трей-індикатора.
        let cur_layout = platform.current_layout();
        let cur_lang = cur_layout.as_str().to_string();

        let ctx = Context {
            active_window: platform.active_window(),
            current_layout: cur_layout,
            languages: &languages,
            config,
            exclusions: &exclusions,
            rules: &rules,
            // Приватність №4: пароль-поле → ядро не буферить/не перемикає.
            secure: platform.is_secure_field(),
        };

        let actions = step(&mut state, event, &ctx);
        // Звук (B2): лише на НАШ справжній авто-перенабір і лише коли увімкнено.
        if sound_on_switch && is_real_switch(&actions) {
            crate::feedback::play_switch_sound();
        }
        apply_actions(&mut platform, &actions);

        // Трей-індикатор поточної розкладки (емітимо лише на зміну).
        notify_layout(&app, &cur_lang, &mut last_lang);
    }
    // Вихід із циклу → drop(platform) знімає хуки.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BehaviorDto, DetectionDto, ExclusionsDto, WordsDto};
    use typofix_core::WindowInfo;

    fn settings_with(exclusions: ExclusionsDto, detection: DetectionDto) -> AppSettings {
        AppSettings {
            exclusions,
            detection,
            ..Default::default()
        }
    }

    #[test]
    fn behavior_flags_map_to_detector_config() {
        // Усе вимкнено → відповідні прапорці DetectorConfig false.
        let settings = AppSettings {
            behavior: BehaviorDto {
                fix_case: false,
                forex: false,
                recognize_extensions: false,
                phonotactics: false,
                fix_capslock: false,
            },
            ..Default::default()
        };
        let cfg = detector_config_from(&settings);
        assert!(!cfg.case_fix_enabled);
        assert!(!cfg.forex_enabled);
        assert!(!cfg.extensions_enabled);
        assert!(!cfg.phonotactics_enabled);
        assert!(!cfg.capslock_fix_enabled);

        // Дефолти (усе true) → всі прапорці увімкнені (поточна поведінка).
        let on = detector_config_from(&AppSettings::default());
        assert!(
            on.case_fix_enabled
                && on.forex_enabled
                && on.extensions_enabled
                && on.phonotactics_enabled
                && on.capslock_fix_enabled
        );
    }

    #[test]
    fn is_real_switch_needs_switch_plus_retype() {
        use typofix_core::Action;
        // Справжній авто-перенабір: delete + switch + type.
        let real = [
            Action::DeleteChars(5),
            Action::SwitchLayout(LayoutId::new("en")),
            Action::TypeUnicode("hello".into()),
        ];
        assert!(is_real_switch(&real));
        // Лише текст / лише switch / самонавчання / порожньо — НЕ наш перенабір.
        assert!(!is_real_switch(&[Action::TypeUnicode("x".into())]));
        assert!(!is_real_switch(&[Action::SwitchLayout(LayoutId::new(
            "uk"
        ))]));
        assert!(!is_real_switch(&[Action::CommitException("слово".into())]));
        assert!(!is_real_switch(&[Action::None]));
        assert!(!is_real_switch(&[]));
    }

    #[test]
    fn send_command_without_engine_is_ignored() {
        // Без активного рушія (пауза/вимкнено) команди тихо ігноруються —
        // `send_command` повертає false, а не панікує/блокує. Крос-платформно
        // (на не-Windows завжди false; на Windows engine=None → false).
        let mgr = RuntimeManager::default();
        assert!(!mgr.send_command(EngineCommand::RevertLast));
        assert!(!mgr.send_command(EngineCommand::ManualSwitch));
        assert!(!mgr.send_command(EngineCommand::ApplyCase(CaseMode::Sentence)));
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
    fn real_models_load_via_data_dir_when_present() {
        // Пропускаємо, якщо реальних моделей немає (CI/інша машина) — тест
        // безпечний скрізь, але доказовий там, де є `data/`.
        let Some(raw) = std::env::var_os("TYPOFIX_DATA_DIR") else {
            return;
        };
        let dir = PathBuf::from(raw);
        if !dir.is_dir() {
            return;
        }
        let profiles = load_language_profiles(LanguagePair::UkEn, Some(&dir)).unwrap();
        assert_eq!(profiles.len(), 2);
        let uk = profiles.iter().find(|p| p.id.as_str() == "uk").unwrap();
        // Справжня uk-LM має оцінювати валідне слово вище за крякозябри.
        assert!(uk.lm.score("привіт") > uk.lm.score("ghbdsn"));
    }

    #[test]
    fn word_rules_empty_without_data_dir_or_settings_words() {
        // Fallback-режим (вбудовані зразки) + порожні words → порожні правила.
        let rules = load_word_rules(LanguagePair::UkEn, None, &WordsDto::default());
        assert!(rules.is_empty());
    }

    #[test]
    fn settings_words_map_to_recognized_and_veto_without_data_dir() {
        // Слова з налаштувань застосовуються навіть без data/ (не залежать від файлів).
        let words = WordsDto {
            always_switch: vec!["вжух".into(), "eurusd".into()],
            never_switch: vec!["vec".into()],
        };
        let rules = load_word_rules(LanguagePair::UkEn, None, &words);
        assert!(!rules.is_empty());
        // always_switch → recognized (позитив, регістронезалежно).
        assert!(rules.recognizes("Вжух"));
        assert!(rules.recognizes("EURUSD"));
        // never_switch → veto (збіг із поточним АБО виправленим текстом).
        assert!(rules.vetoes("vec", "будь-що"));
        assert!(!rules.recognizes("vec"));
    }

    #[test]
    fn word_rules_load_short_service_from_data_dir_when_present() {
        // Доказово лише там, де є реальний `data/` (CI/інша машина — пропуск).
        let Some(raw) = std::env::var_os("TYPOFIX_DATA_DIR") else {
            return;
        };
        let dir = PathBuf::from(raw);
        if !dir.join("dicts").join("uk.short.txt").is_file() {
            return;
        }
        let rules = load_word_rules(LanguagePair::UkEn, Some(&dir), &WordsDto::default());
        assert!(!rules.is_empty());
        // Куроване службове слово розпізнається per-мова; шум зі словника — ні.
        assert!(rules.is_short_service(&LayoutId::new("uk"), "от"));
        assert!(rules.is_short_service(&LayoutId::new("en"), "we"));
        assert!(!rules.is_short_service(&LayoutId::new("uk"), "ат"));
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

    #[test]
    fn learned_for_display_dedups_and_sorts_case_insensitive() {
        let path = std::env::temp_dir().join(format!("typofix-disp-{}.txt", std::process::id()));
        let _ = fs::remove_file(&path);

        append_learned(&path, "Сонце").unwrap();
        append_learned(&path, "ліс").unwrap();
        append_learned(&path, "сонце").unwrap(); // дубль за регістром
        append_learned(&path, "Авто").unwrap();

        // Дедуп лишає ПЕРШИЙ варіант ("Сонце"), сортує регістронезалежно.
        let words = learned_for_display(&path);
        assert_eq!(words, vec!["Авто", "ліс", "Сонце"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn remove_learned_drops_word_case_insensitive_and_keeps_rest() {
        let path = std::env::temp_dir().join(format!("typofix-rm-{}.txt", std::process::id()));
        let _ = fs::remove_file(&path);

        append_learned(&path, "привіт").unwrap();
        append_learned(&path, "світ").unwrap();

        // Видалення регістронезалежне; повертає true.
        assert!(remove_learned(&path, "ПРИВІТ").unwrap());
        assert_eq!(load_learned(&path), vec!["світ"]);

        // Слова немає → false, файл не змінено.
        assert!(!remove_learned(&path, "немає").unwrap());
        assert_eq!(load_learned(&path), vec!["світ"]);

        // Порожнє слово → no-op false.
        assert!(!remove_learned(&path, "   ").unwrap());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn write_learned_empty_clears_file() {
        let path = std::env::temp_dir().join(format!("typofix-clr-{}.txt", std::process::id()));
        let _ = fs::remove_file(&path);

        append_learned(&path, "слово").unwrap();
        write_learned(&path, &[]).unwrap();
        assert!(load_learned(&path).is_empty());

        let _ = fs::remove_file(&path);
    }
}
