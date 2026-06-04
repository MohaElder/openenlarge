<script lang="ts">
  import "../styles/theme.css";
  import { module, hasImages, allDeveloped, images, undevelopedCount } from "$lib/store";
  import { developAll, undevelopedIds } from "$lib/workflow";
  import Library from "$lib/tabs/Library.svelte";
  import Develop from "$lib/tabs/Develop.svelte";
  import ProgressOverlay from "$lib/overlay/ProgressOverlay.svelte";
  import ConfirmDevelop from "$lib/overlay/ConfirmDevelop.svelte";

  let confirmCount = 0;
  let confirming = false;

  function gotoDevelop() {
    if (!$hasImages) return;
    if ($allDeveloped) { module.set("develop"); return; }
    confirmCount = undevelopedIds($images).length;
    confirming = true;
  }
</script>

<div class="app">
  <header class="topbar">
    <div class="brand"><span class="dot"></span> RedRoom</div>
    <nav class="tabs">
      <button class:active={$module === "library"} on:click={() => module.set("library")}>Library</button>
      <button class:active={$module === "develop"} disabled={!$hasImages} on:click={gotoDevelop}>
        Develop
        {#if $undevelopedCount > 0}<span class="badge">{$undevelopedCount}</span>{/if}
      </button>
    </nav>
    <div class="spacer"></div>
  </header>
  <main>
    {#if $module === "library"}<Library />{:else}<Develop />{/if}
  </main>
</div>

<ProgressOverlay />
{#if confirming}
  <ConfirmDevelop count={confirmCount}
    on:confirm={() => { confirming = false; developAll(); }}
    on:cancel={() => (confirming = false)} />
{/if}

<style>
  .app { display: flex; flex-direction: column; height: 100vh; }
  .topbar { display: flex; align-items: center; gap: 18px; padding: 10px 16px;
    border-bottom: 1px solid var(--glass-brd); }
  .brand { font-weight: 600; letter-spacing: 0.3px; display: flex; align-items: center; gap: 8px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--accent); box-shadow: 0 0 12px var(--accent); }
  .tabs button { background: transparent; border: 0; padding: 6px 14px; border-radius: 8px; color: var(--text-dim); position: relative; }
  .tabs button.active { color: var(--text); background: rgba(224,52,52,0.14); box-shadow: inset 0 0 0 1px rgba(224,52,52,0.4); }
  .tabs button:disabled { opacity: 0.35; cursor: not-allowed; }
  .badge { position: absolute; top: -7px; right: -8px; min-width: 18px; height: 18px; padding: 0 5px;
    border-radius: 9px; background: var(--accent); color: #fff; font-size: 11px; font-weight: 700;
    display: grid; place-items: center; box-shadow: 0 2px 8px rgba(224,52,52,0.6); }
  .spacer { flex: 1; }
  main { flex: 1; min-height: 0; padding: 12px; }
</style>
