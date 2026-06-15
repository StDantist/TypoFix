<script>
  import { t } from "../i18n.js";

  /**
   * Секція-плейсхолдер сторінки налаштувань.
   * @type {{ titleKey: string, descKey: string }}
   */
  let { titleKey, descKey } = $props();
</script>

<section class="card">
  <header>
    <h2>{$t(titleKey)}</h2>
    <p class="desc">{$t(descKey)}</p>
  </header>
  <div class="body">
    <span class="empty">{$t("placeholder.empty")}</span>
  </div>
</section>

<style>
  .card {
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 1rem 1.25rem;
  }

  header h2 {
    font-size: 1rem;
  }

  .desc {
    margin: 0.35rem 0 0;
    color: var(--text-dim);
    font-size: 0.85rem;
  }

  .body {
    margin-top: 0.85rem;
    padding: 0.85rem;
    border: 1px dashed var(--border);
    border-radius: 8px;
  }

  .empty {
    color: var(--text-dim);
    font-size: 0.85rem;
    font-style: italic;
  }
</style>
