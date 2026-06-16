//! Калібрувальний харнес детектора: міряє precision/recall/F1/accuracy на
//! розміченому eval-датасеті (`data/eval/dataset.jsonl`).
//!
//! Логіка тут (а не в бінарі/тесті), щоб її переюзали і `src/bin/calibrate.rs`,
//! і `tests/calibrate.rs`. Сам детектор у `typofix-core` НЕ чіпаємо — лише
//! викликаємо публічний [`typofix_core::detector::decide`].
//!
//! ## Що рахуємо
//! Прогон зразкових (дрібних!) моделей: для кожного рядка беремо `text` (те, що
//! на екрані) у `typed_layout`, перетворюємо назад на фізичні страйки через
//! зворотний індекс розкладки (`Layout::stroke_for`), будуємо [`Context`] з
//! поточною розкладкою = `typed_layout` і кличемо `decide`.
//!
//! ## Визначення «правильного» рішення (precision > recall)
//! Перемикання вважається ПРАВИЛЬНИМ, лише якщо `should_switch == true` **і**
//! детектор перемкнув **на правильну мову** (`best == intended_layout`). Тож:
//! - **TP** — треба було перемкнути й перемкнули на правильну мову;
//! - **FP** — перемкнули, коли не треба, **або** перемкнули не на ту мову
//!   (обидва псують легітимний текст → караємо precision);
//! - **FN** — треба було, але не перемкнули;
//! - **TN** — не треба й не перемкнули.
//!
//! Зразкові моделі дрібні → числа будуть грубі. Це очікувано: деліверабл —
//! харнес + baseline-знімок, не хороші числа (калібрація — на реальному корпусі).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use typofix_core::detector::{self, DetectorConfig};
use typofix_core::{
    Context, ExclusionRules, FrequencyMap, KeyStroke, LanguageProfile, Layout, LayoutId, WordRules,
};

/// Один розмічений приклад із `dataset.jsonl` (схема — `data/eval/CLAUDE.md`).
#[derive(Debug, Clone, Deserialize)]
pub struct Example {
    /// Текст, який фактично на екрані.
    pub text: String,
    /// Розкладка, активна на момент набору.
    pub typed_layout: String,
    /// Розкладка, яку користувач мав на увазі.
    pub intended_layout: String,
    /// Головна мітка: чи треба перемикати.
    pub should_switch: bool,
    /// Група для аналізу по зрізах.
    pub category: String,
}

/// Шлях до eval-датасету за замовчуванням (відносно цього крейту).
pub fn default_dataset_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
        .join("eval")
        .join("dataset.jsonl")
}

/// Прочитати датасет із JSONL (один приклад на рядок; порожні рядки пропускаємо).
pub fn load_dataset(path: &Path) -> io::Result<Vec<Example>> {
    let raw = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let ex: Example = serde_json::from_str(line).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("рядок {}: {e}", idx + 1),
            )
        })?;
        out.push(ex);
    }
    Ok(out)
}

/// Побудувати профілі uk+en для калібрування.
///
/// Бере **реальні** натреновані моделі з `data/lm/{lang}.bin` і
/// `data/dicts/{lang}.fst`, якщо вони є (`train_models`), інакше — fallback на
/// вбудовані зразки (`sample_*`). Так калібрування показує реальні числа
/// локально, а в CI (де `.bin`/`.fst` gitignored) лишається відтворюваним.
pub fn build_profiles() -> Result<Vec<LanguageProfile>, crate::ModelError> {
    let data = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data");
    let lm_dir = data.join("lm");
    let dict_dir = data.join("dicts");

    let mut profiles = Vec::new();
    for lang in ["uk", "en"] {
        let layout = crate::embedded_layout(lang).expect("вбудована розкладка має парситися");
        // Частотна мапа — опційна: є `{lang}.freq.fst` → градуйований сигнал,
        // нема → лише baseline dict-бонус (CI без даних лишається відтворюваним).
        let freq_path = dict_dir.join(format!("{lang}.freq.fst"));
        let freq = if freq_path.exists() {
            Some(FrequencyMap::from_fst_map(crate::load_freq_map_file(
                &freq_path,
            )?))
        } else {
            None
        };
        profiles.push(LanguageProfile {
            id: LayoutId::new(lang),
            layout,
            lm: crate::load_lm(lang, Some(&lm_dir))?,
            dict: crate::load_dict(lang, Some(&dict_dir))?,
            freq,
        });
    }
    Ok(profiles)
}

