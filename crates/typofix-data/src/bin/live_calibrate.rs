//! Калібрування precision/recall фічі «перемикання на льоту» (`detector::live_decide`)
//! на РЕАЛЬНИХ моделях (`uk.fst`/`en.fst`/LM/freq через `eval::build_profiles`).
//!
//! НЕ змінює поведінку фічі — лише вимірює. Прапорець `live_switch_enabled=true`.
//!
//! ## Тест 1 — PRECISION (головний)
//! Реальні укр. слова, набрані ПРАВИЛЬНО у UK-розкладці. Прокручуємо префікси від
//! `live_min_len` до кінця; якщо `live_decide` повертає `Some` на будь-якому
//! префіксі — це ХИБНЕ раннє перемикання (FP = діра `uk.fst`).
//!   - Held-out набір: `data/corpora/freq/uk.freq.txt` (OpenSubtitles, з частотами;
//!     у словник НЕ вливається → реальний вжиток + власні назви/сленг/неологізми).
//!   - In-dict sanity: `data/corpora/uk.words.txt` (вливається в `uk.fst` → ~0 FP).
//!
//! ## Тест 2 — RECALL (користь)
//! Реальні англ. слова (`data/corpora/en.words.txt`), «набрані» фізичними клавішами
//! при АКТИВНІЙ UK-розкладці (на екрані кирилична каша). На якому префіксі (якщо
//! взагалі) `live_decide` спрацьовує? % спрацювань + медіанна позиція.
//!
//! Запуск: `cargo run -p typofix-data --release --bin live_calibrate`

use std::path::{Path, PathBuf};

use typofix_core::detector::{self, DetectorConfig};
use typofix_core::{
    Context, ExclusionRules, KeyStroke, LanguageProfile, Layout, LayoutId, WordRules,
};
use typofix_data::eval;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

fn cfg(min_len: usize) -> DetectorConfig {
    DetectorConfig {
        live_switch_enabled: true,
        live_min_len: min_len,
        ..Default::default()
    }
}

/// Фізичні страйки для слова в заданій розкладці; `None`, якщо хоч один символ
/// не мапиться (тоді чесно не можемо зімітувати — слово пропускаємо).
fn strokes_for(word: &str, layout: &Layout) -> Option<Vec<KeyStroke>> {
    let mut out = Vec::with_capacity(word.chars().count());
    for ch in word.chars() {
        out.push(layout.stroke_for(ch)?);
    }
    Some(out)
}

/// Найраніший префікс (за к-стю страйків), на якому `live_decide` спрацьовує; `None`,
/// якщо жоден префікс [min_len..=len] не тригерить.
fn earliest_trigger(strokes: &[KeyStroke], ctx: &Context, min_len: usize) -> Option<usize> {
    let n = strokes.len();
    for p in min_len..=n {
        if detector::live_decide(&strokes[..p], ctx).is_some() {
            return Some(p);
        }
    }
    None
}

/// Прочитати `слово<TAB>count` (freq-список); відфільтрувати службові порожні.
fn read_freq_list(path: &Path) -> Vec<(String, u64)> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    raw.lines()
        .filter_map(|l| {
            let (w, c) = l.split_once('\t')?;
            let w = w.trim();
            if w.is_empty() {
                return None;
            }
            Some((w.to_owned(), c.trim().parse::<u64>().unwrap_or(0)))
        })
        .collect()
}

fn read_word_list(path: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Чи всі символи слова — українські літери (+ апостроф/дефіс), без цифр/латиниці.
fn is_clean_uk(word: &str) -> bool {
    let mut has_letter = false;
    for ch in word.chars() {
        if ('а'..='я').contains(&ch)
            || ('А'..='Я').contains(&ch)
            || ch == 'і'
            || ch == 'І'
            || ch == 'ї'
            || ch == 'Ї'
            || ch == 'є'
            || ch == 'Є'
            || ch == 'ґ'
            || ch == 'Ґ'
        {
            has_letter = true;
        } else if ch == '\'' || ch == '\u{2019}' || ch == '-' {
            // дозволені внутрішні
        } else {
            return false;
        }
    }
    has_letter
}

fn is_clean_en(word: &str) -> bool {
    let mut has_letter = false;
    for ch in word.chars() {
        if ch.is_ascii_alphabetic() {
            has_letter = true;
        } else if ch == '\'' || ch == '-' {
        } else {
            return false;
        }
    }
    has_letter
}

struct Profiles {
    profiles: Vec<LanguageProfile>,
}

impl Profiles {
    fn ctx<'a>(
        &'a self,
        current: &LayoutId,
        config: DetectorConfig,
        excl: &'a ExclusionRules,
        rules: &'a WordRules,
    ) -> Context<'a> {
        Context {
            active_window: Default::default(),
            current_layout: current.clone(),
            languages: &self.profiles,
            config,
            exclusions: excl,
            rules,
            secure: false,
        }
    }

    fn layout(&self, id: &LayoutId) -> &Layout {
        &self
            .profiles
            .iter()
            .find(|p| &p.id == id)
            .expect("профіль є")
            .layout
    }
}

