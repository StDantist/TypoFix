# -*- coding: utf-8 -*-
"""
Частотні списки слів для EN та UK із OpenSubtitles (розмовний регістр).

Джерело: hermitdave/FrequencyWords (OpenSubtitles 2018), ліцензія MIT. Формат
вхідних `{lang}_full.txt`: `слово count` на рядок. Чому OpenSubtitles, а не наш
Leipzig/Wikipedia: Вікіпедія — формальний регістр, де розмовні короткі слова
(`ну`,`ха`,`що`) мають count≈1, нерозрізнювані від шуму (`ші`,`ру`). Субтитри
дають реальну розмовну частоту → саме вона розв'язує precision/recall.

Вивід (gitignored, у data/corpora/freq/): `{uk,en}.freq.txt` — `слово<TAB>count`,
відсортовано за словом, для побудови fst::Map (`{lang}.freq.fst`) у train_models.

Чистка EN: лише `a-z`; ВИКИНУТО фрагменти контракцій субтитрів (`s d t m ll re ve`
та одиночні літери крім `a`/`i`) — вони НЕ слова (з `it's`→`s`), інакше бруднять
негатив-клас і дзеркальну перевірку. Поріг `freq>=MIN_COUNT` (старт 5).
Чистка UK: лише укр. алфавіт (+апострофи); `freq>=MIN_COUNT`.

ЛІЦЕНЗІЯ даних: MIT (hermitdave) поверх OpenSubtitles. Деталі: data/NOTICE.md.

Запуск: python data/fetch_freq.py   (качає вхідні, якщо їх ще немає)
"""

import os
import urllib.request

HERE = os.path.dirname(__file__)
FREQ = os.path.join(HERE, "corpora", "freq")
URL = ("https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/"
       "content/2018/{lang}/{lang}_full.txt")

UK_LETTERS = set("абвгґдеєжзиіїйклмнопрстуфхцчшщьюя")
EN_LETTERS = set("abcdefghijklmnopqrstuvwxyz")
APOS = {"'", "’"}

MIN_COUNT = 5

# Фрагменти контракцій OpenSubtitles (не справжні англ. слова).
EN_CONTRACTION_FRAGMENTS = {"s", "d", "t", "m", "ll", "re", "ve"}


def download(lang, dst):
    if os.path.exists(dst):
        return
    url = URL.format(lang=lang)
    print(f"[{lang}] завантаження {url}")
    urllib.request.urlretrieve(url, dst)


def clean_en(word):
    w = word.lower()
    if not w or any(c not in EN_LETTERS for c in w):
        return None
    if w in EN_CONTRACTION_FRAGMENTS:
        return None
    if len(w) == 1 and w not in ("a", "i"):
        return None
    return w


def clean_uk(word):
    w = word.lower().strip("'’")
    core = [c for c in w if c not in APOS]
    if not core or any(c not in UK_LETTERS for c in core):
        return None
    return w


def build(lang, cleaner):
    src = os.path.join(FREQ, f"{lang}_full.txt")
    download(lang, src)
    out = {}
    with open(src, encoding="utf-8") as f:
        for line in f:
            parts = line.split()
            if len(parts) != 2 or not parts[1].isdigit():
                continue
            count = int(parts[1])
            if count < MIN_COUNT:
                continue
            w = cleaner(parts[0])
            if w:
                out[w] = out.get(w, 0) + count  # злити дублі після lowercase
    dst = os.path.join(FREQ, f"{lang}.freq.txt")
    with open(dst, "w", encoding="utf-8", newline="\n") as f:
        for w in sorted(out):
            f.write(f"{w}\t{out[w]}\n")
    print(f"[{lang}] {len(out)} слів (freq>={MIN_COUNT}) -> {dst}")
    return out


def main():
    os.makedirs(FREQ, exist_ok=True)
    en = build("en", clean_en)
    uk = build("uk", clean_uk)
    print("--- перевірка цілей ---")
    for w in ("the", "you", "and"):
        print(f"  en {w}: {en.get(w, 'ABSENT')}")
    for w in ("ye", "lox", "qwe"):
        print(f"  en {w}: {en.get(w, 'ABSENT')}")
    for w in ("що", "щоб", "ну", "ха", "то", "ще", "ші", "ру"):
        print(f"  uk {w}: {uk.get(w, 'ABSENT')}")


if __name__ == "__main__":
    main()