/// Перетворити екранний текст на фізичні страйки в заданій розкладці.
///
/// Символи, яких немає в розкладці (напр. латиниця у `uk`), пропускаються —
/// повертаємо їх кількість, щоб чесно показати, скільки прикладів зрепрезентовано
/// неповно (обмеження зразкового харнеса).
fn strokes_for(text: &str, layout: &Layout) -> (Vec<KeyStroke>, usize) {
    let mut strokes = Vec::new();
    let mut unmapped = 0usize;
    for ch in text.chars() {
        match layout.stroke_for(ch) {
            Some(s) => strokes.push(s),
            None => unmapped += 1,
        }
    }
    (strokes, unmapped)
}

/// Бал кандидата, відтворений через ПУБЛІЧНИЙ API (`lm.score`/`dict.contains`).
///
/// Дзеркалить приватну формулу `detector::LanguageProfile::score` — лише для
/// діагностики (показати, ЧОМУ детектор так вирішив). Якщо core згодом
/// експортує бали кандидатів, це можна прибрати.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Мова кандидата.
    pub lang: String,
    /// Інтерпретація страйків у цій розкладці.
    pub text: String,
    /// Лог-ймовірність LM.
    pub lm: f64,
    /// Чи слово є у словнику.
    pub in_dict: bool,
    /// Відтворений сукупний бал (`lm_weight·lm + dict_bonus?`).
    pub score: f64,
}

/// Категорія результату для одного прикладу.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// True positive: треба було й перемкнули на правильну мову.
    Tp,
    /// False positive: перемкнули помилково або не на ту мову.
    Fp,
    /// False negative: треба було, але не перемкнули.
    Fn,
    /// True negative: не треба й не перемкнули.
    Tn,
}

/// Повний результат прогону одного прикладу (для звіту/діагностики).
#[derive(Debug, Clone)]
pub struct EvalRow {
    /// Вхідний приклад.
    pub example: Example,
    /// Чи детектор вирішив перемкнути.
    pub switched: bool,
    /// На яку мову (best).
    pub best: String,
    /// Перевага best над поточною.
    pub confidence: f64,
    /// Скільки символів не зрепрезентовано (немає в розкладці).
    pub unmapped: usize,
    /// Бали кандидатів (діагностика).
    pub candidates: Vec<Candidate>,
    /// Підсумкова категорія.
    pub outcome: Outcome,
}

/// Матриця помилок + метрики.
#[derive(Debug, Clone, Copy, Default)]
pub struct Confusion {
    /// True positives.
    pub tp: usize,
    /// False positives.
    pub fp: usize,
    /// False negatives.
    pub fn_: usize,
    /// True negatives.
    pub tn: usize,
}

impl Confusion {
    /// Усього прикладів.
    pub fn total(&self) -> usize {
        self.tp + self.fp + self.fn_ + self.tn
    }
    /// Облікувати один результат.
    pub fn add(&mut self, o: Outcome) {
        match o {
            Outcome::Tp => self.tp += 1,
            Outcome::Fp => self.fp += 1,
            Outcome::Fn => self.fn_ += 1,
            Outcome::Tn => self.tn += 1,
        }
    }
    /// Precision = TP/(TP+FP); `NaN`, якщо не було жодного перемикання.
    pub fn precision(&self) -> f64 {
        ratio(self.tp, self.tp + self.fp)
    }
    /// Recall = TP/(TP+FN); `NaN`, якщо не було позитивів.
    pub fn recall(&self) -> f64 {
        ratio(self.tp, self.tp + self.fn_)
    }
    /// F1 = 2PR/(P+R).
    pub fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p.is_nan() || r.is_nan() || (p + r) == 0.0 {
            f64::NAN
        } else {
            2.0 * p * r / (p + r)
        }
    }
    /// Accuracy = (TP+TN)/total.
    pub fn accuracy(&self) -> f64 {
        ratio(self.tp + self.tn, self.total())
    }
}

fn ratio(num: usize, den: usize) -> f64 {
    if den == 0 {
        f64::NAN
    } else {
        num as f64 / den as f64
    }
}

/// Підсумковий звіт калібрувального прогону.
#[derive(Debug, Clone)]
pub struct Report {
    /// Глобальна матриця.
    pub overall: Confusion,
    /// Матриці по категоріях (детермінований порядок — BTreeMap).
    pub by_category: BTreeMap<String, Confusion>,
    /// Усі рядки (для списку промахів і діагностики).
    pub rows: Vec<EvalRow>,
    /// Параметри детектора, з якими ганяли.
    pub config: DetectorConfig,
    /// Скільки прикладів мали хоч один нерепрезентований символ.
    pub rows_with_unmapped: usize,
    /// Скільки перемикань відбулося не на ту мову (підмножина FP).
    pub wrong_target_switches: usize,
    /// Джерело моделей (для шапки звіту): "реальний корпус" чи "зразки".
    pub model_source: String,
}

