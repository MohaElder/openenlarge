<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { t } from "$lib/i18n";
  import { selectedFolder } from "../store";
  import { startTether, stopTether } from "./controller";
  import { tetherWatching, tetherDir, tetherAutoAdvance, tetherLast } from "./store";

  let error = "";
  let busy = false;

  async function toggle() {
    if (busy) return;
    busy = true;
    error = "";
    try {
      if ($tetherWatching) {
        await stopTether();
        return;
      }
      const dir = await open({ directory: true, defaultPath: $selectedFolder ?? undefined });
      if (!dir || Array.isArray(dir)) return;
      await startTether(dir);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="tether">
  <button class="toggle" class:on={$tetherWatching} on:click={toggle} disabled={busy}>
    {$tetherWatching ? $t("tether.stop") : $t("tether.start")}
  </button>

  {#if $tetherWatching}
    <div class="status">{$t("tether.watching", { dir: $tetherDir ?? "" })}</div>
    <label class="adv">
      <input type="checkbox" bind:checked={$tetherAutoAdvance} />
      {$t("tether.autoAdvance")}
    </label>
    <div class="hint">{$t("tether.hint")}</div>
    {#if $tetherLast}
      <div class="last" class:err={!$tetherLast.ok}>
        {$tetherLast.ok
          ? $t("tether.lastOk", { name: $tetherLast.name })
          : $t("tether.lastErr", { name: $tetherLast.name })}
      </div>
    {/if}
  {/if}
  {#if error}<div class="err-msg">{error}</div>{/if}
</div>

<style>
  .tether { display: flex; flex-direction: column; gap: 6px; margin-top: 10px; }
  .toggle { width: 100%; padding: 9px; border-radius: 10px; border: 0;
    background: rgba(255,255,255,0.08); color: var(--text); font-weight: 600; cursor: pointer; }
  .toggle.on { background: rgba(244,157,78,0.22); }
  .status { font-size: 12px; opacity: 0.8; }
  .adv { display: flex; align-items: center; gap: 6px; font-size: 12px; opacity: 0.9; }
  .hint { font-size: 11px; opacity: 0.6; }
  .last { font-size: 12px; opacity: 0.85; }
  .last.err { color: #ff8a8a; }
  .err-msg { font-size: 12px; color: #ff8a8a; }
</style>
