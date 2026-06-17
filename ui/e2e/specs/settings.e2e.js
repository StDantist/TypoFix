// Повна клік-проходка по вікну налаштувань TypoFix через tauri-driver (WebView2).
// Застосунок запущено з TYPOFIX_E2E=1 → вікно видиме, движок/хоткеї НЕ стартують,
// тож тест ніяк не чіпає глобальну клавіатуру.
//
// Селектори — стабільні `data-testid` з `ui/src/App.svelte` (+ видимі тексти з
// `ui/src/i18n.js`). Асерти — на текст/стан DOM, не на пікселі.

/** Дочекатись і повернути елемент за data-testid. */
async function tid(name, timeout = 15000) {
  const el = await $(`[data-testid="${name}"]`);
  await el.waitForExist({ timeout });
  return el;
}

/**
 * Клік із попереднім скролом у ЦЕНТР в'юпорта. Липкий футер `.actions`
 * (`position: sticky; bottom: 0`) перекриває елементи внизу видимої області
 * (WebDriver скролить елемент саме під нього) → «element click intercepted».
 * Центрування виводить ціль з-під футера.
 */
async function clickCentered(el) {
  await el.scrollIntoView({ block: "center", inline: "center" });
  await el.click();
}

describe("TypoFix — вікно налаштувань (UI-e2e)", () => {
  it("заголовок сторінки відрендерився", async () => {
    const h1 = await $("h1");
    await h1.waitForExist({ timeout: 20000 });
    // i18n: settings.title = «Налаштування TypoFix»
    expect(await h1.getText()).toContain("TypoFix");
  });

  it("усі ключові картки присутні (заголовки)", async () => {
    const expected = [
      ["card-hotkeys", "Гарячі клавіші"],
      ["card-behavior", "Поведінка"],
      ["card-feedback", "Звук і сповіщення"],
      ["card-system", "Системне"],
      ["card-learned", "Навчені слова"],
      ["card-language", "Мовна пара"],
      ["card-exclusions", "Вимкнено для програм"],
      ["card-words", "Слова-винятки"],
    ];
    for (const [testid, heading] of expected) {
      const card = await tid(testid);
      const h2 = await card.$("h2");
      expect(await h2.getText()).toContain(heading);
    }
  });

  it("картка «Поведінка»: 5 тогглів + повзунок чутливості наявні", async () => {
    const keys = [
      "fix_case",
      "forex",
      "recognize_extensions",
      "phonotactics",
      "fix_capslock",
    ];
    for (const k of keys) {
      const toggle = await tid(`behavior-${k}`);
      expect(await toggle.isExisting()).toBe(true);
    }
    const slider = await tid("sensitivity-slider");
    expect(await slider.getAttribute("type")).toBe("range");
  });

  it("клік по тогглу поведінки міняє його стан", async () => {
    const label = await tid("behavior-fix_case");
    const input = await $('[data-testid="behavior-fix_case-input"]');
    const before = await input.isSelected();
    await clickCentered(label);
    await browser.waitUntil(async () => (await input.isSelected()) !== before, {
      timeout: 5000,
      timeoutMsg: "стан тоггла не змінився після кліку",
    });
    expect(await input.isSelected()).toBe(!before);
    // Повертаємо у вихідний стан, щоб не лишати «брудних» правок.
    await clickCentered(label);
    await browser.waitUntil(async () => (await input.isSelected()) === before, {
      timeout: 5000,
    });
  });

  it("повзунок чутливості рухається (Стрілка вправо)", async () => {
    const slider = await tid("sensitivity-slider");
    const before = Number(await slider.getValue());
    await clickCentered(slider);
    await browser.keys(["ArrowRight"]);
    await browser.waitUntil(
      async () => Number(await slider.getValue()) !== before,
      { timeout: 5000, timeoutMsg: "значення повзунка не змінилось" },
    );
    const after = Number(await slider.getValue());
    expect(after).not.toBe(before);
    // Відкотити на крок назад (чистий стан).
    await browser.keys(["ArrowLeft"]);
  });

  it("картка «Навчені слова» відкривається (список або дружній порожній стан)", async () => {
    await tid("card-learned");
    const count = await tid("learned-count");
    expect(await count.getText()).toContain("Слів у списку");
    const empty = await $('[data-testid="learned-empty"]');
    const list = await $(".learned-list");
    // Або список, або порожній стан — обидва валідні (дружній UX).
    expect((await empty.isExisting()) || (await list.isExisting())).toBe(true);
  });

  it("селектор мовної пари показує uk-en", async () => {
    const select = await tid("language-select");
    expect(await select.getValue()).toBe("uk-en");
  });

  it("секція «Розкладки клавіатури» рендериться зі щонайменше однією розкладкою", async () => {
    const card = await tid("card-layouts");
    const h2 = await card.$("h2");
    expect(await h2.getText()).toContain("Розкладки клавіатури");
    // На реальній Windows-машині встановлена принаймні одна розкладка (англ.).
    const list = await tid("layouts-list");
    const items = await list.$$("li");
    expect(items.length).toBeGreaterThanOrEqual(1);
    // Кнопка «Оновити» працює без помилки (список лишається непорожнім).
    await clickCentered(await tid("layouts-refresh"));
    await browser.waitUntil(async () => (await list.$$("li")).length >= 1, {
      timeout: 5000,
      timeoutMsg: "після оновлення список розкладок порожній",
    });
  });

  it("клік «Зберегти» не дає помилки", async () => {
    // Робимо форму «брудною» (вмикаємо Save): перемикаємо тоггл звуку в картці.
    const soundLabel = await $('[data-testid="card-feedback"] .toggle');
    await clickCentered(soundLabel);

    const save = await tid("save-button");
    await browser.waitUntil(async () => await save.isEnabled(), {
      timeout: 5000,
      timeoutMsg: "кнопка «Зберегти» не активувалась після зміни",
    });
    await clickCentered(save);

    const status = await tid("save-status");
    await browser.waitUntil(
      async () => (await status.getAttribute("data-status")) === "saved",
      { timeout: 10000, timeoutMsg: "збереження не завершилось статусом 'saved'" },
    );
    // Головний асерт: НЕ помилка збереження.
    expect(await status.getAttribute("data-status")).not.toBe("saveError");

    // Відкотити зміну й зберегти знову — лишаємо конфіг як був.
    await clickCentered(soundLabel);
    await browser.waitUntil(async () => await save.isEnabled(), { timeout: 5000 });
    await clickCentered(save);
    await browser.waitUntil(
      async () => (await status.getAttribute("data-status")) === "saved",
      { timeout: 10000 },
    );
  });

  it("«Скинути до стандартних»: параметри — дефолтні, дані користувача лишаються", async () => {
    const TEST_WORD = "zzqqe2etest";
    const save = await tid("save-button");
    const status = await tid("save-status");

    // 1) Додаємо слово-виняток (never_switch) і ЗБЕРІГАЄМО — щоб reset на бекенді
    //    мав що зберегти (reset переносить words/exclusions із персистованого стану).
    const neverInput = await tid("never-word-input");
    await neverInput.setValue(TEST_WORD);
    await browser.keys(["Enter"]);
    const wordsCard = await tid("card-words");
    await browser.waitUntil(
      async () => (await wordsCard.getText()).includes(TEST_WORD),
      { timeout: 5000, timeoutMsg: "слово не зʼявилось у списку" },
    );
    await browser.waitUntil(async () => await save.isEnabled(), { timeout: 5000 });
    await clickCentered(save);
    await browser.waitUntil(
      async () => (await status.getAttribute("data-status")) === "saved",
      { timeout: 10000 },
    );

    // 2) Псуємо «параметр»: вимикаємо тоггл поведінки fix_case (дефолт = увімкнено).
    const fixCaseInput = await $('[data-testid="behavior-fix_case-input"]');
    if (await fixCaseInput.isSelected()) {
      await clickCentered(await tid("behavior-fix_case"));
    }
    expect(await fixCaseInput.isSelected()).toBe(false);

    // 3) Скидання через модалку підтвердження.
    await clickCentered(await tid("reset-button"));
    const modal = await tid("reset-modal");
    expect(await modal.isExisting()).toBe(true);
    await clickCentered(await tid("reset-confirm"));

    await browser.waitUntil(
      async () => (await status.getAttribute("data-status")) === "reset",
      { timeout: 10000, timeoutMsg: "скидання не завершилось статусом 'reset'" },
    );

    // 4) Параметр повернувся до дефолту (fix_case знову увімкнено)...
    await browser.waitUntil(async () => await fixCaseInput.isSelected(), {
      timeout: 5000,
      timeoutMsg: "fix_case не повернувся до дефолту після скидання",
    });
    expect(await fixCaseInput.isSelected()).toBe(true);
    // ...мовна пара — дефолтна...
    expect(await (await tid("language-select")).getValue()).toBe("uk-en");
    // ...а дані користувача (слово-виняток) ЛИШИЛИСЯ.
    expect(await (await tid("card-words")).getText()).toContain(TEST_WORD);

    // Прибираємо тестове слово й зберігаємо — лишаємо конфіг як був.
    const rmBtn = await $(`//code[@title="${TEST_WORD}"]/following-sibling::button`);
    await clickCentered(rmBtn);
    await browser.waitUntil(
      async () => !(await (await tid("card-words")).getText()).includes(TEST_WORD),
      { timeout: 5000, timeoutMsg: "тестове слово не вдалось прибрати" },
    );
    await browser.waitUntil(async () => await save.isEnabled(), { timeout: 5000 });
    await clickCentered(save);
    await browser.waitUntil(
      async () => (await status.getAttribute("data-status")) === "saved",
      { timeout: 10000 },
    );
  });
});
