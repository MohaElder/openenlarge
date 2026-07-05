<script lang="ts">
  import "../styles/theme.css";
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { hydrate, initPersistence } from "$lib/catalog";
  import { module, hasImages, images, undevelopedCount, deleteTarget, activeId, selection, copySettingsOpen, telemetryDecided, hotkeyBindings, debugMode } from "$lib/store";
  import { confirmCopyDevelopSettings, PASTE_DEFAULT_GROUPS } from "$lib/develop/copySettings";
  import { noneSelected } from "$lib/selection";
  import { get } from "svelte/store";
  import { matchUndoRedo } from "$lib/develop/history";
  import { matchCombo } from "$lib/keymap/hotkeys";
  import { inTextField } from "$lib/keymap/focus";
  import { commitActive, undoActive, redoActive, seedActive } from "$lib/develop/historyStore";
  import { developAll, deleteImage } from "$lib/workflow";
  import Library from "$lib/tabs/Library.svelte";
  import Develop from "$lib/tabs/Develop.svelte";
  import Roll from "$lib/tabs/Roll.svelte";
  import ProgressOverlay from "$lib/overlay/ProgressOverlay.svelte";
  import ConfirmDevelop from "$lib/overlay/ConfirmDevelop.svelte";
  import ConfirmDelete from "$lib/overlay/ConfirmDelete.svelte";
  import ConfirmApplySettings from "$lib/overlay/ConfirmApplySettings.svelte";
  import Toast from "$lib/overlay/Toast.svelte";
  import SettingsMenu from "$lib/settings/SettingsMenu.svelte";
  import KeymapModal from "$lib/keymap/KeymapModal.svelte";
  import AboutModal from "$lib/about/AboutModal.svelte";
  import UpdatePrompt from "$lib/update/UpdatePrompt.svelte";
  import { runAutoCheck } from "$lib/update/updater";
  import { initRollBaseMigration } from "$lib/roll/rollBase";
  import Icon from "$lib/icons/Icon.svelte";
  import { hasDeveloped } from "$lib/export/eligible";
  import ExportModal from "$lib/export/ExportModal.svelte";
  import TelemetryPrompt from "$lib/settings/TelemetryPrompt.svelte";
  import { track } from "$lib/telemetry";
  import { t } from "$lib/i18n";
  import { isMac } from "$lib/keymap/hotkeys";
  import { api } from "$lib/api";
  import { initTourAutoStart } from "$lib/onboarding/tour";
  import { tourTarget } from "$lib/onboarding/tourTarget";
  import { save } from "@tauri-apps/plugin-dialog";
  import { flushDebugQueue } from "$lib/debug";
  import { showToast } from "$lib/toast";

  // The macOS window uses a transparent native title bar (see tauri.conf.json)
  // so its strip paints the app background colour. Offset our content below it
  // so the toolbar sits under the traffic lights. Other platforms keep a normal
  // title bar and need no offset.
  const onMac = isMac();

  // Gate the first-run analytics prompt on hydration: only after the persisted
  // choice (if any) has loaded do we know whether it's actually undecided.
  let hydrated = false;

  onMount(() => {
    let flush: (() => void) | undefined;
    let stopTour: (() => void) | undefined;
    hydrate().finally(() => {
      flush = initPersistence();
      runAutoCheck();
      hydrated = true;
      // First-run tour (issue #21 upstream): armed only AFTER hydration so the
      // persisted onboarding_done pref (and a restored module) is respected.
      stopTour = initTourAutoStart();
      track("app_launched"); // no-op unless the user has opted in
    });
    initRollBaseMigration();
    // Start an undo/redo timeline for each image the moment it becomes active.
    const unseed = activeId.subscribe(() => seedActive());
    return () => { flush?.(); unseed(); stopTour?.(); };
  });

  let confirmCount = 0;
  let confirming = false;
  let developTarget: "develop" | "roll" = "develop";
  let settingsOpen = false;
  let keymapOpen = false;
  let exporting = false;
  let aboutOpen = false;

  function gotoDevelop() {
    if (!$hasImages) return;
    developTarget = "develop";
    // Develop is scoped to the selected folder: only its undeveloped images count.
    if ($undevelopedCount === 0) { module.set("develop"); return; }
    confirmCount = $undevelopedCount;
    confirming = true;
  }

  function gotoRoll() {
    if (!$hasImages) return;
    if ($undevelopedCount === 0) { module.set("roll"); return; }
    confirmCount = $undevelopedCount;
    developTarget = "roll";
    confirming = true;
  }

  $: deleteCount = $deleteTarget.length;
  $: deleteName = deleteCount === 1
    ? ($images.find((i) => i.id === $deleteTarget[0])?.file_name ?? "")
    : "";
  async function runDelete(deleteFile: boolean) {
    const ids = $deleteTarget;
    deleteTarget.set([]);
    selection.set(noneSelected());
    for (const id of ids) await deleteImage(id, deleteFile);
  }

  // ⌘Z inside a text field should do the browser's native text undo, not image
  // undo. Range sliders are <input> too but have no text to undo, so they keep
  // affecting the image (inTextField() excludes them — see lib/keymap/focus.ts).
  function onKey(e: KeyboardEvent) {
    // The user's (rebindable) undo/redo bindings win; matchUndoRedo is the fixed
    // fallback so the Ctrl+Y redo alias keeps working.
    const id = matchCombo(e, get(hotkeyBindings), false);
    const action =
      id === "edit.undo" ? "undo" : id === "edit.redo" ? "redo" : matchUndoRedo(e);
    if (!action) return;
    if (inTextField()) return; // let native text undo win
    e.preventDefault();
    if (action === "undo") undoActive(); else redoActive();
  }

  async function exportDebugLog() {
    try {
      await flushDebugQueue();
      const path = await save({
        defaultPath: `openenlarge-debug-${Date.now()}.txt`,
        filters: [{ name: "Text", extensions: ["txt"] }],
      });
      if (!path) return; // user cancelled
      await api.saveLog(path);
      showToast($t("app.debug.exported"));
    } catch {
      showToast($t("app.debug.exportFailed"));
    }
  }
