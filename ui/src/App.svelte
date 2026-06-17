<script>
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "./i18n.js";
  import {
    loadSettings,
    saveSettings,
    pickExe,
    pickFolder,
  } from "./lib/api.js";
  import Toggle from "./lib/Toggle.svelte";
  import RuleList from "./lib/RuleList.svelte";
  import ProcessPicker from "./lib/ProcessPicker.svelte";

  /** Дефолти-дзеркало бекенду (на випадок запуску поза Tauri / першого старту). */
  function defaultSettings() {
    return {
      version: 1,
      enabled: true,
      language: "uk-en",
      exclusions: { process_names: [], exe_paths: [], folders: [] },
      words: { always_switch: [], never_switch: [] },
      detection: { min_word_len: 3, confidence_threshold: 0.75 },
    };
  }

  let settings = $state(defaultSettings());
  /** Останній збережений знімок — база для визначення «брудних» змін. */
  let baseline = $state(JSON.stringify(defaultSettings()));
  let processInput = $state("");
  let alwaysWordInput = $state("");
  let neverWordInput = $state("");
  /** Чи показано модалку-пікер запущених процесів. */
  let showProcessPicker = $state(false);
  /** @type {"" | "saved" | "saveError" | "loadError"} */
  let statusKey = $state("");
  let statusDetail = $state("");

  const dirty = $derived(JSON.stringify(settings) !== baseline);

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
    return () => {
      unlisten.then((fn) => fn());
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
</script>

<main>
  <header class="page-head">
    <h1>{$t("settings.title")}</h1>
    <p class="subtitle">{$t("settings.subtitle")}</p>
  </header>

  <!-- Загальне: увімкнено / пауза -->
  <section class="card">
    <h2>{$t("section.general.title")}</h2>
    <div class="row">
      <Toggle bind:checked={settings.enabled} label={$t("toggle.enabled.label")} />
      <span class="hint">
        {settings.enabled ? $t("toggle.enabled.on") : $t("toggle.enabled.off")}
      </span>
    </div>
  </section>

  <!-- Мовна пара -->
  <section class="card">
    <h2>{$t("section.language.title")}</h2>
    <p class="desc">{$t("section.language.desc")}</p>
    <select bind:value={settings.language}>
      <option value="uk-en">{$t("language.uk-en")}</option>
    </select>
  </section>

  <!-- Виключення -->
  <section class="card">
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
  <section class="card">
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
    <div class="status">
      {#if statusKey === "saved"}
        <span class="ok">✓ {$t("status.saved")}</span>
      {:else if statusKey === "saveError"}
        <span class="err" title={statusDetail}>{$t("status.saveError")}: {statusDetail}</span>
      {:else if statusKey === "loadError"}
        <span class="err" title={statusDetail}>{$t("status.loadError")}: {statusDetail}</span>
      {:else if dirty}
        <span class="dim">{$t("status.dirty")}</span>
      {/if}
    </div>
    <div class="buttons">
      <button class="secondary" onclick={reload} disabled={!dirty}>
        {$t("action.cancel")}
      </button>
      <button class="primary" onclick={save} disabled={!dirty}>
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
</style>
