# UI-e2e — рідний автотест вікна налаштувань (tauri-driver + WebView2)

Клік-проходка по вікну налаштувань TypoFix через **tauri-driver** (WebView2
WebDriver) і **WebdriverIO**. Запускає РЕАЛЬНИЙ зібраний `typofix-app.exe`,
дочікується DOM і перевіряє картки/взаємодії (асерти на текст/стан, не на пікселі).

## Тест-режим застосунку (`TYPOFIX_E2E=1`)

Чистий UI-тест блокують два прод-факти: вікно `settings` стартує приховане
(`visible:false`), а застосунок ставить глобальні клавіатурні хуки на старті. Тому
застосунок має афорданс: за env-змінною `TYPOFIX_E2E=1` —

- вікно `settings` стартує **видимим** (WebDriver бачить DOM без трей-взаємодії);
- движок і глобальні хоткеї **НЕ стартують** (тест не чіпає глобальну клавіатуру) —
  гард у `src-tauri/src/lib.rs` (`e2e_mode()`): `setup` пропускає `sync_runtime`/
  `hotkeys::apply`, а `sync_runtime` має early-return, тож і `save_settings`/трей-toggle/
  learned-команди не піднімають хуки.

Прод-поведінка без змінної — **без змін**.

## Передумови (Windows)

1. **Зібраний застосунок** з вбудованим фронтендом:
   ```powershell
   # З КОРЕНЯ репо. Саме `tauri build` (не `cargo build`!) — інакше exe вказує на
   # dev-сервер localhost:1420, а не на вбудований ui/dist, і тест не бачить розмітку.
   .\ui\node_modules\.bin\tauri build --no-bundle
   ```
   Якщо лінк падає «Access is denied» — спершу вбийте запущений процес:
   `Get-Process typofix-app -ErrorAction SilentlyContinue | Stop-Process -Force`.

2. **tauri-driver** (intermediary WebDriver):
   ```powershell
   cargo install tauri-driver    # лягає в ~/.cargo/bin/tauri-driver.exe
   ```

3. **msedgedriver**, версія = версії **WebView2 Runtime** (не Edge-браузера):
   ```powershell
   # дізнатись версію WebView2 Runtime:
   (Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}').pv
   # завантажити відповідну версію:
   #   https://msedgedriver.microsoft.com/<версія>/edgedriver_win64.zip
   # і розпакувати msedgedriver.exe у ./drivers/
   ```
   У репо очікується `ui/e2e/drivers/msedgedriver.exe` (gitignored — локальний бінар).

## Запуск

```powershell
cd ui\e2e
npm install        # одноразово (WebdriverIO + раннер)
npm run test:e2e
```

`wdio.conf.js` сам піднімає `tauri-driver --native-driver ./drivers/msedgedriver.exe`,
запускає `../../src-tauri/target/release/typofix-app.exe` з `TYPOFIX_E2E=1` іганяє
specs із `./specs/**/*.e2e.js`.

## Що перевіряє `specs/settings.e2e.js`

- заголовок сторінки відрендерився;
- усі ключові картки присутні (Гарячі клавіші, Поведінка, Звук і сповіщення,
  Системне, Навчені слова, Мовна пара, Виключення, Слова-винятки);
- картка «Поведінка»: 5 тогглів + повзунок чутливості;
- клік по тогглу поведінки міняє його стан (і відкат назад);
- повзунок чутливості рухається (ArrowRight);
- картка «Навчені слова» (список або дружній порожній стан);
- селектор мовної пари = `uk-en`;
- клік «Зберегти» завершується статусом `saved`, не помилкою (зі скиданням правок).

## Готчі

- **Селектори** — стабільні `data-testid` з `ui/src/App.svelte` (+ видимі тексти з
  `i18n.js`). Картки: `card-*`; тоггли: `behavior-<key>` (клікабельний `<label>`) +
  `behavior-<key>-input` (читання `isSelected`); слайдер: `sensitivity-slider`;
  Save: `save-button`; статус: `save-status` з атрибутом `data-status`.
- **Липкий футер** `.actions` (`position: sticky; bottom: 0`) перекриває елементи
  внизу видимої області → «element click intercepted». Тому інтерактивні кліки йдуть
  через `clickCentered()` (скрол у центр в'юпорта перед кліком).
- Перед кожним прогоном варто вбити лишки: `typofix-app`, `msedgedriver`, `tauri-driver`.
