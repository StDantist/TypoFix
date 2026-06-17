<script>
  /**
   * Перемикач (switch). Двостороння прив'язка через $bindable.
   * `testid` (опц.) — стабільний селектор для UI-e2e: лягає на клікабельний
   * `<label>`, а на прихований `<input>` — як `${testid}-input` (читання стану).
   * @type {{ checked: boolean, label?: string, testid?: string }}
   */
  let { checked = $bindable(false), label = "", testid = "" } = $props();
</script>

<label class="toggle" data-testid={testid || undefined}>
  <input
    type="checkbox"
    bind:checked
    role="switch"
    data-testid={testid ? `${testid}-input` : undefined}
  />
  <span class="track"><span class="thumb"></span></span>
  {#if label}<span class="label">{label}</span>{/if}
</label>

<style>
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    cursor: pointer;
    user-select: none;
  }

  input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .track {
    position: relative;
    width: 42px;
    height: 24px;
    border-radius: 999px;
    background: var(--border);
    transition: background 0.15s ease;
    flex: none;
  }

  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.15s ease;
  }

  input:checked + .track {
    background: var(--accent);
  }

  input:checked + .track .thumb {
    transform: translateX(18px);
  }

  input:focus-visible + .track {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .label {
    font-size: 0.95rem;
  }
</style>