/// Один FP-рядок Тесту 1 (для прикладів).
struct Fp {
    word: String,
    count: u64,
    trigger_len: usize,
    uk_prefix: String,
    en_twin: String,
}

/// Результат прогону Тесту 1 для одного min_len.
struct T1Result {
    #[allow(dead_code)]
    min_len: usize,
    /// Слова, які пройшли фільтр і мають char-len >= min_len (знаменник).
    eligible: usize,
    /// Слова з хоч одним FP.
    fp_words: usize,
    /// Токен-зважений знаменник (сума count придатних).
    token_total: u64,
    /// Токен-зважена сума count FP-слів.
    token_fp: u64,
    /// Гістограма позиції спрацювання (trigger_len -> к-сть слів).
    by_pos: std::collections::BTreeMap<usize, usize>,
    /// Топ-приклади (за частотою).
    examples: Vec<Fp>,
}

#[allow(clippy::too_many_arguments)]
fn run_test1(
    p: &Profiles,
    words: &[(String, u64)],
    min_len: usize,
    uk: &LayoutId,
    en: &LayoutId,
    excl: &ExclusionRules,
    rules: &WordRules,
    collect_examples: bool,
) -> T1Result {
    let config = cfg(min_len);
    let ctx = p.ctx(uk, config, excl, rules);
    let uk_layout = p.layout(uk);
    let en_layout = p.layout(en);

    let mut res = T1Result {
        min_len,
        eligible: 0,
        fp_words: 0,
        token_total: 0,
        token_fp: 0,
        by_pos: Default::default(),
        examples: Vec::new(),
    };

    for (word, count) in words {
        if !is_clean_uk(word) {
            continue;
        }
        if word.chars().count() < min_len {
            continue;
        }
        let strokes = match strokes_for(word, uk_layout) {
            Some(s) => s,
            None => continue,
        };
        res.eligible += 1;
        res.token_total += *count;

        if let Some(tl) = earliest_trigger(&strokes, &ctx, min_len) {
            res.fp_words += 1;
            res.token_fp += *count;
            *res.by_pos.entry(tl).or_insert(0) += 1;
            if collect_examples {
                let uk_prefix = uk_layout.interpret(&strokes[..tl]);
                let en_twin = en_layout.interpret(&strokes[..tl]);
                res.examples.push(Fp {
                    word: word.clone(),
                    count: *count,
                    trigger_len: tl,
                    uk_prefix,
                    en_twin,
                });
            }
        }
    }
    if collect_examples {
        res.examples.sort_by_key(|e| std::cmp::Reverse(e.count));
    }
    res
}

