<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { api } from "../api";
  import { params, activeId } from "../store";
  import GlassPanel from "../glass/GlassPanel.svelte";

  let exporting = false;
  let msg = "";

  async function exportTiff() {
    if (!$activeId) return;
    const out = await save({ defaultPath: "redroom-export.tiff", filters: [{ name: "TIFF", extensions: ["tiff"] }] });
    if (!out) return;
    exporting = true; msg = "";
    try { await api.exportImage($activeId, $params, out); msg = "Exported ✓"; }
    catch (e) { msg = "Error: " + e; }
    exporting = false;
  }
</script>

<GlassPanel>
  <div class="grp">
    <label>Mode</label>
    <div class="seg">
      <button class:on={$params.mode === "b"} on:click={() => params.update((p) => ({ ...p, mode: "b" }))}>B · density</button>
      <button class:on={$params.mode === "c"} on:click={() => params.update((p) => ({ ...p, mode: "c" }))}>C · per-channel</button>
    </div>
  </div>

  <div class="grp">
    <label>Film stock</label>
    <select bind:value={$params.stock}>
      <option value="none">None (identity)</option>
      <option value="portra400">Kodak Portra 400</option>
      <option value="fujic200">Fuji C200</option>
    </select>
  </div>

  <div class="grp">
    <label>Exposure <span>{$params.exposure.toFixed(2)}</span></label>
    <input type="range" min="0.2" max="3" step="0.01" bind:value={$params.exposure} />
  </div>
  <div class="grp">
    <label>Black <span>{$params.black.toFixed(3)}</span></label>
    <input type="range" min="0" max="0.3" step="0.001" bind:value={$params.black} />
  </div>
  <div class="grp">
    <label>Gamma <span>{$params.gamma.toFixed(3)}</span></label>
    <input type="range" min="0.2" max="1" step="0.001" bind:value={$params.gamma} />
  </div>

  <button class="export" on:click={exportTiff} disabled={exporting || !$activeId}>
    {exporting ? "Exporting…" : "Export 16-bit TIFF"}
  </button>
  {#if msg}<div class="msg">{msg}</div>{/if}
</GlassPanel>

<style>
  .grp { margin-bottom: 14px; }
  label { display: flex; justify-content: space-between; color: var(--text-dim); margin-bottom: 6px; }
  label span { color: var(--text); }
  .seg { display: flex; gap: 6px; }
  .seg button { flex: 1; padding: 7px; border-radius: 8px; border: 1px solid var(--glass-brd);
    background: transparent; color: var(--text-dim); }
  .seg button.on { color: white; background: rgba(224,52,52,0.18);
    border-color: rgba(224,52,52,0.5); }
  select { width: 100%; padding: 7px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .export { width: 100%; margin-top: 8px; padding: 10px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 600; }
  .export:disabled { opacity: 0.5; }
  .msg { margin-top: 8px; color: var(--text-dim); }
</style>
