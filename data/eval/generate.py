# -*- coding: utf-8 -*-
"""
Генератор eval-датасету для калібрування детектора TypoFix.

Тримаємо ГЕНЕРАТОР у git (а не лише результат) заради відтворюваності — так само,
як скрипти генерації LM/словників (див. data/CLAUDE.md). Запуск:

    python data/eval/generate.py

Перезаписує `data/eval/dataset.jsonl`. Схему й призначення див. data/eval/CLAUDE.md.

ПРИВАТНІСТЬ: усі дані тут СИНТЕТИЧНІ. Нуль реальних секретів — паролів, токенів,
особистих даних. URL/email/шляхи — лише плейсхолдери (example.com, /usr/...).
"""

import json
import os

# --- Карта фізичних позицій клавіш en(QWERTY) <-> uk(ЙЦУКЕН) ----------------
# Джерело істини: data/layouts/{en,uk}.toml (Windows scancode set 1).
# Ключ = символ у розкладці en, значення = символ на ТІЙ САМІЙ фізичній клавіші
# в розкладці uk. Тому "ghbdsn" (фізично) читається як "привіт".
EN2UK = {
    "q": "й", "w": "ц", "e": "у", "r": "к", "t": "е", "y": "н", "u": "г",
    "i": "ш", "o": "щ", "p": "з", "[": "х", "]": "ї",
    "a": "ф", "s": "і", "d": "в", "f": "а", "g": "п", "h": "р", "j": "о",
    "k": "л", "l": "д", ";": "ж", "'": "є",
    "z": "я", "x": "ч", "c": "с", "v": "м", "b": "и", "n": "т", "m": "ь",
    ",": "б", ".": "ю", "/": ".", "\\": "ґ", "`": "’",
}
UK2EN = {v: k for k, v in EN2UK.items()}


def translate(word, mapping):
    """Покласти кожен символ на ту саму фізичну клавішу в іншій розкладці.

    Регістр зберігаємо; символи поза мапою (пробіл, цифри) лишаємо як є.
    """
    out = []
    for ch in word:
        low = ch.lower()
        if low in mapping:
            mapped = mapping[low]
            out.append(mapped.upper() if ch.isupper() else mapped)
        else:
            out.append(ch)
    return "".join(out)


# --- POSITIVE: реальні слова, набрані у НЕПРАВИЛЬНІЙ розкладці ---------------
# Користувач хотів UK-слово, але активна була EN -> на екрані латинські
# крякозябри. should_switch=true, typed=en, intended=uk.
UK_WORDS = [
    # короткі (детектору найважче — мало контексту). «ви»(db)/«де»(lt) ПРИБРАНО:
    # їхній en-двійник — РЕАЛЬНИЙ англ. токен у en.fst (db=database, lt=less-than),
    # тож ядро свідомо НЕ перемикає (precision); вони тепер у NEG_SHORT_EN_ABBREV.
    "як", "що", "не", "на", "по", "до", "ми", "ти", "та", "чи",
    # середні
    "привіт", "дякую", "добрий", "день", "ранок", "вечір", "ніч", "друг",
    "мама", "тато", "сонце", "небо", "море", "вода", "хліб", "час", "рік",
    "місто", "село", "мова", "слово", "книга", "школа", "робота", "гроші",
    "любов", "життя", "серце", "рука", "голова", "пісня", "квітка", "зима",
    # довші
    "будинок", "телефон", "комп’ютер", "інтернет", "програма", "питання",
    "відповідь", "університет", "навчання", "майбутнє", "кохання", "здоров’я",
    "погода", "музика", "природа", "дитина", "родина", "країна", "свобода",
]

# 1-літерні службові укр. слова (сполучники/прийменники/частки/займенники):
# а,в,з,й,і,о,у,я. Їхні en-двійники — одиночні латинські літери (f,d,p,q,s,j,e,z).
# ВАЖЛИВО: ці приклади тепер НЕГАТИВНІ (див. NEG_SHORT_SINGLE нижче), бо ядро
# СВІДОМО не перемикає одиночні токени (precision-рішення, `min_switch_len`):
# самотня латинська літера в en-розкладці — це майже завжди легітимний ввід
# (індекс, ініціал, змінна), а не недонабране укр. слово. Колишня позитивна
# розмітка (`pos_short_uk_1`, switch=true) суперечила ядру й давала фантомні FN.
UK_SHORT1 = ["а", "в", "з", "й", "і", "о", "у", "я"]

