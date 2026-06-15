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
  - **FOLLOW-UP (окрема задача):** повний корпус uk/en (десятки МБ) — тренувати
    тим самим `train_lm`/`build_dict` і класти у `lm/`,`dicts/`. Можливо з
    допомогою користувача через проксі (великі дампи сюди не тягнемо).
- `eval/` — розмічений датасет калібрування: позитив (крякозябри↔правильні) +
  **негативний клас** (легітимні uk/en/змішані/сленг/код, які НЕ перемикати).
  Без реальних секретів (див. `fixtures/CLAUDE.md`).
- Українська LM/словник: **усі словоформи** (флективна мова), не лише леми.
