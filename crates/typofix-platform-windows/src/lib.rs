//! # typofix-platform-windows
//!
//! Реалізація [`typofix_platform::Platform`] для Windows: `WH_KEYBOARD_LL`
//! хук, `SendInput`, `ActivateKeyboardLayout`, `ToUnicodeEx`. Деталі й
//! підводні камені — `docs/ARCHITECTURE.md` §4.
//!
//! Поки порожня заглушка — реальний WinAPI-код додається в наступних фазах
//! (за `#[cfg(target_os = "windows")]`).

// TODO(phase-1): struct WindowsPlatform + impl Platform.
