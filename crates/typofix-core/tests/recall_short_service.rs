//! Регресія DATA-DRIVEN коротко-словного перемикання (`detector::decide`).
//! Принцип «справжнє ЧАСТЕ слово ↔ біліберда»: коротке (len 2..=max, ОДИНОЧНІ не
//! чіпаємо) перемикається, якщо укр-двійник Є у словнику І ЧАСТИЙ
//! (`best_score.freq >= short_word_freq_switch_min`), а поточний (en) текст НЕ
//! частий (`current_score.freq < short_word_current_freq_max`). Whitelist
//! (`uk.short.txt`/`is_short_service`) — лише ДОДАТКОВИЙ override, НЕ єдиний шлях.
//! Реальні часті англ. (`of`/`it`/`is`/`hi`) — НЕ чіпати (висока en-частота).
//!
//! Вантажить РЕАЛЬНІ моделі/словники/ЧАСТОТИ з `data/`. Якщо їх нема (CI,
//! gitignored `.bin`/`.fst`) — тест SKIP'иться; герметичне покриття механіки —
//! в `detector.rs` (юніт-тести `mirror_*`/`short_word_*`).

use std::path::PathBuf;

use typofix_core::{
    detector, Context, DetectorConfig, ExclusionRules, FrequencyMap, KeyStroke, LanguageProfile,
    Layout, LayoutId, WordRules,
};

static NO_EXCL: ExclusionRules = ExclusionRules::new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

fn real_profiles() -> Option<Vec<LanguageProfile>> {
    real_profiles_inner(false)
}

/// Профілі з ЧАСТОТНИМ шаром (`{lang}.freq.fst`) — потрібен для data-driven
/// коротко-словного перемикання (частота двійника — головний сигнал).
fn real_profiles_with_freq() -> Option<Vec<LanguageProfile>> {
    real_profiles_inner(true)
}

fn real_profiles_inner(with_freq: bool) -> Option<Vec<LanguageProfile>> {
    let data = data_dir();
    let lm_dir = data.join("lm");
    let dict_dir = data.join("dicts");
    if !lm_dir.join("uk.bin").exists() {
        return None;
    }
    let mut v = Vec::new();
    for lang in ["uk", "en"] {
        let fp = dict_dir.join(format!("{lang}.freq.fst"));
        let freq = (with_freq && fp.exists())
            .then(|| FrequencyMap::from_fst_map(typofix_data::load_freq_map_file(&fp).unwrap()));
        v.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout: typofix_data::embedded_layout(lang).unwrap(),
            lm: typofix_data::load_lm(lang, Some(&lm_dir)).unwrap(),
            dict: typofix_data::load_dict(lang, Some(&dict_dir)).unwrap(),
            freq,
        });
    }
    Some(v)
}

/// Фізичні страйки для слова, як його НАБИРАЮТЬ у заданій розкладці (зворотний
/// індекс): ті самі клавіші дають це слово в `layout` і біліберду в іншій.
fn strokes_in(layout: &Layout, word: &str) -> Vec<KeyStroke> {
    word.chars()
        .map(|c| layout.stroke_for(c).expect("символ має бути в розкладці"))
        .collect()
}

fn ctx<'a>(langs: &'a [LanguageProfile], current: &str, rules: &'a WordRules) -> Context<'a> {
    ctx_cfg(langs, current, rules, DetectorConfig::default())
}

fn ctx_cfg<'a>(
    langs: &'a [LanguageProfile],
    current: &str,
    rules: &'a WordRules,
    config: DetectorConfig,
) -> Context<'a> {
    Context {
        active_window: Default::default(),
        current_layout: LayoutId::new(current),
        languages: langs,
        config,
        exclusions: &NO_EXCL,
        rules,
        secure: false,
    }
}

