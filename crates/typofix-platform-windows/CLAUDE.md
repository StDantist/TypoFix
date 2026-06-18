# typofix-platform-windows — правила

Жива реалізація `trait Platform` поверх WinAPI. `unsafe` тут дозволений (на
відміну від core). Нижче — **неочевидне**, що легко зламати.

## Структура
- `keystate.rs`, `scancode.rs` — **чисті, без WinAPI**, компілюються й тестуються
  на будь-якій ОС. Уся логіка модифікаторів/класифікації — тут, щоб бути
  тестованою без живої системи. Не тягни сюди WinAPI.
- `layout.rs` — `ToUnicodeEx`-запит розкладки + `LayoutId`↔HKL (Windows).
- `window.rs` — активне вікно (`GetForegroundWindow`/`QueryFullProcessImageNameW`).
- `inject.rs` — `SendInput`/перемикання розкладки + `send_ctrl_c` (Ctrl+C через VK).
- `selection.rs` — `get_selection_text()`: синтетичний Ctrl+C → читання буфера.
- `hook.rs` — LL-хуки (клавіатура/миша) + `EVENT_SYSTEM_FOREGROUND` (лише емісія
  `FocusChange` для інвалідації буфера) + message-pump потік. 🔴 **Жодного UIA/COM/
  `SendMessageTimeoutW` тут.**
- `secure.rs` — кеш секретності (`CACHE: AtomicBool`) + `recompute` (ЛИШЕ нативна
  перевірка, БЕЗ UIA). `secure_thread.rs` — виділений потік: WinEvent focus-хуки
  (`EVENT_SYSTEM_FOREGROUND`+`EVENT_OBJECT_FOCUS`) + дебаунс 60мс + нативний перерахунок.
  UIA з рантайму прибрано назавжди — деталі/аудит у секції «Детекція секретних полів».
- `src/bin/live_spike.rs` — **РУЧНИЙ** харнес (див. нижче).
- `src/bin/selection_smoke.rs` — **АВТОНОМНИЙ** live-харнес B1 (notepad-round-trip,
  сам перевіряє себе; код виходу 0/1). Запуск/готчі — нижче.
- `src/bin/e2e_retype_smoke.rs` — **АВТОНОМНИЙ real-OS наскрізний** харнес (detection
  +switch+retype у живий Notepad; за feature `e2e-harness`). Запуск/готчі — нижче.
- Не-Windows ціль → тонка заглушка `stub` (щоб CI на Linux лишався зеленим).

## Готчі (порушиш — зламаєш тихо)

1. **Анти-цикл проти власного SendInput.** Хук бачить НАШ перенабір. Власні
   події мають `LLKHF_INJECTED` → ставимо `is_synthetic=true`; ядро їх ігнорує.
   Додатково мітимо `dwExtraInfo = INJECT_SIGNATURE` (точна ідентифікація саме
   нашого вводу). Якщо забути — `TypeUnicode` спричинить нескінченний перенабір.

2. **LL-хуки потребують message-pump на ТОМУ Ж потоці.** Без `GetMessage`/
   `DispatchMessage` callbacks не викликаються взагалі. Тому окремий потік
   (`hook.rs`) ставить хуки і крутить насос; стоп — `PostThreadMessage(WM_QUIT)`
   з `Drop for HookHandle`. Хуки знімаються у тому ж потоці після виходу з pump.

3. **`ToUnicodeEx` мутує per-thread dead-key стан.** Передаємо ВЛАСНИЙ очищений
   key-state (ніколи не чіпаємо глобальний `GetKeyboardState`) і **зливаємо**
   мертвий стан пробілом до/після (`flush_dead_key`). Інакше запит «^» лишить
   діакритику висіти й зіпсує наступний реальний символ. `-1` = мертва клавіша.

4. **AltGr ≠ Ctrl+Alt.** Windows під AltGr тримає Ctrl+Alt натиснутими. У
   `ModSnapshot::to_modifiers` при `altgr` ставимо лише `ALTGR`, прибираючи
   фантомні CTRL/ALT — інакше ядро вважатиме це командною комбінацією й
   інвалідовуватиме буфер замість набору символу. AltGr виявляємо за `VK_RMENU`.

