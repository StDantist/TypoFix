//! Per-window буфер натискань поточного «слова».
//!
//! Накопичує layout-незалежні [`KeyStroke`] поточного слова, щоб детектор міг
//! прочитати їх у різних розкладках. **Критично:** буфер відображає текст
//! безпосередньо *перед курсором*; будь-що, що розриває цей зв'язок (клік,
//! навігація, зміна фокуса, auto-repeat, командні сполучення), **мусить
//! відкинути буфер** — інакше `DeleteChars(n)` зітре чужий текст (§3.4).
//!
//! Тут лише структура зберігання; коли саме штовхати/скидати/інвалідувати —
//! вирішує [`crate::engine`].

use std::collections::HashMap;

use crate::KeyStroke;

/// Буфер натискань одного слова (для одного вікна).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WordBuffer {
    strokes: Vec<KeyStroke>,
}

impl WordBuffer {
    /// Додати натискання у кінець слова.
    pub fn push(&mut self, stroke: KeyStroke) {
        self.strokes.push(stroke);
    }

    /// Скинути буфер на межі слова (пробіл/Enter/пунктуація) — нормальне
    /// завершення слова.
    pub fn reset(&mut self) {
        self.strokes.clear();
    }

    /// **Інвалідувати** буфер: зв'язок із текстом перед курсором розірвано
    /// (клік/навігація/фокус/auto-repeat/командна комбінація). Семантично — те
    /// саме очищення, що й [`reset`](Self::reset), але назва підкреслює намір:
    /// після цього перенабирати **не можна**.
    pub fn invalidate(&mut self) {
        self.strokes.clear();
    }

    /// Поточні натискання слова.
    pub fn strokes(&self) -> &[KeyStroke] {
        &self.strokes
    }

    /// Чи буфер порожній.
    pub fn is_empty(&self) -> bool {
        self.strokes.is_empty()
    }

    /// Кількість накопичених натискань.
    pub fn len(&self) -> usize {
        self.strokes.len()
    }
}

/// Сховище буферів **per-window**: буфер слова прив'язаний до конкретного вікна,
/// бо фокус між вікнами розриває зв'язок із текстом.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BufferStore {
    per_window: HashMap<String, WordBuffer>,
}

impl BufferStore {
    /// Отримати (створивши за потреби) буфер для вікна за його ключем.
    pub fn for_window(&mut self, key: &str) -> &mut WordBuffer {
        self.per_window.entry(key.to_string()).or_default()
    }

    /// Інвалідувати буфер конкретного вікна (якщо існує).
    pub fn invalidate_window(&mut self, key: &str) {
        if let Some(buf) = self.per_window.get_mut(key) {
            buf.invalidate();
        }
    }

    /// Кількість вікон, для яких є буфер (для тестів/дебагу).
    pub fn tracked_windows(&self) -> usize {
        self.per_window.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Modifiers;

    fn stroke(sc: u32) -> KeyStroke {
        KeyStroke::new(sc, Modifiers::empty())
    }

    #[test]
    fn accumulates_and_resets() {
        let mut b = WordBuffer::default();
        b.push(stroke(0x22));
        b.push(stroke(0x23));
        assert_eq!(b.len(), 2);
        b.reset();
        assert!(b.is_empty());
    }

    #[test]
    fn invalidate_clears() {
        let mut b = WordBuffer::default();
        b.push(stroke(0x22));
        b.invalidate();
        assert!(b.is_empty());
    }

    #[test]
    fn store_is_per_window() {
        let mut s = BufferStore::default();
        s.for_window("a.exe").push(stroke(0x22));
        s.for_window("b.exe").push(stroke(0x23));
        assert_eq!(s.tracked_windows(), 2);
        // Інвалідація одного вікна не чіпає інше.
        s.invalidate_window("a.exe");
        assert!(s.for_window("a.exe").is_empty());
        assert_eq!(s.for_window("b.exe").len(), 1);
    }
}
