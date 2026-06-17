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

**ГОТЧА (cwd before-команд):** `tauri` CLI запускає `beforeDevCommand`/
`beforeBuildCommand` з cwd = **батьківської теки `frontendDist`** (`../ui/dist` →
`ui/`), а НЕ з кореня репо. Тому команди — звичайні `npm run dev` / `npm run build`
(виконуються вже в `ui/`). НЕ `npm --prefix ui …` — це шукало б `ui/ui/package.json`
(ENOENT). Раніше це не спливало, бо `build.bat` робив сирий `cargo build` і
beforeBuildCommand узагалі не виконувався.

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
  {process_names/exe_paths/folders}, **words {always_switch/never_switch}**, detection).
  Усі поля `#[serde(default)]` → старі/часткові файли читаються без падіння
  (forward/back-compat: старий settings.json без `words` → порожні списки).
- **Секція `words` (винятки по СЛОВАХ, як Punto):** `always_switch` = позитивний
  особистий словник (слова, які апка ВИЗНАЄ й перемикає — жаргон/нікнейми/forex,
  напр. `лох`); `never_switch` = per-word veto (слова, які лишати недоторканими).
  На відміну від `exclusions` (де регістр шляхів зберігається) `sanitized()`
  нормалізує слова в **lowercase** (+trim+dedup), бо матчинг у ядрі регістронезалежний.
  UI керує тими ж даними, що й `user.txt` (звір семантику: `always_switch` =
  позитив, як `user.txt`; обидва йдуть у `WordRules.recognized`).
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
- **`list_running_processes() -> Vec<ProcessEntry>`** (`lib.rs`): перелік ЗАРАЗ
  запущених процесів для пікера виключень. `ProcessEntry { name, exe_path: Option,
  icon: Option, has_window: bool }`. **Дедуп за exe-іменем** (lowercase-ключ; один запис на застосунок,
  не на PID), сортовано за іменем. Через `sysinfo` (default-features off, лише
  `system`). `name` = file_name з exe-шляху (повне `chrome.exe`), fallback
  `process.name()`. Приватність: лише імена/шляхи/іконки, локально, нічого не пишемо/
  не шлемо. Власна app-команда → працює в межах `core:default`, **новий permission НЕ
  потрібен** (як `load_settings`).
  - **ГОТЧА `with_exe`:** `ProcessRefreshKind::nothing()` НЕ заповнює exe-шлях →
    `exe_path`/`name`/іконка були б порожні. Обов'язково
    `ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)`.
  - **`icon`** = base64 PNG data-URL (`data:image/png;base64,…`) іконки exe. Витяг —
    **Win32 напряму** (`mod win_icon`, уся unsafe-FFI ізольована): `SHGetFileInfoW`
    → `HICON` → `GetIconInfo`/`GetDIBits` (32bpp top-down BGRA) → RGBA (альфа з
    колірного bitmap; якщо вся 0 — відновлюємо з AND-маски) → PNG (`image` crate) →
    base64. **Чому не `systemicons`:** він безумовно тягне `gtk-sys 0.14` (links
    "gtk-3") → конфлікт у дереві. `windows-sys` 0.59 — та сама версія, що в
    `typofix-platform-windows` (без дубля). `SHGetFileInfoW` гейтований на features
    `Win32_Storage_FileSystem` + `Win32_UI_WindowsAndMessaging` (+ Shell, Gdi,
    Foundation). На не-Windows — заглушка `icon: None` (macOS-витяг згодом).
  - **Кеш іконок** `ICON_CACHE` (`OnceLock<Mutex<HashMap<path, Option<data-url>>>>`,
    процес-глобальний, з негативним кешем): холодний витяг ~700мс на ~110 застосунків
    (~6мс/exe), теплий (кеш) ~34мс → «Оновити список» миттєвий; перше відкриття під
    спінером. Якщо колись стане критично — винести у лінивий `process_icon(path)`.
  - **`has_window`** — чи має застосунок видиме верхньорівневе вікно. `window_pids()`
    (cfg windows): Win32 `EnumWindows` → `IsWindowVisible` + `GetWindowTextLengthW > 0`
    + не `WS_EX_TOOLWINDOW` (`GWL_EXSTYLE`) → `GetWindowThreadProcessId` → множина PID.
    Оскільки дедуп за exe-іменем: `has_window = true`, якщо **БУДЬ-ЯКИЙ** PID цього exe
    у множині (`e.has_window |= …`). `proc.pid().as_u32()` для звірки з множиною. На
    не-Windows — порожня множина (фільтр нічого не ховає). Features вже є
    (`Win32_UI_WindowsAndMessaging`). UI вмикає фільтр «лише з вікнами» за замовчуванням.
  - **CSP (готча!):** іконки — data-URL, тож `tauri.conf.json` `security.csp` має
    мати **`img-src 'self' data:`** (без нього default-src 'self' блокує data:-зображення).
    Зовнішні джерела НЕ додаємо (приватність).
  - Тестовно без GUI/хуків: юніти `list_running_processes_returns_deduped_sorted_nonempty`
    (+ валідність data-URL), `icons_are_extracted_for_most_processes_and_are_fast`
    (покриття ≥50% процесів зі шляхом + друк часу холодний/теплий) і
    `window_pids_consistent_with_has_window_flag` (вікна Є → ≥1 запис has_window;
    headless 0 вікон — не стверджуємо; друкує лічильники).
