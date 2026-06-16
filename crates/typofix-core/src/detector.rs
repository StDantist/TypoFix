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

use crate::{Context, Dictionary, KeyStroke, Layout, LayoutId, NgramModel};

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
    /// Словник для бусту впевненості.
    pub dict: Dictionary,
}

/// Розкладений бал кандидата: повний (`total`) і **лише LM-складова** (`lm`).
///
/// LM-складова потрібна окремо, бо для дуже коротких слів ми вимагаємо, щоб
/// перевагу давав не лише збіг у словнику, а й сама мовна модель (див.
/// [`DetectorConfig::short_word_lm_margin`]).
#[derive(Debug, Clone, Copy)]
struct CandidateScore {
    /// Повний бал: `lm_weight·lm + (dict ? dict_bonus : 0)`.
    total: f64,
    /// Лише зважена LM-складова: `lm_weight·lm` (без бонусу словника).
    lm: f64,
}

impl LanguageProfile {
    /// Бал кандидата для заданого тексту (вже інтерпретованого в його розкладці).
    fn score(&self, text: &str, cfg: &DetectorConfig) -> CandidateScore {
        if text.is_empty() {
            return CandidateScore {
                total: f64::NEG_INFINITY,
                lm: f64::NEG_INFINITY,
            };
        }
        let lm = cfg.lm_weight * self.lm.score(text);
        let bonus = if self.dict.contains(text) {
            cfg.dict_bonus
        } else {
            0.0
        };
        CandidateScore {
            total: lm + bonus,
            lm,
        }
    }
}

/// Налаштування детектора (ваги й крива порогу). Калібруватиметься на eval-датасеті
/// (Фаза 2/наступна задача); тут — розумні дефолти для доведення логіки.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectorConfig {
    /// Вага лог-ймовірності LM (`w1`).
    pub lm_weight: f64,
    /// Бонус за наявність слова у словнику (`w2`-еквівалент).
    pub dict_bonus: f64,
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
    /// **Стеля LM джерельного двійника для дзеркальної релаксації** коротких
    /// слів: дзеркальна гілка (службове слово ↔ біліберда) спрацьовує лише якщо
    /// LM-складова ПОТОЧНОГО тексту (двійника у вихідній мові) НИЖЧА за цю межу —
    /// тобто двійник фонотактично неправдоподібний (реальне слово мав би LM
    /// вище). Це і є умова «двійник НЕ справжнє слово» (поряд із «не в словнику»).
    /// Дефолт `0.0`: біліберда має LM ≪ 0; справжні короткі слова — LM > 0.
    pub short_word_twin_lm_max: f64,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            lm_weight: 1.0,
            dict_bonus: 4.0,
            base_threshold: 1.0,
            short_word_extra: 4.0,
            min_switch_len: 2,
            short_word_max_len: 2,
            short_word_lm_margin: 2.0,
            short_word_twin_lm_max: 0.0,
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
        .map(|p| p.score(&current_text, cfg))
        .unwrap_or(CandidateScore {
            total: f64::NEG_INFINITY,
            lm: f64::NEG_INFINITY,
        });

    // Чи ПОТОЧНИЙ текст (двійник у вихідній мові) — реальне слово в її словнику.
    // Потрібно для дзеркальної релаксації: форсимо коротке лише коли двійник НЕ
    // справжнє слово (інакше це легітимний короткий ввід — `is`/`db` — не чіпати).
    let current_is_dict = current
        .map(|p| p.dict.contains(&current_text))
        .unwrap_or(false);

    // Початково найкраща — поточна (щоб за відсутності переваги нічого не міняти).
    let mut best = ctx.current_layout.clone();
    let mut best_text = current_text.clone();
    let mut best_score = current_score;
    let mut best_is_dict = current_is_dict;

    for p in ctx.languages {
        let text = p.layout.interpret(strokes);
        let sc = p.score(&text, cfg);
        if sc.total > best_score.total {
            best_score = sc;
            best = p.id.clone();
            best_is_dict = p.dict.contains(&text);
            best_text = text;
        }
    }

    let len = current_text.chars().count();
    let confidence = best_score.total - current_score.total;
    // LM-перевага кандидата БЕЗ бонусу словника: для дуже коротких слів вимагаємо,
    // щоб за кандидата голосувала і сама модель, а не лише збіг у словнику.
    let lm_confidence = best_score.lm - current_score.lm;
    let short_word_ok = len > cfg.short_word_max_len || lm_confidence > cfg.short_word_lm_margin;

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
        && (forced || standard_ok || mirror_ok);

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

    // Precision-замок: трактуємо хвостову пунктуацію як ЛІТЕРИ лише за словникового
    // слова й вищої впевненості; інакше лишаємо її роздільником (безпечно).
    let use_letter = letter.switch && letter.best_is_dict && letter.confidence > sep.confidence;

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
}
