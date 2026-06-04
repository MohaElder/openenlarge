<script lang="ts">
  import "../styles/theme.css";
  import { module } from "$lib/store";
  import Library from "$lib/tabs/Library.svelte";
  import Develop from "$lib/tabs/Develop.svelte";
</script>

<div class="app">
  <header class="topbar">
    <div class="brand"><span class="dot"></span> RedRoom</div>
    <nav class="tabs">
      <button class:active={$module === "library"} on:click={() => module.set("library")}>Library</button>
      <button class:active={$module === "develop"} on:click={() => module.set("develop")}>Develop</button>
    </nav>
    <div class="spacer"></div>
  </header>
  <main>
    {#if $module === "library"}<Library />{:else}<Develop />{/if}
  </main>
</div>

<style>
  .app { display: flex; flex-direction: column; height: 100vh; }
  .topbar {
    display: flex; align-items: center; gap: 18px;
    padding: 10px 16px; border-bottom: 1px solid var(--glass-brd);
  }
  .brand { font-weight: 600; letter-spacing: 0.3px; display: flex; align-items: center; gap: 8px; }
  .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--accent);
         box-shadow: 0 0 12px var(--accent); }
  .tabs button {
    background: transparent; border: 0; padding: 6px 14px; border-radius: 8px;
    color: var(--text-dim);
  }
  .tabs button.active { color: var(--text); background: rgba(224,52,52,0.14);
    box-shadow: inset 0 0 0 1px rgba(224,52,52,0.4); }
  .spacer { flex: 1; }
  main { flex: 1; min-height: 0; padding: 12px; }
</style>
