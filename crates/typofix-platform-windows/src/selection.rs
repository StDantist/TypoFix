//! Примітив «узяти виділений текст» через синтетичний Ctrl+C + буфер обміну.
//!
//! Потрібен для гарячих клавіш B1 (змінити регістр / перемкнути виділення).
//! Працює так: зберігаємо поточний буфер обміну → шлемо Ctrl+C (нашою
//! SendInput-машинерією з [`INJECT_SIGNATURE`], тож хук не реагує) → чекаємо
//! оновлення буфера за `GetClipboardSequenceNumber` → читаємо `CF_UNICODETEXT`
//! → **відновлюємо** попередній буфер. Користувач не має втратити свій
//! clipboard (правило приватності №4).
//!
//! **Приватність:** уся робота — лише в RAM, нічого на диск, нічого не логуємо.

use std::ptr;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

use crate::inject;

/// `CF_UNICODETEXT` (UTF-16, NUL-термінований). Константа стабільна; не тягнемо
/// зайвий модуль заради неї.
const CF_UNICODETEXT: u32 = 13;

/// Скільки разів пробуємо `OpenClipboard` (його може тримати інший процес).
const OPEN_RETRIES: u32 = 10;
/// Пауза між спробами `OpenClipboard`.
const OPEN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(5);

/// Максимум очікування оновлення буфера після Ctrl+C.
const COPY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);
/// Крок опитування `GetClipboardSequenceNumber`.
const COPY_POLL_STEP: std::time::Duration = std::time::Duration::from_millis(10);

/// Знімок одного формату буфера обміну: код формату + копія його global-памʼяті.
/// Памʼять належить нам, поки не передамо її системі через `SetClipboardData`
/// (тоді власність переходить до ОС) або не звільнимо.
struct SavedFormat {
    format: u32,
    /// Власна HGLOBAL-копія даних. `null`, якщо вже передана системі.
    handle: HGLOBAL,
}

/// Узяти текст поточного виділення активного вікна, не зруйнувавши буфер обміну.
///
/// Повертає `None`, якщо виділення порожнє / не текст / буфер не оновився за
/// таймаут. Жодних побічних ефектів для clipboard у разі `None` (нічого не
/// перезаписуємо — отже й нічого відновлювати).
pub fn get_selection_text() -> Option<String> {
    // 1. Знімок поточного буфера (щоб відновити після читання).
    let saved = save_clipboard();
    // 2. Запамʼятати лічильник до копіювання — за ним ловимо факт оновлення.
    let seq_before = unsafe { GetClipboardSequenceNumber() };

    // 3. Синтетичний Ctrl+C (підписаний, хук його ігнорує).
    inject::send_ctrl_c();

    // 4. Дочекатись, поки буфер реально оновиться (інакше прочитаємо старе).
    if !wait_for_clipboard_change(seq_before) {
        // Нічого не скопійовано (порожнє виділення / нетекстовий контекст /
        // таймаут). Буфер ми не чіпали — відновлювати нічого, копії звільняємо.
        free_saved(saved);
        return None;
    }

    // 5. Прочитати свіжий текст.
    let text = read_unicode_text();

    // 6. Відновити попередній буфер (його ми щойно перезаписали через Ctrl+C).
    restore_clipboard(saved);

    // Порожній рядок трактуємо як «немає виділення».
    text.filter(|s| !s.is_empty())
}

/// Відкрити буфер з кількома спробами (його тимчасово може тримати інший процес).
fn open_clipboard() -> bool {
    for _ in 0..OPEN_RETRIES {
        if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
            return true;
        }
        std::thread::sleep(OPEN_RETRY_DELAY);
    }
    false
}

/// Зняти повний знімок буфера: усі формати, що тримаються у global-памʼяті.
///
/// Handle-формати (CF_BITMAP тощо) і delayed-rendering (`GetClipboardData ==
/// null`) пропускаємо — їх не дублюємо через `GlobalSize`/`GlobalLock`.
fn save_clipboard() -> Vec<SavedFormat> {
    let mut saved = Vec::new();
    if !open_clipboard() {
        return saved;
    }
    unsafe {
        let mut format = EnumClipboardFormats(0);
        while format != 0 {
            if is_global_format(format) {
                if let Some(handle) = clone_global(GetClipboardData(format)) {
                    saved.push(SavedFormat { format, handle });
                }
            }
            format = EnumClipboardFormats(format);
        }
        CloseClipboard();
    }
    saved
}

