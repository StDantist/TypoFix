//! Чисті хелпери класифікації клавіш — **без WinAPI**, тестуються будь-де.
//!
//! Перетворюють VK-код у логічний намір для ядра: навігаційні клавіші →
//! [`KeyKind::CaretMove`] (інвалідація буфера), решта → [`KeyKind::Text`]
//! (звичайний страйк). Scancode уже приходить як **set 1** прямо з
//! `KBDLLHOOKSTRUCT.scanCode` (фізична позиція), тож додаткового мапінгу не
//! потребує — узгоджено з `data/layouts/*.toml` і `core::layout_mapper`.

/// Як ядро має трактувати клавішу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// Звичайна текстова/символьна клавіша — піде як `InputEvent::Key`.
    Text,
    /// Навігація курсором (стрілки/Home/End/PageUp-Down) — розриває звʼязок
    /// буфера з текстом перед курсором → `InputEvent::CaretMove`.
    CaretMove,
}

// VK-коди навігації (значення WinAPI; тут — прості константи).
const VK_PRIOR: u32 = 0x21; // PageUp
const VK_NEXT: u32 = 0x22; // PageDown
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;

/// Класифікувати клавішу за її VK-кодом.
///
/// Навігація йде окремо, бо `caret` міг переміститись, і буфер слова більше не
/// відповідає тексту перед курсором (див. залізне правило №3 про інвалідацію).
pub fn classify_vk(vk: u32) -> KeyKind {
    match vk {
        VK_PRIOR | VK_NEXT | VK_END | VK_HOME | VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN => {
            KeyKind::CaretMove
        }
        _ => KeyKind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_navigation_are_caret_moves() {
        for vk in [
            VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_HOME, VK_END, VK_PRIOR, VK_NEXT,
        ] {
            assert_eq!(classify_vk(vk), KeyKind::CaretMove, "vk {vk:#x}");
        }
    }

    #[test]
    fn letters_and_space_are_text() {
        // 'A' = 0x41, '0' = 0x30, Space = 0x20 (VK_SPACE).
        for vk in [0x41u32, 0x30, 0x20, 0x42] {
            assert_eq!(classify_vk(vk), KeyKind::Text, "vk {vk:#x}");
        }
    }
}
