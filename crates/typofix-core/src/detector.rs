//! Рішення про мову/розкладку слова: поєднує `layout_mapper` (інтерпретація
//! страйків), `lm` (правдоподібність) і `dict` (буст упевненості). §3.3.
//!
//! **Чисто й детерміновано.** Жодного завантаження даних: усі ресурси мов
//! ([`LanguageProfile`]) передаються ззовні через [`Context`] як позичені дані.
//!
//! ## Алгоритм
//! Для кожної ввімкненої мови інтерпретуємо ту саму послідовність фізичних
//! страйків у її розкладці й оцінюємо отриманий рядок:
//!
//! ```text
//! score(lang) = w1 · lm.score(text) + (dict.contains(text) ? bonus : 0)
//! ```
//!
//! Обираємо найкращу. Перемикаємо, лише якщо вона ≠ поточної, слово не надто
//! коротке, перевага над поточною інтерпретацією перевищує `threshold(len)` і
//! немає вето правил. **Принцип: за сумніву НЕ перемикати** (precision > recall).

use crate::{Context, Dictionary, FrequencyMap, KeyStroke, Layout, LayoutId, NgramModel};

/// Ресурси однієї мови, потрібні детектору. Власник — оркестратор/тест; у
/// `Context` потрапляє позиченим зрізом (core нічого не вантажить сам).
#[derive(Debug, Clone)]
pub struct LanguageProfile {
    /// Ідентифікатор мови/розкладки (`"uk"`, `"en"`).
    pub id: LayoutId,
    /// Розкладка для інтерпретації страйків.
    pub layout: Layout,
    /// Мовна n-gram модель.
    pub lm: NgramModel,
    /// Словник для бусту впевненості (бінарне членство).
    pub dict: Dictionary,
    /// **Частотна мапа** (опційно): градуйований сигнал поверх `dict`. `None` —
    /// частотного шару немає (працює лише baseline dict-бонус). Слово, що Є в
    /// `dict`, але якого НЕМАЄ в `freq`, отримує лише baseline (частота не карає —
    /// вона лише ДОДАЄ зважування зверху для слів, що Є в мапі). Див. [`score`].
    ///
    /// [`score`]: LanguageProfile::score
    pub freq: Option<FrequencyMap>,
}

/// Розкладений бал кандидата: повний (`total`) і **лише LM-складова** (`lm`).
///
/// LM-складова потрібна окремо, бо для дуже коротких слів ми вимагаємо, щоб
/// перевагу давав не лише збіг у словнику, а й сама мовна модель (див.
/// [`DetectorConfig::short_word_lm_margin`]).
#[derive(Debug, Clone, Copy)]
struct CandidateScore {
    /// Повний бал: `lm_weight·lm + dict_bonus? + freq_term?`.
    total: f64,
    /// Лише зважена LM-складова: `lm_weight·lm` (без бонусів словника/частоти).
    lm: f64,
    /// Лише **частотна** надбавка (`freq_term`, завжди ≥ 0). Тримається окремо,
    /// щоб коротко-словний гейт міг вимагати ЧАСТОТНОЇ переваги кандидата —
    /// аналогічно до того, як `lm` тримається окремо для LM-переваги. Для слова
    /// поза `dict` або поза `freq` дорівнює 0.
    freq: f64,
}

impl LanguageProfile {
    /// Бал кандидата для заданого тексту (вже інтерпретованого в його розкладці).
    ///
    /// **Частотно-зважений dict-бонус:** слово зі словника отримує `dict_bonus`
    /// (baseline). Якщо воно ще й є в частотній мапі — ДОДАЄМО зважену
    /// log-ймовірність понад поріг: `freq_weight · max(0, lp − freq_floor)`, де
    /// `lp = ln(count) − ln(total)` (нормалізована частка, зіставна між мовами).
    /// Поріг `freq_floor` відсікає рідкісні слова/шум корпусу (їхня надбавка = 0,
    /// тобто вони лишаються на baseline — як dict-член без частоти). Так часте
    /// слово дає БІЛЬШИЙ бонус за рідкісне, і `ну`(часте) б'є `ye`(рідкісне).
    ///
    /// **Особистий словник:** `recognized` (з `ctx.rules.recognizes`) додає
    /// dict-членність ПОЗА `LanguageProfile.dict` — слово з `user.txt` (`лох`)
    /// дістає той самий baseline `dict_bonus`. Частоти в нього зазвичай немає
    /// (`freq.log_prob → None` → надбавка 0), тож рівно baseline.
    fn score(&self, text: &str, cfg: &DetectorConfig, recognized: bool) -> CandidateScore {
        if text.is_empty() {
            return CandidateScore {
                total: f64::NEG_INFINITY,
                lm: f64::NEG_INFINITY,
                freq: 0.0,
            };
        }
        let lm = cfg.lm_weight * self.lm.score(text);
        let (dict_bonus, freq_term) = if self.dict.contains(text) || recognized {
            // Baseline зберігається ЗАВЖДИ для dict-члена. Частота лише додає
            // зверху для слів, що Є в мапі й частіші за поріг (інакше +0).
            let ft = self
                .freq
                .as_ref()
                .and_then(|m| m.log_prob(text))
                .map(|lp| cfg.freq_weight * (lp - cfg.freq_floor).max(0.0))
                .unwrap_or(0.0);
            (cfg.dict_bonus, ft)
        } else {
            (0.0, 0.0)
        };
        CandidateScore {
            total: lm + dict_bonus + freq_term,
            lm,
            freq: freq_term,
        }
    }
}

/// Налаштування детектора (ваги й крива порогу). Калібруватиметься на eval-датасеті
/// (Фаза 2/наступна задача); тут — розумні дефолти для доведення логіки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Вага лог-ймовірності LM (`w1`).
    pub lm_weight: f64,
    /// Бонус за наявність слова у словнику (`w2`-еквівалент, baseline для
    /// dict-члена незалежно від частоти).
    pub dict_bonus: f64,
    /// Вага частотної надбавки: множник на `max(0, lp − freq_floor)`, де `lp` —
    /// log-ймовірність слова (нормалізована частка в корпусі). `0.0` вимикає
    /// частотний шар (лишається плаский `dict_bonus`).
    pub freq_weight: f64,
    /// Поріг log-ймовірності, нижче якого частотної надбавки немає (слово рідкісне
    /// / шум корпусу → лишається на baseline). У одиницях `ln(частка)`: `−9.0`
    /// ≈ частка `1.2·10⁻⁴`. Калібровано так, щоб реальні часті слова (`ну`≈−5.85,
    /// `от`≈−6.66) були ВИЩЕ порога, а код-двійники (`ат`≈−13.7, `ye`≈−11.1) — НИЖЧЕ.
    pub freq_floor: f64,
    /// Базовий поріг переваги (для довгих слів).
    pub base_threshold: f64,
    /// Додаток до порогу, обернено пропорційний довжині (карає короткі слова).
    pub short_word_extra: f64,
    /// Мінімальна довжина слова, яке взагалі можна перемикати (коротші —
    /// неоднозначні в обох мовах, не чіпаємо).
    pub min_switch_len: usize,
    /// Максимальна довжина «дуже короткого» слова, для якого збігу в словнику
    /// САМОГО ПО СОБІ недостатньо: на таких словах `dict_bonus ≈ threshold`, тож
    /// одинокий збіг у словнику тривіально пробиває поріг (FP типу `fn`→«ат`).
    pub short_word_max_len: usize,
    /// Для слів `len <= short_word_max_len` вимагаємо, щоб LM-складова кандидата
    /// перевищувала LM-складову поточної розкладки щонайменше на цей запас —
    /// тобто за кандидата має «голосувати» і мовна модель, а не лише словник.
    pub short_word_lm_margin: f64,
    /// **Альтернатива LM-маржі для коротких слів — ЧАСТОТНА маржа.** Коли обидва
    /// двійники — реальні слова (dict-бонуси скасовуються, LM майже рівні —
    /// `ну`↔`ye`), LM-гейт не пускає, але якщо ЧАСТОТНА перевага кандидата
    /// (`best.freq − current.freq`) перевищує цей запас, коротко-словний гейт
    /// відкривається. Так часте слово б'є рідкісного двійника. Стандартний поріг
    /// `threshold(len)` і `best≠current`/veto лишаються в силі.
    pub short_word_freq_margin: f64,
    /// **Стеля LM джерельного двійника для дзеркальної релаксації** коротких
    /// слів: дзеркальна гілка (службове слово ↔ біліберда) спрацьовує лише якщо
    /// LM-складова ПОТОЧНОГО тексту (двійника у вихідній мові) НИЖЧА за цю межу —
    /// тобто двійник фонотактично неправдоподібний (реальне слово мав би LM
    /// вище). Це і є умова «двійник НЕ справжнє слово» (поряд із «не в словнику»).
    /// Дефолт `0.0`: біліберда має LM ≪ 0; справжні короткі слова — LM > 0.
    pub short_word_twin_lm_max: f64,
    /// **Прапорець фонотактичного сигналу** (default `true`). В українській НЕМАЄ
    /// слів, що починаються з «ь» — таке читання НЕМОЖЛИВЕ → перемкнути на латиницю
    /// (за умови правдоподібного EN-двійника). Вимкнення → сигнал ігнорується
    /// (для майбутнього UI-тоглу). Див. [`starts_impossible_uk`].
    pub phonotactics_enabled: bool,
    /// Запас правдоподібності EN-двійника для фонотактичного перемикання: латинський
    /// кандидат форсить перемикання, лише якщо його LM перевищує LM (неможливого)
    /// укр. читання щонайменше на цей запас. Дефолт `0.0`: неможливе укр. читання
    /// фонотактично провальне, тож будь-який реальний латинський двійник його б'є;
    /// гейт лише відсікає випадок «обидва боки — суцільне сміття».
    pub phonotactic_twin_lm_margin: f64,
    /// **Прапорець сигналу файлових розширень** (default `true`). EN-двійник
    /// кандидата — відоме розширення (`txt`/`md`), а укр. читання НЕ реальне слово
    /// → перемкнути на латиницю. Вимкнення → сигнал ігнорується (UI-тогл).
    pub extensions_enabled: bool,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            lm_weight: 1.0,
            dict_bonus: 4.0,
            freq_weight: 1.0,
            freq_floor: -9.0,
            base_threshold: 1.0,
            short_word_extra: 4.0,
            min_switch_len: 2,
            short_word_max_len: 2,
            short_word_lm_margin: 2.0,
            short_word_freq_margin: 2.0,
            short_word_twin_lm_max: 0.0,
            phonotactics_enabled: true,
            phonotactic_twin_lm_margin: 0.0,
            extensions_enabled: true,
        }
    }
}

