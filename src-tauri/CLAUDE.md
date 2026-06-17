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
  Відкрити налаштування, Автозапуск (CheckMenuItem з галочкою — стан із реєстру), Вихід.
- Лівий клік по іконці трею → відкрити налаштування.
- Вікно налаштувань стартує прихованим (`visible: false` у conf); закриття вікна
  його **ховає** (`api.prevent_close()` + `hide()`), а не закриває застосунок.
  Застосунок живе у треї; вийти — лише через пункт «Вихід».

## Конфіг (`config.rs` + вікно налаштувань)
- **Файл:** `settings.json` у Tauri **app config dir** (`app.path().app_config_dir()`).
  На Windows це `%APPDATA%\dev.typofix.app\settings.json` (identifier із conf).
- **Формат:** pretty-JSON DTO `AppSettings` (version, enabled, language, exclusions
  {process_names/exe_paths/folders}, **words {always_switch/never_switch}**, hotkeys,
  **behavior**, detection). Усі поля `#[serde(default)]` → старі/часткові файли
  читаються без падіння (forward/back-compat: старий settings.json без секції → дефолти).
  `SCHEMA_VERSION` росте з кожною новою секцією: v2 додав `hotkeys`, v3 — `behavior`.
- **Секція `behavior` (B4, перемикачі поведінки):** 5 bool-тогглів, **усі default `true`**
  (= поточна повна поведінка детектора, тож відсутня секція нічого не змінює).
  Дзеркалять `*_enabled`-прапорці `DetectorConfig` 1:1 — мапінг у `detector_config_from`
  (`runtime.rs`): `fix_case`→`case_fix_enabled`, `forex`→`forex_enabled`,
  `recognize_extensions`→`extensions_enabled`, `phonotactics`→`phonotactics_enabled`,
  `fix_capslock`→`capslock_fix_enabled`. UI — картка «Поведінка» (`Toggle` на кожен).
  **Чутливість (людський повзунок):** окремого поля НЕ має — це перепрочитання
  наявного `detection.confidence_threshold`. UI-слайдер 0 (Обережно) → 100 (Агресивно)
  мапиться лінійно у поріг `[1.0 .. 0.5]` (`THR_CAUTIOUS`/`THR_AGGRESSIVE` у `App.svelte`):
  **вищий поріг = обережніше** (менше спрацювань). Числовий `confidence_threshold` +
  `min_word_len` лишились у картці «Поріг впевненості (розширені)» — слайдер і числове
  поле правлять те саме поле (двостороннє через `$derived sensitivity`).
