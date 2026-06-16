//! Виконання [`Action`] через `SendInput` + перемикання розкладки.
//!
//! **Залізне правило №2:** перенабір — лише готовими символами через
//! `KEYEVENTF_UNICODE`, НІКОЛИ не повтор scancode у новій розкладці (інакше
//! гонка з асинхронним перемиканням).
//!
//! Кожна наша ін'єкція несе підпис [`INJECT_SIGNATURE`] у `dwExtraInfo` — хук за
//! ним (і за `LLKHF_INJECTED`) розпізнає власний ввід і ставить `is_synthetic`,
//! щоб не зациклитись на власному SendInput.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    KEYEVENTF_UNICODE, VK_BACK,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, PostMessageW, WM_INPUTLANGCHANGEREQUEST,
};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL;

/// Підпис власного синтетичного вводу в `dwExtraInfo` («TPFx»). Дозволяє точно
/// відрізнити НАШ перенабір від чужих ін'єкцій (макроси, інші утиліти).
pub const INJECT_SIGNATURE: usize = 0x5450_4678;

/// Backspace scancode (set 1).
const SCANCODE_BACKSPACE: u16 = 0x0E;

/// Зібрати один keyboard-`INPUT`.
fn kbd_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECT_SIGNATURE,
            },
        },
    }
}

/// Відправити пакет подій одним викликом (атомарно щодо черги вводу).
fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Набрати готовий Unicode-текст (down+up на кожен UTF-16 code unit).
///
/// Сурогатні пари (емодзі тощо) проходять як два code unit — Windows збирає їх
/// назад. `wVk = 0`, `wScan` = code unit, прапор `KEYEVENTF_UNICODE`.
pub fn type_unicode(text: &str) {
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        inputs.push(kbd_input(0, unit, KEYEVENTF_UNICODE));
        inputs.push(kbd_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
    send(&inputs);
}

/// Стерти `n` символів перед курсором (Backspace × n) фізичним scancode.
pub fn delete_chars(n: u32) {
    let mut inputs = Vec::with_capacity(n as usize * 2);
    for _ in 0..n {
        inputs.push(kbd_input(VK_BACK, SCANCODE_BACKSPACE, KEYEVENTF_SCANCODE));
        inputs.push(kbd_input(
            VK_BACK,
            SCANCODE_BACKSPACE,
            KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP,
        ));
    }
    send(&inputs);
}

/// Попросити вікно на передньому плані перемкнути активну розкладку на `hkl`.
///
/// `WM_INPUTLANGCHANGEREQUEST` адресуємо самому застосунку (його потоку), а не
/// нашому — `ActivateKeyboardLayout` змінив би лише наш потік. Асинхронно: саме
/// тому перенабраний текст іде окремо як готовий Unicode (правило №2).
pub fn switch_layout(hkl: HKL) {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return;
        }
        PostMessageW(hwnd, WM_INPUTLANGCHANGEREQUEST, 0, hkl as isize);
    }
}
