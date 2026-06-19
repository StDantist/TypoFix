//! Оркестрація кроку рішення: buffer → detector → replacer.
//!
//! Тут вирішується, коли штовхати страйк у буфер, коли інвалідувати його, а
//! коли (на межі слова) запускати детектор і будувати план перенабору.

use crate::{detector, replacer, Context, EngineState, KeyStroke, Layout, LayoutId};
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
/// Backspace у тому ж вікні, вважаємо, що користувач відкинув перенабір
/// (див. [`step`]). Несе також усе потрібне для ЯВНОГО ручного відкату гарячою
/// клавішею ([`revert_last`]): скільки символів зараз на екрані від перенабору,
/// повний оригінальний текст для відновлення і стару розкладку.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRetype {
    /// Вікно, в якому стався перенабір.
    pub(crate) window_key: String,
    /// Оригінальне СЛОВО (без суфікса/роздільника) — для learned-veto й навчання.
    pub(crate) original_word: String,
    /// Скільки СИМВОЛІВ перенабору зараз на екрані (`best_text` + суфікс +
    /// роздільник) — рівно стільки треба стерти при відкаті.
    pub(crate) retyped_len: u32,
    /// Повний оригінальний текст для відновлення на екрані: слово РАЗОМ із
    /// хвостовим суфіксом і друкованим роздільником (те, що було перед перенабором).
    pub(crate) original_full: String,
    /// Розкладка, на яку треба повернутись при відкаті (стара, до перемикання).
    /// `None` — корекція без зміни розкладки (caps-only) → `SwitchLayout` не треба.
    pub(crate) restore_layout: Option<LayoutId>,
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

