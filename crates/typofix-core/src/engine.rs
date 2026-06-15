//! Оркестрація кроку рішення: buffer → detector → replacer.
//!
//! Тут вирішується, коли штовхати страйк у буфер, коли інвалідувати його, а
//! коли (на межі слова) запускати детектор і будувати план перенабору.

use crate::{buffer::BufferStore, detector, replacer, Context, EngineState, KeyStroke, Layout};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers, WindowInfo};

/// Структурні клавіші — межа слова незалежно від розкладки (scancode set 1).
const SC_SPACE: u32 = 0x39;
const SC_ENTER: u32 = 0x1C;
const SC_TAB: u32 = 0x0F;

/// Класифікація натискання щодо слова.
enum Class {
    /// Літера/апостроф — частина слова.
    Word,
    /// Пробіл/Enter/Tab/пунктуація/цифра — межа слова.
    Boundary,
}

/// Чи модифікатори означають **командну** комбінацію (Ctrl/Alt/Win — шорткат,
/// не ввід тексту). AltGr — виняток: він *створює* символи третього рівня.
fn is_command_combo(m: Modifiers) -> bool {
    (m.contains(Modifiers::CTRL) || m.contains(Modifiers::ALT) || m.contains(Modifiers::META))
        && !m.contains(Modifiers::ALTGR)
}

/// Класифікувати натискання. Структурні клавіші — завжди межа; інші — за
/// символом у поточній розкладці (літера/апостроф → слово, решта → межа). Без
/// поточної розкладки нелітерні структурні все одно ловляться, а решта
/// накопичується (детектор без поточного профілю однаково не перемкне).
fn classify(stroke: KeyStroke, current_layout: Option<&Layout>) -> Class {
    if matches!(stroke.scancode, SC_SPACE | SC_ENTER | SC_TAB) {
        return Class::Boundary;
    }
    match current_layout.and_then(|l| l.char_at(stroke.scancode, stroke.modifiers)) {
        Some(ch) if ch.is_alphabetic() || ch == '\'' || ch == '’' => Class::Word,
        Some(_) => Class::Boundary, // цифра/пунктуація
        None => {
            // Невідома клавіша без розкладки: F-клавіші, Backspace тощо мапи не
            // мають → вважаємо межею (безпечно завершує слово).
            Class::Boundary
        }
    }
}

/// Ключ вікна для per-window буфера: повний шлях до exe, інакше ім'я процесу.
fn window_key(w: &WindowInfo) -> String {
    if !w.exe_path.is_empty() {
        w.exe_path.clone()
    } else {
        w.process_name.clone()
    }
}

/// Обробити подію межі слова: запустити детектор і, за рішенням, повернути план.
fn handle_boundary(buffers: &mut BufferStore, key: &str, ctx: &Context) -> Vec<Action> {
    let actions = {
        let buf = buffers.for_window(key);
        if buf.is_empty() {
            Vec::new()
        } else {
            let decision = detector::decide(buf.strokes(), ctx);
            replacer::plan(&decision)
        }
    };
    buffers.for_window(key).reset();
    actions
}

/// Внутрішня реалізація кроку (див. [`crate::step`]).
pub fn step(state: &mut EngineState, ev: InputEvent, ctx: &Context) -> Vec<Action> {
    // Виключене вікно (застосунок/папка) — повний bypass: не буферимо й не
    // перемикаємо. Перевіряємо ПЕРЕД detector (порядок: bypass → veto).
    if ctx.is_window_excluded() {
        return Vec::new();
    }

    let wkey = window_key(&ctx.active_window);

    let key: KeyEvent = match ev {
        InputEvent::Key(k) => k,
        // Нелінійні події рвуть зв'язок буфера з текстом → інвалідувати.
        InputEvent::MouseClick | InputEvent::CaretMove | InputEvent::FocusChange(_) => {
            state.buffers.invalidate_window(&wkey);
            return Vec::new();
        }
    };

    // Наші власні ін'єкції ігноруємо (без циклу перенабору).
    if key.is_synthetic {
        return Vec::new();
    }
    // Реагуємо лише на натиск (Up не несе тексту).
    if key.dir != KeyDir::Down {
        return Vec::new();
    }
    // Auto-repeat і командні комбінації рвуть надійність буфера → інвалідувати.
    if key.is_autorepeat || is_command_combo(key.modifiers) {
        state.buffers.invalidate_window(&wkey);
        return Vec::new();
    }

    let stroke = KeyStroke::from(&key);
    let current_layout = ctx.current_profile().map(|p| &p.layout);

    match classify(stroke, current_layout) {
        Class::Boundary => handle_boundary(&mut state.buffers, &wkey, ctx),
        Class::Word => {
            state.buffers.for_window(&wkey).push(stroke);
            Vec::new()
        }
    }
}