- **UI-пікер процесів** (`ui/src/lib/ProcessPicker.svelte`): модалка з полем-фільтром
  (пошук за іменем/шляхом), кнопкою «Оновити список», закриттям по Esc / кліку поза
  вмістом. Клік по запису → `onpick(name)` → `addUnique(process_names)` (можна додати
  кілька й закрити; уже додані позначені й disabled). Кнопка «Обрати із запущених…»
  у картці «Виключення». IPC-обгортка — `api.js::listRunningProcesses`. Кожен рядок
  показує `<img>`-іконку (data-URL) перед іменем; нема іконки → нейтральна заглушка
  `.picon-ph` (тримає вирівнювання) + `exe_path` дрібним текстом. Чекбокс «Лише
  застосунки з вікнами» (default ON) фільтрує за `has_window`; пошук працює поверх.
  Коли пошук нічого не дав серед віконних, але є приховані збіги — натяк із кнопкою
  «Показати всі процеси» (знімає чекбокс).
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
  `languages`/`config`/`exclusions`/`rules` (owned у потоці, борроваться). `rules`
  тепер несе whitelist коротких службових слів (див. нижче), не порожній.
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
- `load_word_rules(pair, data_dir, words)`: будує `core::WordRules`, **об'єднуючи**
  файлові джерела з `data/` зі словами-винятками з налаштувань (`&AppSettings.words`):
  - whitelist коротких СЛУЖБОВИХ слів (`data/dicts/{lang}.short.txt` через
    `typofix_data::load_short_words`, далі `WordRules::allow_short_service` per-`LayoutId`);
  - **recognized** (позитив) = `user.txt` ∪ `words.always_switch` → `recognize_word`;
  - **veto** (ніколи) = `words.never_switch` → `veto_word`;
  - forex-коди ISO 4217 (`data/dicts/iso4217.txt`).
  Слова з `words` застосовуються ПОВЕРХ файлових і **не залежать від `data/`** (працюють
  навіть у fallback-режимі без моделей). Прокид: `sync_runtime`→`apply`→`start_engine`
  (бере `&settings.words`)→`engine_loop`. Вмикає **дзеркальну
  релаксацію порога** коротких слів у детекторі (`от`/`ти`/`we`...). `data_dir` —
  корінь `data/` (функція додає `dicts/`). Готча: whitelist — НЕ повний словник
  (`ат`/`ді` Є у `uk.fst` як шум, але НЕ у whitelist → код-токени `fn`/`ls` не
  перемикаються); деталі — `crates/typofix-core/CLAUDE.md`. Немає data-dir/файлів
  або помилка читання → порожні rules (фіча вимкнена, **м'яка деградація**). Прокид:
  `start_engine` → `engine_loop(rules)` → `Context.rules` (раніше передавали
  `WordRules::new()` — короткі слова у проді не працювали).
- `load_language_profiles(pair, data_dir)`: uk+en через `typofix-data`. `data_dir` —
  **корінь** `data/`; функція сама додає піддиректорії: `load_layout`←`data/layouts`,
  `load_lm`←`data/lm`, `load_dict`←`data/dicts` (типове `{lang}.{toml,bin,fst}`).
  Відсутній файл → fallback на вбудований зразок.
- **Резолв data-dir (standalone, БЕЗ env)** — `lib.rs::resolve_data_dir(app)`. Щоб
  застосунок працював подвійним кліком, шукаємо корінь `data/` за пріоритетом:
  1. `TYPOFIX_DATA_DIR` (`resolved_data_dir()`) — явний override (dev/демо/`live_engine`);
  2. `resource_dir()/data` — ресурси бандла (`cargo tauri build`; `bundle.resources`
     у `tauri.conf.json` мапить `../data/{layouts,lm,dicts}`→`data/...`);
  3. `data` поряд з `.exe` і **вгору по предках** шляху — портативний zip і dev
     release-білд (`cargo build --release`: exe у `src-tauri/target/release/`, предок-
     репо містить `data/` → подвійний клік працює БЕЗ копіювання).
  Кандидата немає → вбудовані зразки (слабші, ~46%). Готча: `cargo build --release`
  НЕ копіює `bundle.resources` (це робить лише `tauri build`) — тому й потрібен
  ancestor-walk. Валідність кандидата = наявність піддиректорії `layouts/`
  (`runtime::data_dir_is_valid`), щоб не схопити чужу теку `data`.
- `find_data_dir`/`data_dir_is_valid` у `runtime.rs` — чисті (лише перевірка
  існування), тестовні; складання списку кандидатів (env/resource/exe) — у `lib.rs`,
  бо потребує `AppHandle`. `live_engine` лишається на env-only `resolved_data_dir()`.

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
