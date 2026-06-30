<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade } from "svelte/transition";
  import { locale, LOCALES, t } from "../i18n";
  import { openaiApiKey, telemetryEnabled, debugMode } from "../store";
  import { setTelemetryChoice } from "../telemetry";
  import { setDebugMode } from "../debug";
  import { runManualCheck } from "../update/updater";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { relaunch } from "@tauri-apps/plugin-process";
  import { api } from "../api";
  import { hydrate } from "../catalog";
  const dispatch = createEventDispatcher();
  const OPENAI_KEYS_URL = "https://platform.openai.com/api-keys";
  const PRIVACY_URL = "https://github.com/mohaelder/openenlarge#telemetry";
  // Aptabase has no public dashboard link; access is by manual invite, so point
  // interested users at a pre-filled access request.
  const DASHBOARD_REQUEST_URL =
    "mailto:calen0909@hotmail.com?subject=OpenEnlarge%20analytics%20dashboard%20access" +
    "&body=Hi%2C%20I%27d%20like%20access%20to%20the%20OpenEnlarge%20analytics%20dashboard.%0A%0A" +
    "Name%20%2F%20GitHub%3A%0AReason%3A%0AEmail%20to%20invite%20%28Aptabase%20account%29%3A%0A%0AThanks%21";

  let cacheBytes = 0;
  let busy = false;

  function humanBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const units = ["KB", "MB", "GB", "TB"];
    let v = n, i = -1;
    do { v /= 1024; i++; } while (v >= 1024 && i < units.length - 1);
    return `${v.toFixed(1)} ${units[i]}`;
  }

  onMount(async () => {
    try { cacheBytes = await api.cacheSize(); } catch { cacheBytes = 0; }
  });

  async function onClearCache() {
    if (busy) return;
    const ok = await confirm($t("settings.storage.clearCacheConfirm"),
      { title: "OpenEnlarge", kind: "warning" });
    if (!ok) return;
    busy = true;
    try {
      await api.clearImageCache();
      await hydrate();
      cacheBytes = await api.cacheSize();
    } finally { busy = false; }
  }

  async function onDebugToggle(on: boolean) {
    if (on) { await setDebugMode(true); return; }
    // Turning off: offer to also clear the log.
    const clear = await confirm($t("settings.debug.clearLogConfirm"),
      { title: "OpenEnlarge", kind: "warning" });
    await setDebugMode(false, clear);
  }

  async function onReset() {
    if (busy) return;
    const ok = await confirm($t("settings.storage.resetConfirm"),
      { title: "OpenEnlarge", kind: "warning" });
    if (!ok) return;
    busy = true;
    try {
      await api.resetAllData();
      await relaunch();
    } catch { busy = false; }
  }
</script>

