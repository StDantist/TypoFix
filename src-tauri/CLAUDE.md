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

## Дозволи (capabilities) — НЕ забути при додаванні команд/плагінів
`capabilities/default.json` (window `settings`) перелічує дозволи. Без потрібного
дозволу `invoke` падає в рантаймі (компіляція мовчить!). Зараз увімкнено: `core:default`,
events, window show/hide/focus, `dialog:allow-open` (file-picker для exe/теки —
плагін `tauri-plugin-dialog`). Власні app-команди (`load_settings`/`save_settings`)
працюють у межах `core:default`. Додаєш плагін → додай його permission сюди.