5. **Емітимо лише key-DOWN.** Key-up уживаємо тільки для обліку натиснутих
   (auto-repeat). Auto-repeat дедуплікуємо: перший повтор → одна подія з
   `is_autorepeat=true`, далі тиша до відпускання. Навігація (стрілки/Home/End/
   PageUp-Down) → `CaretMove`, не `Key`.

6. **Модифікатори читаємо `GetAsyncKeyState` (фізичний стан), не `GetKeyState`.**
   LL-хук-потік не має фокусу клавіатури, тож черго-залежний `GetKeyState` бреше.
   Caps — це toggle-біт `GetKeyState(VK_CAPITAL)&1`.

7. **`SwitchLayout` адресуємо вікну на передньому плані** (`WM_INPUTLANGCHANGEREQUEST`),
   НЕ `ActivateKeyboardLayout` (той змінив би лише наш потік). Невідому
   `LayoutId` тихо ігноруємо (precision > recall: краще не перемкнути).

8. **scancode вже set 1** прямо з `KBDLLHOOKSTRUCT.scanCode` — збігається з
   `data/layouts/*.toml` і `core::layout_mapper`, додаткового мапінгу не треба.

9. **🔴 НІКОЛИ не `LoadKeyboardLayoutW` — вона ВСТАНОВЛЮЄ розкладку в систему.**
   Якщо точного KLID немає, Windows тихо додає його в список розкладок користувача
   (засмічення системи дублями) — і це спрацьовувало навіть на запит символів та в
   тестах. Працюємо **виключно з уже встановленими** розкладками: перелік через
   `GetKeyboardLayoutList`, матч за **`PRIMARYLANGID`** (молодше слово HKL & 0x3FF;
   `en`→0x09, `uk`→0x22 — збігається з будь-яким варіантом мови). Єдина точка —
   `installed_hkl_for_layout_id`; немає мови в системі → `None` (НЕ перемикаємо, НЕ
   інсталюємо). Запит символів і перемикання йдуть лише через неї.

10. **🔴 `GetKeyboardLayout(fg_tid)` БРЕШЕ для UWP/консольних вікон** —
    `GetForegroundWindow` віддає обгортку `ApplicationFrameWindow`, чий потік має
    дефолтну розкладку, а не реальну (підтверджено: при активній EN UWP-Notepad
    читалась uk → детектор не перемикав). **Продакшн-метод — M2 (`current_hkl`
    = `m2_hkl`):** `GetGUIThreadInfo(fgTid).hwndFocus` дає СПРАВЖНЄ фокусне вікно
    (всередині UWP-хоста) → читаємо `GetKeyboardLayout` ЙОГО потоку. Fallback:
    немає `hwndFocus` → потік самого `GetForegroundWindow()`; tid=0 →
    `GetKeyboardLayout(0)`. Емпірично (`layoutprobe`, десктоп-вікно): M1/M2/M3
    рівноцінні, M4=наш потік (хибний); **M2 обрано заради UWP/console**.
    Діагностику (`probe_layout_methods`, режими `layoutprobe`/`layout`) ЛИШЕНО —
    знадобиться, якщо M2 десь не дотягне. `AttachThreadInput`-шлях (M3) лишається
    лише в діагностиці, НЕ в продакшні (його `GetKeyboardLayout(0)` читав наш
    потік → завжди en). Оптимізація (поки НЕ зроблено): кешувати розкладку й
    оновлювати по `EVENT_SYSTEM_FOREGROUND` + на межі слова, замість запиту на
    кожен виклик. Зараз пріоритет — коректність; кеш — follow-up.

