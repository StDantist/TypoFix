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
- **`dicts/{lang}.short.txt`** (committed) — **whitelist коротких службових слів**
  (1-2 літери; авторська компіляція — факти мови, ліцензія не діє). Навіщо: Leipzig
  `clean_corpus.py` відкидав ВСІ 1-літерні слова (`len>=1` лише з whitelist, інакше
  OCR-шум), тож `і,й,в,у,з,о,а,я,є,ж` були ПОВНІСТЮ відсутні. Тепер: (1) clean_corpus
  пускає 1-літерні з whitelist; (2) `train_models` БЕЗУМОВНО вливає весь файл у
  `.fst` (гарантія незалежно від корпусу/частоти); (3) ті самі слова додано в
  `samples/*.words.txt` для fallback. Кожне слово валідовано дзеркально: en-двійник
  НЕ валідне англ. слово (0 конфліктів). `#` — коментар. **Інтеграцію whitelist у
  ПОРІГ детектора (`threshold(1)=5.0 > dict_bonus`) робить core (Den)** — тут лише
  ДАНІ. Формат для core: один рядок = одне слово (lowercase), `#`-коментарі;
  шлях `data/dicts/{lang}.short.txt`; читати як список членства.
- **`dicts/{lang}.full.txt`** (gitignored, великий) — **повний морфословник**
  (uk: VESUM/dict_uk, ~3.82 млн словоформ; en: поки нема). Готує `fetch_dict_uk.py`
  (download релізу brown-uk/dict_uk v6.8.0 → bunzip → витяг чистих укр. словоформ).
  `train_models` БЕЗУМОВНО вливає його у `.fst` поряд із корпусним словником і
  whitelist (FST дедуплікує). Підсумковий `uk.fst` ≈ 3.67 МБ (FST стискає флексії).
  Покриває інфлексію (`привіт/привіту/привітом…`) і розмовну лексику (`рашка`,
  `кацап`, `москаль` — Є у VESUM). Ліцензія CC BY-NC-SA — особисте некомерційне
  використання (див. `NOTICE.md`). LM (`uk.bin`) НЕ залежить від цього (char-level,
  тренується на зв'язному корпусі, не на вордлісті).
- `eval/` — розмічений датасет калібрування: позитив (крякозябри↔правильні) +
  **негативний клас** (легітимні uk/en/змішані/сленг/код, які НЕ перемикати).
  Без реальних секретів (див. `fixtures/CLAUDE.md`).
- Українська LM/словник: **усі словоформи** (флективна мова), не лише леми.