fn main() {
    let profiles = eval::build_profiles().expect("реальні профілі мають вантажитись");
    let real = data_dir().join("lm").join("uk.bin").exists();
    println!(
        "== LIVE-SWITCH КАЛІБРУВАННЯ ==\nмоделі: {}\n",
        if real {
            "РЕАЛЬНІ (data/lm,data/dicts)"
        } else {
            "вбудовані зразки (ДРІБНІ — числа НЕдостовірні!)"
        }
    );
    let p = Profiles { profiles };
    let uk = LayoutId::new("uk");
    let en = LayoutId::new("en");
    let excl = ExclusionRules::default();
    // user.txt + iso + extensions як у runtime/eval (user.txt — додатковий запобіжник
    // діри #1: recognized-слова НЕ форсять live).
    let rules = eval::build_word_rules(&["uk", "en"]);

    let corpora = data_dir().join("corpora");

    // ---- ТЕСТ 1: PRECISION ----
    // Held-out: OpenSubtitles freq-список (НЕ у словнику) — реальний вжиток.
    let uk_freq = read_freq_list(&corpora.join("freq").join("uk.freq.txt"));
    // In-dict sanity: корпусні слова (вливаються в uk.fst).
    let uk_words: Vec<(String, u64)> = read_word_list(&corpora.join("uk.words.txt"))
        .into_iter()
        .map(|w| (w, 1))
        .collect();

    println!("################ ТЕСТ 1 — PRECISION (укр. ввід НЕ має смикатись) ################\n");

    println!(
        "Held-out набір: data/corpora/freq/uk.freq.txt — {} рядків (реальний вжиток, з частотами)",
        uk_freq.len()
    );
    println!(
        "In-dict sanity: data/corpora/uk.words.txt — {} слів (вливаються в uk.fst)\n",
        uk_words.len()
    );

    println!("--- Чутливість до live_min_len (held-out OpenSubtitles) ---");
    println!(
        "{:>8} | {:>9} | {:>10} {:>9} | {:>12} {:>9}",
        "min_len", "eligible", "FP-слів", "FP%(type)", "FP-токенів", "FP%(tok)"
    );
    for ml in [2usize, 3, 4, 5] {
        let r = run_test1(&p, &uk_freq, ml, &uk, &en, &excl, &rules, ml == 3);
        let type_pct = 100.0 * r.fp_words as f64 / r.eligible.max(1) as f64;
        let tok_pct = 100.0 * r.token_fp as f64 / r.token_total.max(1) as f64;
        println!(
            "{:>8} | {:>9} | {:>10} {:>8.2}% | {:>12} {:>8.3}%",
            ml, r.eligible, r.fp_words, type_pct, r.token_fp, tok_pct
        );
        if ml == 3 {
            // Детальний звіт для дефолтного порога.
            // Детальний зріз на min_len=3 (а не на дефолтних 4) — навмисно: тут
            // більше FP, краще видно конкретні діри fst та засмічення набору.
            println!("\n  >>> Деталі для live_min_len=3 (детальний зріз дір) <<<");
            println!("  позиція спрацювання (к-сть страйків -> к-сть слів):");
            for (pos, n) in &r.by_pos {
                println!("    prefix_len={:>2}: {:>6} слів", pos, n);
            }
            println!("\n  Топ-25 проблемних слів за частотою (uk_prefix → en_twin @prefix_len):");
            for fp in r.examples.iter().take(25) {
                println!(
                    "    {:>8}×  '{}'  →  раннє @{}: '{}' → en '{}'",
                    fp.count, fp.word, fp.trigger_len, fp.uk_prefix, fp.en_twin
                );
            }
            println!();
        }
    }

    println!("\n--- In-dict sanity (uk.words.txt, очікуємо ~0 FP) ---");
    for ml in [3usize] {
        let r = run_test1(&p, &uk_words, ml, &uk, &en, &excl, &rules, true);
        let type_pct = 100.0 * r.fp_words as f64 / r.eligible.max(1) as f64;
        println!(
            "  min_len={}: eligible={} FP-слів={} ({:.3}%)",
            ml, r.eligible, r.fp_words, type_pct
        );
        for fp in r.examples.iter().take(15) {
            println!(
                "    '{}' → @{}: '{}' → en '{}'",
                fp.word, fp.trigger_len, fp.uk_prefix, fp.en_twin
            );
        }
    }

    // ---- ТЕСТ 2: RECALL ----
    println!(
        "\n################ ТЕСТ 2 — RECALL (cross-layout англ. МАЄ смикатись рано) ################\n"
    );
    let en_words = read_word_list(&corpora.join("en.words.txt"));
    let en_layout = p.layout(&en);

    println!(
        "{:>8} | {:>9} | {:>10} {:>9} | {:>8} {:>8}",
        "min_len", "eligible", "triggered", "recall%", "median", "mean"
    );
    let mut never_examples: Vec<String> = Vec::new();
    for ml in [3usize, 4, 5] {
        let config = cfg(ml);
        let ctx = p.ctx(&uk, config, &excl, &rules); // АКТИВНА розкладка = uk (каша)
        let mut eligible = 0usize;
        let mut positions: Vec<usize> = Vec::new();
        for word in &en_words {
            if !is_clean_en(word) || word.chars().count() < ml {
                continue;
            }
            let strokes = match strokes_for(word, en_layout) {
                Some(s) => s,
                None => continue,
            };
            eligible += 1;
            match earliest_trigger(&strokes, &ctx, ml) {
                Some(tl) => positions.push(tl),
                None => {
                    if ml == 3 && never_examples.len() < 25 {
                        never_examples.push(word.clone());
                    }
                }
            }
        }
        let triggered = positions.len();
        positions.sort_unstable();
        let median = if positions.is_empty() {
            0.0
        } else {
            let m = positions.len() / 2;
            if positions.len() % 2 == 0 {
                (positions[m - 1] + positions[m]) as f64 / 2.0
            } else {
                positions[m] as f64
            }
        };
        let mean = if positions.is_empty() {
            0.0
        } else {
            positions.iter().sum::<usize>() as f64 / positions.len() as f64
        };
        println!(
            "{:>8} | {:>9} | {:>10} {:>8.1}% | {:>8} {:>8.2}",
            ml,
            eligible,
            triggered,
            100.0 * triggered as f64 / eligible.max(1) as f64,
            median,
            mean
        );
    }
    if !never_examples.is_empty() {
        println!("\nПриклади англ. слів (min_len=3), що НЕ спрацювали (recall-промахи):");
        println!("  {}", never_examples.join(", "));
    }
    println!(
        "\nПримітка: held-out uk.freq.txt (OpenSubtitles) засмічений рос. словами\n\
         (еще/где/иду/использовать/ищу — рос., НЕ укр.) і layout-шумом (уфп/фьфе/рщт/шшш),\n\
         тож реальна укр. FP-частота НИЖЧА за виміряну. Зокрема при min_len=3 два рос.\n\
         слова 'еще'(3632)+'где'(3515) дають 7147 з ~10874 FP-токенів (≈66%)."
    );
}