11. **`get_selection_text` (selection.rs) ОБОВʼЯЗКОВО відновлює буфер обміну.**
    Шле підписаний Ctrl+C (`inject::send_ctrl_c`, той самий `INJECT_SIGNATURE` —
    хук ігнорує), чекає зміни `GetClipboardSequenceNumber` (таймаут 400 мс,
    крок 10 мс), читає `CF_UNICODETEXT`, тоді **повертає** попередній вміст
    (правило приватності №4 — користувач не має втратити clipboard). Якщо seq не
    змінився (порожнє виділення/таймаут) → буфер НЕ чіпали, відновлювати нічого,
    повертаємо `None`. 🔴 Знімок робимо ЛИШЕ для global-памʼятних форматів
    (`is_global_format`): `GlobalSize`/`GlobalLock` на handle-форматах (CF_BITMAP,
    CF_PALETTE, CF_METAFILEPICT, CF_ENHMETAFILE, DSP-варіанти) = **пошкодження
    купи** (STATUS_HEAP_CORRUPTION). `SetClipboardData` передає власність памʼяті
    ОС → прийняті handle більше НЕ звільняємо (`GlobalFree`), решту копій — так.
    Тест свідомо ОДИН (`headless_safety_and_clipboard_preserved`): буфер процес-
    глобальний, два паралельні clipboard-тести гонилися б за ним і падали.
    Виклик передбачено з ОДНОГО потоку движка (не конкурентно).

## `installed_layouts()` — перелік розкладок ОС із людськими назвами (для UI)
`installed_layouts() -> Vec<InstalledLayout{ name, primary_langid, is_active }>`
(реекспорт із lib.rs; не-Windows — `vec![]`). HKL з `GetKeyboardLayoutList`
(нічого НЕ інсталює), назва — `LCIDToLocaleName(langid)`→`GetLocaleInfoEx(
LOCALE_SLOCALIZEDLANGUAGENAME)`, fallback `0x{langid:04x}`. Дублі langid лишаємо
(користувач має бачити варіанти). `is_active` = HKL == `current_hkl()`.
- ⚠️ **Назва локалізується під мову UI Windows, НЕ під саму мову розкладки:** на
  англомовній Windows uk-розкладка зветься `"Ukrainian"`, не `"Українська"`
  (`SLOCALIZEDLANGUAGENAME` = «як називає цю мову поточний UI»). Це коректно для UI
  застосунку (єдина мова інтерфейсу). Якщо колись треба ендонім — `LOCALE_SNATIVELANGUAGENAME`.
- `primary_langid` (0x09/0x22…) дає src-tauri звʼязати розкладку з нашою парою.

## Що перевірено автоматично (частина A, безпечно, без вводу в систему)
`cargo test -p typofix-platform-windows` (15 тестів) реально б'є по WinAPI:
- `ToUnicodeEx` серед **уже встановлених** розкладок (`installed_hkl_for_layout_id`,
  US якщо є): a/A, 1/!, пробіл, Caps — нічого не інсталює (skip, якщо мови немає);
- власний `QueryFullProcessImageNameW` → шлях до тест-exe;
- `GetForegroundWindow`/`current_layout_id` не панікують;
- чисті модифікатори/класифікація (AltGr, навігація).

## Як ганяти LIVE-харнес (частина B — ⚠️ ПОБІЧНІ ЕФЕКТИ, лише вручну)
**Не запускати наосліп:** ставить реальні хуки (перехоплює ВЕСЬ ввід), а
`SendInput` друкує у вікно з фокусом.
- Безпечний лог: `cargo run -p typofix-platform-windows --bin live_spike`
  (~8 c друкує захоплені події; фізичні → `is_synthetic=false`).
- З перенабором (ДРУКУЄ!): `... --bin live_spike -- type` — за 3 c один
  `SwitchLayout(uk)+TypeUnicode("привіт")`; переключись у порожній Notepad.
- Очікуваний доказ анти-циклу: під час кроку `type` власний ввід повертається
  вже `is_synthetic=true`.

## Live smoke-тест B1 (`selection_smoke`, автономний)
`cargo run -p typofix-platform-windows --bin selection_smoke` — сам запускає
notepad, друкує `hello world`, Ctrl+A, `get_selection_text`, перевіряє відновлення
clipboard, друк поверх виділення; вбиває notepad. ⚠️ Побічні ефекти (друкує у fg,
перезаписує clipboard→відновлює sentinel, force-kill усіх notepad). Потребує
**інтерактивної GUI-сесії**.
- 🔴 **Foreground-стілінг заблоковано Windows:** свіжо-запущене вікно НЕ стає
  активним саме по собі (інше вікно лишається fg). Обхід — `AttachThreadInput` до
  потоку поточного fg-вікна + `SetForegroundWindow/BringWindowToTop` (див.
  `force_foreground_notepad`). Без цього тест падав з `foreground="Deck.exe"`.