/// Чи зберігається формат у global-памʼяті (можна `GlobalSize`/`GlobalLock`).
///
/// Предефайнені формати на основі GDI/handle (CF_BITMAP, CF_PALETTE,
/// CF_METAFILEPICT, CF_ENHMETAFILE, їх DSP-варіанти, CF_OWNERDISPLAY) НЕ є
/// global-памʼяттю — `GlobalSize` на них = UB/пошкодження купи. Їх пропускаємо
/// (не дублюємо й не відновлюємо). Усе інше (текст, CF_DIB/DIBV5, CF_HDROP,
/// CF_LOCALE, зареєстровані `>= 0xC000`) — global.
fn is_global_format(format: u32) -> bool {
    !matches!(
        format,
        2      // CF_BITMAP
        | 3    // CF_METAFILEPICT
        | 9    // CF_PALETTE
        | 14   // CF_ENHMETAFILE
        | 0x80 // CF_OWNERDISPLAY
        | 0x82 // CF_DSPBITMAP
        | 0x83 // CF_DSPMETAFILEPICT
        | 0x8E // CF_DSPENHMETAFILE
    )
}

/// Зробити власну global-копію даних формату. `None`, якщо handle null або це
/// не global-памʼять (`GlobalSize == 0`).
unsafe fn clone_global(src: HANDLE) -> Option<HGLOBAL> {
    if src.is_null() {
        return None;
    }
    let size = GlobalSize(src);
    if size == 0 {
        return None;
    }
    let src_ptr = GlobalLock(src);
    if src_ptr.is_null() {
        return None;
    }
    let dst = GlobalAlloc(GMEM_MOVEABLE, size);
    if dst.is_null() {
        GlobalUnlock(src);
        return None;
    }
    let dst_ptr = GlobalLock(dst);
    if dst_ptr.is_null() {
        GlobalUnlock(src);
        return None;
    }
    ptr::copy_nonoverlapping(src_ptr as *const u8, dst_ptr as *mut u8, size);
    GlobalUnlock(dst);
    GlobalUnlock(src);
    Some(dst)
}

/// Опитувати лічильник буфера, поки він не зміниться або не вийде таймаут.
fn wait_for_clipboard_change(seq_before: u32) -> bool {
    let deadline = std::time::Instant::now() + COPY_TIMEOUT;
    loop {
        if unsafe { GetClipboardSequenceNumber() } != seq_before {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(COPY_POLL_STEP);
    }
}

/// Прочитати `CF_UNICODETEXT` з буфера як `String`. `None`, якщо формату немає.
fn read_unicode_text() -> Option<String> {
    if !open_clipboard() {
        return None;
    }
    let result = unsafe {
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            None
        } else {
            read_wide_from_global(handle)
        }
    };
    unsafe { CloseClipboard() };
    result
}

/// Прочитати NUL-термінований UTF-16 рядок із global-памʼяті.
unsafe fn read_wide_from_global(handle: HANDLE) -> Option<String> {
    let ptr = GlobalLock(handle) as *const u16;
    if ptr.is_null() {
        return None;
    }
    // Довжина — до першого NUL (буфер CF_UNICODETEXT завжди NUL-термінований).
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    let text = String::from_utf16_lossy(slice);
    GlobalUnlock(handle);
    Some(text)
}

/// Відновити буфер зі знімка: очистити й повернути кожен збережений формат.
/// `SetClipboardData` передає власність памʼяті системі — звідти ми її не чіпаємо.
fn restore_clipboard(mut saved: Vec<SavedFormat>) {
    if saved.is_empty() {
        return;
    }
    if !open_clipboard() {
        // Не змогли відкрити — звільняємо копії, щоб не текла памʼять.
        free_saved(saved);
        return;
    }
    unsafe {
        EmptyClipboard();
        for item in &mut saved {
            if SetClipboardData(item.format, item.handle).is_null() {
                // Не прийнято системою — памʼять усе ще наша, звільнимо нижче.
            } else {
                // Власність перейшла до ОС — більше не звільняємо.
                item.handle = ptr::null_mut();
            }
        }
        CloseClipboard();
    }
    free_saved(saved);
}

/// Звільнити global-копії, власність яких лишилась за нами (не передана системі).
fn free_saved(saved: Vec<SavedFormat>) {
    for item in saved {
        if !item.handle.is_null() {
            // handle ще не залочений (баланс lock/unlock у clone_global) → вільний.
            unsafe { GlobalFree(item.handle) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless-перевірка (в одному тесті — буфер обміну процес-глобальний, тож
    /// два паралельні clipboard-тести гонилися б за ним). Перевіряємо, що:
    /// знімок/звільнення безпечні; `get_selection_text` не панікує й повертає
    /// `Option`; текстовий вміст буфера лишається незмінним (відновлення
    /// працює). Реальний Ctrl+C-раунд-тріп — вручну на живій ОС.
    #[test]
    fn headless_safety_and_clipboard_preserved() {
        // Знімок + звільнення поточного буфера (порожнього чи ні) не падають.
        free_saved(save_clipboard());

        let before = read_unicode_text();
        let _ = get_selection_text();
        let after = read_unicode_text();
        // Текстовий вміст буфера має лишитись таким самим.
        assert_eq!(before, after);
    }
}