/// Класифікувати натискання. Структурні клавіші (пробіл/Enter/Tab) — завжди межа.
///
/// **Готча — пунктуація-що-є-літерою-в-кандидаті.** Межу НЕ можна визначати лише
/// за поточною розкладкою: клавіша `,` (`0x33`) в `en` — пунктуація, але в `uk` —
/// літера `б`. Якби ми вважали її твердою межею, буфер рвався б посеред слова
/// (`lj,ht`→`добре` неможливо було б розпізнати). Тому страйк — частина слова,
/// якщо він літера хоч у ОДНІЙ увімкненій розкладці; твердою межею лишаються лише
/// справжні роздільники й символи, що НЕ літера в жодній розкладці (цифри/`-`/`=`
/// тощо). Дизамбігуацію «літера чи хвостовий роздільник» робить уже детектор на
/// справжній межі (див. [`detector::decide`]).
fn classify(stroke: KeyStroke, ctx: &Context) -> Class {
    if matches!(stroke.scancode, SC_SPACE | SC_ENTER | SC_TAB) {
        return Class::Boundary;
    }
    if detector::letter_in_any_layout(stroke, ctx) {
        Class::Word
    } else {
        // Цифра/символ, що не літера в жодній розкладці, або невідома клавіша
        // (F-клавіші, Delete) → межа, що безпечно завершує слово. Backspace сюди
        // не доходить — його перехоплює окрема гілка в `step` (поп/інвалідація).
        Class::Boundary
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
    // **Коротке замикання live-switch (ПЕРША лінія).** Якщо посеред слова вже був
    // mid-word перенабір (`live_locked`), екран УЖЕ коректний: ОС допечатала решту
    // слова в новій розкладці. Boundary-перенабір тут зробив би ПОДВОЄННЯ → лише
    // скидаємо буфер (це знімає й пін) і виходимо. Друга лінія захисту —
    // self-heal у detector (`best==current` → switch=false) — спрацювала б і так,
    // але цей вихід дешевший і не залежить від словникового збігу всього слова.
    if state.buffers.for_window(wkey).live_locked {
        state.buffers.for_window(wkey).reset();
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
        // Відкриваємо коротке вікно очікування відкидання цього перенабору й
        // запам'ятовуємо все для можливого ЯВНОГО відкату (revert_last).
        state.pending_retype = Some(build_pending(wkey, &decision, separator, ctx));
    }
    state.buffers.for_window(wkey).reset();
    actions
}

/// Спроба перемикання НА ЛЬОТУ (mid-word) після `push` чергового страйка слова.
///
/// **Двосторонній гейт тримає detector** ([`detector::live_decide`]): прапорець
/// `live_switch_enabled`, поточна мова — глухий кут (`!has_prefix && !recognizes`),
/// інша — живий dict-префікс, `live_min_len`, veto, `best≠current`. Тут лише
/// проводка: пін одного свічу на слово + learned-veto + мід-ворд перенабір.
///
/// **Когерентність буфера (НЕ скидаємо!):** після свічу лишаємо ті самі layout-
/// незалежні `KeyStroke` — ОС тепер у новій розкладці друкує наступні фізичні
/// клавіші правильно, і вони `push`-аться далі в той самий буфер. Межу
/// (`handle_boundary`) коротко замикає `live_locked` → `reset` без зайвого
/// перенабору (екран уже коректний).
///
/// **Undo — варіант Б:** `pending_retype` НЕ виставляємо (інакше Backspace завчив
/// би ФРАГМЕНТ слова в learned). Backspace після live → звичайний `pop` страйка
/// (слово лишається когерентним); жодного `learn()` на live.
fn try_live_switch(state: &mut EngineState, wkey: &str, ctx: &Context) -> Vec<Action> {
    // Рання відсіч ДО будь-якої алокації: у проді дефолт OFF, тож гарячий шлях
    // (кожна літера) не має навіть копіювати страйки. `live_decide` все одно
    // перевіряє цей прапорець, але тут — щоб не платити `to_vec()` дарма.
    if !ctx.config.live_switch_enabled {
        return Vec::new();
    }
    if state.buffers.for_window(wkey).live_locked {
        return Vec::new(); // один свіч на слово
    }
    let strokes: Vec<KeyStroke> = state.buffers.for_window(wkey).strokes().to_vec();
    let Some(decision) = detector::live_decide(&strokes, ctx) else {
        return Vec::new();
    };
    // Самонавчений veto (як у `handle_boundary`): слово, яке користувач уже
    // відкидав, не перемикаємо навіть на льоту.
    if state.learned.contains(&decision.current_text) {
        return Vec::new();
    }
    // Пін: у межах цього слова більше не перемикати; межа зробить `reset` без
    // boundary-перенабору. Буфер НЕ чіпаємо — страйки лишаються когерентними.
    state.buffers.for_window(wkey).live_locked = true;
    // separator = None (межі ще не було, слово на екрані) — як `force_switch_last`.
    replacer::plan(&decision, None)
}

/// Зібрати [`PendingRetype`] з рішення, що перемикає: рахує, скільки символів
/// перенабору опинилось на екрані, і формує повний оригінал для відновлення.
///
/// На екрані після перенабору: `best_text + suffix + separator?`; до перенабору
/// було: `current_text + suffix + separator?`. Хвостовий суфікс і друкований
/// роздільник однакові в обох (та сама к-сть символів), різниться лише саме слово.
fn build_pending(
    wkey: &str,
    decision: &crate::detector::Decision,
    separator: Option<char>,
    ctx: &Context,
) -> PendingRetype {
    let mut tail = decision.suffix.clone();
    if let Some(sep) = separator {
        tail.push(sep);
    }
    let retyped_len = (decision.best_text.chars().count() + tail.chars().count()) as u32;
    let original_full = format!("{}{}", decision.current_text, tail);
    // caps-корекція не міняла розкладку → повертати нічого не треба.
    let restore_layout = if decision.caps_only {
        None
    } else {
        Some(ctx.current_layout.clone())
    };
    PendingRetype {
        window_key: wkey.to_string(),
        original_word: decision.current_text.clone(),
        retyped_len,
        original_full,
        restore_layout,
    }
}

/// Внутрішня реалізація кроку (див. [`crate::step`]).
pub fn step(state: &mut EngineState, ev: InputEvent, ctx: &Context) -> Vec<Action> {
    let wkey = window_key(&ctx.active_window);

    // Приватність №4: секретне (пароль) поле — повний bypass ПЕРШИМ ділом, ще
    // до rejection-сигналу/detector. НЕ буферимо й НЕ перемикаємо, а буфер вікна
    // СКИДАЄМО (на відміну від виключення вікна) — у пам'яті не має лишитись
    // нічого про набране в полі пароля. `pending_retype` теж гасимо.
    if ctx.secure {
        state.buffers.invalidate_window(&wkey);
        state.pending_retype = None;
        return Vec::new();
    }

    // Виключене вікно (застосунок/папка) — повний bypass: не буферимо й не
    // перемикаємо. Перевіряємо ПЕРЕД detector (порядок: bypass → veto).
    if ctx.is_window_excluded() {
        return Vec::new();
    }

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

    match classify(stroke, ctx) {
        Class::Boundary => {
            let separator = separator_char(stroke, current_layout);
            handle_boundary(state, &wkey, ctx, separator)
        }
        Class::Word => {
            state.buffers.for_window(&wkey).push(stroke);
            try_live_switch(state, &wkey, ctx)
        }
    }
}

/// Скасувати ОСТАННІЙ авто-перенабір (гаряча клавіша B1) — ручний аналог
/// Backspace-rejection, але викликається ЯВНО (не залежить від наступної події).
///
/// Стирає щойно набраний текст ([`Action::DeleteChars`]), повертає стару розкладку
/// ([`Action::SwitchLayout`], якщо перемикання було), друкує ОРИГІНАЛ
/// ([`Action::TypeUnicode`]) і ЗАВЧАЄ слово ([`LearnedExceptions::learn`] +
/// [`Action::CommitException`]), щоб апка більше його не чіпала. Якщо скасовувати
/// нічого (немає `pending_retype`) → порожній план.
pub fn revert_last(state: &mut EngineState) -> Vec<Action> {
    let Some(pending) = state.pending_retype.take() else {
        return Vec::new();
    };
    let mut actions = Vec::with_capacity(4);
    if pending.retyped_len > 0 {
        actions.push(Action::DeleteChars(pending.retyped_len));
    }
    if let Some(layout) = pending.restore_layout {
        actions.push(Action::SwitchLayout(layout));
    }
    actions.push(Action::TypeUnicode(pending.original_full));
    // Завчити слово (миттєвий ефект у сесії) + емісія для персистенції app-шаром.
    state.learned.learn(&pending.original_word);
    actions.push(Action::CommitException(pending.original_word));
    actions
}

/// Примусово перемкнути ОСТАННЄ слово активного вікна в ІНШУ мову БЕЗ порогу
/// впевненості (гаряча клавіша B1 — і «перемкнути вручну», і «змінити розкладку
/// останнього слова», аналог подвійного Shift).
///
/// Бере поточний word-буфер активного вікна й перенабирає його у найкращий
/// НЕ-поточний кандидат (ручне рішення користувача переважає поріг/довжину/veto —
/// див. [`detector::force_decision`]). Якщо буфер порожній, немає поточного
/// профілю чи іншої мови — порожній план. Залишає `pending_retype`, тож ручне
/// перемикання теж можна відкотити через [`revert_last`].
pub fn force_switch_last(state: &mut EngineState, ctx: &Context) -> Vec<Action> {
    let wkey = window_key(&ctx.active_window);
    // Приватність №4: секретне поле — повний bypass + скидання буфера (як у `step`).
    // Ручна команда НЕ переважає приватність: у полі пароля нічого не робимо.
    if ctx.secure {
        state.buffers.invalidate_window(&wkey);
        state.pending_retype = None;
        return Vec::new();
    }
    // Виключене вікно — повний bypass (як у `step`).
    if ctx.is_window_excluded() {
        return Vec::new();
    }
    let strokes: Vec<KeyStroke> = state.buffers.for_window(&wkey).strokes().to_vec();
    if strokes.is_empty() {
        return Vec::new();
    }
    let Some(decision) = detector::force_decision(&strokes, ctx) else {
        return Vec::new();
    };
    // Ручне перемикання тригериться без друкованого роздільника (слово ще на
    // екрані, межі не було) → separator = None.
    let actions = replacer::plan(&decision, None);
    state.pending_retype = Some(PendingRetype {
        window_key: wkey.clone(),
        original_word: decision.current_text.clone(),
        retyped_len: decision.best_text.chars().count() as u32,
        original_full: decision.current_text,
        restore_layout: Some(ctx.current_layout.clone()),
    });
    // Слово «завершено» цим ручним рішенням → буфер більше не відображає екран.
    state.buffers.for_window(&wkey).reset();
    actions
}