- Win11-Notepad (packaged) стартує повільно й може жити в іншому PID, ніж spawned
  child → шукаємо вікно за заголовком («notepad»/«блокнот»), не за PID.
- **Підтверджено на живій Win11 (2× поспіль, exit 0):** (а) `get_selection_text` →
  `Some("hello world")`; (б) clipboard відновлено (sentinel на місці) — privacy-
  гарантія тримається; (в) `TypeUnicode` поверх активного виділення **затирає**
  його (CF=`HELLO WORLD`, без дублювання) → **DeleteChars/Backspace перед друком НЕ
  потрібен** для ApplyCase у стандартних edit-контролах.

## Real-OS наскрізний e2e (`e2e_retype_smoke`, за feature `e2e-harness`)
`cargo run -p typofix-platform-windows --features e2e-harness --bin e2e_retype_smoke`
— сам будує движковий стек як `src-tauri::runtime::engine_loop` (реальна
`WindowsPlatform` з хуками + uk/en профілі з `typofix-data` + `core::step` +
реальний `apply`), друкує фізичні scancode у Notepad, виправляє, звіряє через
`WM_GETTEXT`. Потребує GUI-сесії + встановлені uk **і** en. Опційні deps
`typofix-core`/`typofix-data` під feature (прод-збірка крейта їх НЕ тягне);
bin має `required-features`, тож `cargo build`/`clippy --all-targets` БЕЗ feature
його пропускають — лінтити з `--features e2e-harness`.
- 🔴 **Чому не можна симулювати «фізичний» ввід через хук:** `hook.rs` визначає
  `is_synthetic` ВИКЛЮЧНО за `LLKHF_INJECTED` (НЕ за `INJECT_SIGNATURE`!), а Windows
  ставить цей прапор на будь-який `SendInput`. Тож user-mode-«ввід» завжди приходить
  у `step` як `is_synthetic=true` → ядро ІГНОРУЄ (це і є анти-цикл). Підтверджено
  емпірично: хук захопив усі 6–7 наших scancode з `is_synthetic=true`. Симулювати
  людський ввід через хук з user-mode неможливо (треба kernel-драйвер). Тому харнес
  годує `step` тими ж ОС-захопленими scancode, але з `is_synthetic=false` (єдина
  підміна); switch+retype і звірка вікна — повністю реальні.
- **Підтверджено наживо (Win11, 2× exit 0):** кейс1 UK-розкладка, фізичні `hello`
  → каша → `DeleteChars(6)+SwitchLayout(en)+TypeUnicode("hello ")` → Notepad = `"hello "`,
  розкладка→en; кейс2 EN-розкладка, фізичні `ghbdsn` → `DeleteChars(7)+SwitchLayout(uk)
  +TypeUnicode("привіт ")` → Notepad = `"привіт "`, розкладка→uk. Реакція ядра
  ~0.1–0.2 мс; застосування (switch+retype+settle) ~540–570 мс (більшість — навмисні
  `sleep` на домальовку/асинхронний switch, не вартість ядра).
- Готчі ті самі, що в `selection_smoke` (foreground через AttachThreadInput, вікно за
  заголовком). `current_layout` читається M2-методом → дає правильний uk/en на
  Notepad. Чистка між кейсами — Ctrl+A+Delete; розкладка ОС виставляється
  `SwitchLayout`+поллінг `current_layout_id`.

## Детекція секретних полів — НАТИВНА (`ES_PASSWORD`/passwordchar), БЕЗ UIA 🔴
**Статус:** УВІМКНЕНА, але **лише дешева нативна** перевірка. `WindowsPlatform::new()`
спавнить потік `typofix-secure` (WinEvent фокуса + дебаунс), `is_secure_field()` читає
кеш-атомік. **UIA назавжди прибрано з рантайму** (нуль `GetFocusedElement`/`CoInitialize`/
`CUIAutomation`) — він вмикав a11y-дерево цільової апки й лагав її (аудит нижче).
- **Покрито:** нативні Win32 пароль-поля — входи/UAC/інсталятори/десктоп-логіни,
  включно з Tab у поле пароля в межах того ж вікна (`EVENT_OBJECT_FOCUS` лишився, але
  тепер тригерить лише ДЕШЕВУ нативну перевірку → лагів немає).
