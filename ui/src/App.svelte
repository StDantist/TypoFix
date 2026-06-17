<script>
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "./i18n.js";
  import {
    loadSettings,
    saveSettings,
    resetSettings,
    pickExe,
    pickFolder,
    getAutostart,
    setAutostart,
    listLearned,
    removeLearned,
    clearLearned,
  } from "./lib/api.js";
  import Toggle from "./lib/Toggle.svelte";
  import RuleList from "./lib/RuleList.svelte";
  import ProcessPicker from "./lib/ProcessPicker.svelte";

  /** @typedef {import("./lib/api.js").AppSettings} AppSettings */
  /** @typedef {import("./lib/api.js").Behavior} Behavior */
  /** @typedef {import("./lib/api.js").Hotkeys} Hotkeys */

  /** Дефолти-дзеркало бекенду (на випадок запуску поза Tauri / першого старту). */
  function defaultSettings() {
    return {
      version: 4,
      enabled: true,
      language: "uk-en",
      exclusions: { process_names: [], exe_paths: [], folders: [] },
      words: { always_switch: [], never_switch: [] },
      behavior: {
        fix_case: true,
        forex: true,
        recognize_extensions: true,
        phonotactics: true,
        fix_capslock: true,
      },
      feedback: { sound_on_switch: false },
      hotkeys: {
        pause_resume: { accelerator: "Ctrl+Alt+P", enabled: false },
        revert_last: { accelerator: "Ctrl+Alt+Z", enabled: false },
        manual_switch: { accelerator: "Ctrl+Alt+S", enabled: false },
        case_upper: { accelerator: "Ctrl+Alt+U", enabled: false },
        case_lower: { accelerator: "Ctrl+Alt+L", enabled: false },
        case_sentence: { accelerator: "Ctrl+Alt+E", enabled: false },
      },
      detection: { min_word_len: 3, confidence_threshold: 0.75 },
    };
  }

  /**
   * Перемикачі поведінки (B4): ключ у `settings.behavior` + i18n-підписи.
   * @type {{ key: keyof Behavior, hint: string }[]}
   */
  const BEHAVIOR_TOGGLES = [
    { key: "fix_case", hint: "behavior.fix_case.hint" },
    { key: "forex", hint: "behavior.forex.hint" },
    { key: "recognize_extensions", hint: "behavior.recognize_extensions.hint" },
    { key: "phonotactics", hint: "behavior.phonotactics.hint" },
    { key: "fix_capslock", hint: "behavior.fix_capslock.hint" },
  ];

  /**
   * Порядок дій у картці хоткеїв (дзеркало `HotkeyAction::ALL` у бекенді).
   * @type {(keyof Hotkeys)[]}
   */
  const HOTKEY_ACTIONS = [
    "pause_resume",
    "revert_last",
    "manual_switch",
    "case_upper",
    "case_lower",
    "case_sentence",
  ];

  /** Зібрати рядок-акселератор Tauri з події клавіатури (`Ctrl+Alt+P`). */
  function accelFromEvent(/** @type {KeyboardEvent} */ e) {
    const mods = [];
    if (e.ctrlKey) mods.push("Ctrl");
    if (e.altKey) mods.push("Alt");
    if (e.shiftKey) mods.push("Shift");
    if (e.metaKey) mods.push("Super");
    const key = e.key;
    // Самі модифікатори — ще не повна комбінація.
    if (["Control", "Alt", "Shift", "Meta"].includes(key)) return null;
    const main = key.length === 1 ? key.toUpperCase() : key;
    return [...mods, main].join("+");
  }

  /** Захопити комбінацію у поле акселератора (Backspace/Delete — очистити). */
  function captureAccel(/** @type {KeyboardEvent} */ e, /** @type {keyof Hotkeys} */ action) {
    if (e.key === "Tab") return; // не ламаємо навігацію
    if (e.key === "Backspace" || e.key === "Delete") {
      settings.hotkeys[action].accelerator = "";
      e.preventDefault();
      return;
    }
    const accel = accelFromEvent(e);
    if (accel) settings.hotkeys[action].accelerator = accel;
    e.preventDefault();
  }

  /** @type {AppSettings} */
  let settings = $state(defaultSettings());
  /** Останній збережений знімок — база для визначення «брудних» змін. */
  let baseline = $state(JSON.stringify(defaultSettings()));
  let processInput = $state("");
  let alwaysWordInput = $state("");
  let neverWordInput = $state("");
  /** Чи показано модалку-пікер запущених процесів. */
  let showProcessPicker = $state(false);
  /** @type {"" | "saved" | "reset" | "saveError" | "loadError"} */
  let statusKey = $state("");
  let statusDetail = $state("");
  /** Чи показано підтвердження скидання параметрів до стандартних. */
  let showResetConfirm = $state(false);

  // Автозапуск (B5). НЕ частина settings.json — джерело істини сам плагін (реєстр).
  // `applied` = останнє значення, надіслане в бекенд: guard, щоб $effect не
  // викликав setAutostart на початкове завантаження чи на оновлення з трею.
  let autostart = $state(false);
  let autostartApplied = $state(false);
  let autostartError = $state(false);

  // Навчені слова (B3): авто-навчені винятки з диска (learned_exceptions.txt).
  // Окремий від `settings` стан — це не конфіг, а список, керований движком.
  /** @type {string[]} */
  let learned = $state([]);
  let learnedError = $state(false);

  async function reloadLearned() {
    try {
      learned = await listLearned();
      learnedError = false;
    } catch {
      learnedError = true;
    }
  }

  async function removeLearnedWord(/** @type {string} */ word) {
    try {
      await removeLearned(word);
      await reloadLearned();
    } catch {
      learnedError = true;
    }
  }

  async function clearAllLearned() {
    try {
      await clearLearned();
      await reloadLearned();
    } catch {
      learnedError = true;
    }
  }

  $effect(() => {
    const want = autostart;
    if (want === autostartApplied) return; // ініціалізація / синк із трею — не реагуємо
    autostartApplied = want;
    setAutostart(want)
      .then((actual) => {
        autostartError = false;
        // Узгоджуємо з фактичним станом плагіна (раптом enable/disable не вдалось).
        if (actual !== want) {
          autostartApplied = actual;
          autostart = actual;
        }
      })
      .catch(() => {
        autostartError = true;
      });
  });

  const dirty = $derived(JSON.stringify(settings) !== baseline);

  // Людська «чутливість» поверх технічного `confidence_threshold`.
  // Обережно = ВИЩИЙ поріг (менше спрацювань), Агресивно = НИЖЧИЙ поріг.
  // Слайдер 0 (обережно) → 100 (агресивно) мапиться лінійно у [1.0 .. 0.5].
  const THR_CAUTIOUS = 1.0; // sensitivity 0
  const THR_AGGRESSIVE = 0.5; // sensitivity 100
  const sensitivity = $derived(
    Math.round(
      ((THR_CAUTIOUS -
        Math.max(
          THR_AGGRESSIVE,
          Math.min(THR_CAUTIOUS, settings.detection.confidence_threshold),
        )) /
        (THR_CAUTIOUS - THR_AGGRESSIVE)) *
        100,
    ),
  );
  function setSensitivity(/** @type {number|string} */ v) {
    const s = Math.max(0, Math.min(100, Number(v)));
    settings.detection.confidence_threshold =
      THR_CAUTIOUS - (s / 100) * (THR_CAUTIOUS - THR_AGGRESSIVE);
  }

  function applyLoaded(/** @type {any} */ loaded) {
    settings = loaded;
    baseline = JSON.stringify(loaded);
  }

  async function reload() {
    try {
      applyLoaded(await loadSettings());
      statusKey = "";
    } catch (err) {
      statusKey = "loadError";
      statusDetail = String(err);
    }
  }

  onMount(() => {
    reload();
    reloadLearned();
    // Стан автозапуску читаємо з плагіна (реєстр — джерело істини), не з конфігу.
    getAutostart()
      .then((on) => {
        autostartApplied = on; // спершу applied, щоб $effect не «застосовував» назад
        autostart = on;
      })
      .catch(() => {});
    // Трей може змінити «увімкнено» поза формою — синхронізуємо перемикач,
    // не чіпаючи решту (можливих незбережених) правок.
    const unlisten = listen("settings:changed", (event) => {
      const payload = /** @type {any} */ (event.payload);
      if (payload && typeof payload.enabled === "boolean") {
        settings.enabled = payload.enabled;
        // Трей уже записав це на диск → оновлюємо базу для цього поля.
        const b = JSON.parse(baseline);
        b.enabled = payload.enabled;
        baseline = JSON.stringify(b);
      }
    });
    // Трей може перемкнути автозапуск — синхронізуємо чекбокс БЕЗ повторного запису
    // (спершу applied = payload, тож $effect побачить рівність і не викличе бекенд).
    const unlistenAutostart = listen("autostart:changed", (event) => {
      const on = /** @type {any} */ (event.payload);
      if (typeof on === "boolean") {
        autostartApplied = on;
        autostart = on;
      }
    });
    return () => {
      unlisten.then((fn) => fn());
      unlistenAutostart.then((fn) => fn());
    };
  });

  /** Додати рядок у список, якщо непорожній і ще не присутній. */
  function addUnique(/** @type {string[]} */ list, /** @type {string} */ value) {
    const v = value.trim();
    if (v && !list.includes(v)) list.push(v);
  }

  function addProcess() {
    addUnique(settings.exclusions.process_names, processInput);
    processInput = "";
  }

  async function addExe() {
    const path = await pickExe();
    if (path) addUnique(settings.exclusions.exe_paths, path);
  }

  async function addFolder() {
    const path = await pickFolder();
    if (path) addUnique(settings.exclusions.folders, path);
  }

  function addAlwaysWord() {
    addUnique(settings.words.always_switch, alwaysWordInput);
    alwaysWordInput = "";
  }

  function addNeverWord() {
    addUnique(settings.words.never_switch, neverWordInput);
    neverWordInput = "";
  }

  async function save() {
    try {
      applyLoaded(await saveSettings(settings));
      statusKey = "saved";
      statusDetail = "";
    } catch (err) {
      statusKey = "saveError";
      statusDetail = String(err);
    }
  }

  /** Скинути параметри до стандартних (бекенд зберігає exclusions/words/паузу). */
  async function doReset() {
    showResetConfirm = false;
    try {
      applyLoaded(await resetSettings());
      statusKey = "reset";
      statusDetail = "";
    } catch (err) {
      statusKey = "saveError";
      statusDetail = String(err);
    }
  }