impl DetectorConfig {
    /// Поріг переваги залежно від довжини слова (символів).
    ///
    /// Короткі слова потребують значно більшої переваги (бо `по`/`gj`, `не`/`yt`
    /// валідні в обох мовах); довгі — меншої. Слова коротші за `min_switch_len`
    /// не перемикаються ніколи (поріг `+∞`).
    pub fn threshold(&self, len: usize) -> f64 {
        if len < self.min_switch_len {
            return f64::INFINITY;
        }
        self.base_threshold + self.short_word_extra / (len as f64)
    }
}

/// Результат розгляду слова детектором.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    /// Найкраща мова за балом.
    pub best: LayoutId,
    /// Текст слова в найкращій розкладці (готовий для перенабору, з регістром).
    pub best_text: String,
    /// Текст слова в поточній розкладці (те, що зараз на екрані).
    pub current_text: String,
    /// Чи варто перемикати+перенабирати.
    pub switch: bool,
    /// Перевага найкращої над поточною (best.score − current.score); для дебагу/тестів.
    pub confidence: f64,
    /// **Хвостовий суфікс**, що зберігається ДОСЛІВНО між виправленим словом і
    /// тригерним роздільником: гліф РОЗДІЛЬНИКА хвостових пунктуаційних клавіш,
    /// які гілка-роздільник трактувала як роздільники, а не літери (напр. `","` у
    /// `ghbdsn,`→`привіт,` і так само в зворотному `рщщвб`→`hood,`, де на екрані
    /// була `б`, а вертаємо `,`). Порожній у звичайному випадку. `replacer`
    /// дописує його після `best_text` і враховує в кількості стирання (та сама
    /// к-сть символів, що й на екрані: один страйк → один символ у будь-якій мові).
    pub suffix: String,
    /// **Корекція ЛИШЕ регістру** (помилка перетриманого Shift), БЕЗ зміни
    /// розкладки. Коли `true`, `best == current_layout`, а `best_text` —
    /// нормалізований регістр того ж слова в тій самій мові; `replacer` НЕ
    /// емітить `SwitchLayout` (перенабір лише стирає й вписує виправлений текст).
    /// `false` для звичайного розкладко-перемикання. Див. [`overheld_shift_fix`].
    pub caps_only: bool,
}

/// Чи символ — частина слова (літера або апостроф). Спільний критерій для межі
/// слова в [`crate::engine`] і для дизамбігуації пунктуації-що-є-літерою тут.
pub(crate) fn is_word_char(ch: char) -> bool {
    ch.is_alphabetic() || ch == '\'' || ch == '’'
}

/// Чи текст починається з символу, **НЕМОЖЛИВОГО на початку українського слова**.
///
/// Поки що єдине ЗАЛІЗНЕ (100%) правило — м'який знак «ь» (U+044C, велике U+042C):
/// в українській НЕМАЄ слів, що починаються з нього. Тож кирилично-«ь»-початкове
/// читання напевно НЕ українське — користувач мав на увазі латиницю. Інші можливі
/// кандидати (напр. апостроф/«ї» на початку) свідомо НЕ додано — лише на 100%
/// неможливі позиції, щоб не зачепити жодне реальне слово (precision > recall).
fn starts_impossible_uk(text: &str) -> bool {
    matches!(text.chars().next(), Some('ь') | Some('Ь'))
}

/// Чи дає цей страйк ЛІТЕРУ хоч у одній увімкненій розкладці (включно з поточною).
///
/// Це й є критерій «пунктуація-що-є-літерою»: клавіша на кшталт `,` (scancode
/// `0x33`) у `en` — пунктуація, але в `uk` — літера `б`. Такі клавіші не можна
/// наївно вважати твердою межею слова (інакше буфер рветься посеред слова), і
/// саме їх гілка-роздільник може стрипнути як хвостовий роздільник.
pub(crate) fn letter_in_any_layout(stroke: KeyStroke, ctx: &Context) -> bool {
    ctx.languages.iter().any(|p| {
        p.layout
            .char_at(stroke.scancode, stroke.modifiers)
            .is_some_and(is_word_char)
    })
}

/// Гліф роздільника для клавіші: її інтерпретація в розкладці, де вона **НЕ
/// літера** (тобто пунктуація/роздільник), якщо така є. Саме цей символ
/// користувач і хотів надрукувати, натиснувши клавішу-роздільник (`,` для
/// `0x33`), незалежно від того, чи ПОТОЧНА розкладка показала там літеру (`б`).
/// Детермінованість — порядок `ctx.languages`.
fn separator_glyph(stroke: KeyStroke, ctx: &Context) -> Option<char> {
    ctx.languages.iter().find_map(
        |p| match p.layout.char_at(stroke.scancode, stroke.modifiers) {
            Some(ch) if !is_word_char(ch) => Some(ch),
            _ => None,
        },
    )
}

/// Скільки ХВОСТОВИХ страйків — «пунктуація-що-є-літерою»: клавіша, що є ЛІТЕРОЮ
/// хоч у одній розкладці І роздільником (не літерою) хоч у одній. **Симетрично:**
/// байдуже, поточна то розкладка чи кандидатна — той самий `,`=`б`(0x33) ловиться
/// і коли поточна EN (на екрані `,`), і коли поточна UK (на екрані `б`). Лише такі
/// хвостові страйки гілка-роздільник може стрипнути.
///
/// Виключає: чисті літери (літера в УСІХ розкладках — ніколи не роздільник) і
/// чисті символи/цифри (не літера НІДЕ — тверда межа, у буфер не потрапляють).
fn trailing_separator_candidates(strokes: &[KeyStroke], ctx: &Context) -> usize {
    if ctx.current_profile().is_none() {
        return 0;
    }
    let mut k = 0;
    for s in strokes.iter().rev() {
        // Пунктуація-що-є-літерою = літера в одній розкладці І роздільник в іншій.
        if letter_in_any_layout(*s, ctx) && separator_glyph(*s, ctx).is_some() {
            k += 1;
        } else {
            break;
        }
    }
    k
}

/// Внутрішня оцінка однієї гілки інтерпретації страйків (без урахування хвостової
/// дизамбігуації — її робить [`decide`]).
struct BranchEval {
    best: LayoutId,
    best_text: String,
    current_text: String,
    switch: bool,
    confidence: f64,
    /// Чи `best_text` є у словнику обраної мови (потрібно для precision-замка).
    best_is_dict: bool,
}

