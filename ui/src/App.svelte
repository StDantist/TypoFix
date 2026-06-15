<script>
  import { t } from "./i18n.js";
  import Section from "./lib/Section.svelte";

  // Секції налаштувань. Поки що — статичні плейсхолдери (без логіки).
  const sections = [
    { titleKey: "section.languagePairs.title", descKey: "section.languagePairs.desc" },
    { titleKey: "section.threshold.title", descKey: "section.threshold.desc" },
    { titleKey: "section.exclusions.title", descKey: "section.exclusions.desc" },
    { titleKey: "section.rules.title", descKey: "section.rules.desc" },
    { titleKey: "section.hotkeys.title", descKey: "section.hotkeys.desc" },
  ];
</script>

<main>
  <header class="page-head">
    <h1>{$t("settings.title")}</h1>
    <p class="subtitle">{$t("settings.subtitle")}</p>
  </header>

  <div class="sections">
    {#each sections as s (s.titleKey)}
      <Section titleKey={s.titleKey} descKey={s.descKey} />
    {/each}
  </div>

  <footer class="page-foot">
    {$t("footer.note")}
  </footer>
</main>

<style>
  main {
    max-width: 720px;
    margin: 0 auto;
    padding: 1.5rem 1.25rem 2.5rem;
  }

  .page-head h1 {
    font-size: 1.4rem;
  }

  .subtitle {
    margin: 0.35rem 0 0;
    color: var(--text-dim);
  }

  .sections {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    margin-top: 1.5rem;
  }

  .page-foot {
    margin-top: 2rem;
    color: var(--text-dim);
    font-size: 0.8rem;
    text-align: center;
  }
</style>
