<script>
  import { t } from "../i18n.js";

  /**
   * Список однотипних правил виключення з кнопкою видалення.
   * @type {{
   *   title: string,
   *   kindLabel: string,
   *   items: string[],
   *   onremove: (index: number) => void,
   * }}
   */
  let { title, kindLabel, items, onremove } = $props();
</script>

<div class="rule-list">
  <h4>{title}</h4>
  {#if items.length === 0}
    <p class="muted">{$t("exclusions.empty")}</p>
  {:else}
    <ul>
      {#each items as item, i (item)}
        <li>
          <span class="badge">{kindLabel}</span>
          <code title={item}>{item}</code>
          <button
            class="rm"
            title={$t("exclusions.remove")}
            aria-label={$t("exclusions.remove")}
            onclick={() => onremove(i)}>✕</button
          >
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .rule-list h4 {
    margin: 0 0 0.4rem;
    font-size: 0.85rem;
    color: var(--text-dim);
    font-weight: 600;
  }

  .muted {
    margin: 0;
    color: var(--text-dim);
    font-size: 0.85rem;
    font-style: italic;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  li {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.3rem 0.4rem;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .badge {
    flex: none;
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 0.1rem 0.4rem;
    border-radius: 4px;
  }

  code {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    user-select: text;
    font-size: 0.85rem;
  }

  .rm {
    flex: none;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 0.9rem;
    padding: 0.1rem 0.3rem;
    border-radius: 4px;
  }

  .rm:hover {
    color: #fff;
    background: #d9534f;
  }
</style>