/// Оцінити одну послідовність страйків в усіх розкладках і вирішити (за тими ж
/// правилами порогу/довжини/veto, що й раніше), чи варто перемикати.
fn eval_branch(strokes: &[KeyStroke], ctx: &Context) -> BranchEval {
    let cfg = &ctx.config;

    let current = ctx.current_profile();
    let current_text = current
        .map(|p| p.layout.interpret(strokes))
        .unwrap_or_default();
    let current_score = current
        .map(|p| p.score(&current_text, cfg, ctx.rules.recognizes(&current_text)))
        .unwrap_or(CandidateScore {
            total: f64::NEG_INFINITY,
            lm: f64::NEG_INFINITY,
            freq: 0.0,
        });

    // Чи ПОТОЧНИЙ текст (двійник у вихідній мові) — реальне слово (словник АБО
    // особистий словник). Потрібно для дзеркальної релаксації: форсимо коротке
    // лише коли двійник НЕ справжнє слово (легітимний короткий ввід не чіпати).
    let current_is_dict = current
        .map(|p| p.dict.contains(&current_text) || ctx.rules.recognizes(&current_text))
        .unwrap_or(false);

    // Початково найкраща — поточна (щоб за відсутності переваги нічого не міняти).
    let mut best = ctx.current_layout.clone();
    let mut best_text = current_text.clone();
    let mut best_score = current_score;
    let mut best_is_dict = current_is_dict;

    for p in ctx.languages {
        let text = p.layout.interpret(strokes);
        let recognized = ctx.rules.recognizes(&text);
        let sc = p.score(&text, cfg, recognized);
        if sc.total > best_score.total {
            best_score = sc;
            best = p.id.clone();
            best_is_dict = p.dict.contains(&text) || recognized;
            best_text = text;
        }
    }

    // **Forex: валютна пара в кандидатній (латинській) розкладці — сильний сигнал
    // «це англійське, перемикай на латиницю».** `is_currency_pair` пропускає лише
    // 6-літерний токен, де ОБИДВІ половини — валідні ISO-коди (EURUSD); кирилична
    // інтерпретація (не-ASCII) ніколи не матчить, тож скануємо всі кандидати
    // безпечно. Фільтр `p.id != current_layout` лишає КОРЕКТНО набрану латиницею
    // пару недоторканою (best == current → не перемикаємо). Перемога forex форсить
    // перемикання в обхід порогу/довжини (але НЕ veto і НЕ best≠current).
    let forex = ctx
        .languages
        .iter()
        .filter(|p| p.id != ctx.current_layout)
        .find_map(|p| {
            let text = p.layout.interpret(strokes);
            if ctx.rules.is_currency_pair(&text) {
                let sc = p.score(&text, cfg, false);
                Some((p.id.clone(), text, sc))
            } else {
                None
            }
        });
    let forex_forced = forex.is_some();
    if let Some((lang, text, sc)) = forex {
        best = lang;
        best_text = text;
        best_is_dict = true;
        best_score = sc; // когерентна confidence у звіті (рішення дає forex_forced)
    }

    // **Файлові розширення: EN-двійник кандидата — відоме розширення (`txt`/`md`) →
    // сигнал «це латиниця».** Гейт precision: укр. читання НЕ реальне слово
    // (`!current_is_dict`, включає особистий словник) — інакше ризикові розширення-
    // слова (`doc`/`log`/`go`), чий укр.-двійник міг би бути валідним, ламали б
    // легітимний ввід (гейт, НЕ список винятків). Фільтр `p.id != current_layout`
    // лишає коректно набране в EN недоторканим. Форсить в обхід порогу/довжини.
    let extension = (cfg.extensions_enabled && !forex_forced && !current_is_dict)
        .then(|| {
            ctx.languages
                .iter()
                .filter(|p| p.id != ctx.current_layout)
                .find_map(|p| {
                    let text = p.layout.interpret(strokes);
                    if ctx.rules.is_known_extension(&text) {
                        let sc = p.score(&text, cfg, false);
                        Some((p.id.clone(), text, sc))
                    } else {
                        None
                    }
                })
        })
        .flatten();
    let extension_forced = extension.is_some();
    if let Some((lang, text, sc)) = extension {
        best = lang;
        best_text = text;
        best_is_dict = false; // розширення — не словникове слово
        best_score = sc;
    }

    // **Фонотактика: поточне укр. читання починається з «ь» — НЕМОЖЛИВЕ як
    // українське (нема укр. слів на «ь»).** Сильний НЕГАТИВНИЙ сигнал на укр.
    // читання → перемкнути на найправдоподібніший латинський двійник. Гейт
    // precision: його LM має перевищувати LM (провального) укр. читання на запас —
    // інакше обидва боки сміття (нічого не форсимо). Форсить в обхід порогу/довжини.
    let phonotactic_forced = if cfg.phonotactics_enabled
        && !forex_forced
        && !extension_forced
        && starts_impossible_uk(&current_text)
    {
        let alt = ctx
            .languages
            .iter()
            .filter(|p| p.id != ctx.current_layout)
            .map(|p| {
                let text = p.layout.interpret(strokes);
                let recognized = ctx.rules.recognizes(&text);
                let sc = p.score(&text, cfg, recognized);
                let is_dict = p.dict.contains(&text) || recognized;
                (p.id.clone(), text, sc, is_dict)
            })
            .max_by(|a, b| {
                a.2.lm
                    .partial_cmp(&b.2.lm)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match alt {
            Some((lang, text, sc, is_dict))
                if sc.lm > current_score.lm + cfg.phonotactic_twin_lm_margin =>
            {
                best = lang;
                best_text = text;
                best_is_dict = is_dict;
                best_score = sc;
                true
            }
            _ => false,
        }
    } else {
        false
    };

    let len = current_text.chars().count();
    let confidence = best_score.total - current_score.total;
    // LM-перевага кандидата БЕЗ бонусу словника: для дуже коротких слів вимагаємо,
    // щоб за кандидата голосувала і сама модель, а не лише збіг у словнику.
    let lm_confidence = best_score.lm - current_score.lm;
    // ЧАСТОТНА перевага кандидата (надбавки log-ймовірності, обидві ≥ 0): коли
    // обидва двійники — реальні слова (LM майже рівні, dict-бонуси скасовуються),
    // саме частота розрізняє часте слово від рідкісного (`ну`≫`ye`). Це другий —
    // частотний — спосіб «проголосувати» за коротке слово, поряд із LM-маржею.
    let freq_confidence = best_score.freq - current_score.freq;
    let short_word_ok = len > cfg.short_word_max_len
        || lm_confidence > cfg.short_word_lm_margin
        || freq_confidence > cfg.short_word_freq_margin;

    // Правила рівня слова: veto (захист precision) має пріоритет; force дозволяє
    // перемкнути в обхід порогу/довжини (але не в обхід veto чи best≠current).
    let vetoed = ctx.rules.vetoes(&current_text, &best_text);
    let forced = ctx.rules.forces(&current_text);

    // Стандартний шлях: довжина + поріг + (для коротких) LM-перевага кандидата.
    let standard_ok = len >= cfg.min_switch_len && confidence > cfg.threshold(len) && short_word_ok;

    // **Дзеркальна релаксація для дуже коротких слів (len ≤ short_word_max_len).**
    // Принцип «справжнє слово ↔ біліберда»: перемикаємо коротке, якщо
    //   (а) кандидат — куроване СЛУЖБОВЕ слово цільової мови (whitelist у `rules`,
    //       не довільний збіг у повному словнику — інакше шум `ат`/`ді` від
    //       корпусу пробивав би поріг, як код-токени `fn`/`ls`); І
    //   (б) джерельний двійник НЕ справжнє слово: його немає у словнику вихідної
    //       мови (`!current_is_dict`) І його LM нижча за стелю (фонотактично
    //       неправдоподібний). Тоді `і`/`ти`/`чи` форсяться на одиночний
    //       dict-hit, а реальні короткі `is`/`to`/`db` — НІ (їхній двійник — теж
    //       справжнє слово → умова (б) хибна). Поріг/min_len/LM-маржу обходимо
    //       (одиночного dict-hit достатньо), але НЕ veto і НЕ `best≠current`.
    let mirror_ok = len <= cfg.short_word_max_len
        && best_is_dict
        && ctx.rules.is_short_service(&best, &best_text)
        && !current_is_dict
        && current_score.lm < cfg.short_word_twin_lm_max
        && confidence > cfg.base_threshold;

    let switch = current.is_some()
        && best != ctx.current_layout
        && !vetoed
        && (forced
            || standard_ok
            || mirror_ok
            || forex_forced
            || extension_forced
            || phonotactic_forced);

    BranchEval {
        best,
        best_text,
        current_text,
        switch,
        confidence,
        best_is_dict,
    }
}

/// Розпізнати помилку **перетриманого Shift** і повернути нормалізований регістр.
///
/// Патерн: слово має **префікс із 2+ великих літер**, після якого йде хоч одна
/// мала (`ПРивіт`, `ПРИвіт`, `HEllo`). Нормалізація — лишити ВЕЛИКОЮ лише першу
/// літеру, решту префікса зробити малими → `Привіт`/`Hello`.
///
/// **Precision-замок (ключовий розрізнювач — словник):** повертаємо `Some` ЛИШЕ
/// якщо нормалізований варіант — РЕАЛЬНЕ слово у словнику поточної мови. Інакше
/// `None` (не чіпаємо). Саме так помилка регістру відрізняється від навмисної
/// абревіатури: `ПРивіт`→`Привіт` (реальне слово ✅), а `EAs`→`Eas` (не слово →
/// НЕ чіпаємо, бо це `EA`+`s`).
///
/// **Не-патерни (повертають `None`):** слово ПОВНІСТЮ велике (`ПРИВІТ`, `EA`,
/// `USD` — немає малих → навмисний капс/акронім), одна велика + малі (`Привіт`
/// — уже коректно), повністю мале (`привіт`).
fn overheld_shift_fix(word: &str, current: &LanguageProfile) -> Option<String> {
    let chars: Vec<char> = word.chars().collect();
    // Лідируючий run великих літер.
    let upper_prefix = chars.iter().take_while(|c| c.is_uppercase()).count();
    if upper_prefix < 2 {
        // 0 великих (`привіт`) або 1 (`Привіт` — уже коректно) → не патерн.
        return None;
    }
    // Має бути хоч одна мала літера: інакше це ALL-CAPS (`ПРИВІТ`/`USD`) →
    // навмисний капс/акронім, не чіпаємо.
    if !chars.iter().any(|c| c.is_lowercase()) {
        return None;
    }
    // Нормалізація: перша літера лишається як є (велика), решту — у нижній регістр
    // (хвіст уже малий, тож це міняє лише «зайві» великі префікса).
    let mut normalized = String::with_capacity(word.len());
    for (i, c) in chars.iter().enumerate() {
        if i == 0 {
            normalized.push(*c);
        } else {
            normalized.extend(c.to_lowercase());
        }
    }
    // Precision-замок: лише якщо нормалізований варіант — реальне слово.
    if normalized != *word && current.dict.contains(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

/// Накласти корекцію регістру (перетриманий Shift) на рішення, що НЕ перемикає
/// розкладку. Якщо детектор уже вирішив перемкнути мову — це основний кейс, його
/// лишаємо (комбінований layout+caps кейс — свідомий follow-up, без подвійних
/// суперечливих дій). Інакше: слово вже в правильній мові, і якщо його регістр
/// має патерн перетриманого Shift, перетворюємо рішення на чисту caps-корекцію.
fn apply_caps_fix(mut d: Decision, ctx: &Context) -> Decision {
    if d.switch {
        return d; // розкладко-перемикання — основний кейс, caps не нашаровуємо
    }
    let Some(current) = ctx.current_profile() else {
        return d;
    };
    if let Some(fixed) = overheld_shift_fix(&d.current_text, current) {
        // Veto захищає precision і тут (слово, яке користувач уже відкидав).
        if ctx.rules.vetoes(&d.current_text, &fixed) {
            return d;
        }
        d.best = ctx.current_layout.clone();
        d.best_text = fixed;
        d.switch = true;
        d.caps_only = true;
    }
    d
}

/// Розглянути буферизоване слово й вирішити, чи перемикати.
///
/// `strokes` — фізичні натискання слова (layout-незалежні). Якщо поточної
/// розкладки немає серед `ctx.languages`, безпечно не перемикаємо (не знаємо,
/// що саме на екрані → не можна коректно стерти).
///
/// ## Дизамбігуація пунктуації-що-є-літерою на межі слова
/// Якщо у хвості буфера є клавіші, що в поточній розкладці виглядають як
/// пунктуація, але в кандидатній є літерами (`,`=`б`, `.`=`ю`, `;`=`ж`, `[`=`х`,
/// `]`=`ї`, `\`=`ґ`), розглядаємо ДВІ гілки:
/// - **гілка-літера:** усі страйки — літери (`lj,ht`→`добре`);
/// - **гілка-роздільник:** хвостові пунктуаційні страйки стрипнуто й трактовано
///   як роздільники (`ghbdsn,`→`привіт` + суфікс `","`).
///
/// **Precision-замок:** гілку-літеру приймаємо ЛИШЕ якщо вона перемикає, дає
/// СЛОВНИКОВЕ слово й має вищу впевненість за гілку-роздільник. Інакше —
/// безпечна гілка-роздільник (стара поведінка: пунктуація лишається роздільником).
/// За рівної впевненості перемагає гілка-роздільник (консервативно). Внутрішня
/// пунктуація-літера завжди лишається літерою (стрипаємо тільки хвіст).
pub fn decide(strokes: &[KeyStroke], ctx: &Context) -> Decision {
    let k = trailing_separator_candidates(strokes, ctx);
    let letter = eval_branch(strokes, ctx);

    // Немає хвостової пунктуації-літери → одна гілка (стара поведінка, без суфікса).
    if k == 0 {
        return apply_caps_fix(
            Decision {
                best: letter.best,
                best_text: letter.best_text,
                current_text: letter.current_text,
                switch: letter.switch,
                confidence: letter.confidence,
                suffix: String::new(),
                caps_only: false,
            },
            ctx,
        );
    }

    let split = strokes.len() - k;
    let sep = eval_branch(&strokes[..split], ctx);
    // Суфікс — гліф РОЗДІЛЬНИКА для кожного хвостового страйка (`,`, а не `б`):
    // те, що користувач хотів надрукувати, і що має лишитись на екрані після
    // перенабору в обраній мові. Та сама к-сть символів, що й на екрані зараз
    // (один страйк → один символ у будь-якій розкладці), тож стирання коректне.
    // Резерв (на випадок незмапованого символу) — поточна інтерпретація.
    let current_layout = ctx.current_profile().map(|p| &p.layout);
    let suffix: String = strokes[split..]
        .iter()
        .filter_map(|s| {
            separator_glyph(*s, ctx)
                .or_else(|| current_layout.and_then(|l| l.char_at(s.scancode, s.modifiers)))
        })
        .collect();

    // **Precision-замок (послаблений на користь ПОВНОГО словникового слова).**
    // Ми вже в гілці `k > 0`, тобто хвостові страйки — пунктуація-що-є-літерою
    // (`б→,`, `х→[`, `ж→;`, `ю→.`, `ї→]`, `ґ→\`). Хвостовий стрип існує для
    // СПРАВЖНЬОГО роздільника (`ghbdsn,`→`привіт`+`,`), але коли укр. слово
    // ЗАКІНЧУЄТЬСЯ такою літерою (`щоб`: `б→,`), стрип хибно дробить слово й губить
    // літеру — навіть коли коротший префікс (`що`) сам словниковий і ЧАСТОТНІШИЙ,
    // тож має вищу raw-confidence. Тому: якщо letter-гілка дає СЛОВНИКОВЕ слово й
    // сама перемикає — віддаємо їй перевагу НАВІТЬ за нижчої впевненості (повне
    // валідне слово б'є коротший префікс+«роздільник»). Гейт `best_is_dict` —
    // це й є замок precision: `ghbdsn,`→мірор `привітб` НЕ в словнику → гілка-
    // роздільник лишається (`привіт,`), нуль нового хибного злиття.
    let use_letter = letter.switch && letter.best_is_dict;

    let decision = if use_letter {
        Decision {
            best: letter.best,
            best_text: letter.best_text,
            current_text: letter.current_text,
            switch: letter.switch,
            confidence: letter.confidence,
            suffix: String::new(),
            caps_only: false,
        }
    } else {
        Decision {
            best: sep.best,
            best_text: sep.best_text,
            current_text: sep.current_text,
            switch: sep.switch,
            confidence: sep.confidence,
            suffix,
            caps_only: false,
        }
    };
    apply_caps_fix(decision, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyCap, Modifiers};

    // Маленькі ручні профілі (без IO): достатньо клавіш для тестових слів.
    // Фізичні позиції (set 1): G=0x22 H=0x23 B=0x30 D=0x20 S=0x1F N=0x31,
    // плюс кілька для коротких слів: O=0x18, T=0x14.
    fn en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x22, KeyCap::letter('g', 'G')),
                (0x23, KeyCap::letter('h', 'H')),
                (0x30, KeyCap::letter('b', 'B')),
                (0x20, KeyCap::letter('d', 'D')),
                (0x1F, KeyCap::letter('s', 'S')),
                (0x31, KeyCap::letter('n', 'N')),
                (0x18, KeyCap::letter('o', 'O')),
                (0x14, KeyCap::letter('t', 'T')),
            ],
        );
        let lm = NgramModel::train("hello world good night not to be on go", 3, 0.5);
        let dict =
            Dictionary::from_words(["hello", "world", "good", "night", "not", "to", "on", "go"])
                .unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn uk_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x22, KeyCap::letter('п', 'П')),
                (0x23, KeyCap::letter('р', 'Р')),
                (0x30, KeyCap::letter('и', 'И')),
                (0x20, KeyCap::letter('в', 'В')),
                (0x1F, KeyCap::letter('і', 'І')),
                (0x31, KeyCap::letter('т', 'Т')),
                (0x18, KeyCap::letter('щ', 'Щ')),
                (0x14, KeyCap::letter('е', 'Е')),
            ],
        );
        let lm = NgramModel::train(
            "привіт світ як справи добрий день привіт друже все добре привіт",
            3,
            0.5,
        );
        let dict =
            Dictionary::from_words(["привіт", "світ", "друже", "добре", "день", "п"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn strokes(scancodes: &[u32]) -> Vec<KeyStroke> {
        scancodes
            .iter()
            .map(|&sc| KeyStroke::new(sc, Modifiers::empty()))
            .collect()
    }

    /// Страйки з керованим SHIFT на кожному (для тестів регістру): `(scancode,
    /// shift?)`. SHIFT → велика літера в розкладці (`char_at` застосовує його).
    fn strokes_shift(items: &[(u32, bool)]) -> Vec<KeyStroke> {
        items
            .iter()
            .map(|&(sc, shift)| {
                let m = if shift {
                    Modifiers::SHIFT
                } else {
                    Modifiers::empty()
                };
                KeyStroke::new(sc, m)
            })
            .collect()
    }

    // --- Профілі для регресій коротких код-токенів --------------------------
    // Відтворюють реальні FP калібрування (`fn`→«ат», `ls`→«ді`): двосимвольний
    // англ. код-токен випадково збігається з коротким словом у uk-словнику, але
    // мовна модель за uk-кандидата НЕ голосує. Контролюємо і словник, і LM.
    fn en_code_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x21, KeyCap::letter('f', 'F')),
                (0x31, KeyCap::letter('n', 'N')),
                (0x26, KeyCap::letter('l', 'L')),
                (0x1F, KeyCap::letter('s', 'S')),
                (0x16, KeyCap::letter('u', 'U')),
                (0x24, KeyCap::letter('j', 'J')),
            ],
        );
        // Англ. корпус/словник без коротких токенів fn/ls/uj.
        let lm = NgramModel::train("function list please value name return", 3, 0.5);
        let dict =
            Dictionary::from_words(["function", "list", "please", "value", "name", "return"])
                .unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn uk_short_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x21, KeyCap::letter('а', 'А')),
                (0x31, KeyCap::letter('т', 'Т')),
                (0x26, KeyCap::letter('д', 'Д')),
                (0x1F, KeyCap::letter('і', 'І')),
                (0x16, KeyCap::letter('т', 'Т')),
                (0x24, KeyCap::letter('о', 'О')),
            ],
        );
        // «то» — часте плинне слово (сильна LM); «ат`/«ді» — у словнику, але як
        // слова в корпусі не трапляються (слабка LM) → патерн реальних FP.
        // «то» — дуже часте плинне слово (сильна LM). «ат`/«ді» теж присутні, але
        // рідко → LM за них голосує лише ледь-ледь (мала, але >0 перевага), як у
        // реальних FP: повний бал (з dict_bonus) пробиває поріг, а LM — ні.
        let lm = NgramModel::train(
            "то це то воно то так то добре то знову то усе то напевно то ат ді",
            3,
            0.5,
        );
        // `о` — однолітерне службове слово для тестів дзеркальної релаксації.
        let dict = Dictionary::from_words(["ат", "ді", "то", "о"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    use crate::{ExclusionRules, WordRules};

    static NO_EXCL: ExclusionRules = ExclusionRules::new();
    static NO_RULES: WordRules = WordRules::new();

    fn ctx_with<'a>(langs: &'a [LanguageProfile], current: &str) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: LayoutId::new(current),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        }
    }

    fn ctx_with_config<'a>(
        langs: &'a [LanguageProfile],
        current: &str,
        config: DetectorConfig,
    ) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: LayoutId::new(current),
            languages: langs,
            config,
            exclusions: &NO_EXCL,
            rules: &NO_RULES,
        }
    }

    fn ctx_with_rules<'a>(
        langs: &'a [LanguageProfile],
        current: &str,
        rules: &'a WordRules,
    ) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: LayoutId::new(current),
            languages: langs,
            config: DetectorConfig::default(),
            exclusions: &NO_EXCL,
            rules,
        }
    }

    #[test]
    fn switches_long_gibberish_to_real_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // g h b d s n → en "ghbdsn", uk "привіт".
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(d.switch, "мало перемкнути (confidence={})", d.confidence);
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "привіт");
        assert_eq!(d.current_text, "ghbdsn");
    }

    #[test]
    fn does_not_switch_when_current_is_already_a_real_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // h e then... "hello" need l/o; use "good": g o o d → en "good" (у словнику).
        let d = decide(&strokes(&[0x22, 0x18, 0x18, 0x20]), &ctx);
        assert!(
            !d.switch,
            "реальне англ. слово не чіпати (best={:?})",
            d.best
        );
        assert_eq!(d.current_text, "good");
    }

    #[test]
    fn does_not_switch_short_ambiguous_word() {
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // 2 страйки: en "to" / uk "пе"? → коротке, поріг дуже високий → не чіпати.
        let d = decide(&strokes(&[0x14, 0x18]), &ctx);
        assert!(
            !d.switch,
            "коротке слово не перемикати (confidence={})",
            d.confidence
        );
    }

    #[test]
    fn threshold_is_stricter_for_short_words() {
        let cfg = DetectorConfig::default();
        assert!(cfg.threshold(2) > cfg.threshold(6));
        assert!(cfg.threshold(6) > cfg.threshold(12));
        // Коротше за min_switch_len → нескінченність (ніколи).
        assert_eq!(cfg.threshold(0), f64::INFINITY);
    }

    #[test]
    fn calibrated_short_word_threshold_holds_recall_margin() {
        // Калібрування кейсу B: `short_word_extra=4.0` (а не 6.0) опускає
        // thr(len=3) до ~2.33, щоб ловити правдоподібні не-слова (`rjk`→`кол`,
        // char-LM перевага ~2.43) БЕЗ нових FP. Запас до найгіршого негатива
        // (`vec`→uk, conf≈1.15 на реальному eval) має лишатись ≥ ~1.0. Цей тест
        // ловить випадкову зміну, що з'їла б margin (поріг ≤ ~1.15 → FP).
        let cfg = DetectorConfig::default();
        assert_eq!(cfg.short_word_extra, 4.0, "калібрований запас кейсу B");
        assert_eq!(cfg.base_threshold, 1.0);
        assert!((cfg.threshold(3) - 7.0 / 3.0).abs() < 1e-9, "thr(3)≈2.33");
        assert!(
            cfg.threshold(3) > 1.15,
            "поріг має лишатись вище найгіршого негатива (vec≈1.15)"
        );
    }

    #[test]
    fn no_current_profile_means_no_switch() {
        let langs = [uk_profile()];
        // Поточна "en" відсутня серед профілів → не знаємо, що на екрані.
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(!d.switch);
    }

    #[test]
    fn veto_word_blocks_high_score_switch() {
        let langs = [en_profile(), uk_profile()];
        let mut rules = WordRules::new();
        rules.veto_word("привіт"); // навіть із високим балом — не чіпати
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert!(!d.switch, "veto має заблокувати перемикання");
        assert_eq!(d.best_text, "привіт"); // детектор усе одно бачить кандидата
    }

    #[test]
    fn force_word_switches_below_min_length() {
        let langs = [en_profile(), uk_profile()];
        // 1 символ: g → en "g", uk "п" (є у словнику uk → bonus робить best=uk).
        // Коротше за min_switch_len(2) → БЕЗ force не перемикається ніколи.
        let plain = ctx_with(&langs, "en");
        let d0 = decide(&strokes(&[0x22]), &plain);
        assert!(!d0.switch, "коротке (1 символ) без force не чіпати");

        let mut rules = WordRules::new();
        rules.force_word("g");
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x22]), &ctx);
        assert_eq!(d.best, LayoutId::new("uk"));
        assert!(d.switch, "force має перемкнути попри min length");
    }

    // --- Калібрування порогу: короткий збіг у словнику без підтримки LM -------

    #[test]
    fn short_code_token_fn_does_not_switch_on_lone_dict_hit() {
        // `fn` (en, код) → uk «ат» є у словнику, але LM за неї не голосує.
        // Регресія реального FP калібрування: dict_bonus сам не має перемикати.
        let langs = [en_code_profile(), uk_short_profile()];
        let token = strokes(&[0x21, 0x31]); // en "fn" / uk "ат"

        // Контроль причинності: БЕЗ LM-guard (short_word_max_len=0 → коротким
        // словам особлива вимога не ставиться) повний бал з dict_bonus пробиває
        // поріг і перемикає — це і був FP.
        let no_guard = DetectorConfig {
            short_word_max_len: 0,
            ..DetectorConfig::default()
        };
        let d_old = decide(&token, &ctx_with_config(&langs, "en", no_guard));
        assert_eq!(d_old.current_text, "fn");
        assert_eq!(
            d_old.best_text, "ат",
            "детектор бачить uk-кандидата зі словника"
        );
        assert!(
            d_old.switch,
            "без guard короткий збіг у словнику перемикав — це й був FP (conf={})",
            d_old.confidence
        );

        // З дефолтним guard саме LM-вимога блокує перемикання → precision збережено.
        let d = decide(&token, &ctx_with(&langs, "en"));
        assert!(
            !d.switch,
            "короткий код-токен не перемикати без підтримки LM (conf={})",
            d.confidence
        );
    }

    #[test]
    fn short_code_token_ls_does_not_switch_on_lone_dict_hit() {
        // `ls` (en, код) → uk «ді`: другий реальний FP калібрування.
        let langs = [en_code_profile(), uk_short_profile()];
        let token = strokes(&[0x26, 0x1F]); // en "ls" / uk "ді"

        let no_guard = DetectorConfig {
            short_word_max_len: 0,
            ..DetectorConfig::default()
        };
        let d_old = decide(&token, &ctx_with_config(&langs, "en", no_guard));
        assert_eq!(d_old.current_text, "ls");
        assert_eq!(d_old.best_text, "ді");
        assert!(
            d_old.switch,
            "без guard `ls` перемикав на «ді» — це й був FP (conf={})",
            d_old.confidence
        );

        let d = decide(&token, &ctx_with(&langs, "en"));
        assert!(
            !d.switch,
            "короткий код-токен `ls` не перемикати (conf={})",
            d.confidence
        );
    }

    #[test]
    fn short_word_with_lm_support_still_switches() {
        // Позитивний контроль: двосимвольне «то» (сильна LM + словник) — за нього
        // голосує і модель, не лише словник → коротке слово ВСЕ ОДНО перемикається.
        let langs = [en_code_profile(), uk_short_profile()];
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x16, 0x24]), &ctx); // en "uj" / uk "то"
        assert_eq!(d.current_text, "uj");
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "то");
        assert!(
            d.switch,
            "коротке слово з підтримкою LM має перемкнутися (conf={})",
            d.confidence
        );
    }

    // --- Дзеркальна релаксація для коротких службових слів --------------------

    /// Whitelist коротких службових слів (через `WordRules`) для тестів дзеркала.
    fn rules_with_short_service(entries: &[(&str, &str)]) -> WordRules {
        let mut r = WordRules::new();
        for (lang, word) in entries {
            r.allow_short_service(&LayoutId::new(*lang), word);
        }
        r
    }

    #[test]
    fn mirror_switches_whitelisted_short_service_word() {
        // `fn` (en, не в en-словнику, слабка LM) → uk «ат» у словнику. САМОГО
        // dict-hit мало (стандартний guard блокує), але якщо «ат» у whitelist
        // службових слів — дзеркало форсить перемикання на одиночний dict-hit.
        let langs = [en_code_profile(), uk_short_profile()];
        let rules = rules_with_short_service(&[("uk", "ат")]);
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x21, 0x31]), &ctx); // en "fn" / uk "ат"
        assert_eq!(d.current_text, "fn");
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "ат");
        assert!(
            d.switch,
            "whitelisted службове слово має перемкнутися на dict-hit (conf={})",
            d.confidence
        );
    }

    #[test]
    fn mirror_requires_whitelist_not_lone_dict_hit() {
        // Той самий `fn`→«ат», але «ат» НЕ у whitelist (whitelist має інше слово).
        // Дзеркало НЕ спрацьовує: довільний збіг у повному словнику (шум корпусу
        // `ат`/`ді`) не має перемикати — інакше код-токени ламали б precision.
        let langs = [en_code_profile(), uk_short_profile()];
        let rules = rules_with_short_service(&[("uk", "то")]); // «ат» відсутнє
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x21, 0x31]), &ctx); // en "fn" / uk "ат"
        assert!(
            !d.switch,
            "не-whitelisted збіг у словнику не має перемикати (conf={})",
            d.confidence
        );
    }

    #[test]
    fn mirror_does_not_switch_when_twin_is_real_word() {
        // Дзеркальна умова (б): джерельний двійник МУСИТЬ бути не-словом. Тут
        // робимо `fn` реальним en-словом (додаємо в en-словник) — навіть із
        // whitelisted «ат» дзеркало НЕ спрацьовує (це легітимний короткий ввід).
        let en = {
            let mut p = en_code_profile();
            p.dict = Dictionary::from_words(["function", "list", "fn"]).unwrap();
            p
        };
        let langs = [en, uk_short_profile()];
        let rules = rules_with_short_service(&[("uk", "ат")]);
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x21, 0x31]), &ctx); // en "fn" (тепер слово) / uk "ат"
        assert!(
            !d.switch,
            "двійник-справжнє-слово не має перемикатися (conf={})",
            d.confidence
        );
    }

    #[test]
    fn mirror_switches_one_letter_service_word() {
        // Однолітерне службове слово: `j`(en, не-слово) → «о»(uk, whitelist).
        // min_switch_len(2) звичайно блокує len=1; дзеркало це обходить.
        let langs = [en_code_profile(), uk_short_profile()];
        let rules = rules_with_short_service(&[("uk", "о")]);
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x24]), &ctx); // en "j" / uk "о"
        assert_eq!(d.current_text, "j");
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "о");
        assert!(d.switch, "однолітерне службове слово має перемкнутися");

        // Без whitelist — лишається заблокованим (контроль).
        let plain = ctx_with(&langs, "en");
        assert!(!decide(&strokes(&[0x24]), &plain).switch);
    }

    // --- Корекція регістру (помилка перетриманого Shift) ---------------------
    // Фізичні позиції (set 1): H=0x23 E=0x12 L=0x26 O=0x18 A=0x1E S=0x1F.

    /// En-профіль для тестів регістру: має літери для `hello`/`eas`-кейсів.
    fn caps_en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x23, KeyCap::letter('h', 'H')),
                (0x12, KeyCap::letter('e', 'E')),
                (0x26, KeyCap::letter('l', 'L')),
                (0x18, KeyCap::letter('o', 'O')),
                (0x1E, KeyCap::letter('a', 'A')),
                (0x1F, KeyCap::letter('s', 'S')),
            ],
        );
        let lm = NgramModel::train("hello world good", 3, 0.5);
        // `eas` НАВМИСНО відсутнє → `EAs`→`Eas` не пройде precision-замок.
        let dict = Dictionary::from_words(["hello"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    #[test]
    fn caps_fix_uk_two_uppercase_prefix() {
        // `ПРивіт`→`Привіт`: 2 великі на початку, решта малі, реальне укр. слово.
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "uk");
        // п р и в і т (0x22 0x23 0x30 0x20 0x1F 0x31), SHIFT на перших двох.
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, true),
                (0x30, false),
                (0x20, false),
                (0x1F, false),
                (0x31, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "ПРивіт");
        assert!(d.switch, "має виправити регістр");
        assert!(d.caps_only, "це чиста caps-корекція, без зміни розкладки");
        assert_eq!(d.best, LayoutId::new("uk"), "та сама розкладка");
        assert_eq!(d.best_text, "Привіт");
    }

    #[test]
    fn caps_fix_uk_three_uppercase_prefix() {
        // `ПРИвіт`→`Привіт`: 3 великі на початку.
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "uk");
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, true),
                (0x30, true),
                (0x20, false),
                (0x1F, false),
                (0x31, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "ПРИвіт");
        assert!(d.switch && d.caps_only);
        assert_eq!(d.best_text, "Привіт");
    }

    #[test]
    fn caps_fix_en_hello() {
        // `HEllo`→`Hello`: працює і для латиниці.
        let langs = [caps_en_profile()];
        let ctx = ctx_with(&langs, "en");
        let d = decide(
            &strokes_shift(&[
                (0x23, true),
                (0x12, true),
                (0x26, false),
                (0x26, false),
                (0x18, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "HEllo");
        assert!(d.switch && d.caps_only);
        assert_eq!(d.best, LayoutId::new("en"));
        assert_eq!(d.best_text, "Hello");
    }

    #[test]
    fn caps_no_fix_all_caps() {
        // `ПРИВІТ` — повністю велике (навмисний капс) → не чіпати.
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "uk");
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, true),
                (0x30, true),
                (0x20, true),
                (0x1F, true),
                (0x31, true),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "ПРИВІТ");
        assert!(!d.switch, "ALL-CAPS не чіпати (conf={})", d.confidence);
    }

    #[test]
    fn caps_no_fix_non_word_abbrev() {
        // `EAs`→норм. `Eas` — НЕ слово (це абревіатура `EA`+`s`) → не чіпати.
        let langs = [caps_en_profile()];
        let ctx = ctx_with(&langs, "en");
        let d = decide(
            &strokes_shift(&[(0x12, true), (0x1E, true), (0x1F, false)]),
            &ctx,
        );
        assert_eq!(d.current_text, "EAs");
        assert!(
            !d.switch,
            "норм. варіант не у словнику → не виправляти (precision-замок)"
        );
    }

    #[test]
    fn caps_no_fix_already_correct() {
        // `Привіт` — одна велика + малі → вже коректно, не чіпати.
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "uk");
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, false),
                (0x30, false),
                (0x20, false),
                (0x1F, false),
                (0x31, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "Привіт");
        assert!(!d.switch, "вже коректний регістр не чіпати");
    }

    #[test]
    fn caps_no_fix_plain_lowercase() {
        // `привіт` — повністю мале → не патерн перетриманого Shift.
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "uk");
        let d = decide(&strokes(&[0x22, 0x23, 0x30, 0x20, 0x1F, 0x31]), &ctx);
        assert_eq!(d.current_text, "привіт");
        assert!(!d.switch, "коректне мале слово не чіпати");
    }

    #[test]
    fn caps_veto_blocks_correction() {
        // Veto захищає precision і для caps-корекції.
        let langs = [en_profile(), uk_profile()];
        let mut rules = WordRules::new();
        rules.veto_word("привіт"); // забороняє і виправлення регістру
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, true),
                (0x30, false),
                (0x20, false),
                (0x1F, false),
                (0x31, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "ПРивіт");
        assert!(!d.switch, "veto має заблокувати й caps-корекцію");
    }

    #[test]
    fn combined_layout_and_caps_does_layout_switch_only() {
        // Комбінований кейс (слово і в неправильній розкладці, і з перетриманим
        // Shift): основний кейс — розкладка. Caps-корекція НЕ нашаровується
        // (свідомий follow-up: без подвійних суперечливих дій). Перенабір дає
        // слово в правильній МОВІ, але з тим самим регістром (`ПРивіт`).
        let langs = [en_profile(), uk_profile()];
        let ctx = ctx_with(&langs, "en");
        // У EN з SHIFT на перших двох: "GHbdsn"; у UK: "ПРивіт".
        let d = decide(
            &strokes_shift(&[
                (0x22, true),
                (0x23, true),
                (0x30, false),
                (0x20, false),
                (0x1F, false),
                (0x31, false),
            ]),
            &ctx,
        );
        assert_eq!(d.current_text, "GHbdsn");
        assert!(d.switch, "перемикання розкладки — основний кейс");
        assert!(!d.caps_only, "це layout-перемикання, не caps-корекція");
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "ПРивіт", "регістр зберігається (follow-up)");
    }

    // --- Частотно-зважений dict-бонус (`score`, `FrequencyMap`) ---------------
    // Цільовий баг: `ну`↔`ye` — ОБИДВА реальні слова у словниках (dict-бонуси
    // скасовуються, LM ~рівні) → бінарний шар не перемикав. Частота розрізняє.
    // Фіз-позиції: sc 0x15 (en `y` / uk `н`), 0x12 (en `e` / uk `у`).

    /// Зібрати `FrequencyMap` зі списку (слово, count) — герметично, без typofix-data.
    fn freq_map(entries: &[(&str, u64)]) -> FrequencyMap {
        use std::collections::BTreeMap;
        let sorted: BTreeMap<String, u64> = entries
            .iter()
            .map(|(w, c)| (w.to_lowercase(), *c))
            .collect();
        FrequencyMap::from_fst_map(fst::Map::from_iter(sorted).unwrap())
    }

    /// En-профіль зі словом `ye` у словнику + керована частотна мапа.
    fn freq_en_profile(freq: Option<FrequencyMap>) -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x15, KeyCap::letter('y', 'Y')),
                (0x12, KeyCap::letter('e', 'E')),
            ],
        );
        // LM бачила «ye» — реальне (архаїчне) слово, тож воно не біліберда.
        let lm = NgramModel::train("ye ye old ye shall ye", 3, 0.5);
        let dict = Dictionary::from_words(["ye", "old"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq,
        }
    }

    /// Uk-профіль зі словом `ну` у словнику + керована частотна мапа.
    fn freq_uk_profile(freq: Option<FrequencyMap>) -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x15, KeyCap::letter('н', 'Н')),
                (0x12, KeyCap::letter('у', 'У')),
            ],
        );
        let lm = NgramModel::train("ну ну та ну добре ну", 3, 0.5);
        let dict = Dictionary::from_words(["ну", "та"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq,
        }
    }

    /// Корпусні масштаби з реального світу: UK-корпус малий, EN — на 2 порядки
    /// більший, тож сирі counts `ну`(12k)≈`ye`(10k), але нормалізована частка
    /// `ну` ≫ `ye`.
    fn uk_freq_common_nu() -> FrequencyMap {
        freq_map(&[("ну", 12_000), ("та", 80_000), ("zzz", 4_300_000)])
    }
    fn en_freq_rare_ye() -> FrequencyMap {
        freq_map(&[("ye", 10_000), ("old", 900_000), ("zzz", 680_000_000)])
    }

    #[test]
    fn freq_switches_nu_over_archaic_ye() {
        // ну(часте)↔ye(рідкісне): обидва у словниках; частота відкриває гейт.
        let langs = [
            freq_en_profile(Some(en_freq_rare_ye())),
            freq_uk_profile(Some(uk_freq_common_nu())),
        ];
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x15, 0x12]), &ctx); // en "ye" / uk "ну"
        assert_eq!(d.current_text, "ye");
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == "ну",
            "часте 'ну' має перемкнутись над рідкісним 'ye' (switch={} best={} conf={:.2})",
            d.switch,
            d.best.as_str(),
            d.confidence
        );
    }

    #[test]
    fn no_freq_layer_leaves_nu_ye_tie_unresolved() {
        // Контроль причинності: без частотного шару той самий кейс НЕ перемикає
        // (бінарні dict-бонуси скасовуються, LM-маржа коротка) — це й був баг.
        let langs = [freq_en_profile(None), freq_uk_profile(None)];
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x15, 0x12]), &ctx);
        assert_eq!(d.current_text, "ye");
        assert!(
            !d.switch,
            "без частоти 'ну'↔'ye' не перемикається (conf={:.2})",
            d.confidence
        );
    }

    #[test]
    fn freq_precision_guard_common_en_stays_when_uk_twin_rare() {
        // Дзеркало кейсу: тепер РІДКІСНЕ укр. «ну», ЧАСТЕ англ. «ye» (на екрані,
        // поточна en). Частота захищає легітимний англ. ввід → НЕ перемикати.
        let uk_rare = freq_map(&[("ну", 30), ("та", 80_000), ("zzz", 4_300_000)]);
        let en_common = freq_map(&[("ye", 5_000_000), ("old", 900_000), ("zzz", 680_000_000)]);
        let langs = [
            freq_en_profile(Some(en_common)),
            freq_uk_profile(Some(uk_rare)),
        ];
        let ctx = ctx_with(&langs, "en");
        let d = decide(&strokes(&[0x15, 0x12]), &ctx);
        assert_eq!(d.current_text, "ye");
        assert!(
            !d.switch,
            "часте англ. 'ye' (рідкісний укр. двійник) НЕ чіпати (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
    }

    #[test]
    fn dict_member_without_freq_keeps_baseline_bonus() {
        // Семантика None ≠ «не слово»: слово Є у словнику, але немає freq-запису →
        // отримує рівно baseline `dict_bonus`, частота НЕ карає. Перевіряємо, що
        // бал = lm + dict_bonus (freq-надбавка = 0), а не нижче.
        let cfg = DetectorConfig::default();
        // Мапа без слова «ну» (інші слова є) → log_prob(ну)=None → надбавка 0.
        let map = freq_map(&[("та", 80_000), ("zzz", 4_300_000)]);
        let uk = freq_uk_profile(Some(map));
        let with_freq = uk.score("ну", &cfg, false);
        let baseline = freq_uk_profile(None).score("ну", &cfg, false);
        assert_eq!(
            with_freq.total, baseline.total,
            "dict-член без freq-запису має лишатись на baseline (без штрафу)"
        );
        assert_eq!(with_freq.freq, 0.0, "немає freq-запису → надбавка 0");
        // А слово, що Є в мапі й часте, отримує СТРОГО більший бал.
        let common = freq_uk_profile(Some(uk_freq_common_nu())).score("ну", &cfg, false);
        assert!(
            common.total > baseline.total && common.freq > 0.0,
            "часте слово з freq-записом має давати більший бонус за baseline"
        );
    }

    #[test]
    fn freq_floor_clamps_rare_words_to_baseline() {
        // Слово у мапі, але РІДКІСНЕ (log-ймовірність нижча за freq_floor) →
        // надбавка обрізається до 0 (шум корпусу не дає переваги).
        let cfg = DetectorConfig::default();
        // ну з мізерним count у великому корпусі → lp ≪ floor(-9).
        let rare = freq_map(&[("ну", 2), ("zzz", 4_300_000)]);
        let s = freq_uk_profile(Some(rare)).score("ну", &cfg, false);
        assert_eq!(s.freq, 0.0, "рідкісне слово нижче freq_floor → надбавка 0");
    }

    // --- Особистий словник (user.txt) = ПОЗИТИВНИЙ сигнал перемикання ----------
    // Фіз-позиції: л=k(0x25), о=j(0x24), х=`[`(0x1A). «лох» закінчується на `х`,
    // що в en — пунктуація `[` → задіює й дизамбігуацію пунктуації-літери.

    fn user_en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x25, KeyCap::letter('k', 'K')),
                (0x24, KeyCap::letter('j', 'J')),
                (0x1A, KeyCap::letter('[', '{')),
            ],
        );
        let lm = NgramModel::train("key just kill keep", 3, 0.5);
        let dict = Dictionary::from_words(["key", "just"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn user_uk_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x25, KeyCap::letter('л', 'Л')),
                (0x24, KeyCap::letter('о', 'О')),
                (0x1A, KeyCap::letter('х', 'Х')),
            ],
        );
        let lm = NgramModel::train("лол хата холод охайо", 3, 0.5);
        // «лох» НАВМИСНО поза стандартним словником (як у реалі: VESUM-фільтр) —
        // ловиться ЛИШЕ через особистий словник.
        let dict = Dictionary::from_words(["хата", "холод"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    #[test]
    fn user_word_switches_via_personal_dictionary() {
        let langs = [user_en_profile(), user_uk_profile()];
        let mut rules = WordRules::new();
        rules.recognize_word("лох");
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&[0x25, 0x24, 0x1A]), &ctx); // en "kj[" / uk "лох"
        assert_eq!(d.best, LayoutId::new("uk"));
        assert_eq!(d.best_text, "лох");
        assert!(
            d.switch,
            "user-слово має перемкнутися (conf={})",
            d.confidence
        );

        // Контроль причинності: без особистого словника «лох» (поза dict) НЕ
        // ловиться — саме user.txt його визнає.
        let plain = ctx_with(&langs, "en");
        let d0 = decide(&strokes(&[0x25, 0x24, 0x1A]), &plain);
        assert!(
            !d0.switch,
            "без user.txt 'лох' (поза словником) не перемикати (conf={})",
            d0.confidence
        );
    }

    // --- Forex: валютна пара = ПОЗИТИВНИЙ сигнал перемикання на латиницю -------
    // Фіз-позиції: e=0x12,u=0x16,r=0x13,s=0x1F,d=0x20; uk-двійники у/г/к/і/в.

    fn forex_en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x12, KeyCap::letter('e', 'E')),
                (0x16, KeyCap::letter('u', 'U')),
                (0x13, KeyCap::letter('r', 'R')),
                (0x1F, KeyCap::letter('s', 'S')),
                (0x20, KeyCap::letter('d', 'D')),
            ],
        );
        let lm = NgramModel::train("euro user reads dress", 3, 0.5);
        let dict = Dictionary::from_words(["euro", "user"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn forex_uk_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x12, KeyCap::letter('у', 'У')),
                (0x16, KeyCap::letter('г', 'Г')),
                (0x13, KeyCap::letter('к', 'К')),
                (0x1F, KeyCap::letter('і', 'І')),
                (0x20, KeyCap::letter('в', 'В')),
            ],
        );
        let lm = NgramModel::train("гра кіно вода рука", 3, 0.5);
        let dict = Dictionary::from_words(["гра", "вода"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn forex_rules(codes: &[&str]) -> WordRules {
        let mut r = WordRules::new();
        for c in codes {
            r.add_currency_code(c);
        }
        r
    }

    // EURUSD на фіз-клавішах: e u r u s d.
    const EURUSD: [u32; 6] = [0x12, 0x16, 0x13, 0x16, 0x1F, 0x20];

    #[test]
    fn forex_pair_switches_from_cyrillic_layout() {
        // Валютна пара, набрана у ВИПАДКОВО ввімкненій UK-розкладці → кирилична
        // каша на екрані; forex впевнено перемикає на латиницю.
        let langs = [forex_en_profile(), forex_uk_profile()];
        let rules = forex_rules(&["EUR", "USD", "GBP"]);
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        let d = decide(&strokes(&EURUSD), &ctx);
        assert_eq!(d.current_text, "угкгів");
        assert_eq!(d.best, LayoutId::new("en"));
        assert_eq!(d.best_text, "eurusd");
        assert!(
            d.switch,
            "валютна пара має перемкнутись (conf={})",
            d.confidence
        );
    }

    #[test]
    fn forex_correct_latin_pair_is_not_touched() {
        // Та сама пара, але вже КОРЕКТНО в EN → не ламати (best==current, фільтр
        // `p.id != current_layout` лишає її недоторканою).
        let langs = [forex_en_profile(), forex_uk_profile()];
        let rules = forex_rules(&["EUR", "USD"]);
        let ctx = ctx_with_rules(&langs, "en", &rules);
        let d = decide(&strokes(&EURUSD), &ctx);
        assert_eq!(d.current_text, "eurusd");
        assert!(!d.switch, "коректну латинську пару не чіпати");
    }

    #[test]
    fn forex_needs_both_halves_iso_no_force_otherwise() {
        // USD НЕ в переліку → «eurusd» не пара → forex НЕ форсить. Доводимо
        // детерміновано: рішення з half-переліком ІДЕНТИЧНЕ рішенню взагалі без
        // forex-правил (адже forex-гілка не спрацювала). Чистоту is_currency_pair
        // окремо стереже юніт у `rules.rs`.
        let langs = [forex_en_profile(), forex_uk_profile()];
        let half = forex_rules(&["EUR"]); // лише половина пари
        let none = WordRules::new();
        let d_half = decide(&strokes(&EURUSD), &ctx_with_rules(&langs, "uk", &half));
        let d_none = decide(&strokes(&EURUSD), &ctx_with_rules(&langs, "uk", &none));
        assert_eq!(
            d_half.switch, d_none.switch,
            "не-пара не повинна форситись як валютна"
        );
        assert_eq!(d_half.best, d_none.best, "forex-гілка не мала змінити best");
    }

    // --- Фонотактика («ь» на початку неможливе) + розширення файлів ------------
    // Фіз-позиції: m=0x32, d=0x20, t=0x14, x=0x2D. uk-двійники: ь, в, е, ч.
    // «md»→«ьв» (старт із «ь»); «txt»→«ече» (відоме розширення).

    fn signal_en_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("en"),
            [
                (0x32, KeyCap::letter('m', 'M')),
                (0x20, KeyCap::letter('d', 'D')),
                (0x14, KeyCap::letter('t', 'T')),
                (0x2D, KeyCap::letter('x', 'X')),
            ],
        );
        // «md»/«txt» — НЕ слова (низька LM, особливо «txt» через рідкісне 'x').
        let lm = NgramModel::train("text mode made data item", 3, 0.5);
        let dict = Dictionary::from_words(["text", "mode", "data"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("en"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    fn signal_uk_profile() -> LanguageProfile {
        let layout = Layout::new(
            LayoutId::new("uk"),
            [
                (0x32, KeyCap::letter('ь', 'Ь')),
                (0x20, KeyCap::letter('в', 'В')),
                (0x14, KeyCap::letter('е', 'Е')),
                (0x2D, KeyCap::letter('ч', 'Ч')),
            ],
        );
        // «ь» НІКОЛИ не на початку (лише в середині/кінці) — як у реальній мові;
        // «ече»/«ьв» — не слова.
        let lm = NgramModel::train("вечір мить день наче ось вода", 3, 0.5);
        let dict = Dictionary::from_words(["день", "вода", "наче"]).unwrap();
        LanguageProfile {
            id: LayoutId::new("uk"),
            layout,
            lm,
            dict,
            freq: None,
        }
    }

    #[test]
    fn phonotactic_soft_sign_start_switches_to_latin() {
        // «ьв» (uk) неможливе (старт із «ь») → перемкнути на латиницю «md». Без
        // правил розширень — спрацьовує ЧИСТА фонотактика.
        let langs = [signal_en_profile(), signal_uk_profile()];
        let rules = WordRules::new();
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        let d = decide(&strokes(&[0x32, 0x20]), &ctx); // uk "ьв" / en "md"
        assert_eq!(d.current_text, "ьв");
        assert_eq!(d.best, LayoutId::new("en"));
        assert_eq!(d.best_text, "md");
        assert!(
            d.switch,
            "укр. читання на «ь» неможливе → перемкнути (conf={:.2})",
            d.confidence
        );
    }

    #[test]
    fn phonotactic_disabled_flag_no_switch() {
        // Той самий «ьв», але прапорець вимкнено → сигнал ігнорується. Без нього
        // коротке «ьв»↔«md» не пробиває звичайний поріг → не перемикає.
        let langs = [signal_en_profile(), signal_uk_profile()];
        let cfg = DetectorConfig {
            phonotactics_enabled: false,
            ..DetectorConfig::default()
        };
        let d = decide(&strokes(&[0x32, 0x20]), &ctx_with_config(&langs, "uk", cfg));
        assert!(
            !d.switch,
            "прапорець phonotactics off → не перемикати (conf={:.2})",
            d.confidence
        );
    }

    #[test]
    fn phonotactic_real_uk_word_unaffected() {
        // Реальне укр. слово «день» (не на «ь») → фонотактика не чіпає; найкраще
        // лишається uk, перемикання нема.
        let langs = [signal_en_profile(), signal_uk_profile()];
        let rules = WordRules::new();
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        // день: д=0x20,е=0x14,н=?,ь=0x32 — немає 'н' у профілі, тож беремо «ведь»?
        // Простіше: слово «вень» (в,е,н...) теж без 'н'. Використаємо «вече» (в-е-ч-е)
        // — не починається з «ь», тож фонотактика мовчить.
        let d = decide(&strokes(&[0x20, 0x14, 0x2D, 0x14]), &ctx); // uk "вече" / en "dtxt"
        assert_eq!(d.current_text, "вече");
        assert!(
            !d.switch || d.best == LayoutId::new("uk"),
            "слово не на «ь» → фонотактика не форсить латиницю (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
    }

    fn ext_rules(exts: &[&str]) -> WordRules {
        let mut r = WordRules::new();
        for e in exts {
            r.add_extension(e);
        }
        r
    }

    #[test]
    fn extension_switches_from_cyrillic_layout() {
        // «txt», набране в UK-розкладці → «ече» (не слово). Відоме розширення →
        // перемкнути на латиницю.
        let langs = [signal_en_profile(), signal_uk_profile()];
        let rules = ext_rules(&["txt", "md", "pdf"]);
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        let d = decide(&strokes(&[0x14, 0x2D, 0x14]), &ctx); // uk "ече" / en "txt"
        assert_eq!(d.current_text, "ече");
        assert_eq!(d.best, LayoutId::new("en"));
        assert_eq!(d.best_text, "txt");
        assert!(
            d.switch,
            "відоме розширення 'txt' має перемкнути (conf={:.2})",
            d.confidence
        );
    }

    #[test]
    fn extension_gate_blocks_when_uk_reading_is_word() {
        // Гейт precision: якщо укр. читання — реальне слово (тут робимо «ече»
        // визнаним через особистий словник), розширення НЕ форсить — захист від
        // ризикових розширень-слів (`doc`/`log`/`go`), чий укр.-двійник валідний.
        let langs = [signal_en_profile(), signal_uk_profile()];
        let mut rules = ext_rules(&["txt"]);
        rules.recognize_word("ече"); // тепер укр. читання — «слово»
        let ctx = ctx_with_rules(&langs, "uk", &rules);
        let d = decide(&strokes(&[0x14, 0x2D, 0x14]), &ctx); // uk "ече" / en "txt"
        assert!(
            !d.switch,
            "укр. читання — слово → розширення не форсить (best={} conf={:.2})",
            d.best.as_str(),
            d.confidence
        );
    }

    #[test]
    fn extension_disabled_flag_no_switch() {
        let langs = [signal_en_profile(), signal_uk_profile()];
        // Прапорець off — навіть із переліком розширень сигнал мовчить. Щоб
        // ізолювати від фонотактики, беремо «ече» (не на «ь»).
        let cfg = DetectorConfig {
            extensions_enabled: false,
            ..DetectorConfig::default()
        };
        // ctx_with_config бере NO_RULES; додамо розширення через окремий ctx.
        let rules = ext_rules(&["txt"]);
        let ctx = Context {
            active_window: Default::default(),
            current_layout: LayoutId::new("uk"),
            languages: &langs,
            config: cfg,
            exclusions: &NO_EXCL,
            rules: &rules,
        };
        let d = decide(&strokes(&[0x14, 0x2D, 0x14]), &ctx);
        assert!(
            !d.switch,
            "прапорець extensions off → не перемикати (conf={:.2})",
            d.confidence
        );
    }
}
