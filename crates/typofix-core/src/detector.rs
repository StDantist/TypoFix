//! Поєднує lm+dict+mapper: для слова рахує найкращу мову/розкладку і
//! `confidence`; вирішує, чи перемикати, за `threshold(len)`. §3.3.

// TODO(phase-2): detect(word_scancodes, ctx) -> Option<(LayoutId, confidence)>.