/// Зібрати [`WordRules`] із курованими whitelist'ами коротких службових слів
/// (`data/dicts/{lang}.short.txt`) для дзеркальної релаксації порога в детекторі.
/// Veto/force лишаються порожні (калібруємо чистий детектор). Відсутній файл →
/// просто менше службових слів (релаксація для мови вимкнена).
pub fn build_word_rules(langs: &[&str]) -> WordRules {
    let dict_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
        .join("dicts");
    let mut rules = WordRules::default();
    for &lang in langs {
        let id = LayoutId::new(lang);
        for w in crate::load_short_words(lang, &dict_dir).unwrap_or_default() {
            rules.allow_short_service(&id, &w);
        }
    }
    rules
}

/// Прогнати датасет через детектор і зібрати метрики.
///
/// `rules` несе whitelist коротких службових слів (`build_word_rules`) для
/// дзеркальної релаксації; veto/force в калібруванні лишаються порожні.
pub fn evaluate(
    examples: &[Example],
    profiles: &[LanguageProfile],
    config: DetectorConfig,
    rules: &WordRules,
) -> Report {
    let mut overall = Confusion::default();
    let mut by_category: BTreeMap<String, Confusion> = BTreeMap::new();
    let mut rows = Vec::with_capacity(examples.len());
    let mut rows_with_unmapped = 0;
    let mut wrong_target_switches = 0;

    // Виключення порожні; правила несуть whitelist коротких службових слів.
    let exclusions = ExclusionRules::default();

    for ex in examples {
        let current = LayoutId::new(&ex.typed_layout);
        let typed_layout = profiles.iter().find(|p| p.id == current).map(|p| &p.layout);
        // Якщо поточної розкладки немає серед профілів — порожні страйки
        // (детектор однаково не перемкне без current_profile).
        let (strokes, unmapped) = match typed_layout {
            Some(l) => strokes_for(&ex.text, l),
            None => (Vec::new(), ex.text.chars().count()),
        };
        if unmapped > 0 {
            rows_with_unmapped += 1;
        }

        let ctx = Context {
            active_window: Default::default(),
            current_layout: current,
            languages: profiles,
            config,
            exclusions: &exclusions,
            rules,
        };
        let decision = detector::decide(&strokes, &ctx);

        // Відтворені бали кандидатів (діагностика через публічний API).
        let candidates = profiles
            .iter()
            .map(|p| {
                let text = p.layout.interpret(&strokes);
                let lm = p.lm.score(&text);
                let in_dict = p.dict.contains(&text);
                // Дзеркалить приватну `score()`: baseline dict-бонус + частотна
                // надбавка `freq_weight·max(0, lp − freq_floor)` для слів у мапі.
                let freq_term = if in_dict {
                    p.freq
                        .as_ref()
                        .and_then(|m| m.log_prob(&text))
                        .map(|lp| config.freq_weight * (lp - config.freq_floor).max(0.0))
                        .unwrap_or(0.0)
                } else {
                    0.0
                };
                let score = config.lm_weight * lm
                    + if in_dict { config.dict_bonus } else { 0.0 }
                    + freq_term;
                Candidate {
                    lang: p.id.as_str().to_string(),
                    text,
                    lm,
                    in_dict,
                    score,
                }
            })
            .collect();

        let correct_target = decision.best.as_str() == ex.intended_layout;
        let outcome = match (ex.should_switch, decision.switch) {
            (true, true) if correct_target => Outcome::Tp,
            (_, true) => {
                // Перемкнули помилково або не на ту мову.
                if !correct_target {
                    wrong_target_switches += 1;
                }
                Outcome::Fp
            }
            (true, false) => Outcome::Fn,
            (false, false) => Outcome::Tn,
        };

        overall.add(outcome);
        by_category
            .entry(ex.category.clone())
            .or_default()
            .add(outcome);

        rows.push(EvalRow {
            example: ex.clone(),
            switched: decision.switch,
            best: decision.best.as_str().to_string(),
            confidence: decision.confidence,
            unmapped,
            candidates,
            outcome,
        });
    }

    Report {
        overall,
        by_category,
        rows,
        config,
        rows_with_unmapped,
        wrong_target_switches,
        model_source: String::new(),
    }
}

/// Чи є реальні натреновані моделі (`data/lm/*.bin`) — для шапки звіту.
fn real_models_present() -> bool {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
        .join("lm")
        .join("uk.bin")
        .exists()
}

