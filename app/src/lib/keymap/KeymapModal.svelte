<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { hotkeyGroups, keyLabel } from "./hotkeys";
  import { t } from "$lib/i18n";

  const dispatch = createEventDispatcher<{ close: void }>();

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") dispatch("close");
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="scrim" on:click|self={() => dispatch("close")} transition:fade={{ duration: 150 }}>
  <div class="card" role="dialog" aria-label={$t('keymap.title')} transition:scale={{ duration: 160, start: 0.96, opacity: 0 }}>
    <div class="head">
      <div class="title">{$t('keymap.title')}</div>
    </div>
    <div class="groups">
      {#each hotkeyGroups as group}
        <div class="grp">{$t(group.heading)}</div>
        {#each group.items as item}
          <div class="row">
            <span class="label">{$t(item.label)}</span>
            <span class="keys">
              {#each item.keys as combo, ci}
                {#if ci > 0}<span class="sep">/</span>{/if}
                <span class="combo">
                  {#each combo as token}<kbd>{keyLabel(token)}</kbd>{/each}
                </span>
              {/each}
            </span>
          </div>
        {/each}
      {/each}
    </div>
    <div class="foot">
      <div class="spacer"></div>
      <button class="go" on:click={() => dispatch("close")}>{$t('keymap.close')}</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); backdrop-filter: blur(6px);
    display: grid; place-items: center; z-index: 80; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px; min-width: 380px; max-width: 440px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .head { margin-bottom: 8px; }
  .title { font-weight: 600; }
  .groups { max-height: 56vh; overflow-y: auto; margin-bottom: 16px; }
  .grp { font-size: 11px; text-transform: uppercase; letter-spacing: 0.5px; color: var(--text-dim);
    margin: 14px 2px 6px; }
  .grp:first-child { margin-top: 6px; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px;
    padding: 6px 2px; }
  .label { font-size: 13px; color: var(--text); }
  .keys { display: flex; align-items: center; gap: 6px; flex: none; }
  .sep { color: var(--text-dim); font-size: 12px; }
  .combo { display: flex; gap: 2px; }
  kbd { display: inline-block; min-width: 20px; text-align: center; padding: 2px 6px;
    font-family: inherit; font-size: 12px; line-height: 1.3; color: var(--text);
    background: var(--glass-hi); border: 1px solid var(--glass-brd); border-radius: 6px;
    box-shadow: 0 1px 0 rgba(0,0,0,0.3); }
  .foot { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .go { padding: 8px 14px; border-radius: 9px; border: 0; background: var(--accent-grad);
    color: white; font-weight: 600; }
</style>
