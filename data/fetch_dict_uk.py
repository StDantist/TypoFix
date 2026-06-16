# -*- coding: utf-8 -*-
"""
Витягти ПОВНИЙ список укр. словоформ із морфословника dict_uk (VESUM, brown-uk).

Джерело: реліз brown-uk/dict_uk asset `dict_corp_vis.txt.bz2` (тегований словник,
рядок = `словоформа лема:теги`; додаткові форми леми — з відступом). Завантаження:

    gh release download v6.8.0 --repo brown-uk/dict_uk \\
        --pattern 'dict_corp_vis.txt.bz2' --dir data/corpora
    bunzip2 -k data/corpora/dict_corp_vis.txt.bz2

Цей скрипт бере 1-шу колонку (поверхневу форму), фільтрує до ОДНО-ТОКЕННИХ укр.
форм (лише укр. алфавіт + апострофи, як typofix_core::lm::tokenize; дефісні
складені/латиниця/абревіатури з крапками відкидаються — їх однаково не запитати як
один токен), lowercase, унікальні, відсортовані → data/dicts/uk.full.txt.

Вивід gitignored (великий, генерований) — у git лише цей скрипт. train_models
вливає uk.full.txt у uk.fst поряд із корпусним словником і whitelist.

ЛІЦЕНЗІЯ: дані VESUM — CC BY-NC-SA 4.0 (некомерційно). Дозволено власником проєкту
для особистого некомерційного використання. Деталі: data/NOTICE.md.

Запуск: python data/fetch_dict_uk.py
"""

import os

HERE = os.path.dirname(__file__)
SRC = os.path.join(HERE, "corpora", "dict_corp_vis.txt")
OUT = os.path.join(HERE, "dicts", "uk.full.txt")

# Той самий алфавіт, що в clean_corpus.py (без рос. ы/ъ/э/ё) + апострофи.
UK_LETTERS = set("абвгґдеєжзиіїйклмнопрстуфхцчшщьюя")
APOS = {"'", "’"}

# Довгі форми (>= LONG_LEN) беремо всі. Короткі (<= 3) — ЛИШЕ з подвійним гейтом
# `розмовна_частота >= SHORT_FREQ_MIN` (∩ VESUM автоматично, бо тягнемо з VESUM).
# Чому: повний VESUM містить рідкісні короткі форми (`ші`,`ше`,`ру`...), які (а)
# збігаються з код/сленг → FP, (б) роблять кирилічний двійник коротких en-слів
# "словниковим" → ламають релаксацію детектора (core/Den) → FN. Частотний гейт
# пускає лише ЧАСТІ розмовні короткі (`що`,`щоб`,`ну`,`ха`,`то`,`ще`), відсікаючи
# шум. Частоти — OpenSubtitles (data/fetch_freq.py → corpora/freq/uk.freq.txt).
# Емпірично: повний VESUM → P98.8/R91.1; len>=4 → P100/R95.5; +частотні короткі
# повертають короткий recall БЕЗ шуму. SHORT_FREQ_MIN — калібрувальний поріг.
LONG_LEN = 4
SHORT_FREQ_MIN = 200
FREQ_PATH = os.path.join(HERE, "corpora", "freq", "uk.freq.txt")


def load_short_freq_gate():
    """Множина коротких (<=3) укр. слів із частотою >= SHORT_FREQ_MIN."""
    gate = set()
    if not os.path.exists(FREQ_PATH):
        print(f"  УВАГА: немає {FREQ_PATH} — короткі форми НЕ ввійдуть "
              f"(спершу: python data/fetch_freq.py). Лише форми >= {LONG_LEN} літер.")
        return gate
    with open(FREQ_PATH, encoding="utf-8") as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) == 2 and parts[1].isdigit():
                w = parts[0]
                if len(w) <= 3 and int(parts[1]) >= SHORT_FREQ_MIN:
                    gate.add(w)
    return gate


def is_clean_uk_form(tok):
    """Лишити форму, лише якщо всі літери — укр. алфавіту (апострофи дозволені
    всередині). Відкидає латиницю, цифри, дефіси, крапки (абревіатури)."""
    core = [c for c in tok if c not in APOS]
    if not core:
        return False
    return all(c in UK_LETTERS for c in core)


def main():
    if not os.path.exists(SRC):
        raise SystemExit(
            f"немає {SRC} — спершу завантаж і розпакуй dict_corp_vis.txt.bz2 "
            f"(див. docstring)"
        )
    short_gate = load_short_freq_gate()
    forms = set()
    short_kept = 0
    total = 0
    with open(SRC, encoding="utf-8") as f:
        for line in f:
            total += 1
            line = line.strip()
            if not line:
                continue
            tok = line.split(None, 1)[0]  # 1-ша колонка = поверхнева форма
            low = tok.lower().strip("'’")
            if not is_clean_uk_form(low):
                continue
            if len(low) >= LONG_LEN:
                forms.add(low)
            elif low in short_gate:  # короткі — лише частотний гейт
                if low not in forms:
                    short_kept += 1
                forms.add(low)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for w in sorted(forms):
            f.write(w + "\n")
    print(f"рядків прочитано: {total}; унікальних чистих укр. форм: {len(forms)}"
          f" (з них коротких <=3 за частотним гейтом: {short_kept})")
    print(f"-> {OUT} ({os.path.getsize(OUT) / 1e6:.1f} МБ тексту)")


if __name__ == "__main__":
    main()
