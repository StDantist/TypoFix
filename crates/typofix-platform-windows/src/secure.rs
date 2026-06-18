//! Детекція секретних (пароль) полів — **лише ДЕШЕВА НАТИВНА**, із кешуванням на
//! зміні фокуса.
//!
//! ## 🔴 БЕЗ UIA (назавжди прибрано з рантайму)
//! Раніше тут був UIA-фолбек (`IUIAutomation::GetFocusedElement` →
//! `UIA_IsPasswordPropertyId`). Його **повністю вилучено**: сам виклик UIA вмикає
//! accessibility-дерево в цільовому застосунку (IDE/Electron/Chromium) у ЙОГО
//! процесі → гальмує цільову апку незалежно від нашого потоку (репро власника: лаг
//! 30–40 с). Тепер `recompute` робить ЛИШЕ дешеву нативну перевірку
//! ([`crate::window::native_focus_is_secure`]: `GetWindowLongPtrW` біт `ES_PASSWORD`
//! та `EM_GETPASSWORDCHAR` лише на edit-класи) — читання з нашого боку, мікросекунди,
//! a11y цільового застосунку НЕ чіпає. Веб/Electron-поля свідомо НЕ покриваємо
//! (потрібен підхід без активації a11y — окреме майбутнє завдання; деталі — `CLAUDE.md`).
//!
//! ## Архітектура (hot-path!)
//! `Platform::is_secure_field()` кличеться ЩОКРОКУ (на кожне натискання) → читає
//! лише атомік [`CACHE`] (дешево, без блокувань). Перерахунок ([`recompute`])
//! обчислюється РАЗ на зміну фокуса на ВИДІЛЕНОМУ потоці ([`crate::secure_thread`])
//! з дебаунсом. (Виділений потік лишився з часів UIA; для суто нативної перевірки
//! він уже не обов'язковий, але зберігає LL-hook потік абсолютно дешевим і дає
//! дебаунс-коалесинг шторму фокус-подій.)

use std::sync::atomic::{AtomicBool, Ordering};

/// Кеш секретності поточного фокуса. Пише потік детекції (на зміні фокуса), читає
/// потік рушія (`is_secure_field`). Один процес → один екземпляр платформи.
static CACHE: AtomicBool = AtomicBool::new(false);

/// Прочитати закешовану секретність поточного фокуса (дешево, hot-path).
pub fn cached_is_secure() -> bool {
    CACHE.load(Ordering::Relaxed)
}

/// Перерахувати секретність поточного фокусного поля й оновити [`CACHE`].
/// Викликати на зміні фокуса (і раз на старті потоку детекції). ЛИШЕ нативна
/// перевірка — жодного UIA/COM (a11y цільової апки не чіпаємо).
pub fn recompute() {
    let secure = match crate::window::foreground_focus_hwnd() {
        Some(hwnd) => crate::window::native_focus_is_secure(hwnd),
        None => false,
    };
    CACHE.store(secure, Ordering::Relaxed);
}

/// Скинути кеш у «не секретне» (на старті потоку детекції — щоб не лишався стан із
/// попередньої сесії).
pub fn reset_cache() {
    CACHE.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Дефолт і скидання кешу — НЕ secure (fail-safe: хибний `secure=true` глушить
    /// УСІ перемикання, тож база й reset мусять бути `false`; `recompute` піднімає
    /// `true` лише для справжнього пароль-поля). Кеш процес-глобальний; цей тест
    /// єдиний його чіпає → без гонок із паралельними.
    #[test]
    fn cache_defaults_and_resets_to_not_secure() {
        // Старт сесії скидає кеш → не secure.
        reset_cache();
        assert!(!cached_is_secure(), "після reset кеш має бути НЕ secure");

        // Імітуємо «зайшли в поле пароля» → потім вихід (reset скидає назад).
        CACHE.store(true, Ordering::Relaxed);
        assert!(cached_is_secure());
        reset_cache();
        assert!(
            !cached_is_secure(),
            "вихід із поля (reset) мусить повернути НЕ secure"
        );
    }
}
