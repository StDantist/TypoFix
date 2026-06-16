# typofix-platform-windows — правила

Жива реалізація `trait Platform` поверх WinAPI. `unsafe` тут дозволений (на
відміну від core). Нижче — **неочевидне**, що легко зламати.

## Структура
- `keystate.rs`, `scancode.rs` — **чисті, без WinAPI**, компілюються й тестуються
  на будь-якій ОС. Уся логіка модифікаторів/класифікації — тут, щоб бути
  тестованою без живої системи. Не тягни сюди WinAPI.
- `layout.rs` — `ToUnicodeEx`-запит розкладки + `LayoutId`↔HKL (Windows).
- `window.rs` — активне вікно (`GetForegroundWindow`/`QueryFullProcessImageNameW`).
- `inject.rs` — `SendInput`/перемикання розкладки.
- `hook.rs` — LL-хуки + message-pump потік.
- `src/bin/live_spike.rs` — **РУЧНИЙ** харнес (див. нижче).
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

## Що перевірено автоматично (частина A, безпечно, без вводу в систему)
`cargo test -p typofix-platform-windows` (15 тестів) реально б'є по WinAPI:
- `ToUnicodeEx` через `LoadKeyboardLayoutW("00000409")`: a/A, 1/!, пробіл, Caps;
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

## Приватність / follow-up
- Натиски лише в RAM (канал mpsc), нічого на диск (правило №4).
- Password/secure-поля поки **не** детектуються (буферити їх не можна) — потрібен
  follow-up: UI Automation / `GetGUIThreadInfo`+`ES_PASSWORD`. Зараз структурно
  не позначаємо; це свідома прогалина, не забути перед релізом.
- `is_fullscreen` — best-effort, лише первинний монітор (follow-up: `MonitorFromWindow`).
