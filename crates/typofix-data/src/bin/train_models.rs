//! Тренування LM/словників із повного корпусу й запис у `data/lm` + `data/dicts`.
//!
//! Той самий API, що й зразки (`train_lm`/`build_dict`/`save_lm`/`save_dict`) —
//! лише з реальним корпусом замість крихітних `data/samples/*`.
//!
//! Передумова: `data/corpora/{lang}.clean.txt` і `{lang}.words.txt` (готує
//! `data/clean_corpus.py` із завантажених Leipzig-дампів, див.
//! `data/fetch_corpora.sh`). Артефакти `.bin`/`.fst` — gitignored.
//!
//! Запуск:
//! ```text
//! python data/clean_corpus.py        # один раз після завантаження корпусів
//! cargo run -p typofix-data --bin train_models
//! ```

use std::path::{Path, PathBuf};

use typofix_core::lm::{DEFAULT_K, DEFAULT_ORDER};
use typofix_data::{build_dict, save_dict, save_lm, train_lm};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("data")
}

fn main() {
    if let Err(e) = run() {
        eprintln!("тренування не вдалося: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let data = data_dir();
    let corpora = data.join("corpora");
    let lm_dir = data.join("lm");
    let dict_dir = data.join("dicts");
    std::fs::create_dir_all(&lm_dir)?;
    std::fs::create_dir_all(&dict_dir)?;

    for lang in ["uk", "en"] {
        let corpus_path = corpora.join(format!("{lang}.clean.txt"));
        let words_path = corpora.join(format!("{lang}.words.txt"));
        if !corpus_path.exists() {
            eprintln!(
                "[{lang}] SKIP — немає {}. Спершу: data/fetch_corpora.sh + python data/clean_corpus.py",
                corpus_path.display()
            );
            continue;
        }

        // --- LM ---
        let corpus = std::fs::read_to_string(&corpus_path)?;
        let model = train_lm(&corpus, DEFAULT_ORDER, DEFAULT_K);
        let lm_path = lm_dir.join(format!("{lang}.bin"));
        save_lm(&model, &lm_path)?;

        // --- Словник ---
        let words_raw = std::fs::read_to_string(&words_path)?;
        let mut words: Vec<String> = words_raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        // Влити whitelist коротких службових слів (data/dicts/{lang}.short.txt),
        // щоб 1-2-літерні точно були у словнику незалежно від корпусу/частоти.
        // FST дедуплікує, тож повтори безпечні.
        let short_path = dict_dir.join(format!("{lang}.short.txt"));
        if let Ok(short_raw) = std::fs::read_to_string(&short_path) {
            for line in short_raw.lines() {
                let w = line.trim();
                if !w.is_empty() && !w.starts_with('#') {
                    words.push(w.to_owned());
                }
            }
        }
        let dict = build_dict(words.iter().map(String::as_str))?;
        let dict_path = dict_dir.join(format!("{lang}.fst"));
        save_dict(&dict, &dict_path)?;

        let lm_mb = std::fs::metadata(&lm_path)?.len() as f64 / 1e6;
        let fst_mb = std::fs::metadata(&dict_path)?.len() as f64 / 1e6;
        println!(
            "[{lang}] LM: vocab={} order={} -> {} ({:.2} МБ); словник: {} слів -> {} ({:.2} МБ)",
            model.vocab_size(),
            model.order(),
            lm_path.display(),
            lm_mb,
            dict.len(),
            dict_path.display(),
            fst_mb,
        );
    }

    println!("Готово. Перепрогін метрик: cargo run -p typofix-data --bin calibrate");
    Ok(())
}