</script>

<svelte:window
  on:keydown={(e) => {
    if (showResetConfirm && e.key === "Escape") showResetConfirm = false;
  }}
/>

<main>
  <header class="page-head">
    <h1>{$t("settings.title")}</h1>
    <p class="subtitle">{$t("settings.subtitle")}</p>
  </header>

  <!-- Загальне: увімкнено / пауза -->
  <section class="card" data-testid="card-general">
    <h2>{$t("section.general.title")}</h2>
    <div class="row">
      <Toggle
        bind:checked={settings.enabled}
        label={$t("toggle.enabled.label")}
        testid="enabled-toggle"
      />
      <span class="hint">
        {settings.enabled ? $t("toggle.enabled.on") : $t("toggle.enabled.off")}
      </span>
    </div>
  </section>

  <!-- Мовна пара -->
  <section class="card" data-testid="card-language">
    <h2>{$t("section.language.title")}</h2>
    <p class="desc">{$t("section.language.desc")}</p>
    <select bind:value={settings.language} data-testid="language-select">
      <option value="uk-en">{$t("language.uk-en")}</option>
    </select>
    <p class="hint lang-note">{$t("section.language.note")}</p>
  </section>

  <!-- Виключення -->
  <section class="card" data-testid="card-exclusions">
    <h2>{$t("section.exclusions.title")}</h2>
    <p class="desc">{$t("section.exclusions.desc")}</p>

    <div class="add-controls">
      <form
        class="add-process"
        onsubmit={(e) => {
          e.preventDefault();
          addProcess();
        }}
      >
        <input
          type="text"
          bind:value={processInput}
          placeholder={$t("exclusions.process.placeholder")}
        />
        <button type="submit" disabled={!processInput.trim()}>
          {$t("exclusions.add.process")}
        </button>
      </form>
      <button type="button" onclick={() => (showProcessPicker = true)}>
        {$t("exclusions.add.fromRunning")}
      </button>
      <button type="button" onclick={addExe}>{$t("exclusions.add.exe")}</button>
      <button type="button" onclick={addFolder}>{$t("exclusions.add.folder")}</button>
    </div>

    <div class="lists">
      <RuleList
        title={$t("exclusions.list.process")}
        kindLabel={$t("exclusions.kind.process")}
        items={settings.exclusions.process_names}
        onremove={(i) => settings.exclusions.process_names.splice(i, 1)}
      />
      <RuleList
        title={$t("exclusions.list.exe")}
        kindLabel={$t("exclusions.kind.exe")}
        items={settings.exclusions.exe_paths}
        onremove={(i) => settings.exclusions.exe_paths.splice(i, 1)}
      />
      <RuleList
        title={$t("exclusions.list.folder")}
        kindLabel={$t("exclusions.kind.folder")}
        items={settings.exclusions.folders}
        onremove={(i) => settings.exclusions.folders.splice(i, 1)}
      />
    </div>
  </section>

  <!-- Слова-винятки (особистий словник) -->
  <section class="card" data-testid="card-words">
    <h2>{$t("section.words.title")}</h2>
    <p class="desc">{$t("section.words.desc")}</p>

    <div class="word-group">
      <form
        class="add-process"
        onsubmit={(e) => {
          e.preventDefault();
          addAlwaysWord();
        }}
      >
        <input
          type="text"
          bind:value={alwaysWordInput}
          placeholder={$t("words.always.placeholder")}
        />
        <button type="submit" disabled={!alwaysWordInput.trim()}>
          {$t("words.add.always")}
        </button>
      </form>
      <RuleList
        title={$t("words.list.always")}
        kindLabel={$t("words.kind.always")}
        items={settings.words.always_switch}
        onremove={(i) => settings.words.always_switch.splice(i, 1)}
      />
    </div>

    <div class="word-group">
      <form
        class="add-process"
        onsubmit={(e) => {
          e.preventDefault();
          addNeverWord();
        }}
      >
        <input
          type="text"
          bind:value={neverWordInput}
          placeholder={$t("words.never.placeholder")}
          data-testid="never-word-input"
        />
        <button type="submit" disabled={!neverWordInput.trim()}>
          {$t("words.add.never")}
        </button>
      </form>
      <RuleList
        title={$t("words.list.never")}
        kindLabel={$t("words.kind.never")}
        items={settings.words.never_switch}
        onremove={(i) => settings.words.never_switch.splice(i, 1)}
      />
    </div>
  </section>

  <!-- Навчені слова (B3): авто-навчені винятки, керовані движком -->
  <section class="card" data-testid="card-learned">
    <h2>{$t("section.learned.title")}</h2>
    <p class="desc">{$t("section.learned.desc")}</p>

    <div class="learned-head">
      <span class="hint" data-testid="learned-count">{$t("learned.count")} {learned.length}</span>
      <div class="learned-actions">
        <button type="button" onclick={reloadLearned}>{$t("learned.refresh")}</button>
        <button
          type="button"
          onclick={clearAllLearned}
          disabled={learned.length === 0}
        >
          {$t("learned.clearAll")}
        </button>
      </div>
    </div>

    {#if learnedError}
      <p class="err">{$t("learned.error")}</p>
    {:else if learned.length === 0}
      <p class="muted" data-testid="learned-empty">{$t("learned.empty")}</p>
    {:else}
      <ul class="learned-list">
        {#each learned as word (word)}
          <li>
            <span class="badge">{$t("learned.badge")}</span>
            <code title={word}>{word}</code>
            <button
              class="rm"
              title={$t("learned.remove")}
              aria-label={$t("learned.remove")}
              onclick={() => removeLearnedWord(word)}>✕</button
            >
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <!-- Поведінка (B4): тоггли евристик + людський повзунок чутливості -->
  <section class="card" data-testid="card-behavior">
    <h2>{$t("section.behavior.title")}</h2>
    <p class="desc">{$t("section.behavior.desc")}</p>

    <div class="behavior-list">
      {#each BEHAVIOR_TOGGLES as b (b.key)}
        <div class="behavior-row">
          <Toggle
            bind:checked={settings.behavior[b.key]}
            label={$t(`behavior.${b.key}`)}
            testid={`behavior-${b.key}`}
          />
          <span class="hint">{$t(b.hint)}</span>
        </div>
      {/each}
    </div>

    <div class="sensitivity">
      <span class="sens-title">{$t("behavior.sensitivity.title")}</span>
      <div class="sens-row">
        <span class="sens-end">{$t("behavior.sensitivity.cautious")}</span>
        <input
          type="range"
          min="0"
          max="100"
          step="5"
          value={sensitivity}
          data-testid="sensitivity-slider"
          oninput={(e) => setSensitivity(e.currentTarget.value)}
        />
        <span class="sens-end">{$t("behavior.sensitivity.aggressive")}</span>
      </div>
      <p class="desc">{$t("behavior.sensitivity.hint")}</p>
    </div>
  </section>

  <!-- Системне (B5): автозапуск разом із Windows -->
  <section class="card" data-testid="card-system">
    <h2>{$t("section.system.title")}</h2>
    <p class="desc">{$t("section.system.desc")}</p>
    <div class="behavior-row">
      <Toggle bind:checked={autostart} label={$t("system.autostart")} testid="autostart-toggle" />
      <span class="hint">{$t("system.autostart.hint")}</span>
    </div>
    {#if autostartError}
      <p class="err">{$t("system.autostart.error")}</p>
    {/if}
  </section>

  <!-- Звук і сповіщення (B2) -->
  <section class="card" data-testid="card-feedback">
    <h2>{$t("section.feedback.title")}</h2>
    <p class="desc">{$t("section.feedback.desc")}</p>
    <div class="behavior-row">
      <Toggle
        bind:checked={settings.feedback.sound_on_switch}
        label={$t("feedback.sound_on_switch")}
      />
      <span class="hint">{$t("feedback.sound_on_switch.hint")}</span>
    </div>
  </section>

  <!-- Гарячі клавіші -->
  <section class="card" data-testid="card-hotkeys">
    <h2>{$t("section.hotkeys.title")}</h2>
    <p class="desc">{$t("section.hotkeys.desc")}</p>

    <div class="hotkey-list">
      {#each HOTKEY_ACTIONS as action (action)}
        <div class="hotkey-row">
          <input
            type="checkbox"
            aria-label={$t("hotkeys.enabled.aria")}
            bind:checked={settings.hotkeys[action].enabled}
          />
          <span class="hk-label">{$t(`hotkeys.action.${action}`)}</span>
          <input
            class="hk-accel"
            type="text"
            bind:value={settings.hotkeys[action].accelerator}
            placeholder={$t("hotkeys.accel.placeholder")}
            disabled={!settings.hotkeys[action].enabled}
            onkeydown={(e) => captureAccel(e, action)}
          />
        </div>
      {/each}
    </div>

    <p class="hk-note">{$t("hotkeys.note")}</p>
  </section>

  <!-- Advanced: пороги детектора -->
  <section class="card">
    <h2>{$t("section.detection.title")}</h2>
    <p class="desc">{$t("section.detection.desc")}</p>
    <div class="field">
      <label for="minlen">{$t("detection.minWordLen")}</label>
      <input
        id="minlen"
        type="number"
        min="1"
        max="20"
        bind:value={settings.detection.min_word_len}
      />
    </div>
    <div class="field">
      <label for="thr">{$t("detection.threshold")}</label>
      <input
        id="thr"
        type="range"
        min="0"
        max="1"
        step="0.01"
        bind:value={settings.detection.confidence_threshold}
      />
      <output>{Number(settings.detection.confidence_threshold).toFixed(2)}</output>
    </div>
  </section>

  <!-- Панель дій -->
  <footer class="actions">
    <div class="status" data-testid="save-status" data-status={statusKey}>
      {#if statusKey === "saved"}
        <span class="ok" data-testid="status-saved">✓ {$t("status.saved")}</span>
      {:else if statusKey === "reset"}
        <span class="ok" data-testid="status-reset">✓ {$t("status.reset")}</span>
      {:else if statusKey === "saveError"}
        <span class="err" title={statusDetail}>{$t("status.saveError")}: {statusDetail}</span>
      {:else if statusKey === "loadError"}
        <span class="err" title={statusDetail}>{$t("status.loadError")}: {statusDetail}</span>
      {:else if dirty}
        <span class="dim">{$t("status.dirty")}</span>
      {/if}
    </div>
    <div class="buttons">
      <button
        class="secondary"
        onclick={() => (showResetConfirm = true)}
        data-testid="reset-button"
      >
        {$t("action.reset")}
      </button>
      <button class="secondary" onclick={reload} disabled={!dirty}>
        {$t("action.cancel")}
      </button>
      <button class="primary" onclick={save} disabled={!dirty} data-testid="save-button">
        {$t("action.save")}
      </button>
    </div>
  </footer>

  <p class="privacy-note">{$t("footer.note")}</p>
</main>

{#if showProcessPicker}
  <ProcessPicker
    added={settings.exclusions.process_names}
    onpick={(name) => addUnique(settings.exclusions.process_names, name)}
    onclose={() => (showProcessPicker = false)}
  />
{/if}

{#if showResetConfirm}
  <!-- Підкладка: клік поза вмістом закриває (логіка лише на backdrop, як у ProcessPicker) -->
  <div
    class="modal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) showResetConfirm = false;
    }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      aria-label={$t("reset.confirm.title")}
      data-testid="reset-modal"
    >
      <h2>{$t("reset.confirm.title")}</h2>
      <p>{$t("reset.confirm.body")}</p>
      <div class="modal-actions">
        <button
          class="secondary"
          onclick={() => (showResetConfirm = false)}
          data-testid="reset-cancel"
        >
          {$t("reset.confirm.cancel")}
        </button>
        <button class="primary" onclick={doReset} data-testid="reset-confirm">
          {$t("reset.confirm.ok")}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 1.5rem 1.25rem 2.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .page-head h1 {
    font-size: 1.4rem;
  }

  .subtitle {
    margin: 0.35rem 0 0;
    color: var(--text-dim);
  }

  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1rem 1.25rem;
  }

  .card h2 {
    font-size: 1rem;
  }

  .desc {
    margin: 0.35rem 0 0.85rem;
    color: var(--text-dim);
    font-size: 0.85rem;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-top: 0.5rem;
  }

  .hint {
    color: var(--text-dim);
    font-size: 0.85rem;
  }

  .lang-note {
    margin: 0.6rem 0 0;
  }

  select,
  input[type="text"],
  input[type="number"] {
    background: var(--bg-elev);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    font: inherit;
  }

  .add-controls {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .add-process {
    display: flex;
    gap: 0.4rem;
    flex: 1;
    min-width: 240px;
  }

  .add-process input {
    flex: 1;
  }

  .lists {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }

  .word-group {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .word-group + .word-group {
    margin-top: 1rem;
  }

  .behavior-list {
    display: flex;
    flex-direction: column;
    gap: 0.7rem;
  }

  .behavior-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
  }

  .behavior-row .hint {
    flex: 1;
    min-width: 180px;
  }

  .sensitivity {
    margin-top: 1.1rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
  }

  .sens-title {
    font-size: 0.95rem;
    font-weight: 600;
  }

  .sens-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin: 0.6rem 0 0.2rem;
  }

  .sens-row input[type="range"] {
    flex: 1;
  }

  .sens-end {
    font-size: 0.82rem;
    color: var(--text-dim);
    white-space: nowrap;
  }

  .hotkey-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .hotkey-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .hotkey-row .hk-label {
    flex: 1;
    font-size: 0.9rem;
  }

  .hotkey-row .hk-accel {
    width: 160px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .hk-note {
    margin: 0.85rem 0 0;
    color: var(--text-dim);
    font-size: 0.78rem;
  }

  .field {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 0.5rem;
  }

  .field label {
    width: 220px;
    font-size: 0.9rem;
  }

  .field output {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    min-width: 2.5rem;
  }

  button {
    font: inherit;
    cursor: pointer;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-elev);
    color: var(--text);
    padding: 0.4rem 0.8rem;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  button:not(:disabled):hover {
    border-color: var(--accent);
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    position: sticky;
    bottom: 0;
    padding: 0.75rem 0;
    background: linear-gradient(transparent, var(--bg) 35%);
  }

  .buttons {
    display: flex;
    gap: 0.5rem;
  }

  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .status .ok {
    color: #3ba55d;
  }
  .status .err {
    color: #d9534f;
  }

  .card .err {
    margin: 0.6rem 0 0;
    color: #d9534f;
    font-size: 0.85rem;
  }

  /* Навчені слова (B3) */
  .learned-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    flex-wrap: wrap;
    margin-bottom: 0.75rem;
  }

  .learned-actions {
    display: flex;
    gap: 0.5rem;
  }

  .learned-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .learned-list li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.4rem;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .learned-list .badge {
    flex: none;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
  }

  .learned-list code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
    font-size: 0.85rem;
  }

  .learned-list .rm {
    flex: none;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 0.9rem;
    padding: 0.1rem 0.3rem;
    border-radius: 4px;
  }

  .learned-list .rm:hover {
    color: #fff;
    background: #d9534f;
  }

  .muted {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.85rem;
    font-style: italic;
  }
  .status .dim {
    color: var(--text-dim);
    font-size: 0.85rem;
  }

  .privacy-note {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.78rem;
    text-align: center;
  }

  /* Модалка підтвердження скидання */
  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1.25rem 1.5rem;
    max-width: 420px;
    margin: 1rem;
  }

  .modal h2 {
    font-size: 1.05rem;
    margin: 0 0 0.5rem;
  }

  .modal p {
    margin: 0 0 1.1rem;
    color: var(--text-dim);
    font-size: 0.9rem;
  }

  .modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }
</style>