# 2-літерні РЕАЛЬНІ часті укр. службові слова (прийменники/сполучники/частки/
# займенники/вигуки), чий en-двійник — беззмістовний кластер (НЕ реальне англ.
# слово). Фіз-послідовність неоднозначна, але намір імовірно укр. → switch=true.
# Це прямий запит користувача (`ти,ми,чи,до,по,на,не…` МУСЯТЬ ловитись), а шкоди
# нема — реального англ. слова не псуємо. Підмножина uk.short.txt із ЧИСТИМ
# літерним двійником (пунктуаційні `аж→f;`, `бо→,j` — поза списком).
# **ⓘ КРИТЕРІЙ «беззмістовний en-двійник» застосовано СУВОРО (precision-аудит):**
# слова, чий en-двійник Є РЕАЛЬНИМ англ. токеном у en.fst, ПРИБРАНО звідси й
# перенесено в NEG_SHORT_EN_ABBREV: `ви→db`, `ге→ut`, `де→lt`, `зі→ps`, `те→nt`
# (і `ну→ye` був поза списком). Ядро їх НЕ перемикає (`!current_is_dict`), тож
# позитивна мітка давала б фантомні FN. switch=true, typed=en, intended=uk.
UK_SHORT2 = [
    "га", "до", "за", "із", "ми", "на", "не", "ні",
    "по", "та", "ти", "то", "це", "ці", "ця", "чи", "як", "би", "ой",
    "он", "от",
]

# Користувач хотів EN-слово, але активна була UK -> на екрані кирилиця.
# should_switch=true, typed=uk, intended=en.
EN_WORDS = [
    # короткі (1-2 літери — детектору найважче; розширено реальними англ. словами)
    "go", "hi", "ok", "no", "we", "the", "and", "for", "why", "who", "you",
    "to", "is", "it", "on", "of", "in", "an", "at", "or", "by", "me", "am",
    "be", "do", "he", "if", "my", "so", "up", "us",
    # середні
    "hello", "world", "thanks", "please", "friend", "family", "love", "life",
    "work", "home", "time", "year", "today", "water", "bread", "music",
    "phone", "email", "house", "money", "happy", "smile", "dream", "light",
    "night", "table", "chair", "green", "black", "white", "house", "river",
    # довші
    "computer", "internet", "program", "question", "answer", "language",
    "keyboard", "message", "project", "meeting", "software", "beautiful",
    "important", "tomorrow", "morning", "evening", "weekend", "weather",
]

