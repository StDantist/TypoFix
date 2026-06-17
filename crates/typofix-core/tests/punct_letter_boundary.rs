//! E2E-гард (герметичний, БЕЗ реальних моделей): пунктуація-що-є-літерою-в-розкладці
//! на межі слова. Закриває діру автономії: eval-харнес годує вже сегментовані
//! слова в `detector::decide`, оминаючи `engine::classify`/буфер, тож САМЕ цей
//! баг (буфер рветься на клавіші `,`=`б`, `.`=`ю`, `;`=`ж`, `[`=`х`, `]`=`ї`)
//! eval НЕ ловив. Тут ганяємо повний шлях `step` через `VirtualPlatform`.
//!
//! Моделі будуємо самотужки: РЕАЛЬНІ вбудовані розкладки (`embedded_layout`,
//! checked-in TOML) + контрольовані LM/словник на потрібних словах. Так тест
//! герметичний (не потребує gitignored `.bin`/`.fst`) і стабільний у CI.

use typofix_core::{
    step, Context, DetectorConfig, Dictionary, EngineState, ExclusionRules, KeyStroke,
    LanguageProfile, Layout, LayoutId, NgramModel, WordRules,
};
use typofix_platform::{Action, InputEvent, KeyDir, KeyEvent, Modifiers};
use typofix_platform_virtual::{drive, VirtualPlatform};

static NO_EXCL: ExclusionRules = ExclusionRules::new();
static NO_RULES: WordRules = WordRules::new();

const SPACE: u32 = 0x39;

fn uk_layout() -> Layout {
    typofix_data::embedded_layout("uk").expect("вбудована uk-розкладка")
}
fn en_layout() -> Layout {
    typofix_data::embedded_layout("en").expect("вбудована en-розкладка")
}

/// Профілі uk+en з контрольованими LM/словником (герметично, без `data/lm`).
fn profiles() -> Vec<LanguageProfile> {
    // Корпус повторює цільові слова, щоб LM упевнено голосувала за uk-кандидата;
    // англ. корпус — звичайний текст без наших крякозябр.
    // `що` навмисно ЧАСТОТНІШЕ за `щоб`, щоб відтворити баг хвостового стрипу:
    // сепаратор-гілка (`що`) має вищу впевненість за letter-гілку (`щоб`).
    let uk_corpus = "\
        привіт добре бюджет хліб жнива їжа любов що що що що що що що що щоб \
        привіт добре бюджет хліб жнива їжа любов що що що що що що що що \
        привіт добре бюджет хліб жнива їжа любов що що що що що що що що \
        привіт як добре все привіт друже добре день бюджет на рік хліб свіжий що щоб";
    let en_corpus =
        "hello world good morning please name return value list function the quick brown fox \
        hood good food mood the hood is good a good hood good hood food hood";

    let uk = LanguageProfile {
        id: LayoutId::new("uk"),
        layout: uk_layout(),
        lm: NgramModel::train(uk_corpus, 3, 0.5),
        dict: Dictionary::from_words([
            "привіт",
            "добре",
            "бюджет",
            "хліб",
            "жнива",
            "їжа",
            "любов",
            "день",
            "друже",
            "що",
            "щоб",
        ])
        .unwrap(),
        freq: None,
    };
    let en = LanguageProfile {
        id: LayoutId::new("en"),
        layout: en_layout(),
        lm: NgramModel::train(en_corpus, 3, 0.5),
        dict: Dictionary::from_words([
            "hello", "world", "good", "morning", "please", "name", "return", "value", "list",
            "function", "hood", "food", "mood",
        ])
        .unwrap(),
        freq: None,
    };
    vec![uk, en]
}

fn key(stroke: KeyStroke) -> InputEvent {
    InputEvent::Key(KeyEvent {
        scancode: stroke.scancode,
        vk: 0,
        dir: KeyDir::Down,
        modifiers: stroke.modifiers,
        timestamp_ms: 0,
        is_synthetic: false,
        is_autorepeat: false,
    })
}

fn key_sc(scancode: u32) -> InputEvent {
    key(KeyStroke::new(scancode, Modifiers::empty()))
}

