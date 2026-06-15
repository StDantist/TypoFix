// Мінімальний каркас локалізації без зовнішніх залежностей.
// Українська — мова за замовчуванням. Додати мову = додати об'єкт у `messages`.
// Доступ до рядків: t("settings.title"). Перемикання мови — store `locale`.

import { writable, derived } from "svelte/store";

/** @type {Record<string, Record<string, string>>} */
const messages = {
  uk: {
    "app.name": "TypoFix",
    "settings.title": "Налаштування TypoFix",
    "settings.subtitle":
      "Виправлення тексту, набраного в неправильній розкладці",

    "section.languagePairs.title": "Мовні пари",
    "section.languagePairs.desc":
      "Між якими розкладками виправляти текст (напр. українська ⇄ англійська).",

    "section.threshold.title": "Поріг впевненості",
    "section.threshold.desc":
      "Наскільки впевнено має бути визначено помилку, щоб перенабрати текст. Вища влучність = менше хибних спрацювань.",

    "section.exclusions.title": "Виключення (застосунки/папки)",
    "section.exclusions.desc":
      "Програми та шляхи, у яких TypoFix не діятиме (термінали, ігри, IDE тощо).",

    "section.rules.title": "Правила та винятки",
    "section.rules.desc":
      "Слова й шаблони, які завжди ігнорувати або, навпаки, завжди виправляти.",

    "section.hotkeys.title": "Гарячі клавіші",
    "section.hotkeys.desc":
      "Ручне перемикання останнього слова, пауза/відновлення, скасування виправлення.",

    "placeholder.empty": "Каркас — налаштування з'являться згодом.",
    "footer.note": "Скелет інтерфейсу. Логіку розпізнавання ще не під'єднано.",
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
