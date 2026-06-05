<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import { locale, LOCALES, t } from "../i18n";
  const dispatch = createEventDispatcher();
</script>

<div class="backdrop" on:click={() => dispatch("close")}></div>
<div class="menu" role="dialog" aria-label={$t("settings.dialogAriaLabel")} transition:fade={{ duration: 120 }}>
  <div class="grp">
    <div class="head">{$t("settings.language.heading")}</div>
    <div class="seg">
      {#each LOCALES as l}
        <button class:on={$locale === l.id} on:click={() => locale.set(l.id)}>{l.label}</button>
      {/each}
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 60; }
  .menu { position: fixed; top: 52px; right: 16px; z-index: 61; min-width: 224px;
    background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 12px;
    padding: 12px; backdrop-filter: blur(20px); box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  .head { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin-bottom: 8px; }
  .seg { display: flex; gap: 6px; }
  .seg button { flex: 1; padding: 7px; border-radius: 8px; font-size: 12px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text-dim); }
  .seg button.on { color: #fff; background: rgba(244,157,78,0.18); border-color: rgba(244,157,78,0.5); }
</style>
