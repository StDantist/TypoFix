# src-tauri — Tauri-оболонка TypoFix

GUI-скелет: трей-іконка з меню + приховуване вікно налаштувань (фронтенд — `../ui`,
Svelte+Vite). Реальної логіки розпізнавання тут НЕМАЄ — лише місця `TODO`.

## ГОТЧА №1 — відокремлений workspace (НЕ ламати!)
`Cargo.toml` починається з **порожньої** секції `[workspace]`. Це навмисно: вона
відв'язує `typofix-app` від кореневого бібліотечного workspace.

**Чому:** core-тести запускають `cargo test --workspace` з кореня репо. Якби цей
крейт був членом кореневого workspace, ті тести тягли б увесь важкий GUI-тулчейн
(Tauri, WebView2, `tauri-build`, ще й вимагали б зібраний `ui/dist`) — повільно й
крихко. Відокремлення тримає core-цикл швидким і зеленим.

**Перевірка інваріанту:** з кореня `cargo metadata --no-deps` НЕ має показувати
`typofix-app`; `cargo test --workspace` з кореня має лишатися швидким і не чіпати
`src-tauri`. У `src-tauri` свій окремий `Cargo.lock` і `target/`.

## ГОТЧА №2 — `tauri` запускати з КОРЕНЯ репо, не з `ui/`
Tauri CLI шукає `./src-tauri/tauri.conf.json` у поточній теці й **не йде вгору** по
дереву. CLI встановлено локально в `ui/node_modules`, але запускати його треба з
кореня (де лежить `src-tauri/`). `npm run tauri …` з теки `ui/` НЕ знайде проєкт.

`beforeDevCommand`/`beforeBuildCommand` тому використовують `npm --prefix ui run …`,
щоб vite стартував незалежно від cwd (а cwd при запуску — корінь репо).

## Як запускати (dev) — з КОРЕНЯ репо `d:\Projects\TypoFix`
```powershell
# 1) (одноразово) залежності фронтенду
npm --prefix ui install

# 2) dev-режим: підніме vite (порт 1420) + збере й запустить застосунок
.\ui\node_modules\.bin\tauri dev
```
Перший запуск довгий (компіляція Tauri-крейтів). Потрібен WebView2 Runtime
(на Windows 11 є з коробки).

## Як зібрати реліз
```powershell
.\ui\node_modules\.bin\tauri build    # з кореня репо
```
Інсталятори/бандли в межах фази 0 робити не обов'язково.

## Іконки
`icons/` згенеровано з `app-icon.png` командою `npx tauri icon ../src-tauri/app-icon.png
-o ../src-tauri/icons` (запуск із `ui/`). Щоб змінити іконку — онови `app-icon.png`
і перегенеруй. Іконку трею беремо в коді з вшитої `default_window_icon()`.

## Поведінка скелета
- Трей-меню: статус (активний/пауза, disabled-індикатор), Пауза/Відновити (toggle),
  Відкрити налаштування, Автозапуск (TODO-заглушка), Вихід.
- Лівий клік по іконці трею → відкрити налаштування.
- Вікно налаштувань стартує прихованим (`visible: false` у conf); закриття вікна
  його **ховає** (`api.prevent_close()` + `hide()`), а не закриває застосунок.
  Застосунок живе у треї; вийти — лише через пункт «Вихід».

## Конфіг (`config.rs` + вікно налаштувань)
- **Файл:** `settings.json` у Tauri **app config dir** (`app.path().app_config_dir()`).
  На Windows це `%APPDATA%\dev.typofix.app\settings.json` (identifier із conf).
- **Формат:** pretty-JSON DTO `AppSettings` (version, enabled, language, exclusions
  {process_names/exe_paths/folders}, detection). Усі поля `#[serde(default)]` →
  старі/часткові файли читаються без падіння (forward/back-compat).
- **ПРИВАТНІСТЬ (залізне правило):** у конфіг ідуть ЛИШЕ налаштування. НІКОЛИ
  натиски/буфер/набраний текст — їх тут немає й не повинно бути.
- **Власний DTO, НЕ типи `typofix-core`:** app-крейт у відокремленому workspace не
  залежить від core. DTO лише дзеркалить форму `ExclusionRules`. Маппінг DTO→core
  (+ нормалізація шляхів) робить core при матчингу — буде у Фазі 5 (жива проводка).
- **Запис атомарний:** tmp-файл → `rename` поверх цілі (без напівзаписаних конфігів).
- **Пошкоджений файл = помилка, не мовчазний дефолт** (UI показує). Відсутній файл
  (перший запуск) = дефолти.
- **Команди:** `load_settings` (диск → in-memory + повертає форму),
  `save_settings(settings)` (валідує `sanitized()`, пише, оновлює трей, повертає
  очищене). Диск — джерело істини; `AppState.settings` — синхронізована копія.
- **Синхрон трей↔вікно:** toggle у треї змінює `enabled`, пише на диск і емітить
  подію `settings:changed` (повний конфіг). Вікно слухає й оновлює ЛИШЕ перемикач
  `enabled`, не чіпаючи можливих незбережених правок у формі.

