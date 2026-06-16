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

# Беремо лише форми ДОВЖИНОЮ >= MIN_LEN. Чому: VESUM містить рідкісні короткі форми
# (`мус`,`фал`,`ші`,`ше`,`ру`...), які (а) збігаються з код/сленг-токенами → FP, і
# (б) роблять кирилічний двійник коротких en-слів "словниковим" → ламають дзеркальну
# релаксацію детектора (core, Den) → FN на pos_uk_for_en. Короткі слова (<=3) і так
# покриті корпусом + whitelist (`*.short.txt`, відкалібровано Den). Емпірично на eval:
# повний VESUM → precision 98.8%/recall 91.1%; VESUM[len>=4] → precision 100%/recall
# 95.5% (vs baseline 100%/95.0%). Втрата покриття мізерна (~3.4k форм із 3.82M).
MIN_LEN = 4


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
    forms = set()
    total = 0
    with open(SRC, encoding="utf-8") as f:
        for line in f:
            total += 1
            line = line.strip()
            if not line:
                continue
            tok = line.split(None, 1)[0]  # 1-ша колонка = поверхнева форма
            low = tok.lower().strip("'’")
            if len(low) >= MIN_LEN and is_clean_uk_form(low):
                forms.add(low)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for w in sorted(forms):
            f.write(w + "\n")
    print(f"рядків прочитано: {total}; унікальних чистих укр. форм: {len(forms)}")
    print(f"-> {OUT} ({os.path.getsize(OUT) / 1e6:.1f} МБ тексту)")


if __name__ == "__main__":
    main()