/// Фізичні страйки, якими набирають `uk_word` (через зворотний індекс uk-розкладки).
/// Це layout-незалежні натискання — ті самі, що дали б крякозябри в `en`.
fn strokes_for_uk_word(word: &str) -> Vec<KeyStroke> {
    let uk = uk_layout();
    word.chars()
        .map(|ch| {
            uk.stroke_for(ch)
                .unwrap_or_else(|| panic!("немає клавіші для '{ch}' у uk"))
        })
        .collect()
}

/// Те саме, але страйки англ. слова через зворотний індекс en-розкладки.
fn strokes_for_en_word(word: &str) -> Vec<KeyStroke> {
    let en = en_layout();
    word.chars()
        .map(|ch| {
            en.stroke_for(ch)
                .unwrap_or_else(|| panic!("немає клавіші для '{ch}' у en"))
        })
        .collect()
}

fn run(platform: &mut VirtualPlatform, langs: &[LanguageProfile]) {
    let mut state = EngineState::default();
    drive(platform, |ev, win, layout| {
        let ctx = Context {
            active_window: win.clone(),
            current_layout: layout.clone(),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
            secure: false,
        };
        step(&mut state, ev, &ctx)
    });
}

/// Сценарій: користувач застряг у `en` і фізично набрав клавіші `uk_word`
/// (+опційні хвостові страйки `tail`), потім пробіл-тригер. На екрані опинилось
/// те, що `en` надрукувала б. Повертає фінальний текст поля.
fn type_in_en_expect(uk_word: &str, tail: &[u32]) -> (String, Vec<Action>) {
    let langs = profiles();
    let en = en_layout();

    let mut strokes = strokes_for_uk_word(uk_word);
    for &sc in tail {
        strokes.push(KeyStroke::new(sc, Modifiers::empty()));
    }

    // Те, що ОС надрукувала б у en (включно з хвостовою пунктуацією) + пробіл.
    let mut screen: String = en.interpret(&strokes);
    screen.push(' ');

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("en"));
    platform.set_text(&screen);
    let mut evs: Vec<InputEvent> = strokes.into_iter().map(key).collect();
    evs.push(key_sc(SPACE));
    platform.enqueue_all(evs);

    run(&mut platform, &langs);
    (platform.text(), platform.applied_actions().to_vec())
}

/// ЗВОРОТНИЙ сценарій (кейс A): користувач застряг у `uk` і фізично набрав
/// клавіші англ. слова `en_word` (+опційний хвіст), потім пробіл. На екрані —
/// uk-інтерпретація (крякозябри). Має перемкнутись на `en_word`.
fn type_in_uk_expect(en_word: &str, tail: &[u32]) -> (String, Vec<Action>) {
    let langs = profiles();
    let uk = uk_layout();

    let mut strokes = strokes_for_en_word(en_word);
    for &sc in tail {
        strokes.push(KeyStroke::new(sc, Modifiers::empty()));
    }

    // Те, що ОС надрукувала б у uk (хвостова кома-клавіша 0x33 → «б») + пробіл.
    let mut screen: String = uk.interpret(&strokes);
    screen.push(' ');

    let mut platform = VirtualPlatform::new();
    platform.set_layout(LayoutId::new("uk"));
    platform.set_text(&screen);
    let mut evs: Vec<InputEvent> = strokes.into_iter().map(key).collect();
    evs.push(key_sc(SPACE));
    platform.enqueue_all(evs);

    run(&mut platform, &langs);
    (platform.text(), platform.applied_actions().to_vec())
}

// ─── Кейс A: асиметрія коми (поточна UK, кома-клавіша=«б», кандидат EN) ──────

#[test]
fn reverse_trailing_comma_uk_to_en_hood() {
    // Поточна UK, набрано h o o d ,(0x33) пробіл → на екрані "рщщвб ". Кома-
    // клавіша = «б» у поточній (UK), але «,» у кандидатній (EN). Гілка-роздільник
    // має стрипнути хвіст → "hood" (словникове en) + кома-роздільник.
    let (text, actions) = type_in_uk_expect("hood", &[0x33]); // +кома-клавіша
    assert_eq!(
        text, "hood, ",
        "UK→EN: кома-клавіша як роздільник, слово «hood» перемкнуто"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::SwitchLayout(id) if id.as_str() == "en")),
        "мав бути перенабір на en"
    );
}

