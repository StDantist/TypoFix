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

    // Виключення
    "section.exclusions.title": "Виключення (застосунки / папки)",
    "section.exclusions.desc":
      "Де TypoFix узагалі не діятиме: за іменем процесу, конкретним exe або цілою текою (рекурсивно).",
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
    "words.always.placeholder": "напр. лох, eurusd",
    "words.never.placeholder": "напр. vec, npm",
    "words.add.always": "Додати слово",
    "words.add.never": "Додати слово",

    // Advanced
    "section.detection.title": "Поріг впевненості (advanced)",
    "section.detection.desc":
      "Наскільки впевнено має бути визначено помилку, щоб перенабрати текст. Вища влучність = менше хибних спрацювань.",
    "detection.minWordLen": "Мінімальна довжина слова",
    "detection.threshold": "Поріг впевненості",

    // Дії
    "action.save": "Зберегти",
    "action.cancel": "Скасувати",
    "status.saved": "Збережено",
    "status.saveError": "Помилка збереження",
    "status.loadError": "Помилка завантаження конфігу",
    "status.dirty": "Є незбережені зміни",

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
