<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { t } from "$lib/i18n";
  import type { FilmTypeChoice } from "$lib/workflow";
  export let count = 0;
  // Roll-level film type (#26): the whole roll is one stock, so one explicit choice
  // here beats fixing per-frame misdetections one by one afterwards.
  let filmType: FilmTypeChoice = "auto";
  const dispatch = createEventDispatcher();
</script>

<div class="scrim" on:click|self={() => dispatch("cancel")} transition:fade={{ duration: 150 }}>
  <div class="card" transition:scale={{ duration: 160, start: 0.96, opacity: 0 }}>
    <div class="title">{$t('confirmDevelop.title', { count, plural: count === 1 ? '' : 's' })}</div>
    <div class="sub">{$t('confirmDevelop.sub')}</div>
    <div class="typerow" title={$t('confirmDevelop.filmTypeTitle')}>
      <span class="typelabel">{$t('confirmDevelop.filmType')}</span>
      <span class="seg">
        {#each ["auto", "negative", "positive"] as ft}
          <button class="opt" class:on={filmType === ft}
                  aria-pressed={filmType === ft}
                  on:click={() => (filmType = ft as FilmTypeChoice)}
                  >{$t(`confirmDevelop.filmType.${ft}`)}</button>
        {/each}
      </span>
    </div>
    <div class="row">
      <button class="ghost" on:click={() => dispatch("cancel")}>{$t('confirmDevelop.cancel')}</button>
      <button class="go" on:click={() => dispatch("confirm", { filmType })}>{$t('confirmDevelop.confirm')}</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); backdrop-filter: blur(6px);
    display: grid; place-items: center; z-index: 60; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px; min-width: 320px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .title { font-weight: 600; margin-bottom: 6px; }
  .sub { color: var(--text-dim); margin-bottom: 18px; font-size: 12px; }
  .typerow { display: flex; align-items: center; justify-content: space-between; gap: 10px;
    margin-bottom: 18px; }
  .typelabel { color: var(--text-dim); font-size: 12px; }
  .seg { display: inline-flex; gap: 4px; }
  .opt { padding: 4px 10px; border-radius: 7px; font-size: 11px; color: var(--text-dim);
    border: 1px solid var(--glass-brd); background: transparent; cursor: pointer; }
  .opt.on { color: var(--text); border-color: rgba(244,157,78,0.5); background: rgba(244,157,78,0.12); }
  .row { display: flex; gap: 10px; justify-content: flex-end; }
  button { padding: 8px 14px; border-radius: 9px; border: 1px solid var(--glass-brd); background: transparent; }
  .go { background: rgba(244,157,78,0.18); border: 1px solid rgba(244,157,78,0.5); color: #fff; font-weight: 600;
    transition: background 0.14s ease, border-color 0.14s ease; }
  .go:hover { background: rgba(244,157,78,0.30); border-color: rgba(244,157,78,0.75); }
</style>
