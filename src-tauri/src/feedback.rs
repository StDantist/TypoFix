//! Звуковий зворотний зв'язок (B2): короткий «блип» при успішному авто-перенаборі.
//!
//! Викликається з ДВИГУНОВОГО потоку (`runtime::engine_loop`) лише коли:
//! - користувач увімкнув `feedback.sound_on_switch` у конфігу, І
//! - крок ядра справді зробив перенабір (`SwitchLayout`+`TypeUnicode`).
//!
//! Анти-цикл: грає лише на НАШ перенабір (не на синтетичний ввід — той не
//! породжує switch-крок), і ніколи на паузі (на паузі потік не крутиться).
//!
//! **Не блокує hot-path:** `PlaySoundW` з `SND_ASYNC` повертається миттєво, звук
//! грає у фоновому потоці winmm. Wav вбудовано (`include_bytes!`) і граємо з
//! пам'яті (`SND_MEMORY`) — без файлового IO в рантаймі.

/// Програти звук перемикання (неблокуюче). На не-Windows — заглушка (порт згодом).
#[cfg(windows)]
pub fn play_switch_sound() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    // Короткий вбудований wav (≈4.4 КБ, 22 кГц моно). SND_MEMORY → перший аргумент
    // вказує на байти wav у пам'яті (не на ім'я файлу), тож кастимо до PCWSTR.
    const WAV: &[u8] = include_bytes!("../assets/switch.wav");
    // SAFETY: WAV — статичні валідні wav-байти; SND_MEMORY означає, що pszSound —
    // вказівник на ці дані, hmod ігнорується. SND_ASYNC не блокує; SND_NODEFAULT —
    // тиша (а не системний біп) при будь-якій помилці відтворення.
    unsafe {
        PlaySoundW(
            WAV.as_ptr().cast::<u16>(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// Заглушка для не-Windows (звук поки лише на Windows; macOS — згодом).
#[cfg(not(windows))]
pub fn play_switch_sound() {}