## Рантайм-цикл рушія (`runtime.rs`) — серце Фази 5
Зв'язує живу платформу (Windows-хук) із чистим ядром.
- **`RuntimeManager`** (Tauri-стан за `Mutex`) керує життям потоку `typofix-engine`.
  `apply(settings, learned_path, data_dir)`: увімкнено → (пере)старт потоку; пауза/
  вимкнено → стоп. Викликається в `setup`, у трей-toggle і в `save_settings`.
- **Потік рушія** створює `WindowsPlatform::new()` (ставить системні хуки!), у циклі
  тягне `try_next_event()` → `typofix_core::step(state, ev, ctx)` → `platform.apply(action)`.
  Порожній канал → `sleep(2ms)`. **Пауза/вихід = стоп потоку**, `Drop` платформи
  знімає хуки (на паузі ввід НЕ перехоплюється взагалі).
- **`Context` будується щокроку** з: `active_window`/`current_layout` від платформи +
  `languages`/`config`/`exclusions` (owned у потоці, борроваться) + порожні `WordRules`.
- **Готча — Windows-only:** `typofix-platform-windows` під `[target.'cfg(windows)'.dependencies]`;
  весь код, що торкається `WindowsPlatform` (потік, `EngineHandle`, `engine_loop`),
  під `#[cfg(windows)]`. На не-Windows `start_engine` — no-op (макос-порт згодом).
  Маппінг/завантаження/персистенція — кросплатформні (компілюються й тестуються всюди).
- **`tauri dev` НЕ запускати** (ставить реальні хуки + інтерактив). Живий прогін —
  контрольовано, як зі спайком. Досить компіляції + тестів мапінгу.

## Маппінг конфіг → ядро (`runtime.rs`, чисте, тестоване)
- `exclusion_rules_from` → `core::ExclusionRules` (process/exe/folder; нормалізацію
  шляхів робить core).
- `detector_config_from`: `min_word_len`→`min_switch_len` (прямий). `confidence_threshold`
  (0..1) масштабує `base_threshold` монотонно навколо 0.75=дефолт — **тимчасова**
  евристика (внутрішній поріг — лог-ймовірнісний, не 0..1); справжня калібровка у фазі eval.
- `load_language_profiles(pair, data_dir)`: uk+en через `typofix-data`. `data_dir` —
  **корінь** `data/`; функція сама додає піддиректорії: `load_layout`←`data/layouts`,
  `load_lm`←`data/lm`, `load_dict`←`data/dicts` (типове `{lang}.{toml,bin,fst}`).
  Відсутній файл → fallback на вбудований зразок.
- **Реальні моделі через env `TYPOFIX_DATA_DIR`** (`resolved_data_dir()`): якщо вказує
  на наявну теку — вантажимо справжні `.bin`/`.fst`; інакше зразки (слабші, ~46%).
  `sync_runtime` передає це у рушій. У проді шлях даватиме інсталяція (ресурси) — TODO.

## Демо-бінар `src/bin/live_engine.rs` (жива перевірка без GUI)
Окремий бінар: реальні моделі (`TYPOFIX_DATA_DIR`) → `WindowsPlatform` (хуки) →
цикл рушія ~20 c (Esc — раніше) → лог `[ВИПРАВЛЕНО] 'було' → 'стало' (мова X→Y)`.
«Було» реконструюється back-translate (символи перенабору → страйки в цільовій
розкладці → інтерпретація у вихідній). ⚠️ Ставить системні хуки — **лише вручну**.
Запуск з кореня репо:
```powershell
$env:TYPOFIX_DATA_DIR = "d:\Projects\TypoFix\data"
cargo run -p typofix-app --bin live_engine
```
Не-Windows: бінар компілюється як заглушка (друкує попередження).

## Самонавчання — файл навчених винятків
- **Де:** `learned_exceptions.txt` у **тому ж** app config dir, що й `settings.json`
  (`%APPDATA%\dev.typofix.app\`). По одному слову на рядок.
- **Потік:** рушій емітить `Action::CommitException(word)` (юзер відкинув перенабір)
  → app дозаписує слово (`append_learned`). На старті `load_learned` засіває
  `EngineState.learned` через `learn()` (дедуп у пам'яті). Ядро саме НІЧОГО не
  персистить (лишається чистим). Приватність: лише самі слова, локально.
- `LearnedExceptions` не має геттера слів — тому персистимо НЕ читанням стану ядра,
  а перехопленням потоку `CommitException` на app-шарі.

## Дозволи (capabilities) — НЕ забути при додаванні команд/плагінів
`capabilities/default.json` (window `settings`) перелічує дозволи. Без потрібного
дозволу `invoke` падає в рантаймі (компіляція мовчить!). Зараз увімкнено: `core:default`,
events, window show/hide/focus, `dialog:allow-open` (file-picker для exe/теки —
плагін `tauri-plugin-dialog`). Власні app-команди (`load_settings`/`save_settings`)
працюють у межах `core:default`. Додаєш плагін → додай його permission сюди.