- **НЕ покрито (свідомо):** браузерні/Electron `<input type=password>` (windowless,
  потребували б UIA), WinRAR v7 (owner-draw маскування без жодної семантики). Це окреме
  майбутнє завдання — потрібен підхід без активації a11y цільового застосунку.
- Плумбінг ядра `Context.secure` незмінний (за `true` → bypass + скид буфера).

### АУДИТ кореня лагу (репро власника) — чому UIA прибрано назавжди
Симптом: при роботі IDE з file-watcher (шторм фокус-подій) курсор/ввід лагав 30-40 с.
- **Спроба 1** (recompute на потоці LL-хука) — лаг, бо важкий UIA блокував message-pump
  LL-хука → `LowLevelHooksTimeout` → ОС лагала ВЕСЬ ввід.
- **Спроба 2** (recompute винесено на окремий потік `typofix-secure` + дебаунс) — **лаг
  ЛИШИВСЯ.** Це спростувало гіпотезу «винна лише блокування нашого pump» і вказало на
  справжній корінь: **сам ВИКЛИК UIA гальмує цільовий застосунок, а не наш потік.**
- **Справжній корінь (за кодом, підтверджено логікою API):** `IUIAutomation::
  GetFocusedElement` + `GetCurrentPropertyValue` — це **крос-процесні COM-виклики, що
  ПРИМУШУЮТЬ цільовий застосунок збудувати/віддати UI Automation provider-дерево**.
  Для Chromium/Electron/великих IDE це вмикає повний accessibility-режим у ЇХНЬОМУ
  процесі (відомий важкий шлях; той самий ефект, що «увімкнути екранний диктор»).
  Гальмується ЦІЛЬОВИЙ застосунок у СВОЄМУ процесі — отже наш потік (хоч окремий, хоч
  ні) не рятує: a11y-дерево будується на кожен новий фокус, а IDE сипле ними пачками.
- **Внесок факторів (оцінка за кодом; точний мікробенч без інтерактивного GUI неможливий):**
  1. **UIA `GetFocusedElement`/property — ОСНОВНИЙ (домінує).** Вмикає a11y у
     цільовому процесі; на VS Code/Chromium — десятки-сотні мс на виклик + стійка
     деградація, поки a11y лишається активним. Це й є причина, що off-thread не помогло.
  2. **Глобальний `EVENT_OBJECT_FOCUS` out-of-context WinEvent-хук — ВТОРИННИЙ.**
     `OBJECT_FOCUS` — дуже частa подія; out-of-context означає маршалінг кожної в наш
     процес. Шторм фокусів IDE = тисячі маршалінгів. Сам по собі легший за UIA, але
     множник частоти робить його помітним; головне — він ТРИГЕРИВ UIA на кожен фокус.
  3. **`SendMessageTimeoutW` 40мс (нативний passwordchar) — НАЙМЕНШИЙ, але не нуль.**
     Синхронний крос-процесний; на «здоровому» вікні ~миттєвий, але на зайнятому/
     завислому з'їдає до 40мс на виклик (× частота фокусів). `SMTO_ABORTIFHUNG` лише
     обмежує найгірший хвіст, не прибирає вартість.

### Що реалізовано зараз (= план безпечного редизайну з аудиту)
Пріоритет: НІКОЛИ не торкати a11y цільового застосунку на гарячому шляху.
- **Лише НАТИВНА детекція, БЕЗ UIA.** `ES_PASSWORD` через `GetWindowLongPtrW(GWL_STYLE)`
  — читання НАШОГО боку (не крос-процесний a11y), мікросекунди. `EM_GETPASSWORDCHAR`
  лише на edit-класи, з `SMTO_ABORTIFHUNG` + малим таймаутом (40мс).
