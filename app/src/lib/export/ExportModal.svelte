<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { join } from "@tauri-apps/api/path";
  import { developedImages } from "./eligible";
  import { editsById, cropById, dustById } from "../store";
  import { defaultParams, type ExportFormat } from "../api";
  import { api } from "../api";
  import { emptyDust } from "../develop/dust";
  import { allSelected, click, isAllSelected, toggleAll, type SelState } from "./selection";
  import { outName } from "./naming";

  const dispatch = createEventDispatcher<{ close: void }>();

  $: imgs = $developedImages;
  $: ids = imgs.map((i) => i.id);

  let sel: SelState = allSelected(ids);
  let initialized = false;
  // Initialize selection once images are known (all selected by default).
  $: if (!initialized && ids.length > 0) { sel = allSelected(ids); initialized = true; }

  function onItemClick(e: MouseEvent, id: string) {
    sel = click(sel, ids, id, { meta: e.metaKey || e.ctrlKey, shift: e.shiftKey });
  }
  $: allOn = isAllSelected(sel, ids);

  // ---- Format panel state ----
  let kind: ExportFormat["kind"] = "jpeg";
  let bitDepth: 8 | 16 = 16;
  let quality = 90;
  let maxMb = 0; // 0 = unlimited

  $: format = {
    kind,
    bitDepth: kind === "jpeg" ? undefined : bitDepth,
    quality: kind === "jpeg" ? quality : undefined,
    maxBytes: kind === "jpeg" && maxMb > 0 ? Math.round(maxMb * 1024 * 1024) : null,
  } as ExportFormat;

  // ---- Export run state ----
  let running = false;
  let done = 0;
  let total = 0;
  let summary = "";

  async function runExport() {
    const chosen = imgs.filter((i) => sel.selected.has(i.id));
    if (chosen.length === 0) return;
    const folder = await open({ directory: true });
    if (!folder || typeof folder !== "string") return;

    running = true; done = 0; total = chosen.length; summary = "";
    const failures: string[] = [];
    for (const img of chosen) {
      try {
        const p = $editsById[img.id] ?? defaultParams();
        const crop = $cropById[img.id] ?? null;
        const imageCrop = crop
          ? ([crop.rect.x, crop.rect.y, crop.rect.w, crop.rect.h] as [number, number, number, number])
          : null;
        const geom = crop
          ? { rot90: crop.rot90, flip_h: crop.flipH, flip_v: crop.flipV, angle: crop.angle }
          : {};
        const d = $dustById[img.id] ?? emptyDust();
        const outPath = await join(folder, outName(img.file_name, kind));
        await api.exportImage(img.id, p, outPath, imageCrop, geom, d.strokes, d.irRemoval, format);
        done++;
      } catch (e) {
        failures.push(`${img.file_name}: ${e}`);
      }
    }
    running = false;
    summary = failures.length
      ? `Exported ${done}/${total}. Failed: ${failures.join("; ")}`
      : `Exported ${done}/${total} ✓`;
  }
</script>

<div class="backdrop" on:click|self={() => dispatch("close")}>
  <div class="modal">
    <header>
      <h2>Export</h2>
      <button class="x" on:click={() => dispatch("close")}>✕</button>
    </header>

    <div class="bar">
      <button class="link" on:click={() => (sel = toggleAll(sel, ids))}>
        {allOn ? "Deselect all" : "Select all"}
      </button>
      <span class="count">{sel.selected.size} / {ids.length} selected</span>
    </div>

    <div class="grid">
      {#each imgs as img (img.id)}
        <button
          class="cell"
          class:on={sel.selected.has(img.id)}
          on:click={(e) => onItemClick(e, img.id)}
        >
          <img src={img.thumbnail} alt={img.file_name} draggable="false" />
          <span class="name">{img.file_name}</span>
          {#if sel.selected.has(img.id)}<span class="check">✓</span>{/if}
        </button>
      {/each}
      {#if imgs.length === 0}<div class="empty">No developed images to export.</div>{/if}
    </div>

    <div class="format">
      <label>Format
        <select bind:value={kind}>
          <option value="jpeg">JPEG</option>
          <option value="tiff">TIFF</option>
          <option value="png">PNG</option>
        </select>
      </label>

      {#if kind === "jpeg"}
        <label>Quality {quality}
          <input type="range" min="1" max="100" bind:value={quality} />
        </label>
        <label>Max size {maxMb === 0 ? "Unlimited" : `${maxMb} MB`}
          <input type="range" min="0" max="20" step="0.5" bind:value={maxMb} />
        </label>
      {:else}
        <label>Bit depth
          <select bind:value={bitDepth}>
            <option value={8}>8-bit</option>
            <option value={16}>16-bit</option>
          </select>
        </label>
      {/if}
    </div>

    <footer>
      {#if running}<span class="msg">Exporting {done}/{total}…</span>
      {:else if summary}<span class="msg">{summary}</span>{/if}
      <button class="primary" on:click={runExport} disabled={running || sel.selected.size === 0}>
        {running ? "Exporting…" : `Export ${sel.selected.size}`}
      </button>
    </footer>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,0.55);
    display: grid; place-items: center; z-index: 50; }
  .modal { width: min(880px, 92vw); max-height: 88vh; display: flex; flex-direction: column;
    background: var(--glass-bg, #1b1b1e); border: 1px solid var(--glass-brd, #333);
    border-radius: 14px; padding: 16px; gap: 12px; }
  header { display: flex; align-items: center; justify-content: space-between; }
  header h2 { margin: 0; font-size: 16px; }
  .x { background: transparent; border: 0; color: var(--text-dim); cursor: pointer; font-size: 14px; }
  .bar { display: flex; align-items: center; gap: 14px; }
  .link { background: transparent; border: 0; color: var(--accent); cursor: pointer; padding: 0; }
  .count { color: var(--text-dim); font-size: 12px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 10px; overflow-y: auto; padding: 4px; min-height: 120px; }
  .cell { position: relative; border: 2px solid transparent; border-radius: 10px;
    background: #0000; padding: 4px; cursor: pointer; display: flex; flex-direction: column; gap: 4px; }
  .cell.on { border-color: var(--accent); background: rgba(224,52,52,0.12); }
  .cell img { width: 100%; aspect-ratio: 1; object-fit: contain; border-radius: 6px; background: #000; }
  .name { font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .check { position: absolute; top: 6px; right: 6px; background: var(--accent); color: #fff;
    width: 18px; height: 18px; border-radius: 50%; display: grid; place-items: center; font-size: 11px; }
  .empty { grid-column: 1 / -1; color: var(--text-dim); place-self: center; padding: 24px; }
  .format { display: flex; flex-wrap: wrap; gap: 16px; align-items: center;
    border-top: 1px solid var(--glass-brd, #333); padding-top: 12px; }
  .format label { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--text-dim); }
  footer { display: flex; align-items: center; justify-content: flex-end; gap: 12px; }
  .msg { color: var(--text-dim); font-size: 12px; margin-right: auto; }
  .primary { padding: 9px 16px; border: 0; border-radius: 10px; background: var(--accent);
    color: #fff; font-weight: 600; cursor: pointer; }
  .primary:disabled { opacity: 0.5; cursor: default; }
</style>