# --- NEGATIVE: легітимний текст, який перемикати НЕ можна --------------------
# (text, category, layout). Це СЕРЦЕ датасету: хибне перемикання дратує сильніше
# за пропуск, тож негатив навмисно багатий і різноманітний.
NEG_UK_LEGIT = [
    "привіт", "дякую", "будь ласка", "доброго ранку", "як справи",
    "добрий день", "гарного дня", "до зустрічі", "на все добре", "дуже дякую",
    "люблю тебе", "все буде добре", "слава україні", "з днем народження",
    "побачимось завтра", "телефонуй мені", "гарної подорожі", "смачного",
    "вибач мене", "нема за що",
]
NEG_EN_LEGIT = [
    "hello", "thank you", "good morning", "how are you", "see you later",
    "have a nice day", "good night", "what time is it", "let me know",
    "talk to you soon", "i love it", "well done", "no problem", "of course",
    "happy birthday", "take care", "see you tomorrow", "best regards",
    "looking forward", "sounds good",
]
# Короткі неоднозначні: і uk, і en варіанти існують як реальні слова.
NEG_SHORT = [
    ("по", "uk"), ("не", "uk"), ("як", "uk"), ("що", "uk"), ("на", "uk"),
    ("до", "uk"), ("ми", "uk"), ("ти", "uk"), ("де", "uk"), ("чи", "uk"),
    ("бо", "uk"), ("за", "uk"),
    ("to", "en"), ("is", "en"), ("in", "en"), ("on", "en"), ("at", "en"),
    ("it", "en"), ("of", "en"), ("or", "en"), ("by", "en"), ("no", "en"),
    ("hi", "en"), ("an", "en"),
]
# Код-токени: латиниця, низькочастотні у природній мові -> ризик хибного флагу.
NEG_CODE = [
    "git", "npm", "fn", "impl", "pub", "mut", "vec", "const", "let", "async",
    "await", "cargo", "clippy", "sudo", "cd", "ls", "grep", "http", "json",
    "html", "css", "sql", "api", "def", "var", "null", "true", "false",
    "fmt", "struct", "enum", "return",
]
# Бренди / нікнейми / юзернейми (синтетичні).
NEG_BRAND = [
    "Steam", "Discord", "Nginx", "Docker", "GitHub", "iPhone", "Android",
    "YouTube", "Spotify", "Telegram", "DarkLord", "xQc", "ProGamer",
    "noob123", "user42", "admin",
]
# URL / email / шляхи / IP — лише плейсхолдери, нуль реальних даних.
NEG_URL = [
    ("https://example.com", "en"), ("github.com/typofix/app", "en"),
    ("user@example.com", "en"), ("C:\\Users\\test\\file.txt", "en"),
    ("/usr/local/bin", "en"), ("192.168.0.1", "en"),
    ("./src/main.rs", "en"), ("www.test.org", "en"),
    ("#typofix", "en"), ("@username", "en"),
]
# Змішані uk/en фрази (домінує кирилиця -> layout uk).
NEG_MIXED = [
    "зроби git push", "купив новий iPhone", "стек на React",
    "відкрий браузер Chrome", "запусти npm install", "помилка 404 на сторінці",
    "мій нік DarkLord", "канал у Telegram", "качаю з GitHub",
    "встанови Docker зараз", "це фреймворк Svelte", "пишу на Rust",
]
# Абревіатури.
NEG_ACRONYM = [
    ("США", "uk"), ("ООН", "uk"), ("ЄС", "uk"), ("ЗСУ", "uk"), ("ВНЗ", "uk"),
    ("СБУ", "uk"),
    ("API", "en"), ("HTTP", "en"), ("FBI", "en"), ("NASA", "en"),
    ("USB", "en"), ("GPS", "en"),
]
# Сленг / жаргон / вигуки.
NEG_SLANG = [
    ("лол", "uk"), ("кек", "uk"), ("норм", "uk"), ("гг", "uk"), ("ага", "uk"),
    ("угу", "uk"), ("хех", "uk"), ("ой", "uk"),
    ("imho", "en"), ("btw", "en"), ("lol", "en"), ("omg", "en"),
    ("brb", "en"), ("afk", "en"),
]
# Буквено-цифрові токени.
NEG_ALNUM = [
    ("id42", "en"), ("v2.0", "en"), ("covid19", "en"), ("room101", "en"),
    ("win10", "en"), ("mp3", "en"), ("h2o", "en"), ("2fa", "en"),
    ("abc123", "en"), ("rust2024", "en"),
]
# Реальні короткі англ. слова (1-2 літери) у СВОЇЙ розкладці — НЕ перемикати.
# Їхні uk-двійники не-слова (a->ф, i->ш, ...), тож хибне перемикання тут особливо
# дратує. Доповнює short_ambiguous словами, яких там не було (зокрема a, i).
NEG_SHORT_EN = [
    "a", "i", "am", "as", "be", "do", "go", "he", "if", "me", "my", "ok",
    "so", "up", "us", "we", "oh",
]
# 2-літерні англ. АБРЕВІАТУРИ/токени, що Є у en.fst, але РІДКІСНІ в розмовному
# корпусі (freq < 1.0). Їхній uk-двійник часто частий (`db→ви`,`lt→де`,`nt→те`,
# `bp→из`,`ye→ну`,`ut→ге`,`ps→зі`) → САМ частотний гейт їх НЕ блокував би.
# Захищає кон'юнкція `!current_is_dict && freq<max`: член en.fst → switch=false.
# Це прямий домен власника (db=database, bp=basis points/Forex, lt=less-than).
# Аудит-знахідка: без цього класу eval був СЛІПИЙ до цих FP. switch=false,
# typed=en, intended=en. (io/ip/os/id/ai/ml/ui/ux/cv — теж реальні англ. токени,
# uk-двійник не частий/не слово → і так не перемикаються; тут для повноти класу.)
NEG_SHORT_EN_ABBREV = [
    "db", "bp", "lt", "nt", "ye", "ut", "ps",
    "io", "ip", "os", "id", "ai", "ml", "ui", "ux", "cv",
]
# "Двійники-сміття": латиниця в en-розкладці, беззмістовна в ОБИДВА боки (uk-варіант
# теж НЕ слово), тож перемикати нікуди — switch=false. Сюди НЕ входять кластери, що
# мапляться на реальні укр. слова (ті — у POSITIVE: nb=ти, vb=ми, gj=по...).
#  - одиночні латинські літери (uk-двійник не слово: g->п, h->р, n->т, ...);
#  - 2-літерні беззмістовні в обидва боки (qw->йц, zx->яч, fg->ап, kl->лд, vn->мт).
NEG_TWIN = [
    "g", "h", "k", "l", "m", "n", "r", "t", "u", "w", "x", "c",
    "qw", "zx", "fg", "kl", "vn",
]
# Одиночні латинські літери, чий uk-двійник Є реальним 1-літерним укр. словом
# (s->і, f->а, d->в, p->з, q->й, j->о, e->у, z->я). На відміну від short_twin_garbage
# (двійник-не-слово), тут укр. читання осмислене — і саме тому це найважливіший
# негатив: ядро МУСИТЬ НЕ перемикати одиночний токен, навіть коли альтернатива —
# валідне слово. Це precision-рішення (`min_switch_len`): лона латинська літера
# майже завжди легітимна (індекс `i`, ініціал, змінна), а ризик зіпсувати її
# переважує рідкісну користь від перемикання на 1-літерне службове слово.
# (Раніше ці 8 були позитивами `pos_short_uk_1` — суперечило ядру.)
NEG_SHORT_SINGLE = [translate(w, UK2EN) for w in UK_SHORT1]