</script>

<!-- Delegated commit trigger: snapshot the active image once a gesture ends.
     pointerup = drags (sliders/curve/eraser); click = button-driven mutations
     (resets/flips/IR toggle) which mutate AFTER pointerup; change = sliders via
     keyboard + text-field blur. commitActive() deep-equal-guards, so the broad
     net produces at most one step per real change. (DOM change only — Svelte
     component "change" events don't bubble to window.) -->
<svelte:window
  on:keydown={onKey}
  on:pointerup={() => commitActive()}
  on:pointercancel={() => commitActive()}
  on:click={() => commitActive()}
  on:change={() => commitActive()}
/>

<div class="app" class:mac={onMac}>
  <header class="topbar">
    <button class="brand" on:click={() => (aboutOpen = true)} aria-label={$t('app.about.ariaLabel')}>
      <img class="logo" src="/favicon.png" alt="" />
    </button>
    <nav class="tabs">
      <button class:active={$module === "library"} on:click={() => module.set("library")}>{$t('app.tab.library')}</button>
      <button class:active={$module === "roll"} disabled={!$hasImages} on:click={gotoRoll}>
        {$t('app.tab.develop')}
        {#if $undevelopedCount > 0}<span class="badge">{$undevelopedCount}</span>{/if}
      </button>
      <!-- Tour step 3 (issue #21 upstream): points the user at per-frame finishing. -->
      <button class:active={$module === "develop"} disabled={!$hasImages} on:click={gotoDevelop}
              use:tourTarget={"tune-tab"}>
        {$t('app.tab.tune')}
        {#if $undevelopedCount > 0}<span class="badge">{$undevelopedCount}</span>{/if}
      </button>
      <button disabled={!$hasDeveloped} on:click={() => (exporting = true)}>{$t('app.tab.export')}</button>
    </nav>
    <div class="spacer"></div>
    {#if $debugMode}
      <button class="gear" on:click={exportDebugLog} aria-label={$t('app.debug.exportAriaLabel')}>
        <Icon name="file-text" size={18} />
      </button>
    {/if}
    <button class="gear" class:on={settingsOpen} on:click={() => (settingsOpen = !settingsOpen)} aria-label={$t('app.settings.ariaLabel')}>
      <Icon name="settings" size={18} />
    </button>
  </header>
  <main>
    {#key $module}
      <div class="page" in:fly={{ y: 10, duration: 220, easing: cubicOut }}>
        {#if $module === "library"}<Library />{:else if $module === "roll"}<Roll />{:else}<Develop />{/if}
      </div>
    {/key}
  </main>
</div>

{#if settingsOpen}<SettingsMenu on:close={() => (settingsOpen = false)} on:shortcuts={() => { settingsOpen = false; keymapOpen = true; }} />{/if}
{#if keymapOpen}<KeymapModal on:close={() => (keymapOpen = false)} />{/if}
{#if aboutOpen}<AboutModal on:close={() => (aboutOpen = false)} />{/if}
{#if hydrated && !$telemetryDecided}<TelemetryPrompt />{/if}
<UpdatePrompt />
<ProgressOverlay />
{#if exporting}
  <ExportModal on:close={() => (exporting = false)} />
{/if}
{#if confirming}
  <ConfirmDevelop count={confirmCount}
    on:confirm={(e) => { confirming = false; developAll(developTarget, e.detail?.filmType ?? "auto"); }}
    on:cancel={() => (confirming = false)} />
{/if}
{#if deleteCount > 0}
  <ConfirmDelete name={deleteName} count={deleteCount}
    on:remove={() => runDelete(false)}
    on:trash={() => runDelete(true)}
    on:cancel={() => deleteTarget.set([])} />
{/if}
{#if $copySettingsOpen}
  <ConfirmApplySettings
    title={$t('confirmApply.copyTitle')}
    sub={$t('confirmApply.copySub')}
    confirmLabel={$t('confirmApply.copyConfirm')}
    defaults={PASTE_DEFAULT_GROUPS}
    on:confirm={(e) => confirmCopyDevelopSettings(e.detail.groups)}
    on:cancel={() => copySettingsOpen.set(false)} />
{/if}
<Toast />

<style>
  .app { display: flex; flex-direction: column; height: 100vh; }
  /* Clear the transparent macOS title bar so the toolbar sits below the traffic
     lights. The topbar's own 10px top padding adds to this, so the logo starts
     ~22px down — clearing the buttons, which bottom out at ~20px.
     box-sizing:border-box (set globally) keeps total height at 100vh, and the
     inset strip shows the body background to match the bar. */
  .app.mac { padding-top: 12px; }
  .topbar { display: flex; align-items: center; gap: 18px; padding: 10px 16px;
    border-bottom: 1px solid var(--glass-brd); }
  .brand { font-weight: 600; letter-spacing: 0.3px; display: flex; align-items: center; gap: 8px;
    background: transparent; border: 0; padding: 4px 8px; margin: -4px -8px; border-radius: 8px;
    color: var(--text); font-size: inherit; cursor: pointer; transition: background 0.12s; }
  .brand:hover { background: var(--glass-hi); }
  .logo { width: 33px; height: 33px; border-radius: 8px; display: block; flex: none; }
  .tabs button { background: transparent; border: 0; padding: 6px 14px; border-radius: 8px; color: var(--text-dim); position: relative;
    transition: color 0.12s, background 0.12s; }
  .tabs button:not(.active):not(:disabled):hover { color: var(--text); background: var(--glass-hi); }
  .tabs button.active { color: var(--text); background: rgba(244,157,78,0.14); box-shadow: inset 0 0 0 1px rgba(244,157,78,0.4); }
  .tabs button.active:hover { background: rgba(244,157,78,0.20); }
  .tabs button:disabled { opacity: 0.35; cursor: not-allowed; }
  .badge { position: absolute; top: -7px; right: -8px; min-width: 18px; height: 18px; padding: 0 5px;
    border-radius: 9px; background: rgba(244,157,78,0.22); border: 1px solid rgba(244,157,78,0.45);
    color: var(--text); font-size: 11px; font-weight: 600;
    display: grid; place-items: center; }
  .spacer { flex: 1; }
  .gear { display: grid; place-items: center; width: 32px; height: 32px; padding: 0;
    background: transparent; border: 0; border-radius: 8px; color: var(--text-dim);
    transition: color 0.12s, background 0.12s; }
  .gear:hover { color: var(--text); background: var(--glass-hi); }
  .gear.on { color: var(--text); background: rgba(244,157,78,0.14); box-shadow: inset 0 0 0 1px rgba(244,157,78,0.4); }
  main { flex: 1; min-height: 0; padding: 12px; position: relative; }
  /* The keyed page wrapper fills main so Library/Develop keep their full height
     while the fly-in transition plays on module switch. */
  .page { height: 100%; }
</style>