#[test]
fn reverse_english_word_plus_comma_in_uk() {
    // Інше англ. слово + кома в UK-розкладці: «good» = g o o d ,
    let (text, _) = type_in_uk_expect("good", &[0x33]);
    assert_eq!(text, "good, ", "UK→EN: «good,» теж має перемкнутись");
}

// ─── TP: пунктуація-літера ВСЕРЕДИНІ слова має тепер працювати ──────────────

#[test]
fn comma_b_inside_word_dobre() {
    // `lj,ht` (кома=б усередині) → «добре». Головний кейс бага.
    let (text, _) = type_in_en_expect("добре", &[]);
    assert_eq!(
        text, "добре ",
        "кома-як-б усередині слова → перенабір «добре»"
    );
}

#[test]
fn multiple_punct_letters_inside_word_byudzhet() {
    // «бюджет» = б(,) ю(.) д(l) ж(;) е(t) т(n): аж три пунктуаційні клавіші
    // всередині. Усі мають буферитись, слово — розпізнатись.
    let (text, _) = type_in_en_expect("бюджет", &[]);
    assert_eq!(text, "бюджет ", "кілька пунктуацій-літер усередині слова");
}

#[test]
fn control_word_without_punct_still_works() {
    // Контроль: звичайне слово без пунктуації-літери не зачеплено фіксом.
    let (text, _) = type_in_en_expect("привіт", &[]);
    assert_eq!(text, "привіт ");
}

// ─── TP: слово, що ЗАКІНЧУЄТЬСЯ пунктуацією-літерою (гілка-літера виграє) ────

#[test]
fn word_ending_in_punct_letter_hlib() {
    // «хліб» закінчується на б(=`,`): на екрані "[ks," (хвостова кома). Гілка-
    // літера дає СЛОВНИКОВЕ «хліб» → виграє precision-замок; кома НЕ роздільник.
    let (text, actions) = type_in_en_expect("хліб", &[]);
    assert_eq!(text, "хліб ", "слово, що закінчується на пунктуацію-літеру");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::SwitchLayout(id) if id.as_str() == "uk")),
        "мав бути перенабір на uk"
    );
}

#[test]
fn word_ending_in_punct_letter_beats_more_frequent_prefix_shchob() {
    // РЕГРЕС кроку (а): `щоб` закінчується на б(=`,`) → на екрані "oj,". Префікс
    // `що` ТЕЖ словниковий і ЧАСТОТНІШИЙ, тож сепаратор-гілка (`що`) має вищу
    // raw-confidence за letter-гілку (`щоб`). До фіксу замок `letter.conf >
    // sep.conf` віддавав перевагу сепаратору → друкував «що,» (губив `б`).
    // Після (а): повне словникове `щоб` б'є коротший префікс → «щоб».
    let (text, actions) = type_in_en_expect("щоб", &[]);
    assert_eq!(
        text, "щоб ",
        "повне словникове слово, що закінчується пунктуацією-літерою, не має губити хвіст"
    );
    assert!(
        !text.contains("що,"),
        "хвостовий `б` НЕ можна губити як роздільник-кому"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::SwitchLayout(id) if id.as_str() == "uk")),
        "мав бути перенабір на uk"
    );
}

#[test]
fn non_dict_word_ending_in_punct_letter_stays_unfixed_step_b() {
    // ЗАГЛУШКА/НОТАТКА для кроку (б): `вжух` закінчується на х(=`[`) → "d;e[", але
    // `вжух` ВІДСУТНІЙ у словнику (тут навмисно не додано; у проді — фільтр
    // MIN_LEN=4 у VESUM). Крок (а) НЕ фіксить його (letter-гілка не словникова →
    // замок `best_is_dict` лишає сепаратор). Поведе крок (б): частотні короткі
    // форми в словнику. Поки що — стабільно НЕ перемикається (precision-safe).
    let (text, actions) = type_in_en_expect("вжух", &[]);
    assert_ne!(
        text, "вжух ",
        "крок (а) свідомо НЕ ловить не-словникове `вжух`"
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, Action::SwitchLayout(id) if id.as_str() == "uk")),
        "без dict-hit перемикання бути не повинно (поїде на кроці б)"
    );
}