def detect_layout(text):
    """uk, якщо є кирилиця; інакше en."""
    return "uk" if any("а" <= c.lower() <= "я" or c in "іїєґ’" for c in text) else "en"


def positive(text, typed, intended, category):
    return {
        "text": text, "typed_layout": typed, "intended_layout": intended,
        "should_switch": True, "category": category,
    }


def negative(text, layout, category):
    return {
        "text": text, "typed_layout": layout, "intended_layout": layout,
        "should_switch": False, "category": category,
    }


def build():
    rows = []
    # POSITIVE: en-набране-замість-uk (на екрані латинські крякозябри)
    for w in UK_WORDS:
        rows.append(positive(translate(w, UK2EN), "en", "uk", "pos_en_for_uk"))
    # POSITIVE: uk-набране-замість-en (на екрані кирилиця)
    for w in EN_WORDS:
        rows.append(positive(translate(w, EN2UK), "uk", "en", "pos_uk_for_en"))
    # POSITIVE: 2-літерні часті укр. службові у en-розкладці (беззмістовний двійник)
    for w in UK_SHORT2:
        rows.append(positive(translate(w, UK2EN), "en", "uk", "pos_short_uk_2"))

    # NEGATIVE
    for t in NEG_UK_LEGIT:
        rows.append(negative(t, "uk", "uk_legit"))
    for t in NEG_EN_LEGIT:
        rows.append(negative(t, "en", "en_legit"))
    for t, lay in NEG_SHORT:
        rows.append(negative(t, lay, "short_ambiguous"))
    for t in NEG_CODE:
        rows.append(negative(t, "en", "code"))
    for t in NEG_BRAND:
        rows.append(negative(t, detect_layout(t), "brand_nick"))
    for t, lay in NEG_URL:
        rows.append(negative(t, lay, "url_path_email"))
    for t in NEG_MIXED:
        rows.append(negative(t, "uk", "mixed"))
    for t, lay in NEG_ACRONYM:
        rows.append(negative(t, lay, "acronym"))
    for t, lay in NEG_SLANG:
        rows.append(negative(t, lay, "slang"))
    for t, lay in NEG_ALNUM:
        rows.append(negative(t, lay, "alphanumeric"))
    for t in NEG_SHORT_EN:
        rows.append(negative(t, "en", "short_en_real"))
    for t in NEG_SHORT_EN_ABBREV:
        rows.append(negative(t, "en", "short_en_abbrev"))
    for t in NEG_TWIN:
        rows.append(negative(t, "en", "short_twin_garbage"))
    # NEGATIVE: одиночні латинські літери з осмисленим uk-двійником — НЕ перемикати
    # (ядро свідомо не чіпає одиночні токени; колишні pos_short_uk_1).
    for t in NEG_SHORT_SINGLE:
        rows.append(negative(t, "en", "short_single_legit"))
    return rows


def main():
    rows = build()
    out_path = os.path.join(os.path.dirname(__file__), "dataset.jsonl")
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False, sort_keys=False) + "\n")

    pos = sum(1 for r in rows if r["should_switch"])
    neg = len(rows) - pos
    by_cat = {}
    for r in rows:
        by_cat[r["category"]] = by_cat.get(r["category"], 0) + 1
    print(f"written {len(rows)} rows -> {out_path}")
    print(f"  positive: {pos}  negative: {neg}")
    for cat in sorted(by_cat):
        print(f"    {cat:18} {by_cat[cat]}")


if __name__ == "__main__":
    main()
