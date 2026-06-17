// Мінімальний каркас локалізації без зовнішніх залежностей.
// Українська — мова за замовчуванням. Додати мову = додати об'єкт у `messages`.
// Доступ до рядків: $t("settings.title"). Перемикання мови — store `locale`.

import { writable, derived } from "svelte/store";

/** @type {Record<string, Record<string, string>>} */
const messages = {
  uk: {
    "app.name": "TypoFix",
    "settings.title": "Налаштування TypoFix",
    "settings.subtitle":
      "Виправлення тексту, набраного в неправильній розкладці",

    // Загальний стан
    "section.general.title": "Загальне",
    "toggle.enabled.label": "Увімкнено",
    "toggle.enabled.on": "TypoFix активний",
    "toggle.enabled.off": "TypoFix на паузі",

    // Мовна пара
    "section.language.title": "Мовна пара",
    "section.language.desc": "Між якими розкладками виправляти текст.",
    "language.uk-en": "Українська ⇄ Англійська",
    "section.language.note":
      "Поки доступна лише ця пара. Інші мови з'являться з додаванням словників (layout / LM / dict).",

    // Виключення (= повне вимкнення per-app: движок ПОВНІСТЮ обходить такі вікна)
    "section.exclusions.title": "Вимкнено для програм / папок",
    "section.exclusions.desc":
      "У цих програмах TypoFix узагалі не працює: не стежить за вводом і не перемикає розкладку. Задайте за іменем процесу, конкретним exe або цілою текою (рекурсивно).",
    "exclusions.kind.process": "процес",
    "exclusions.kind.exe": "exe",
    "exclusions.kind.folder": "тека",
    "exclusions.list.process": "Процеси",
    "exclusions.list.exe": "Файли (.exe)",
    "exclusions.list.folder": "Теки",
    "exclusions.empty": "Порожньо",
    "exclusions.process.placeholder": "напр. game.exe",
    "exclusions.add.process": "Додати процес",
    "exclusions.add.fromRunning": "Обрати із запущених…",
    "exclusions.add.exe": "Додати .exe…",
    "exclusions.add.folder": "Додати теку…",
    "exclusions.remove": "Видалити",

    // Пікер запущених процесів
    "picker.title": "Запущені процеси",
    "picker.filter.placeholder": "Пошук за іменем або шляхом…",
    "picker.refresh": "Оновити список",
    "picker.windowsOnly": "Лише застосунки з вікнами",
    "picker.hint.uncheck": "Серед застосунків з вікнами нічого не знайдено.",
    "picker.hint.showAll": "Показати всі процеси",
    "picker.loading": "Завантаження списку процесів…",
    "picker.none": "Нічого не знайдено",
    "picker.error": "Не вдалося отримати список процесів",
    "picker.added": "додано",
    "picker.close": "Закрити",
    "picker.done": "Готово",

    // Слова-винятки (особистий словник)
    "section.words.title": "Слова-винятки",
    "section.words.desc":
      "Особистий словник за словами. «Завжди перемикати» — слова, які TypoFix має визнавати й перемикати (жаргон, нікнейми, forex-пари). «Ніколи не перемикати» — слова, які лишати недоторканими.",
    "words.kind.always": "перемикати",
    "words.kind.never": "не чіпати",
    "words.list.always": "Завжди перемикати",
    "words.list.never": "Ніколи не перемикати",
    "words.always.placeholder": "напр. вжух, eurusd",
    "words.never.placeholder": "напр. vec, npm",
    "words.add.always": "Додати слово",
    "words.add.never": "Додати слово",

    // Розкладки клавіатури (візуалізація: які дві TypoFix використовує)
    "section.layouts.title": "Розкладки клавіатури",
    "section.layouts.desc":
      "Встановлені в системі розкладки. TypoFix працює лише з двома (поточна мовна пара); решту не чіпає.",
    "layouts.refresh": "Оновити",
    "layouts.badge.used": "використовується",
    "layouts.badge.ignored": "ігнорується",
    "layouts.active": "активна",
    "layouts.explain.lead": "TypoFix перемикає лише між",
    "layouts.explain.and": "та",
    "layouts.explain.tail": ". Інші розкладки не чіпаються.",
    "layouts.missing.lead": "Розкладку",
    "layouts.missing.tail":
      "не встановлено — перемикання на неї не працюватиме. Додайте її в розкладках Windows.",
    "layouts.lang.uk": "українську",
    "layouts.lang.en": "англійську",
    "layouts.none": "Розкладок не знайдено.",
    "layouts.error": "Не вдалося отримати список розкладок.",

    // Системне (B5): автозапуск
    "section.system.title": "Системне",
    "section.system.desc": "Інтеграція з операційною системою.",
    "system.autostart": "Запускати разом із Windows",
    "system.autostart.hint":
      "TypoFix запускатиметься автоматично при вході в систему (згорнутий у трей).",
    "system.autostart.error": "Не вдалося змінити автозапуск",

    // Звук і сповіщення (B2)
    "section.feedback.title": "Звук і сповіщення",
    "section.feedback.desc": "Як TypoFix підтверджує виправлення.",
    "feedback.sound_on_switch": "Звук при перемиканні",
    "feedback.sound_on_switch.hint":
      "короткий сигнал щоразу, коли TypoFix перенабирає слово (за замовчуванням вимкнено)",

    // Навчені слова (B3): авто-навчені винятки (відкинуті користувачем перенабори)
    "section.learned.title": "Навчені слова",
    "section.learned.desc":
      "Слова, які TypoFix більше не чіпає, бо ви скасували їх перенабір (Backspace одразу після виправлення або хоткей «Скасувати»). Приберіть помилкове — і TypoFix знову на нього реагуватиме.",
    "learned.count": "Слів у списку:",
    "learned.refresh": "Оновити",
    "learned.clearAll": "Очистити все",
    "learned.remove": "Прибрати",
    "learned.empty": "Список порожній — TypoFix ще нічого не завчив.",
    "learned.error": "Не вдалося завантажити список навчених слів",
    "learned.badge": "слово",

    // Гарячі клавіші
    "section.hotkeys.title": "Гарячі клавіші",
    "section.hotkeys.desc":
      "Глобальні комбінації для швидких дій. Усі вимикані й перепризначувані: увімкніть потрібні, клацніть поле й натисніть бажану комбінацію (Backspace — очистити). Дефолти неконфліктні (Ctrl+Alt+…).",
    "hotkeys.action.pause_resume": "Пауза / відновлення",
    "hotkeys.action.revert_last": "Скасувати останнє перемикання",
    "hotkeys.action.manual_switch": "Перемкнути розкладку вручну",
    "hotkeys.action.case_upper": "Регістр: ВЕЛИКІ",
    "hotkeys.action.case_lower": "Регістр: малі",
    "hotkeys.action.case_sentence": "Регістр: Як речення",
    "hotkeys.accel.placeholder": "напр. Ctrl+Alt+P",
    "hotkeys.enabled.aria": "Увімкнути цей хоткей",
    "hotkeys.note":
      "Дії з виділенням (регістр, ручне перемикання) працюють лише коли TypoFix активний (не на паузі).",

    // Поведінка (B4)
    "section.behavior.title": "Поведінка",
    "section.behavior.desc":
      "Які типи виправлень TypoFix робить. Вимикайте те, що заважає — решта працюватиме як є.",
    "behavior.fix_case": "Виправляти регістр",
    "behavior.fix_case.hint": "ПРивіт → Привіт (перетриманий Shift)",
    "behavior.forex": "Forex-режим",
    "behavior.forex.hint": "валютні пари та коди валют (EURUSD, USD)",
    "behavior.recognize_extensions": "Розпізнавати файлові розширення",
    "behavior.recognize_extensions.hint": ".txt, .md та інші — не плутати з укр. словами",
    "behavior.phonotactics": "Фонотактика української",
    "behavior.phonotactics.hint": "неможливі для української сполуки (напр. ь на початку)",
    "behavior.fix_capslock": "Виправляти випадковий CapsLock",
    "behavior.fix_capslock.hint": "пРИВІТ → Привіт (ненавмисний CapsLock)",
    // Чутливість (людський слайдер поверх порога впевненості)
    "behavior.sensitivity.title": "Чутливість",
    "behavior.sensitivity.cautious": "Обережно",
    "behavior.sensitivity.aggressive": "Агресивно",
    "behavior.sensitivity.hint":
      "Обережно = менше хибних спрацювань; Агресивно = більше виправлень (вищий ризик зайвого).",

    // Advanced
    "section.detection.title": "Поріг впевненості (розширені)",
    "section.detection.desc":
      "Наскільки впевнено має бути визначено помилку, щоб перенабрати текст. Вища влучність = менше хибних спрацювань.",
    "detection.minWordLen": "Мінімальна довжина слова",
    "detection.threshold": "Поріг впевненості",

    // Дії
    "action.save": "Зберегти",
    "action.cancel": "Скасувати",
    "action.reset": "Скинути до стандартних",
    "status.saved": "Збережено",
    "status.reset": "Скинуто до стандартних",
    "status.saveError": "Помилка збереження",
    "status.loadError": "Помилка завантаження конфігу",
    "status.dirty": "Є незбережені зміни",

    // Скидання параметрів до стандартних (зберігає списки/словники/навчені слова)
    "reset.confirm.title": "Скинути параметри до стандартних?",
    "reset.confirm.body":
      "Поведінка, чутливість, хоткеї, звук і мовна пара повернуться до стандартних значень. Списки програм, словники й навчені слова залишаться без змін.",
    "reset.confirm.ok": "Скинути",
    "reset.confirm.cancel": "Скасувати",

    "footer.note":
      "Конфіг зберігається локально. Натиски й набраний текст ніколи не пишуться на диск.",
  },
};

const DEFAULT_LOCALE = "uk";

export const locale = writable(DEFAULT_LOCALE);

/**
 * Реактивна функція перекладу: `$t("ключ")`.
 * Повертає сам ключ, якщо переклад відсутній (видно прогалини в розробці).
 */
export const t = derived(locale, ($locale) => {
  const dict = messages[$locale] ?? messages[DEFAULT_LOCALE];
  return (/** @type {string} */ key) => dict[key] ?? key;
});
