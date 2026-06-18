# -*- coding: utf-8 -*-
"""
Очищення сирих корпусів Leipzig у монолінгвальний текст для тренування LM/словника.

Джерело: Leipzig Corpora Collection (https://wortschatz-leipzig.de/), формат
`<id>\\t<речення>` на рядок. Завантаження — `data/fetch_corpora.sh`.

Очищення (навіщо): char-trigram LM розрізняє мови за символами, тож корпус має
бути МОНОЛІНГВАЛЬНИМ — лишаємо лише токени цільового алфавіту (кирилиця для uk,
латиниця для en) + апострофи (`'`,`’`). Латинські імена в uk-тексті (і навпаки)
викидаємо, щоб не змішувати n-грами й тримати V (розмір алфавіту) малим — це
покращує розділення мов і відтворюваність.

Вивід:
- `data/corpora/{lang}.clean.txt` — очищений текст (по реченню на рядок) → train_lm.
- `data/corpora/{lang}.words.txt` — словник (lowercase, унікальні, відсортовані),
  з words-файлу за частотою (>= MIN_COUNT) → build_dict.

ПРИВАТНІСТЬ: Wikipedia — публічний текст; жодних особистих даних не додаємо.

Запуск: python data/clean_corpus.py
"""

import os
import re

HERE = os.path.dirname(__file__)
CORP = os.path.join(HERE, "corpora")
DICTS = os.path.join(HERE, "dicts")


def load_short_whitelist(lang):
    """Короткі службові слова (1-2 літери) з data/dicts/{lang}.short.txt.

    Потрібні, щоб пускати 1-літерні слова крізь фільтр довжини нижче (інакше
    легітимні `і, й, в, у, з, о, а, я, є, ж` гинуть як OCR-шум). # — коментар.
    """
    path = os.path.join(DICTS, f"{lang}.short.txt")
    words = set()
    if not os.path.exists(path):
        return words
    with open(path, encoding="utf-8") as f:
        for line in f:
            w = line.strip()
            if w and not w.startswith("#"):
                words.add(w.lower())
    return words

# Український алфавіт (без російських ы/ъ/э/ё) + апострофи.
UK_LETTERS = set("абвгґдеєжзиіїйклмнопрстуфхцчшщьюя")
EN_LETTERS = set("abcdefghijklmnopqrstuvwxyz")
APOS = {"'", "’"}

# Мінімальна частота слова, щоб потрапити у словник (відсікає OCR-шум/одруки).
MIN_COUNT = 5

SOURCES = {
    "uk": {
        "sentences": "ukr_wikipedia_2021_100K/ukr_wikipedia_2021_100K-sentences.txt",
        "words": "ukr_wikipedia_2021_100K/ukr_wikipedia_2021_100K-words.txt",
        "letters": UK_LETTERS,
    },
    "en": {
        "sentences": "eng_wikipedia_2016_100K/eng_wikipedia_2016_100K-sentences.txt",
        "words": "eng_wikipedia_2016_100K/eng_wikipedia_2016_100K-words.txt",
        "letters": EN_LETTERS,
    },
}

# Токен = пробіг літер/апострофів (як у typofix_core::lm::tokenize).
TOKEN_RE = re.compile(r"[^\W\d_]+(?:['’][^\W\d_]+)*", re.UNICODE)


def clean_token(tok, letters):
    """Lowercase; лишити токен, лише якщо всі його літери — цільового алфавіту."""
    t = tok.lower()
    core = [c for c in t if c not in APOS]
    if not core:
        return None
    if all(c in letters for c in core):
        return t.strip("'’")
    return None


def clean_sentences(path, letters):
    kept_lines = 0
    tokens_out = 0
    out_lines = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            # відкидаємо колонку id (до першого табу)
            text = line.split("\t", 1)[-1]
            toks = []
            for m in TOKEN_RE.findall(text):
                ct = clean_token(m, letters)
                if ct:
                    toks.append(ct)
            if toks:
                out_lines.append(" ".join(toks))
                kept_lines += 1
                tokens_out += len(toks)
    return out_lines, kept_lines, tokens_out


# Сміттєві КОРОТКІ токени (OCR/токенізаційний шум, НЕ реальні слова), що потрапили
# в корпус і ХИБНО блокують перемикання частих двійників іншою мовою: напр. en `nf`
# був членом en.fst → `current_is_dict` блокував укр. `та` (nf→та). Прибираємо ЛИШЕ
# ЯВНЕ сміття — реальні абревіатури (`db`/`bp`/`lt`/`nt`/`ye`) НЕ чіпаємо (вони мають
# лишатися у словнику як precision-захист). Деталі: crates/typofix-core/CLAUDE.md.
JUNK_SHORT = {"nf"}


def clean_words(path, letters, short_whitelist):
    words = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            # формат: id \t word \t count (інколи без count)
            if len(parts) < 2:
                continue
            word = parts[1]
            count = 0
            if len(parts) >= 3 and parts[2].isdigit():
                count = int(parts[2])
            if count < MIN_COUNT:
                continue
            ct = clean_token(word, letters)
            if ct in JUNK_SHORT:
                continue
            # 1-літерні пускаємо ЛИШЕ з whitelist (інакше OCR-шум/латиниця).
            if ct and (len(ct) >= 2 or ct in short_whitelist):
                words.append(ct)
    return sorted(set(words))


def main():
    for lang, cfg in SOURCES.items():
        sent_path = os.path.join(CORP, cfg["sentences"])
        word_path = os.path.join(CORP, cfg["words"])
        if not os.path.exists(sent_path):
            print(f"[{lang}] SKIP — немає {sent_path}")
            continue

        lines, kept, ntok = clean_sentences(sent_path, cfg["letters"])
        out_corpus = os.path.join(CORP, f"{lang}.clean.txt")
        with open(out_corpus, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines) + "\n")

        words = clean_words(word_path, cfg["letters"], load_short_whitelist(lang))
        out_words = os.path.join(CORP, f"{lang}.words.txt")
        with open(out_words, "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(words) + "\n")

        size_mb = os.path.getsize(out_corpus) / 1e6
        print(
            f"[{lang}] corpus: {kept} рядків, {ntok} токенів, {size_mb:.1f} МБ "
            f"-> {os.path.basename(out_corpus)}; словник: {len(words)} слів "
            f"-> {os.path.basename(out_words)}"
        )


if __name__ == "__main__":
    main()