/// Зручність: завантажити дефолтний датасет, побудувати профілі, прогнати.
pub fn run_default() -> io::Result<Report> {
    let examples = load_dataset(&default_dataset_path())?;
    let profiles = build_profiles().map_err(|e| io::Error::other(format!("профілі: {e}")))?;
    let rules = build_word_rules(&["uk", "en"]);
    let mut report = evaluate(&examples, &profiles, DetectorConfig::default(), &rules);
    report.model_source = if real_models_present() {
        "реальний корпус (data/lm,data/dicts)".to_string()
    } else {
        "вбудовані зразки (дрібні → числа грубі)".to_string()
    };
    Ok(report)
}

// --- Форматування звіту ----------------------------------------------------

fn pct(x: f64) -> String {
    if x.is_nan() {
        "  n/a".to_string()
    } else {
        format!("{:5.1}%", x * 100.0)
    }
}

/// Відформатувати звіт у текст для друку (бінар) і smoke-перевірки (тест).
pub fn format_report(report: &Report) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let o = &report.overall;

    writeln!(s, "TypoFix — калібрувальні метрики детектора").unwrap();
    writeln!(s, "моделі: {}\n", report.model_source).unwrap();
    writeln!(
        s,
        "config: lm_weight={} dict_bonus={} base_threshold={} short_word_extra={} min_switch_len={}",
        report.config.lm_weight,
        report.config.dict_bonus,
        report.config.base_threshold,
        report.config.short_word_extra,
        report.config.min_switch_len,
    )
    .unwrap();
    writeln!(
        s,
        "прикладів: {}  з нерепрезентованими символами: {}  перемикань не на ту мову: {}\n",
        o.total(),
        report.rows_with_unmapped,
        report.wrong_target_switches,
    )
    .unwrap();

    writeln!(s, "== ГЛОБАЛЬНО ==").unwrap();
    writeln!(s, "  TP={} FP={} FN={} TN={}", o.tp, o.fp, o.fn_, o.tn).unwrap();
    writeln!(
        s,
        "  precision={}  recall={}  F1={}  accuracy={}\n",
        pct(o.precision()),
        pct(o.recall()),
        pct(o.f1()),
        pct(o.accuracy()),
    )
    .unwrap();

    writeln!(s, "== ПО КАТЕГОРІЯХ ==").unwrap();
    writeln!(
        s,
        "  {:18} {:>3} {:>3} {:>3} {:>3}  {:>6} {:>6} {:>6} {:>6}",
        "category", "TP", "FP", "FN", "TN", "prec", "rec", "F1", "acc"
    )
    .unwrap();
    for (cat, c) in &report.by_category {
        writeln!(
            s,
            "  {:18} {:>3} {:>3} {:>3} {:>3}  {} {} {} {}",
            cat,
            c.tp,
            c.fp,
            c.fn_,
            c.tn,
            pct(c.precision()),
            pct(c.recall()),
            pct(c.f1()),
            pct(c.accuracy()),
        )
        .unwrap();
    }

    // Промахи: спершу FP (псують легітимний текст — пріоритет precision), потім FN.
    let fps: Vec<&EvalRow> = report
        .rows
        .iter()
        .filter(|r| r.outcome == Outcome::Fp)
        .collect();
    let fns: Vec<&EvalRow> = report
        .rows
        .iter()
        .filter(|r| r.outcome == Outcome::Fn)
        .collect();

    writeln!(
        s,
        "\n== FALSE POSITIVES ({}) — перемкнули, коли НЕ треба ==",
        fps.len()
    )
    .unwrap();
    for r in fps.iter().take(40) {
        write_miss(&mut s, r);
    }
    if fps.len() > 40 {
        writeln!(s, "  … ще {}", fps.len() - 40).unwrap();
    }

    writeln!(
        s,
        "\n== FALSE NEGATIVES ({}) — НЕ перемкнули, хоч треба ==",
        fns.len()
    )
    .unwrap();
    for r in fns.iter().take(40) {
        write_miss(&mut s, r);
    }
    if fns.len() > 40 {
        writeln!(s, "  … ще {}", fns.len() - 40).unwrap();
    }

    s
}

fn write_miss(s: &mut String, r: &EvalRow) {
    use std::fmt::Write;
    let cand: Vec<String> = r
        .candidates
        .iter()
        .map(|c| {
            format!(
                "{}={:.2}{}/'{}'",
                c.lang,
                c.score,
                if c.in_dict { "+dict" } else { "" },
                c.text
            )
        })
        .collect();
    writeln!(
        s,
        "  [{}] '{}' typed={} intended={} → switch={} best={} conf={:.2} | {}",
        r.example.category,
        r.example.text,
        r.example.typed_layout,
        r.example.intended_layout,
        r.switched,
        r.best,
        r.confidence,
        cand.join("  "),
    )
    .unwrap();
}