<div class="backdrop" on:click={() => dispatch("close")} transition:fade={{ duration: 120 }}></div>
<div class="menu" role="dialog" aria-label={$t("settings.dialogAriaLabel")} transition:fade={{ duration: 120 }}>
  <div class="grp">
    <div class="head">{$t("settings.language.heading")}</div>
    <div class="seg">
      {#each LOCALES as l}
        <button class:on={$locale === l.id} on:click={() => locale.set(l.id)}>{l.label}</button>
      {/each}
    </div>
  </div>
  <div class="grp">
    <button type="button" class="head head-link"
            on:click={() => openUrl(OPENAI_KEYS_URL).catch(() => {})}>
      {$t("settings.ai.heading")} ↗
    </button>
    <input
      class="key" type="password" autocomplete="off" spellcheck="false"
      placeholder={$t("settings.ai.keyPlaceholder")}
      value={$openaiApiKey}
      on:input={(e) => openaiApiKey.set((e.target as HTMLInputElement).value)} />
    <div class="hint">{$t("settings.ai.hint")}</div>
  </div>
  <div class="grp">
    <button type="button" class="head head-link"
            on:click={() => openUrl(PRIVACY_URL).catch(() => {})}>
      {$t("settings.telemetry.heading")} ↗
    </button>
    <div class="seg">
      <button class:on={!$telemetryEnabled} on:click={() => setTelemetryChoice(false)}>{$t("settings.telemetry.off")}</button>
      <button class:on={$telemetryEnabled} on:click={() => setTelemetryChoice(true)}>{$t("settings.telemetry.on")}</button>
    </div>
    <div class="hint">{$t("settings.telemetry.hint")}</div>
    <button type="button" class="req-link"
            on:click={() => openUrl(DASHBOARD_REQUEST_URL).catch(() => {})}>
      {$t("settings.telemetry.requestAccess")} ↗
    </button>
  </div>
  <div class="grp">
    <div class="head">{$t("settings.storage.heading")}</div>
    <div class="row">
      <span class="lbl">{$t("settings.storage.cacheLabel")}</span>
      <span class="val">{humanBytes(cacheBytes)}</span>
    </div>
    <button class="store-btn" disabled={busy} on:click={onClearCache}>
      {$t("settings.storage.clearCache")}
    </button>
    <div class="hint">{$t("settings.storage.clearCacheHint")}</div>
    <button class="store-btn danger" disabled={busy} on:click={onReset}>
      {$t("settings.storage.reset")}
    </button>
    <div class="hint">{$t("settings.storage.resetHint")}</div>
  </div>
  <div class="grp">
    <div class="head">{$t("settings.debug.heading")}</div>
    <div class="seg">
      <button class:on={!$debugMode} on:click={() => onDebugToggle(false)}>{$t("settings.debug.off")}</button>
      <button class:on={$debugMode} on:click={() => onDebugToggle(true)}>{$t("settings.debug.on")}</button>
    </div>
    <div class="hint">{$t("settings.debug.hint")}</div>
  </div>
  <button class="shortcuts" on:click={() => dispatch("shortcuts")}>
    <span class="kbd-icon" aria-hidden="true">⌨</span>
    {$t("settings.shortcuts.button")}
  </button>
  <button class="shortcuts" on:click={() => { dispatch("close"); runManualCheck(); }}>
    <span class="kbd-icon" aria-hidden="true">↑</span>
    {$t("settings.checkUpdates")}
  </button>
</div>

<style>
  .backdrop { position: fixed; inset: 0; z-index: 60; background: rgba(0,0,0,0.5);
    backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px); }
  .menu { position: fixed; top: 52px; right: 16px; z-index: 61; min-width: 224px;
    background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 12px;
    padding: 12px; backdrop-filter: blur(20px) saturate(140%); -webkit-backdrop-filter: blur(20px) saturate(140%);
    box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  .head { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin-bottom: 8px; }
  /* Clickable heading → opens the OpenAI API-keys page. Dashed underline cues the link. */
  .head-link { display: inline-flex; align-items: center; gap: 4px; padding: 0;
    background: none; border: none; cursor: pointer; font: inherit;
    text-transform: uppercase; letter-spacing: 0.05em; font-size: 11px;
    text-decoration: underline; text-decoration-style: dashed; text-underline-offset: 3px; }
  .head-link:hover, .head-link:focus-visible { color: var(--accent); outline: none; }
  .seg { display: flex; gap: 6px; }
  .seg button { flex: 1; padding: 7px; border-radius: 8px; font-size: 12px; cursor: pointer;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text-dim);
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease; }
  .seg button:not(.on):hover { background: var(--glass-hi); color: var(--text); }
  .seg button.on { color: #fff; background: rgba(244,157,78,0.18); border-color: rgba(244,157,78,0.5); }
  .seg button.on:hover { background: rgba(244,157,78,0.30); border-color: rgba(244,157,78,0.75); }
  .shortcuts { display: flex; align-items: center; gap: 8px; width: 100%; margin-top: 12px;
    padding: 9px 10px; border-radius: 8px; font-size: 12px; text-align: left;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text); }
  .shortcuts:hover { background: var(--glass-hi); }
  .kbd-icon { font-size: 14px; color: var(--text-dim); }
  .grp + .grp { margin-top: 12px; }
  .key { width: 100%; box-sizing: border-box; padding: 8px 10px; border-radius: 8px;
    font-size: 12px; border: 1px solid var(--glass-brd); background: transparent;
    color: var(--text); }
  .key::placeholder { color: var(--text-dim); }
  .hint { font-size: 11px; color: var(--text-dim); margin-top: 6px; line-height: 1.4; }
  .req-link { display: inline-block; margin-top: 8px; padding: 0; background: none; border: 0;
    cursor: pointer; font: inherit; font-size: 11px; color: var(--text-dim);
    text-decoration: underline; text-decoration-style: dashed; text-underline-offset: 3px; }
  .req-link:hover, .req-link:focus-visible { color: var(--accent); outline: none; }
  .row { display: flex; justify-content: space-between; align-items: baseline;
    font-size: 12px; margin-bottom: 8px; }
  .row .lbl { color: var(--text-dim); }
  .row .val { color: var(--text); font-variant-numeric: tabular-nums; }
  .store-btn { width: 100%; margin-top: 6px; padding: 8px 10px; border-radius: 8px;
    font-size: 12px; cursor: pointer; text-align: left;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text);
    transition: background 0.12s ease, border-color 0.12s ease; }
  .store-btn:hover:not(:disabled) { background: var(--glass-hi); }
  .store-btn:disabled { opacity: 0.5; cursor: default; }
  .store-btn.danger { color: #f4a9a9; border-color: rgba(244,120,120,0.4); }
  .store-btn.danger:hover:not(:disabled) { background: rgba(244,120,120,0.12);
    border-color: rgba(244,120,120,0.7); }
</style>
