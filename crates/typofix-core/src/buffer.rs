//! Per-window буфер натисків поточного «слова» (scancodes + регістр +
//! модифікатори). Скидається на межі слова та інвалідується на будь-якій
//! нелінійній події (клік, навігація, зміна фокуса) — інакше `DeleteChars(n)`
//! зітре чужий текст. Деталі — `docs/ARCHITECTURE.md` §3.2 і §3.4.

// TODO(phase-1): WordBuffer { scancodes, per-window key }, push/reset/invalidate.