- **UIA прибрано з коду назавжди** (не `allow(dead_code)`, а ВИДАЛЕНО): жодних
  `GetFocusedElement`/`CoInitialize`/`CUIAutomation`/vtable/VARIANT. Features windows-sys
  `Win32_System_Com`/`Win32_System_Variant` теж прибрано.
- **Веб/Electron — майбутнє завдання.** Потрібен підхід без активації a11y (напр.
  process-allowlist + явний opt-in), точно НЕ UIA на кожен фокус.

## Архітектура нативної детекції — `secure.rs` + `secure_thread.rs` + `window.rs`
- Натиски лише в RAM (канал mpsc), нічого на диск (правило №4).
- **КЕШ + ОКРЕМИЙ ПОТІК (hot-path).** `Platform::is_secure_field()` кличеться ЩОКРОКУ →
  лише читає атомік (`secure::cached_is_secure`, без блокувань). Перерахунок
  (`secure::recompute`) — РАЗ на зміну фокуса, на виділеному потоці `typofix-secure`
  (`SecureHandle`, поряд із `_hook` у `WindowsPlatform`). LL-hook потік (`hook.rs`)
  лишає за собою лише дешевий `EVENT_SYSTEM_FOREGROUND` → емісія `FocusChange` (для
  інвалідації буфера; її НЕ чіпати). Окремий потік уже не обов'язковий для суто
  нативної перевірки, але тримає LL-pump абсолютно дешевим і дає дебаунс.
- **WinEvent-хуки на потоці `secure_thread`:** `EVENT_SYSTEM_FOREGROUND` (зміна вікна)
  + `EVENT_OBJECT_FOCUS` (Tab між контролами в межах вікна, напр. у поле пароля). Обидві
  → лише (пере)зводять дебаунс-таймер. **Тепер це безпечно, бо перерахунок — дешевий
  нативний** (раніше OBJECT_FOCUS тригерив важкий UIA → лаг; UIA прибрано).
- **ДЕБАУНС:** `FOCUS_DEBOUNCE_MS=60мс`, NULL-window `SetTimer` → `WM_TIMER` у чергу
  потоку; перерахунок ОДИН раз після осідання фокуса. `KillTimer` обов'язковий на
  спрацюванні (SetTimer періодичний).
- **Нативна перевірка (`window::native_focus_is_secure`, дешево, без COM):** фокусний
  контрол (`window::foreground_focus_hwnd` = `GetForegroundWindow`→`GetGUIThreadInfo.
  hwndFocus`, M2-метод як у `layout.rs`) має **`ES_PASSWORD`** у `GWL_STYLE` АБО маскує
  ввід (**`EM_GETPASSWORDCHAR`≠0** через `SendMessageTimeoutW`, 40мс+`SMTO_ABORTIFHUNG`).
  🔴 **ГЕЙТ EDIT-КЛАСУ (фікс false-positive secure):** перевіряємо ЛИШЕ якщо клас
  edit-подібний (`class_is_edit_like`: `GetClassNameW` містить `"edit"` —
  `Edit`/`RichEdit*`/`RichEditD2DPT`…). Біт `0x20` на НЕ-edit вікні означає геть інше
  (BS_*/SS_*…), а `EM_GETPASSWORDCHAR` — невизначене повідомлення → був би хибний
  `secure=true`, що **глушив би ВСІ перемикання** у звичайному контролі.
- **Покриття:** ✅ нативні Win32-edit (логіни/UAC/інсталятори/десктоп) включно з Tab
  у межах вікна. ❌ браузер/Electron/веб-`<input type=password>` (windowless — потрібен
  UIA, свідомо НЕ робимо), ❌ WinRAR v7 (owner-draw, без жодної семантики пароля).
- **Тести:** `secure::cache_defaults_and_resets_to_not_secure` (дефолт/скид → false),
  `window::password_style_bit_is_parsed` (парсинг `ES_PASSWORD`-біта). Live-доказ
  нативного шляху — `examples/secure_synth.rs` (ES_PASSWORD-edit→TRUE, звичайний→false).
- `is_fullscreen` — best-effort, лише первинний монітор (follow-up: `MonitorFromWindow`).
