//! # typofix-platform-macos
//!
//! Реалізація [`typofix_platform::Platform`] для macOS: `CGEventTap` (+
//! Accessibility), `CGEventPost`, `TISSelectInputSource`, `UCKeyTranslate`.
//! Деталі й підводні камені — `docs/ARCHITECTURE.md` §4.
//!
//! Поки порожня заглушка — реальний CoreGraphics-код додається в наступних
//! фазах (за `#[cfg(target_os = "macos")]`).

// TODO(phase-1): struct MacosPlatform + impl Platform.
