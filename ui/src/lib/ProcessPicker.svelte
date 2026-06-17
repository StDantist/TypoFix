<script>
  import { onMount } from "svelte";
  import { t } from "../i18n.js";
  import { listRunningProcesses } from "./api.js";

  /**
   * Модалка-пікер запущених процесів. Клік по запису → `onpick(name)`
   * (додавання робить батько через addUnique). Можна додати кілька й закрити.
   * @type {{
   *   onpick: (name: string) => void,
   *   onclose: () => void,
   *   added?: string[],
   * }}
   */
  let { onpick, onclose, added = [] } = $props();

  /** @type {import("./api.js").ProcessEntry[]} */
  let processes = $state([]);
  let filter = $state("");
  /** Показувати лише застосунки з видимим вікном (увімкнено за замовчуванням). */
  let windowsOnly = $state(true);
  let loading = $state(true);
  /** @type {string} */
  let errorDetail = $state("");

  const addedLower = $derived(new Set(added.map((s) => s.toLowerCase())));

  /** Збіг із текстовим пошуком (без урахування фільтра вікон). */
  function matchesQuery(/** @type {import("./api.js").ProcessEntry} */ p, /** @type {string} */ q) {
    if (!q) return true;
    return (
      p.name.toLowerCase().includes(q) ||
      (p.exe_path ?? "").toLowerCase().includes(q)
    );
  }

  const filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return processes.filter(
      (p) => (!windowsOnly || p.has_window) && matchesQuery(p, q),
    );
  });

  /** Чи є збіги пошуку, приховані саме фільтром «лише з вікнами» (для натяку). */
  const hiddenByWindowFilter = $derived.by(() => {
    if (!windowsOnly) return 0;
    const q = filter.trim().toLowerCase();
    return processes.filter((p) => !p.has_window && matchesQuery(p, q)).length;
  });

  async function refresh() {
    loading = true;
    errorDetail = "";
    try {
      processes = await listRunningProcesses();
    } catch (err) {
      errorDetail = String(err);
      processes = [];
    } finally {
      loading = false;
    }
  }

  onMount(refresh);

  function onKeydown(/** @type {KeyboardEvent} */ e) {
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    }
  }
</script>

<svelte:window on:keydown={onKeydown} />

<!-- Підкладка: клік поза вмістом закриває -->
<div
  class="backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onclose();
  }}
>
  <div class="modal" role="dialog" aria-modal="true" aria-label={$t("picker.title")}>
    <header class="modal-head">
      <h3>{$t("picker.title")}</h3>
      <button class="close" aria-label={$t("picker.close")} onclick={onclose}>✕</button>
    </header>

    <div class="controls">
      <!-- svelte-ignore a11y_autofocus -->
      <input
        type="text"
        autofocus
        bind:value={filter}
        placeholder={$t("picker.filter.placeholder")}
      />
      <button type="button" onclick={refresh} disabled={loading}>
        {$t("picker.refresh")}
      </button>
    </div>

    <label class="windows-only">
      <input type="checkbox" bind:checked={windowsOnly} />
      {$t("picker.windowsOnly")}
    </label>

    <div class="body">
      {#if loading}
        <p class="muted">{$t("picker.loading")}</p>
      {:else if errorDetail}
        <p class="err" title={errorDetail}>{$t("picker.error")}: {errorDetail}</p>
      {:else if filtered.length === 0}
        <p class="muted">{$t("picker.none")}</p>
        {#if hiddenByWindowFilter > 0}
          <p class="hint-uncheck">
            {$t("picker.hint.uncheck")}
            <button type="button" class="link" onclick={() => (windowsOnly = false)}>
              {$t("picker.hint.showAll")}
            </button>
          </p>
        {/if}
      {:else}
        <ul>
          {#each filtered as p (p.name)}
            {@const isAdded = addedLower.has(p.name.toLowerCase())}
            <li>
              <button
                class="row"
                disabled={isAdded}
                title={p.exe_path ?? p.name}
                onclick={() => onpick(p.name)}
              >
                {#if p.icon}
                  <img class="picon" src={p.icon} alt="" width="18" height="18" />
                {:else}
                  <span class="picon picon-ph" aria-hidden="true"></span>
                {/if}
                <span class="pname">{p.name}</span>
                {#if p.exe_path}<span class="ppath">{p.exe_path}</span>{/if}
                <span class="state">{isAdded ? $t("picker.added") : "+"}</span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <footer class="modal-foot">
      <span class="count">{filtered.length} / {processes.length}</span>
      <button class="primary" onclick={onclose}>{$t("picker.done")}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
    padding: 1.5rem;
  }

  .modal {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    width: 100%;
    max-width: 560px;
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.9rem 1rem;
    border-bottom: 1px solid var(--border);
  }

  .modal-head h3 {
    margin: 0;
    font-size: 1rem;
  }

  .close {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 1rem;
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
  }
  .close:hover {
    color: #fff;
    background: #d9534f;
  }

  .controls {
    display: flex;
    gap: 0.5rem;
    padding: 0.8rem 1rem;
  }

  .controls input {
    flex: 1;
    background: var(--bg-elev);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.45rem 0.6rem;
    font: inherit;
  }

  .controls button,
  .modal-foot button {
    font: inherit;
    cursor: pointer;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-elev);
    color: var(--text);
    padding: 0.45rem 0.8rem;
  }
  .controls button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .windows-only {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    padding: 0 1rem 0.6rem;
    color: var(--text-dim);
    font-size: 0.85rem;
    cursor: pointer;
    user-select: none;
  }
  .windows-only input {
    cursor: pointer;
  }

  .body {
    overflow-y: auto;
    padding: 0 1rem;
    flex: 1;
  }

  .hint-uncheck {
    color: var(--text-dim);
    font-size: 0.82rem;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
  }

  .muted {
    color: var(--text-dim);
    font-style: italic;
    font-size: 0.85rem;
  }
  .err {
    color: #d9534f;
    font-size: 0.85rem;
  }

  ul {
    list-style: none;
    margin: 0.4rem 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
  }

  .row {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    text-align: left;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.4rem 0.6rem;
    color: var(--text);
    cursor: pointer;
    font: inherit;
  }
  .row:not(:disabled):hover {
    border-color: var(--accent);
  }
  .row:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .picon {
    flex: none;
    width: 18px;
    height: 18px;
    object-fit: contain;
    border-radius: 3px;
    align-self: center;
  }
  /* Заглушка-плейсхолдер: тримає вирівнювання, коли іконки немає. */
  .picon-ph {
    border: 1px solid var(--border);
    background: var(--bg-card);
    opacity: 0.6;
  }

  .pname {
    flex: none;
    font-weight: 600;
    font-size: 0.9rem;
  }
  .ppath {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
    font-size: 0.78rem;
  }
  .state {
    flex: none;
    color: var(--accent);
    font-size: 0.85rem;
  }

  .modal-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.8rem 1rem;
    border-top: 1px solid var(--border);
  }
  .count {
    color: var(--text-dim);
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
</style>
