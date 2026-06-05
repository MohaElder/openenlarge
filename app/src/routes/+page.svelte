<script lang="ts">
  import "../styles/theme.css";
  import { onMount } from "svelte";
  import { hydrate, initPersistence } from "$lib/catalog";
  import { module, hasImages, images, undevelopedCount, deleteTarget } from "$lib/store";
  import { developAll, deleteImage } from "$lib/workflow";
  import Library from "$lib/tabs/Library.svelte";
  import Develop from "$lib/tabs/Develop.svelte";
  import ProgressOverlay from "$lib/overlay/ProgressOverlay.svelte";
  import ConfirmDevelop from "$lib/overlay/ConfirmDevelop.svelte";
  import ConfirmDelete from "$lib/overlay/ConfirmDelete.svelte";
  import SettingsMenu from "$lib/settings/SettingsMenu.svelte";
  import AboutModal from "$lib/about/AboutModal.svelte";
  import Icon from "$lib/icons/Icon.svelte";
  import { hasDeveloped } from "$lib/export/eligible";
  import ExportModal from "$lib/export/ExportModal.svelte";
  import { t } from "$lib/i18n";

  onMount(() => {
    let flush: (() => void) | undefined;
    hydrate().finally(() => { flush = initPersistence(); });
    return () => flush?.();
  });

  let confirmCount = 0;
  let confirming = false;
  let settingsOpen = false;
  let exporting = false;
  let aboutOpen = false;

  function gotoDevelop() {
    if (!$hasImages) return;
    // Develop is scoped to the selected folder: only its undeveloped images count.
    if ($undevelopedCount === 0) { module.set("develop"); return; }
    confirmCount = $undevelopedCount;
    confirming = true;
  }

  $: deleteName = $deleteTarget
    ? ($images.find((i) => i.id === $deleteTarget)?.file_name ?? "")
    : "";
  function runDelete(deleteFile: boolean) {
    const id = $deleteTarget;
    deleteTarget.set(null);
    if (id) deleteImage(id, deleteFile);
  }
</script>

<div class="app">
  <header class="topbar">
    <button class="brand" on:click={() => (aboutOpen = true)} aria-label={$t('app.about.ariaLabel')}>
      <img class="logo" src="/favicon.png" alt="" /> {$t('app.brand')}
    </button>
    <nav class="tabs">
      <button class:active={$module === "library"} on:click={() => module.set("library")}>{$t('app.tab.library')}</button>
      <button class:active={$module === "develop"} disabled={!$hasImages} on:click={gotoDevelop}>
        {$t('app.tab.develop')}
        {#if $undevelopedCount > 0}<span class="badge">{$undevelopedCount}</span>{/if}
      </button>
      <button disabled={!$hasDeveloped} on:click={() => (exporting = true)}>{$t('app.tab.export')}</button>
    </nav>
    <div class="spacer"></div>
    <button class="gear" class:on={settingsOpen} on:click={() => (settingsOpen = !settingsOpen)} aria-label={$t('app.settings.ariaLabel')}>
      <Icon name="settings" size={18} />
    </button>
  </header>
  <main>
    {#if $module === "library"}<Library />{:else}<Develop />{/if}
  </main>
</div>

{#if settingsOpen}<SettingsMenu on:close={() => (settingsOpen = false)} />{/if}
{#if aboutOpen}<AboutModal on:close={() => (aboutOpen = false)} />{/if}
<ProgressOverlay />
{#if exporting}
  <ExportModal on:close={() => (exporting = false)} />
{/if}
{#if confirming}
  <ConfirmDevelop count={confirmCount}
    on:confirm={() => { confirming = false; developAll(); }}
    on:cancel={() => (confirming = false)} />
{/if}
{#if $deleteTarget}
  <ConfirmDelete name={deleteName}
    on:remove={() => runDelete(false)}
    on:trash={() => runDelete(true)}
    on:cancel={() => deleteTarget.set(null)} />
{/if}

<style>
  .app { display: flex; flex-direction: column; height: 100vh; }
  .topbar { display: flex; align-items: center; gap: 18px; padding: 10px 16px;
    border-bottom: 1px solid var(--glass-brd); }
  .brand { font-weight: 600; letter-spacing: 0.3px; display: flex; align-items: center; gap: 8px;
    background: transparent; border: 0; padding: 4px 8px; margin: -4px -8px; border-radius: 8px;
    color: var(--text); font-size: inherit; cursor: pointer; transition: background 0.12s; }
  .brand:hover { background: var(--glass-hi); }
  .logo { width: 33px; height: 33px; border-radius: 8px; display: block; flex: none; }
  .tabs button { background: transparent; border: 0; padding: 6px 14px; border-radius: 8px; color: var(--text-dim); position: relative; }
  .tabs button.active { color: var(--text); background: rgba(244,157,78,0.14); box-shadow: inset 0 0 0 1px rgba(244,157,78,0.4); }
  .tabs button:disabled { opacity: 0.35; cursor: not-allowed; }
  .badge { position: absolute; top: -7px; right: -8px; min-width: 18px; height: 18px; padding: 0 5px;
    border-radius: 9px; background: var(--accent); color: #fff; font-size: 11px; font-weight: 700;
    display: grid; place-items: center; box-shadow: 0 2px 8px rgba(244,157,78,0.6); }
  .spacer { flex: 1; }
  .gear { display: grid; place-items: center; width: 32px; height: 32px; padding: 0;
    background: transparent; border: 0; border-radius: 8px; color: var(--text-dim);
    transition: color 0.12s, background 0.12s; }
  .gear:hover { color: var(--text); background: var(--glass-hi); }
  .gear.on { color: var(--text); background: rgba(244,157,78,0.14); box-shadow: inset 0 0 0 1px rgba(244,157,78,0.4); }
  main { flex: 1; min-height: 0; padding: 12px; }
</style>