- **Секція `words` (винятки по СЛОВАХ, як Punto):** `always_switch` = позитивний
  особистий словник (слова, які апка ВИЗНАЄ й перемикає — жаргон/нікнейми/forex,
  напр. `вжух`); `never_switch` = per-word veto (слова, які лишати недоторканими).
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
- **`reset_settings() -> AppSettings`** (`lib.rs`): скидання «параметрів» до стандартних
  БЕЗ втрати даних користувача. **Єдине джерело істини дефолтів — Rust**
  (`AppSettings::default()`): будуємо свіжий default і ПЕРЕНОСИМО в нього з поточного
  `AppState.settings` лише `exclusions`, `words` і `enabled`; далі `sanitized()` →
  атомарний запис → оновлення стану/трею → `sync_runtime` (+`hotkeys::apply`, обидва
  no-op у e2e). Повертає нові settings (UI робить `applyLoaded` → форма = повернене,
  dirty скидається, статус «скинуто»).
  - **СКИДАЄ до дефолтів:** `behavior` (5 тогглів), `detection` (`min_word_len` +
    `confidence_threshold` ⇒ і людський повзунок чутливості), `feedback`
    (`sound_on_switch`), `hotkeys` (прив'язки → дефолтні = всі вимкнені),
    `language` (→ `uk-en`).
  - **ЗБЕРІГАЄ як є:** `exclusions` (process/exe/теки), `words` (always/never switch),
    `enabled` (стан паузи — це вимикач, не «параметр»). НЕ чіпає файл навчених слів
    (`learned_exceptions.txt`) і автозапуск (реєстр, поза `settings.json`).
  - **ГОТЧА:** reset переносить `exclusions`/`words` із ПЕРСИСТОВАНОГО стану
    (`AppState.settings`), а не з форми. Незбережені правки списків/словника пропадуть
    при скиданні — тож UI має модалку підтвердження. Власна app-команда → `core:default`,
    новий permission НЕ потрібен. UI — кнопка «Скинути до стандартних» (`data-testid="reset-button"`)
    + модалка (`reset-modal`/`reset-confirm`/`reset-cancel`), i18n `reset.*`/`action.reset`/`status.reset`.
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

## Гарячі клавіші (`hotkeys.rs` + `HotkeysDto`) — Фаза B1
- **Плагін:** `tauri-plugin-global-shortcut` v2. Реєструється у `run()` через
  `hotkeys::plugin()` (єдиний `with_handler`). Стан `HotkeyRegistry` (Tauri-`manage`,
  `Mutex<HashMap<Shortcut, HotkeyAction>>`) мапить акселератор, що спрацював, → дію.
- **Дозволи (capabilities):** `global-shortcut:allow-register/-unregister/-unregister-all/-is-registered`
  у `capabilities/default.json`. Реєстрація йде з Rust (не через JS-`invoke`), але
  дозволи додано про запас (якщо UI колись керуватиме напряму).
- **DTO:** `HotkeysDto` в `AppSettings` (`config.rs`) — по `HotkeyBinding {accelerator, enabled}`
  на дію. Дії (`HotkeyAction::ALL`): `pause_resume`, `revert_last`, `manual_switch`,
  `case_upper`, `case_lower`, `case_sentence`. **Усі дефолтно ВИМКНЕНІ**, акселератори
  неконфліктні (`Ctrl+Alt+{P,Z,S,U,L,E}`). `serde(default)` → старий `settings.json`
  без секції читається в дефолти (back-compat). `SCHEMA_VERSION` піднято 1→2.
  `sanitized()` лише тримить акселератори (валідність формату перевіряє вже плагін
  при `Shortcut::from_str` — невалідний/зайнятий просто не активується, лог у stderr).
- **`hotkeys::apply(app, settings)`:** зняти ВСІ (`unregister_all`) → поставити заново
  лише `enabled` прив'язки з непорожнім акселератором. Викликається у `setup` і після
  кожного `save_settings`. **Хоткеї НЕ залежать від `enabled`** (пауза/активний):
  інакше не відновити роботу з клавіатури.
- **Роутинг (`hotkeys::route`):** handler реагує лише на `ShortcutState::Pressed`
  (кличеться й на Released). `PauseResume` → `crate::toggle_enabled` (`pub(crate)`;
  інверсія `enabled`, запис на диск, оновлення трею, емісія `settings:changed`) —
  НЕ через канал, бо пауза/відновлення мають працювати й коли рушій зупинено.
  Решта дій (`RevertLast`/`ManualSwitch`/`CaseUpper|Lower|Sentence`) → команда в
  потік рушія через `RuntimeManager::send_command` (див. нижче).

### Командний канал рушія (`runtime.rs`) — НЕОЧЕВИДНЕ
**Чому канал, а не прямий виклик core-API з хендлера:** рушій крутиться в ОКРЕМОМУ
потоці (`engine_loop`) і ВОЛОДІЄ `EngineState` + `WindowsPlatform` (хуки/ввід).
Хоткей-хендлер (потік Tauri) не має до них доступу й не сміє їх шарити між потоками.
- `enum EngineCommand { RevertLast, ManualSwitch, ApplyCase(CaseMode) }` (крос-платформний).
- `start_engine` створює `mpsc::channel`; `EngineHandle` тримає `tx`, потік отримує `rx`.
- `engine_loop` на КОЖНІЙ ітерації спершу **неблокуюче** поллить `cmd_rx.try_recv()`
  (пріоритет над input-подіями), виконує команду на СВОЇХ `state`+`platform`, далі
  `continue`. Так доступ до стану серіалізовано (хоткеї й ввід не конкурують).
  - `RevertLast` → `revert_last(&mut state)` → `apply_actions` (із персистом `CommitException`).
  - `ManualSwitch` → будує `Context` (як звичайний крок) → `force_switch_last(&mut state, &ctx)` → apply.
  - `ApplyCase(mode)` → `get_selection_text()` (синтет. Ctrl+C, відновлює clipboard) →
    `transform_case(&text, mode)`; якщо змінилось → `apply(TypeUnicode(out))` (друк
    поверх виділення замінює його; `DeleteChars` не потрібен — НЕ перевірено живцем
    у всіх полях, потенційна готча).
- `RuntimeManager::send_command(cmd) -> bool`: `false`, якщо рушій НЕ запущено
  (пауза/`enabled=false`) → хоткей-дія тихо ігнорується (revert/manual/case без
  активного движка не мають сенсу). На не-Windows завжди `false` (заглушка).
- `apply_actions` — локальний хелпер у `engine_loop`: платформа йде ПАРАМЕТРОМ
  (не захоплюється), щоб не конфліктувати по борроу з основним циклом.
- **UI-картка «Гарячі клавіші»** (`App.svelte`): рядок на дію — чекбокс `enabled` +
  поле акселератора. Поле захоплює комбінацію по `onkeydown` (`accelFromEvent` →
  `Ctrl+Alt+P`; Backspace/Delete — очистити), але лишається й текстово редагованим.
  i18n — `hotkeys.*` у `i18n.js`; typedef `Hotkeys`/`HotkeyBinding` у `api.js`.

## Зворотний зв'язок (B2): звук + трей-індикатор
- **Прапорець:** `feedback.sound_on_switch: bool` (default `false`) у новій `FeedbackDto`
  (`config.rs`, окремо від `behavior` — це сигнал, не евристика). `serde(default)` →
  back-compat; `SCHEMA_VERSION` 3→4. UI — картка «Звук і сповіщення» (`feedback.*` i18n,
  typedef `Feedback`).
- **Звук (`feedback.rs`):** короткий вбудований wav `assets/switch.wav` (~4.4 КБ, згенеровано),
  `include_bytes!`. Відтворення — Win32 `PlaySoundW` з `SND_MEMORY|SND_ASYNC|SND_NODEFAULT`
  (з пам'яті, **не блокує** hot-path, тиша при помилці). Не-Windows — заглушка. Феатура
  `Win32_Media_Audio` у `windows-sys`.
- **Коли грати:** у `engine_loop` ПІСЛЯ `step()`, якщо `sound_on_switch && is_real_switch(&actions)`.
  `runtime::is_real_switch` (чисте, тестоване) = дії містять І `SwitchLayout`, І `TypeUnicode`
  (справжній авто-перенабір, а не пропуск/самонавчання). **Анти-цикл:** грає лише на НАШ
  перенабір (синтетичний ввід не дає switch-крок), і ніколи на паузі (потік не крутиться).
  Прокид: `start_engine` бере `settings.feedback.sound_on_switch` → `engine_loop`.
- **Трей-індикатор розкладки:** `engine_loop` тримає `AppHandle` (клон) і на ЗМІНУ розкладки
  (`platform.current_layout()`, debounce через `last_lang`) кличе `crate::on_engine_layout`
  **через `app.run_on_main_thread`** (tray-операції Win32 мусять іти з головного потоку).
  `on_engine_layout` пише `AppState.current_lang` і викликає `refresh_tray`.
- **`refresh_tray(app, enabled)`** тепер ставить: (1) **іконку** — `TrayIcons.active` vs
  `.paused` (приглушена grayscale+напівпрозора копія, `make_paused_icon`, будується раз у
  `setup` через `Image::new_owned` → `'static`); (2) **tooltip** — «на паузі» / «активний (UK/EN)»
  (мова з `current_lang`). Пауза-toggle скидає `current_lang=None`.
- **Готча — порядок у `setup`:** `TrayIcons` керується ДО `sync_runtime`; стартова емісія
  розкладки з потоку рушія йде через `run_on_main_thread` (відкладено), тож виконається
  вже ПІСЛЯ побудови трею. `on_engine_layout` має `#[cfg_attr(not(windows), allow(dead_code))]`
  (на не-Windows рушій-потоку нема).
- **`RuntimeManager::apply`/`start_engine` тепер беруть `&AppHandle`** (для клону в потік).
  Єдиний кличе — `lib.rs::sync_runtime`. `live_engine.rs` свій цикл, `apply` не зачіпає.

## Автозапуск із Windows (B5) — `tauri-plugin-autostart`
- **Плагін:** `tauri-plugin-autostart` v2. Реєструється у `run()`:
  `tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None::<Vec<&str>>)`.
  Args=None — застосунок і так стартує прихованим у трей (окремий `--minimized` не
  потрібен). На Windows керує Run-ключем реєстру; macOS — LaunchAgent.
- **ДЖЕРЕЛО ІСТИНИ — сам плагін (реєстр), НЕ `settings.json`.** Стан автозапуску
  навмисно НЕ дублюємо в конфігу, щоб він не розійшовся з реальним записом реєстру.
  UI при відкритті читає `get_autostart` (= `app.autolaunch().is_enabled()`).
- **Дозволи (capabilities):** `autostart:allow-enable / -disable / -is-enabled` у
  `capabilities/default.json` (без них `invoke` падає в рантаймі).
- **Команди (`lib.rs`):** `get_autostart() -> bool` (читає плагін),
  `set_autostart(enabled: bool) -> bool` (enable/disable через `AutoLaunchManager`,
  оновлює трей, повертає ПЕРЕЧИТАНИЙ фактичний стан — не «бажаний»). Обгортки в
  `api.js`: `getAutostart`/`setAutostart`. i18n — `system.*`.
- **Трей-пункт `MENU_AUTOSTART`:** `CheckMenuItem` (галочка = `is_enabled()`), по
  кліку `toggle_autostart` → enable/disable, `refresh_tray`, емісія `autostart:changed`
  (payload `bool`). `build_tray_menu` щоразу перечитує `is_enabled()` (помилка → знято).
- **Синхрон трей↔UI↔реєстр:** три точки правлять одне (реєстр):
  - UI-toggle → `setAutostart` (через `$effect` з guard `autostartApplied`, щоб не
    спрацьовувати на завантаженні/синку з трею) → плагін → `refresh_tray` оновлює галочку.
  - Трей-пункт → `toggle_autostart` → плагін + емісія `autostart:changed` → App слухає,
    оновлює чекбокс БЕЗ повторного запису (виставляє `autostartApplied` = payload першим).
  - Відкриття вікна → `getAutostart` читає реєстр як стартове значення.
  UI-стан автозапуску ОКРЕМИЙ від `settings`/`dirty` (Save його не чіпає — застосовується
  миттєво при перемиканні).
- **Single-instance:** плагіна `tauri-plugin-single-instance` у проєкті НЕМАЄ (не
  додавали в межах B5). Автозапуск сам подвійного інстансу не плодить (один Run-запис),
  але захист від ручного повторного запуску відсутній — окрема задача.

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

## Мовна пара (B6) — параметричний шлях `language` → движок
**Стан:** повністю data-driven. Конфіг несе `language: LanguagePair` (enum, serde
kebab-ключ `uk-en`). Єдина зв'язка «пара → мови» — **`LanguagePair::langs()`**
(`config.rs`, поряд із варіантом enum) → `["uk","en"]`. `runtime::langs_for` —
тонка обгортка над нею (БЕЗ власного захардкодженого uk/en). Лоадери
(`load_language_profiles`/`load_word_rules`) лише ітерують `langs_for(pair)` і
вантажать дані за рядком-мовою через `typofix-data` — мовно-агностичні. Движок
(`step`/`Context.languages`) і платформний layout-switch (за PRIMARYLANGID серед
ВСТАНОВЛЕНИХ розкладок) теж не знають конкретних мов. Зміна пари в UI → `save_settings`
→ `sync_runtime` перезапускає движок із новими профілями.

## Розкладки клавіатури (візуалізація — секція «Розкладки клавіатури»)
ЛИШЕ показ/переконливість: користувач БАЧИТЬ встановлені в ОС розкладки, дві з яких
TypoFix використовує (мови активної пари), і що решта ігнорується. **Логіку
перемикання НЕ зачіпає** (вона вже коректно ігнорує третю розкладку).
- **Команда `list_keyboard_layouts() -> Vec<KeyboardLayoutDto>`** (`lib.rs`). DTO:
  `{ name: String, langid: String ("0x0022"), role: "uk"|"en"|"ignored", active: bool }`.
  Бере мови активної пари (`AppState.settings.language.langs()`), кличе
  `installed_layouts()`, і для кожної розкладки зіставляє її `primary_langid` із
  мовами пари: збіг → `role` = мова, інакше `"ignored"`. Працює в межах `core:default`
  (новий permission НЕ потрібен). Не-Windows → порожньо. Обгортка `listKeyboardLayouts`
  у `api.js`.
- **Мапінг `lang → primary_langid`** — `primary_langid_for` у `lib.rs` (єдине джерело
  на app-шарі): `uk`→`0x22`, `en`→`0x09`. Додаєш мову в `LanguagePair` → додай і сюди.
- **Джерело даних — платформа:** `layout_dtos` (cfg windows) кличе
  `typofix_platform_windows::installed_layouts() -> Vec<InstalledLayout{ name,
  primary_langid, is_active }>` (реекспорт із крейта; є windows + non-windows стаб).
  На не-Windows src-tauri не залежить від крейта → `layout_dtos` повертає порожньо.
- **UI** (`App.svelte`, картка `data-testid="card-layouts"`): список розкладок (назва +
  langid + «● активна»), бейдж «використовується» на ролях uk/en і приглушений
  «ігнорується» на решті (`li.ignored` — opacity); пояснення «TypoFix перемикає лише
  між <uk> та <en>…» (коли обидві є); кнопка «Оновити» (`layouts-refresh`). **Попередження
  про відсутню мову:** `missingLangs` рахується в UI (мова пари без жодної розкладки тієї
  ролі) → «Розкладку <мову> не встановлено — …» (`data-testid="layouts-missing"`).
  i18n — `layouts.*`/`section.layouts.*`.

### ЧЕКЛИСТ: як додати мовну пару (напр. `pl-en`)
1. **Дані** в `data/` для КОЖНОЇ мови пари (через наявні loader'и, крейти НЕ чіпати):
   `layouts/{lang}.toml`, `lm/{lang}.bin`, `dicts/{lang}.fst` (+ опц.
   `dicts/{lang}.freq.fst`, `dicts/{lang}.short.txt`, спільні `user.txt`/`iso4217.txt`/
   `extensions.txt`). Відсутній файл → м'яка деградація на вбудований зразок.
2. **Enum** у `config.rs`: варіант `PlEn` з `#[serde(rename = "pl-en")]` + його арм у
   `LanguagePair::langs()` → `["pl","en"]`. (Обидва в одному файлі, поряд.)
3. **UI** (`App.svelte`): `<option value="pl-en">{$t("language.pl-en")}</option>` +
   i18n-рядок `language.pl-en` (`i18n.js`). За потреби онови `section.language.note`.
4. **НІЧОГО з логіки не міняти:** ні лоадери, ні `engine`, ні платформу, ні
   `sync_runtime`. Контракт перевіряє тест `language_pair_langs_matches_serde_key`
   (мови = частини kebab-ключа). Реальні дані для другої мови — окремий датасет-проєкт;
   фейкову мову НЕ додавати.

## Маппінг конфіг → ядро (`runtime.rs`, чисте, тестоване)
- `exclusion_rules_from` → `core::ExclusionRules` (process/exe/folder; нормалізацію
  шляхів робить core).
- `detector_config_from`: `min_word_len`→`min_switch_len` (прямий). `confidence_threshold`
  (0..1) масштабує `base_threshold` монотонно навколо 0.75=дефолт — **тимчасова**
  евристика (внутрішній поріг — лог-ймовірнісний, не 0..1); справжня калібровка у фазі eval.
  **B4-тоггли** (`settings.behavior`) прокидаються в `*_enabled`-прапорці тут же
  (`fix_case`/`forex`/`recognize_extensions`/`phonotactics`/`fix_capslock` →
  `case_fix_enabled`/`forex_enabled`/`extensions_enabled`/`phonotactics_enabled`/
  `capslock_fix_enabled`); решта полів — з `DetectorConfig::default`.
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

### Перегляд/керування навченими словами (B3, частина 1)
- **Команди (`lib.rs`):** `list_learned() -> Vec<String>` (через
  `runtime::learned_for_display`: дедуп регістронезалежно + сортування),
  `remove_learned(word) -> bool` (прибрати одне), `clear_learned()` (очистити все).
  Обгортки в `api.js`: `listLearned`/`removeLearned`/`clearLearned`. i18n — `learned.*`.
  Власні app-команди → працюють у межах `core:default` (новий permission НЕ потрібен).
- **Запис АТОМАРНИЙ:** `runtime::write_learned`/`remove_learned` — tmp(`.txt.tmp`)→
  `rename` поверх цілі (як `config::save_to_disk`). Видалення регістронезалежне;
  `remove_learned` повертає `false`, якщо слова не було (файл не чіпаємо).
- **СИНХРОН IN-MEMORY (готча!):** `EngineState.learned` засівається з файлу ЛИШЕ при
  старті потоку (`start_engine`→`load_learned`); core НЕ має API видалення слова з
  пам'яті (і ми не чіпаємо `crates/*`). Тому після `remove_learned`/`clear_learned`
  команда викликає `sync_runtime(&app, &settings)` — той самий шлях, що `save_settings`:
  `stop_engine`+`start_engine` → свіжий `load_learned` із редукованого файлу. Без цього
  слово ігнорувалось би до перезапуску застосунку. `sync_runtime` сам зважає на `enabled`
  (на паузі движка нема — просто перезапис файлу).
- **UI:** картка «Навчені слова» (`App.svelte`) — лічильник + «Оновити» + «Очистити все»
  + список із кнопкою ✕ на кожен запис; порожній стан — дружнє повідомлення. Стан
  `learned` ОКРЕМИЙ від `settings`/`dirty` (це не конфіг; Save його не чіпає,
  застосовується миттєво). Стиль повторює `RuleList`.
- **Приватність:** ці слова вже на диску (як і раніше); нічого нового нікуди не шлемо.

### Per-app повне вимкнення (B3, частина 2) — ВЖЕ є через `exclusions`
Семантика виключень = **ПОВНЕ вимкнення**, не часткове. `typofix_core::step` (і
`force_switch_last`) ПЕРШИМ ділом перевіряє `ctx.is_window_excluded()` і повертає
`Vec::new()` — вікно у списку виключень НЕ буферимо й НЕ перемикаємо взагалі. Тож
B3.4 «у цій програмі не працювати» забезпечує наявний механізм `exclusions`
(process/exe/folder) — окремого «повного off» додавати НЕ треба. У B3 лише уточнено
формулювання UI картки виключень («У цих програмах TypoFix узагалі не працює»).

## UI-e2e (рідний автотест вікна через tauri-driver/WebView2)
- **Афорданс `TYPOFIX_E2E=1`** (`lib.rs`, `e2e_mode()`): вікно `settings` стартує
  ВИДИМИМ + движок і глобальні хоткеї НЕ стартують (тест не чіпає глобальну
  клавіатуру). Прод-поведінка без env — без змін. Гард — у двох місцях: `setup`
  (пропускає `sync_runtime`/`hotkeys::apply`) і **early-return на початку
  `sync_runtime`** (тож `save_settings`/трей-toggle/learned-команди теж no-op у тесті;
  `hotkeys::apply` у `save_settings` теж за `!e2e_mode()`).
- **Харнес:** `ui/e2e/` (WebdriverIO + `tauri-driver`). `npm run test:e2e`. Деталі/
  передумови — `ui/e2e/README.md`.
- **ГОТЧА — збірка для тесту:** лише `tauri build --no-bundle` (НЕ `cargo build
  --release`!). `cargo build` робить бінар, що вантажить **dev-сервер
  `localhost:1420`** замість вбудованого `ui/dist` → WebDriver не бачить розмітку
  (порожній DOM без `data-testid`). `tauri build` вшиває `frontendDist`.
- **ГОТЧА — msedgedriver під WebView2:** версія = версії **WebView2 Runtime**
  (не Edge). Лежить у `ui/e2e/drivers/msedgedriver.exe` (gitignored); `wdio.conf.js`
  передає його як `--native-driver`.
- **Селектори:** стабільні `data-testid` у `App.svelte` (`card-*`, `behavior-<key>`(+`-input`),
  `sensitivity-slider`, `language-select`, `save-button`, `save-status`/`data-status`).
  Toggle.svelte має опц. `testid` (лягає на клікабельний `<label>` + `${testid}-input`).
- **ГОТЧА — липкий футер:** `.actions` (`position:sticky; bottom:0`) перекриває нижні
  елементи → «element click intercepted». Кліки в тесті — через `clickCentered()`
  (scrollIntoView `block:center`).

## Дозволи (capabilities) — НЕ забути при додаванні команд/плагінів
`capabilities/default.json` (window `settings`) перелічує дозволи. Без потрібного
дозволу `invoke` падає в рантаймі (компіляція мовчить!). Зараз увімкнено: `core:default`,
events, window show/hide/focus, `dialog:allow-open` (file-picker для exe/теки —
плагін `tauri-plugin-dialog`), `global-shortcut:allow-register/-unregister/-unregister-all/-is-registered`
(плагін `tauri-plugin-global-shortcut`), `autostart:allow-enable/-disable/-is-enabled`
(плагін `tauri-plugin-autostart`, B5). Власні app-команди (`load_settings`/`save_settings`/
`get_autostart`/`set_autostart`/`list_learned`/`remove_learned`/`clear_learned`)
працюють у межах `core:default`. Додаєш плагін → додай його permission сюди.