// ─── PRECISION-GUARD: пунктуація-як-роздільник (НЕ нашкодити) ───────────────

#[test]
fn trailing_comma_is_separator_not_letter_privit() {
    // `ghbdsn,` (хотів «привіт» + кома) → «привіт,», а НЕ «привітб». Кома —
    // хвостовий роздільник: гілка-роздільник дає словникове «привіт», гілка-
    // літера «привітб» — не слово → precision-замок лишає кому роздільником.
    let (text, actions) = type_in_en_expect("привіт", &[0x33]); // +кома
    assert_eq!(
        text, "привіт, ",
        "кома лишається роздільником, не з'їдається як «б»"
    );
    assert!(
        !text.contains("привітб"),
        "кому НЕ можна трактувати як літеру б"
    );
    // Стерто слово+кому+пробіл (8), набрано «привіт, ».
    assert_eq!(actions[0], Action::DeleteChars(8));
    assert_eq!(actions[2], Action::TypeUnicode("привіт, ".into()));
}

#[test]
fn trailing_period_is_separator_privit() {
    // Те саме з крапкою (=ю): `ghbdsn.` → «привіт.», не «привітю».
    let (text, _) = type_in_en_expect("привіт", &[0x34]); // +крапка
    assert_eq!(text, "привіт. ");
}

#[test]
fn trailing_semicolon_is_separator_privit() {
    // …і з крапкою-з-комою (=ж): `ghbdsn;` → «привіт;», не «привітж».
    let (text, _) = type_in_en_expect("привіт", &[0x27]); // +;
    assert_eq!(text, "привіт; ");
}

#[test]
fn english_text_with_punct_untouched() {
    // Англ. текст із внутрішньою комою без пробілу — чіпати НЕ можна.
    let langs = profiles();
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("en"));
    p.set_text("hello,world ");
    // h e l l o , w o r l d  + пробіл
    let scs = [
        0x23, 0x12, 0x26, 0x26, 0x18, 0x33, 0x11, 0x18, 0x13, 0x26, 0x20, SPACE,
    ];
    p.enqueue_all(scs.into_iter().map(key_sc));
    run(&mut p, &langs);
    assert_eq!(p.text(), "hello,world ", "англ. текст із комою не чіпати");
    assert!(p.applied_actions().is_empty());
}

#[test]
fn number_with_comma_untouched() {
    // Число «3,14»: цифри — тверда межа (не літери в жодній розкладці) → буфер
    // не накопичує перемикабельного слова. Текст незмінний.
    let langs = profiles();
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("en"));
    p.set_text("3,14 ");
    // 3(0x04) ,(0x33) 1(0x02) 4(0x05) пробіл
    p.enqueue_all([0x04, 0x33, 0x02, 0x05, SPACE].into_iter().map(key_sc));
    run(&mut p, &langs);
    assert_eq!(p.text(), "3,14 ", "число з комою не чіпати");
    assert!(p.applied_actions().is_empty());
}

#[test]
fn code_semicolon_untouched() {
    // Код `a;b` (; = ж): не словникове в жодній мові → не перемикати.
    let langs = profiles();
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("en"));
    p.set_text("a;b ");
    // a(0x1E) ;(0x27) b(0x30) пробіл
    p.enqueue_all([0x1E, 0x27, 0x30, SPACE].into_iter().map(key_sc));
    run(&mut p, &langs);
    assert_eq!(p.text(), "a;b ", "код a;b не чіпати");
    assert!(p.applied_actions().is_empty());
}

#[test]
fn code_bracket_untouched() {
    // `arr[0]` ([ = х, ] = ї): не словникове → не перемикати.
    let langs = profiles();
    let mut p = VirtualPlatform::new();
    p.set_layout(LayoutId::new("en"));
    p.set_text("arr[0] ");
    // a r r [ 0 ] пробіл
    p.enqueue_all(
        [0x1E, 0x13, 0x13, 0x1A, 0x0B, 0x1B, SPACE]
            .into_iter()
            .map(key_sc),
    );
    run(&mut p, &langs);
    assert_eq!(p.text(), "arr[0] ", "код arr[0] не чіпати");
    assert!(p.applied_actions().is_empty());
}
