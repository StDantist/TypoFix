//! Чисті хелпери стану клавіатури — **без WinAPI**, тож тестуються на будь-якій
//! ОС.
//!
//! Дві задачі:
//! 1. Скласти наші [`Modifiers`] зі знімка натиснутих модифікаторів
//!    (`physical → logical`), коректно відділивши **AltGr** від `Ctrl+Alt`.
//! 2. Зібрати 256-байтовий масив `key state` для `ToUnicodeEx` із наших
//!    [`Modifiers`] (`logical → physical`), щоб детерміновано спитати ОС, який
//!    символ дасть клавіша. Працюємо з **переданим** буфером, ніколи не чіпаючи
//!    глобальний стан клавіатури (залізне правило про чистоту запиту розкладки).

use typofix_platform::Modifiers;

/// VK-коди, потрібні для побудови key-state (значення з WinAPI, але тут — прості
/// константи, щоб модуль лишався кросплатформним і тестованим).
pub const VK_SHIFT: usize = 0x10;
pub const VK_CONTROL: usize = 0x11;
pub const VK_MENU: usize = 0x12; // Alt
pub const VK_CAPITAL: usize = 0x14; // Caps Lock

/// Старший біт у key-state = «клавіша натиснута».
const KEY_DOWN: u8 = 0x80;
/// Молодший біт у key-state = «toggle увімкнено» (для Caps Lock).
const KEY_TOGGLED: u8 = 0x01;

/// Знімок фізичного стану модифікаторів у момент події (читається з ОС через
/// `GetAsyncKeyState`/`GetKeyState`, але сам тип — чисті дані).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModSnapshot {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Win / Cmd.
    pub meta: bool,
    /// Caps Lock **увімкнено** (toggle), не «натиснуто».
    pub caps: bool,
    /// AltGr активний (фізично правий Alt). На Windows він рапортується як
    /// `Ctrl+Alt`, тому виявляється окремо (за стан правого Alt).
    pub altgr: bool,
}

impl ModSnapshot {
    /// Звести знімок у наші [`Modifiers`].
    ///
    /// **Готча AltGr:** коли активний AltGr, Windows тримає піднятими і Ctrl, і
    /// Alt. Якби ми лишили `CTRL|ALT`, ядро вважало б це командною комбінацією
    /// (інвалідація буфера), а насправді AltGr **створює символ**. Тому при
    /// `altgr` ми ставимо лише `ALTGR`, прибираючи фантомні `CTRL`/`ALT`.
    pub fn to_modifiers(self) -> Modifiers {
        let mut m = Modifiers::empty();
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.caps {
            m |= Modifiers::CAPS;
        }
        if self.meta {
            m |= Modifiers::META;
        }
        if self.altgr {
            m |= Modifiers::ALTGR;
        } else {
            if self.ctrl {
                m |= Modifiers::CTRL;
            }
            if self.alt {
                m |= Modifiers::ALT;
            }
        }
        m
    }
}

/// Заповнити 256-байтовий key-state для `ToUnicodeEx` за нашими [`Modifiers`].
///
/// Буфер передається ззовні (зазвичай нульовий) — ми **не читаємо й не пишемо**
/// глобальний стан клавіатури ОС. AltGr моделюється як `Ctrl+Alt` (саме так його
/// очікує драйвер розкладки).
pub fn fill_key_state(buf: &mut [u8; 256], modifiers: Modifiers) {
    buf.fill(0);
    if modifiers.contains(Modifiers::SHIFT) {
        buf[VK_SHIFT] = KEY_DOWN;
    }
    if modifiers.contains(Modifiers::ALTGR) {
        // AltGr = Ctrl+Alt на рівні драйвера розкладки.
        buf[VK_CONTROL] = KEY_DOWN;
        buf[VK_MENU] = KEY_DOWN;
    }
    if modifiers.contains(Modifiers::CAPS) {
        buf[VK_CAPITAL] = KEY_TOGGLED;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_snapshot_is_empty() {
        assert_eq!(ModSnapshot::default().to_modifiers(), Modifiers::empty());
    }

    #[test]
    fn shift_and_caps_compose() {
        let s = ModSnapshot {
            shift: true,
            caps: true,
            ..Default::default()
        };
        assert_eq!(s.to_modifiers(), Modifiers::SHIFT | Modifiers::CAPS);
    }

    #[test]
    fn altgr_suppresses_phantom_ctrl_alt() {
        // AltGr фізично = Ctrl+Alt down; маємо отримати ЛИШЕ ALTGR.
        let s = ModSnapshot {
            ctrl: true,
            alt: true,
            altgr: true,
            ..Default::default()
        };
        let m = s.to_modifiers();
        assert_eq!(m, Modifiers::ALTGR);
        assert!(!m.contains(Modifiers::CTRL));
        assert!(!m.contains(Modifiers::ALT));
    }

    #[test]
    fn real_ctrl_alt_without_altgr_stays_command_combo() {
        let s = ModSnapshot {
            ctrl: true,
            alt: true,
            altgr: false,
            ..Default::default()
        };
        assert_eq!(s.to_modifiers(), Modifiers::CTRL | Modifiers::ALT);
    }

    #[test]
    fn fill_key_state_sets_only_requested_bits() {
        let mut buf = [0u8; 256];
        fill_key_state(&mut buf, Modifiers::SHIFT);
        assert_eq!(buf[VK_SHIFT], 0x80);
        assert_eq!(buf[VK_CONTROL], 0);
        assert_eq!(buf[VK_MENU], 0);

        fill_key_state(&mut buf, Modifiers::ALTGR);
        assert_eq!(buf[VK_SHIFT], 0, "буфер очищається перед заповненням");
        assert_eq!(buf[VK_CONTROL], 0x80);
        assert_eq!(buf[VK_MENU], 0x80);

        fill_key_state(&mut buf, Modifiers::CAPS);
        assert_eq!(buf[VK_CAPITAL], 0x01, "Caps — це toggle-біт (0x01)");
    }
}
