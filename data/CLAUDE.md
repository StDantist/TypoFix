# data/ — правила

Мовно-залежні дані (мовна-агностичність: додати мову = додати дані, не код).

- `layouts/{id}.toml` — масив записів `[[key]]` із `scancode → {normal, shift?,
  altgr?}`. **Це лише fallback/еталон.** У рантаймі мапінг беремо з ОС
  (`ToUnicodeEx`/`UCKeyTranslate`), бо TOML не покриває AltGr/dead-keys і
  фактичну розкладку системи.
  - **Конвенція scancode — Windows scancode set 1** (make-коди: `Q=0x10`,
    `A=0x1E`, `G=0x22`, пробіл `0x39`). Це **фізична** позиція клавіші, спільна
    для всіх розкладок (тому `ghbdsn` ↔ `привіт`). macOS-бекенд транслює свої
    keycode у цю ж конвенцію. Парсер/мапінг: `typofix-data`,
    `typofix-core::layout_mapper`. Символи у Unicode (апостроф — `’` U+2019).
- `lm/{lang}.bin` — натреновані n-gram моделі; `dicts/{lang}.fst` — словники;
  `corpora/` — сирі корпуси. **Усі ці артефакти gitignored** (великі, генеровані).
  У git тримаємо лише ВХІДНІ дані (layouts), **вбудовані зразки** і **код генерації**.
  - **`.bin`** = `bincode`-серіалізована `typofix_core::NgramModel` (символьна
    n-gram, add-k згладжування, лічильники у `BTreeMap` → детерміновані байти).
    API: `typofix_data::{train_lm, serialize_lm/deserialize_lm, save_lm/load_lm_file}`.
  - **`.fst`** = сирі байти `fst::Set` (`typofix_core::Dictionary`). Слова — нижній
    регістр, відсортовані/унікальні. API: `build_dict, save_dict/load_dict_file`.
  - **`samples/`** (committed, кілька КБ): `{uk,en}.corpus.txt` +
    `{uk,en}.words.txt`. Тести й дефолт будують модель/FST із них **у рантаймі**
    (`sample_lm`/`sample_dict`), бо `.bin`/`.fst` не комітяться. `load_lm`/`load_dict`
    беруть `override_dir/{lang}.{bin,fst}`, інакше fallback на зразок.
  - **Повний корпус (зроблено).** Джерело — Leipzig Corpora Collection
    (Wikipedia uk/en, публічний текст, 100K речень кожна). Пайплайн:
    1. `bash data/fetch_corpora.sh` — завантажує дампи в `corpora/` (gitignored).
    2. `python data/clean_corpus.py` — монолінгвальне очищення (лише цільовий
       алфавіт+апострофи, як `lm::tokenize`) → `corpora/{lang}.clean.txt` +
       `corpora/{lang}.words.txt` (частота ≥5).
    3. `cargo run -p typofix-data --bin train_models` → `lm/{lang}.bin`,
       `dicts/{lang}.fst` (gitignored).
    4. `cargo run -p typofix-data --bin calibrate` — метрики (бере реальні
       моделі, fallback на зразки).
    Обсяг: uk 1.36M токенів / 26.6K слів, en 2.05M / 21.3K; `.bin` ≈0.2–0.4 МБ
    (чистий моноалфавіт → vocab uk=36, en=29). **Приріст метрик** (на eval-датасеті,
    `DetectorConfig::default`): recall 45.6%→**94.4%**, F1 62.6%→**96.3%**,
    precision 100%→**98.3%** (2 FP — короткі код-токени `fn`/`ls`, що збігаються з
    короткими uk-словами у словнику; решта FN — короткі слова на межі порогу →
    орієнтир для калібрування `threshold` у core).
- `eval/` — розмічений датасет калібрування: позитив (крякозябри↔правильні) +
  **негативний клас** (легітимні uk/en/змішані/сленг/код, які НЕ перемикати).
  Без реальних секретів (див. `fixtures/CLAUDE.md`).
- Українська LM/словник: **усі словоформи** (флективна мова), не лише леми.