#[test]
fn one_letter_tokens_never_switch() {
    // КОНТРАКТ ЗМІНЕНО (раніше assert був ЗВОРОТНИЙ). Одиночний токен (len=1)
    // НІКОЛИ не перемикається — навіть якщо його укр.-двійник у whitelist коротких
    // службових слів. Причина: репро власника — кома `,` (en) сидить на клавіші
    // `б`(uk, whitelist) → дзеркало хибно робило з коми «б». Самотня літера
    // практично ніколи не є самостійним словом, вартим перемикання; precision >
    // recall. Межа дзеркала — ЛІТЕРАЛ `len >= 2` (не cfg). Свідомий FN на одиночних.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["і", "й", "в", "у", "з"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            !d.switch,
            "1-літерний токен '{w}' НЕ має перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn two_letter_service_words_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let uk = &langs[0].layout;
    for w in ["ти", "чи", "ми", "до", "по"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx(&langs, "en", &rules));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "2-літерне службове '{w}' має перемкнутись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn que_and_to_switch_with_runtime_min_len_3() {
    // РЕПРО власника на РЕАЛЬНИХ моделях ІЗ ЖИВИМ КОНФІГОМ: рантайм `src-tauri`
    // дефолтить `min_word_len=3` → `min_switch_len=3`. «oj»(en)→«що», «nj»(en)→«то»
    // (o→щ, j→о, n→т на ЙЦУКЕН). Поки межа дзеркала була `cfg.min_switch_len`, тут
    // обидва НЕ перемикалися (2>=3 false) — саме цей сценарій тести на default()=2
    // НЕ ловили. Після фікса (літерал 2) перемикаються попри min_switch_len=3.
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let cfg = DetectorConfig {
        min_switch_len: 3,
        ..DetectorConfig::default()
    };
    let uk = &langs[0].layout;
    for w in ["що", "то"] {
        let d = detector::decide(&strokes_in(uk, w), &ctx_cfg(&langs, "en", &rules, cfg));
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "'{w}' (репро, min_switch_len=3) має перемкнутись (switch={} best={} '{}' conf={:.2})",
            d.switch,
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn real_english_short_words_do_not_switch() {
    let Some(langs) = real_profiles() else {
        eprintln!("SKIP: реальні моделі відсутні");
        return;
    };
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let en = &langs[1].layout;
    // Реальні короткі англ., набрані в EN → двійник у вихідній en — справжнє
    // слово → дзеркало НЕ спрацьовує, нічого не чіпаємо (precision-замок).
    for w in ["a", "i", "is", "to", "it"] {
        let d = detector::decide(&strokes_in(en, w), &ctx(&langs, "en", &rules));
        assert!(
            !d.switch,
            "реальне англ. '{w}' НЕ має перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

// === DATA-DRIVEN коротко-словне перемикання (частота, БЕЗ ручного whitelist) ===
// Усі тести — РАНТАЙМ-конфіг `min_switch_len=3` (урок min=3-бага) і **ПОРОЖНІЙ
// whitelist**, щоб довести: покриття йде з частотного словника, не зі списку.

fn runtime_cfg() -> DetectorConfig {
    DetectorConfig {
        min_switch_len: 3,
        ..DetectorConfig::default()
    }
}

#[test]
fn frequent_short_uk_words_switch_without_whitelist() {
    // Репро власника + узагальнення: часті 2-літерні укр. перемикаються АВТОМАТИЧНО
    // з частот (`nf`→`та` — слово, якого власник скаржився, що нема у whitelist).
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let empty = WordRules::new(); // НЕМАЄ whitelist — лише частота
    let uk = &langs[0].layout;
    for (w, en_twin) in [
        ("та", "nf"),
        ("що", "oj"),
        ("то", "nj"),
        ("як", "zr"),
        ("чи", "xb"),
        ("бо", ",j"),
        ("ні", "ys"),
        ("на", "yf"),
        ("не", "yt"),
        ("до", "lj"),
        ("по", "gj"),
    ] {
        let d = detector::decide(
            &strokes_in(uk, w),
            &ctx_cfg(&langs, "en", &empty, runtime_cfg()),
        );
        assert!(
            d.switch && d.best == LayoutId::new("uk") && d.best_text == w,
            "часте '{w}' (en '{en_twin}') має перемкнутись БЕЗ whitelist \
             (switch={} best={} '{}' conf={:.2})",
            d.switch,
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn short_english_words_never_switch_data_driven() {
    // PRECISION-замок симетричного частотного гейта: РЕАЛЬНІ часті англ. короткі,
    // набрані в EN, НЕ чіпаємо (їхня en-частота висока → `current_not_frequent` хибне).
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let empty = WordRules::new();
    let en = &langs[1].layout;
    for w in [
        "of", "it", "is", "in", "on", "at", "we", "to", "us", "so", "no", "an", "or", "by", "hi",
        "ok", "go", "me", "my", "up",
    ] {
        let d = detector::decide(
            &strokes_in(en, w),
            &ctx_cfg(&langs, "en", &empty, runtime_cfg()),
        );
        assert!(
            !d.switch,
            "реальне англ. '{w}' НЕ сміє перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn short_code_tokens_do_not_switch_data_driven() {
    // Код-токени, набрані в EN: їхній укр-двійник — словниковий ШУМ (`ат`/`ді`/`св`,
    // freq ≈ 0 < поріг) → частотний гейт не пускає (як і раніше whitelist).
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let empty = WordRules::new();
    let en = &langs[1].layout;
    for w in ["fn", "ls", "cd"] {
        let d = detector::decide(
            &strokes_in(en, w),
            &ctx_cfg(&langs, "en", &empty, runtime_cfg()),
        );
        assert!(
            !d.switch,
            "код-токен '{w}' НЕ сміє перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn rare_english_abbreviations_in_dict_do_not_switch() {
    // PRECISION-РЕГРЕС (аудит-знахідка): 2-літерні англ. АБРЕВІАТУРИ, що Є у en.fst,
    // але РІДКІСНІ в корпусі (freq < поріг) — `db`/`bp`/`lt`/`nt`/`ye`. Їхній
    // uk-двійник частий (db→ви, lt→де, nt→те, ye→ну, bp→из), тож САМ частотний
    // гейт їх НЕ блокував би → хибне перемикання (домен власника: database/Forex).
    // Захищає КОН'ЮНКЦІЯ `!current_is_dict && freq<max`. Падало б до фікса гейта.
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    // І з whitelist, і без — мають лишатися недоторканими.
    let rules = typofix_data::eval::build_word_rules(&["uk", "en"]);
    let en = &langs[1].layout;
    for w in ["db", "bp", "lt", "nt", "ye"] {
        let d = detector::decide(
            &strokes_in(en, w),
            &ctx_cfg(&langs, "en", &rules, runtime_cfg()),
        );
        assert!(
            !d.switch,
            "англ. абревіатура '{w}' (у en.fst) НЕ сміє перемикатись (best={} '{}' conf={:.2})",
            d.best.as_str(),
            d.best_text,
            d.confidence
        );
    }
}

#[test]
fn ta_switches_after_junk_twin_removed_from_corpus() {
    // RECALL чиститься в ДАНИХ, не послабленням гейта: «та» (en двійник `nf`)
    // перемикається ЛИШЕ тому, що сміттєвий `nf` прибрано з en-корпусу (раніше
    // `nf` ∈ en.fst → `current_is_dict` блокував). Якщо `nf` повернеться у дані —
    // цей тест почервоніє (сигнал, що чищення/JUNK_SHORT треба відновити).
    let Some(langs) = real_profiles_with_freq() else {
        eprintln!("SKIP: реальні моделі/частоти відсутні");
        return;
    };
    let empty = WordRules::new();
    let uk = &langs[0].layout;
    let d = detector::decide(
        &strokes_in(uk, "та"),
        &ctx_cfg(&langs, "en", &empty, runtime_cfg()),
    );
    assert!(
        d.switch && d.best_text == "та",
        "«та» має перемкнутись (nf прибрано з en.fst) (switch={} best='{}' conf={:.2})",
        d.switch,
        d.best_text,
        d.confidence
    );
}
