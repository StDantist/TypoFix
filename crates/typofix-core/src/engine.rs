//! Оркестрація кроку рішення: buffer → detector → replacer.
//!
//! Тут вирішується, коли штовхати страйк у буфер, коли інвалідувати його, а
//! коли (на межі слова) запускати детектор і будувати план перенабору.

use crate::{detector, replacer, Context, EngineState, KeyStroke, Layout};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers, WindowInfo};

/// Структурні клавіші — межа слова незалежно від розкладки (scancode set 1).
const SC_SPACE: u32 = 0x39;
const SC_ENTER: u32 = 0x1C;
const SC_TAB: u32 = 0x0F;
/// Backspace (scancode set 1) — сигнал негайного відкидання перенабору.
const SC_BACKSPACE: u32 = 0x0E;

/// Останній автоперенабір, що очікує можливого негайного відкидання.
///
/// Зберігається в [`EngineState`] між кроками: якщо НАСТУПНА реальна клавіша —
/// Backspace у тому ж вікні, вважаємо, що користувач відкинув перенабір.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRetype {
    /// Вікно, в якому стався перенабір.
    pub(crate) window_key: String,
    /// Оригінальне слово (те, що було на екрані до виправлення).
    pub(crate) original_word: String,
}

/// Класифікація натискання щодо слова.
enum Class {
    /// Літера/апостроф — частина слова.
    Word,
    /// Пробіл/Enter/Tab/пунктуація/цифра — межа слова.
    Boundary,
}

/// Друкований символ роздільника межі слова, який на реальній ОС УЖЕ опинився на
/// екрані перед курсором (хук пропустив натиск далі) у момент перенабору.
///
/// Структурні клавіші мають фіксований символ (пробіл/`\n`/`\t`); пунктуація/
/// цифра — символ у поточній розкладці. Недруковані тригери межі (F-клавіші,
/// Delete: `char_at` → `None`) повертають `None` — на екрані нічого не з'явилось.
fn separator_char(stroke: KeyStroke, current_layout: Option<&Layout>) -> Option<char> {
    match stroke.scancode {
        SC_SPACE => Some(' '),
        SC_ENTER => Some('\n'),
        SC_TAB => Some('\t'),
        _ => current_layout.and_then(|l| l.char_at(stroke.scancode, stroke.modifiers)),
    }
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
            // Невідома клавіша без символу (F-клавіші, Delete тощо) → межа
            // (безпечно завершує слово). Backspace сюди не доходить — його
            // перехоплює окрема гілка в `step` (поп/інвалідація).
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

/// Чи подія — сигнал відкидання щойно зробленого перенабору: реальний (не
/// синтетичний) натиск Backspace у тому самому вікні.
fn is_rejection(ev: &InputEvent, wkey: &str, pending: &PendingRetype) -> bool {
    if pending.window_key != wkey {
        return false;
    }
    matches!(
        ev,
        InputEvent::Key(k)
            if k.scancode == SC_BACKSPACE
                && k.dir == KeyDir::Down
                && !k.is_synthetic
    )
}

/// Обробити подію межі слова: запустити детектор (із learned-veto) і, за
/// рішенням, повернути план + зафіксувати очікування можливого відкидання.
///
/// `separator` — друкований символ роздільника, що тригернув межу (вже на екрані
/// на реальній ОС); передається в [`replacer::plan`] для коректного стирання.
fn handle_boundary(
    state: &mut EngineState,
    wkey: &str,
    ctx: &Context,
    separator: Option<char>,
) -> Vec<Action> {
    if state.buffers.for_window(wkey).is_empty() {
        return Vec::new();
    }
    // Клонуємо страйки, щоб звільнити позику буфера (далі чіпаємо learned/pending).
    let strokes: Vec<KeyStroke> = state.buffers.for_window(wkey).strokes().to_vec();
    let mut decision = detector::decide(&strokes, ctx);

    // Самонавчений veto: слово, яке користувач уже відкидав, не перемикаємо.
    if decision.switch && state.learned.contains(&decision.current_text) {
        decision.switch = false;
    }

    let actions = replacer::plan(&decision, separator);
    if decision.switch {
        // Відкриваємо коротке вікно очікування відкидання цього перенабору.
        state.pending_retype = Some(PendingRetype {
            window_key: wkey.to_string(),
            original_word: decision.current_text,
        });
    }
    state.buffers.for_window(wkey).reset();
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

    // Самонавчання: чи це відкидання щойно зробленого перенабору?
    if let Some(pending) = state.pending_retype.as_ref() {
        if is_rejection(&ev, &wkey, pending) {
            let word = state.pending_retype.take().unwrap().original_word;
            state.learned.learn(&word);
            // Емітимо дію для app-шару (персистенція); core сам не персистить.
            return vec![Action::CommitException(word)];
        }
        // Будь-яка інша подія закриває вікно undo — користувач прийняв перенабір.
        state.pending_retype = None;
    }

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
    // (Сюди ж потрапляє Ctrl+Backspace — видалення цілого слова → інвалідація.)
    if key.is_autorepeat || is_command_combo(key.modifiers) {
        state.buffers.invalidate_window(&wkey);
        return Vec::new();
    }

    // Backspace усередині редагування слова (пріоритет: rejection-сигнал вище
    // вже оброблено, тож сюди доходить лише «звичайний» Backspace):
    //  - буфер непорожній → ПОП останнього страйка (слово ще когерентне);
    //  - буфер порожній → стирання у попереднє слово, синхрон утрачено →
    //    ІНВАЛІДАЦІЯ (наступне слово не змішується з попереднім).
    if key.scancode == SC_BACKSPACE {
        let buf = state.buffers.for_window(&wkey);
        if buf.is_empty() {
            buf.invalidate();
        } else {
            buf.pop();
        }
        return Vec::new();
    }

    let stroke = KeyStroke::from(&key);
    let current_layout = ctx.current_profile().map(|p| &p.layout);

    match classify(stroke, current_layout) {
        Class::Boundary => {
            let separator = separator_char(stroke, current_layout);
            handle_boundary(state, &wkey, ctx, separator)
        }
        Class::Word => {
            state.buffers.for_window(&wkey).push(stroke);
            Vec::new()
        }
    }
}
