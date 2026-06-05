<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { open } from "@tauri-apps/plugin-dialog";
  import { join } from "@tauri-apps/api/path";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { developedImages } from "./eligible";
  import { editsById, cropById, dustById, metaById } from "../store";
  import { defaultParams, type ExportFormat } from "../api";
  import { api } from "../api";
  import { emptyDust } from "../develop/dust";
  import { allSelected, noneSelected, click, isAllSelected, toggleAll, type SelState } from "./selection";
  import { outName } from "./naming";
  import { t } from "$lib/i18n";

  const dispatch = createEventDispatcher<{ close: void }>();

  // Height+opacity transition for the option-panel swap (JPEG sliders ⇄ bit depth).
  function slideFade(node: HTMLElement, { duration = 240 } = {}) {
    const h = node.offsetHeight;
    const s = getComputedStyle(node);
    const pt = parseFloat(s.paddingTop), pb = parseFloat(s.paddingBottom);
    const mt = parseFloat(s.marginTop), mb = parseFloat(s.marginBottom);
    return {
      duration,
      easing: cubicOut,
      css: (t: number) =>
        `overflow:hidden;opacity:${t};height:${t * h}px;` +
        `padding-top:${t * pt}px;padding-bottom:${t * pb}px;` +
        `margin-top:${t * mt}px;margin-bottom:${t * mb}px;`,
    };
  }

  $: imgs = $developedImages;
  $: ids = imgs.map((i) => i.id);

  // Start empty: `ids` is a `$:`-derived value and is still undefined during this
  // initializer (reactive statements run after init). The guard below selects-all
  // once the developed-image list is known.
  let sel: SelState = noneSelected();
  let initialized = false;
  $: if (!initialized && ids.length > 0) { sel = allSelected(ids); initialized = true; }

  function onItemClick(e: MouseEvent, id: string) {
    if (running) return;
    sel = click(sel, ids, id, { meta: e.metaKey || e.ctrlKey, shift: e.shiftKey });
  }
  $: allOn = isAllSelected(sel, ids);

  // ---- Format panel state ----
  let kind: ExportFormat["kind"] = "jpeg";
  let bitDepth: 8 | 16 = 16;
  let quality = 90;
  let maxMb = 0; // 0 = unlimited

  $: kindIndex = kind === "jpeg" ? 0 : kind === "tiff" ? 1 : 2;

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
  let finished = false;
  let failedCount = 0;
  let lastFolder = "";
  let exportedPaths: string[] = [];

  // Reactive so the summary re-evaluates (and re-translates) on language change.
  $: summary = !finished
    ? ""
    : failedCount > 0
      ? $t("export.summaryPartial", { done, total, failed: failedCount })
      : $t("export.summaryDone", {
          done,
          imageOrImages: $t(done === 1 ? "export.imageSingular" : "export.imagePlural"),
        });

  async function runExport() {
    const chosen = imgs.filter((i) => sel.selected.has(i.id));
    if (chosen.length === 0) return;
    const folder = await open({ directory: true });
    if (!folder || typeof folder !== "string") return;

    running = true; done = 0; total = chosen.length; finished = false;
    failedCount = 0; exportedPaths = []; lastFolder = folder;
    const written: string[] = [];
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
        const metaOverride = $metaById[img.id] ?? null;
        const outPath = await join(folder, outName(img.file_name, kind));
        await api.exportImage(img.id, p, outPath, imageCrop, geom, d.strokes, d.irRemoval, format, metaOverride);
        written.push(outPath);
        done++;
      } catch (e) {
        failures.push(`${img.file_name}: ${e}`);
      }
    }
    running = false;
    exportedPaths = written;
    failedCount = failures.length;
    finished = true;
  }

  async function openFolder() {
    const target = exportedPaths[0] ?? lastFolder;
    if (!target) return;
    try { await revealItemInDir(target); } catch { /* ignore */ }
  }
</script>

<div class="backdrop" transition:fade={{ duration: 160 }} on:click|self={() => dispatch("close")}>
  <div class="modal" transition:scale={{ start: 0.96, opacity: 0, duration: 240, easing: cubicOut }}>
    <header>
      <div class="title">
        <span class="dot"></span>
        <h2>{$t('export.title')}</h2>
      </div>
      <button class="x" on:click={() => dispatch("close")} aria-label={$t('export.close')}>✕</button>
    </header>

    <div class="bar">
      <button class="link" on:click={() => (sel = toggleAll(sel, ids))} disabled={running}>
        {allOn ? $t('export.deselectAll') : $t('export.selectAll')}
      </button>
      <span class="count">{$t('export.selectionCount', { selected: sel.selected.size, total: ids.length })}</span>
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
      {#if imgs.length === 0}<div class="empty">{$t('export.emptyState')}</div>{/if}
    </div>

    <div class="format" class:busy={running}>
      <div class="field">
        <span class="flabel">{$t('export.formatLabel')}</span>
        <div class="seg" style="--n:3; --i:{kindIndex}">
          <button type="button" class:active={kind === "jpeg"} on:click={() => (kind = "jpeg")}>{$t('export.formatJpeg')}</button>
          <button type="button" class:active={kind === "tiff"} on:click={() => (kind = "tiff")}>{$t('export.formatTiff')}</button>
          <button type="button" class:active={kind === "png"} on:click={() => (kind = "png")}>{$t('export.formatPng')}</button>
          <span class="seg-ind"></span>
        </div>
      </div>

      {#if kind === "jpeg"}
        <div class="opts" transition:slideFade>
          <div class="field">
            <span class="flabel">{$t('export.quality')} <b>{quality}</b></span>
            <input class="range" type="range" min="1" max="100" bind:value={quality}
                   style="--pct:{((quality - 1) / 99) * 100}%" />
          </div>
          <div class="field">
            <span class="flabel">{$t('export.maxSize')} <b>{maxMb === 0 ? $t('export.unlimited') : $t('export.maxSizeMb', { mb: maxMb })}</b></span>
            <input class="range" type="range" min="0" max="20" step="0.5" bind:value={maxMb}
                   style="--pct:{(maxMb / 20) * 100}%" />
          </div>
        </div>
      {:else}
        <div class="opts" transition:slideFade>
          <div class="field">
            <span class="flabel">{$t('export.bitDepth')}</span>
            <div class="seg" style="--n:2; --i:{bitDepth === 8 ? 0 : 1}">
              <button type="button" class:active={bitDepth === 8} on:click={() => (bitDepth = 8)}>{$t('export.bitDepth8')}</button>
              <button type="button" class:active={bitDepth === 16} on:click={() => (bitDepth = 16)}>{$t('export.bitDepth16')}</button>
              <span class="seg-ind"></span>
            </div>
          </div>
        </div>
      {/if}
    </div>

    <footer>
      {#if running}
        <span class="msg"><span class="spinner"></span> {$t('export.exportingProgress', { done, total })}</span>
      {:else if summary}
        <span class="msg" class:ok={failedCount === 0} class:warn={failedCount > 0}>{summary}</span>
      {/if}
      <div class="actions">
        {#if !running && exportedPaths.length > 0}
          <button class="ghost" on:click={openFolder} in:fade={{ duration: 200 }}>{$t('export.openFolder')}</button>
        {/if}
        <button class="primary" on:click={runExport} disabled={running || sel.selected.size === 0}>
          {running ? $t('export.exporting') : $t('export.exportCount', { count: sel.selected.size })}
        </button>
      </div>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed; inset: 0; z-index: 50;
    display: grid; place-items: center;
    background: rgba(6, 6, 9, 0.5);
    backdrop-filter: blur(16px) saturate(125%);
    -webkit-backdrop-filter: blur(16px) saturate(125%);
  }
  .modal {
    width: min(880px, 92vw); max-height: 88vh;
    display: flex; flex-direction: column; gap: 14px;
    padding: 18px;
    background: linear-gradient(180deg, rgba(34, 34, 40, 0.94), rgba(19, 19, 23, 0.94));
    border: 1px solid var(--glass-brd);
    border-radius: var(--radius);
    box-shadow: 0 28px 80px rgba(0, 0, 0, 0.6), inset 0 1px 0 rgba(255, 255, 255, 0.05);
  }

  header { display: flex; align-items: center; justify-content: space-between; }
  .title { display: flex; align-items: center; gap: 9px; }
  .title h2 { margin: 0; font-size: 15px; font-weight: 600; letter-spacing: 0.2px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--accent);
    box-shadow: 0 0 10px var(--accent); }
  .x { background: transparent; border: 0; color: var(--text-faint); font-size: 13px;
    width: 26px; height: 26px; border-radius: 8px; transition: color 0.15s, background 0.15s; }
  .x:hover { color: var(--text); background: var(--glass-hi); }

  .bar { display: flex; align-items: center; gap: 14px; }
  .link { background: transparent; border: 0; color: var(--accent); padding: 0;
    font-size: 12px; font-weight: 600; transition: opacity 0.15s; }
  .link:hover { opacity: 0.8; }
  .link:disabled { opacity: 0.4; cursor: default; }
  .count { color: var(--text-dim); font-size: 12px; }

  .grid {
    display: grid; grid-template-columns: repeat(auto-fill, minmax(118px, 1fr)); gap: 10px;
    flex: 1 1 auto; min-height: 0; overflow-y: auto;
    padding: 4px 6px 4px 4px;
  }
  .grid::-webkit-scrollbar { width: 10px; }
  .grid::-webkit-scrollbar-thumb {
    background: var(--glass-brd); border-radius: 999px;
    border: 2px solid transparent; background-clip: padding-box;
  }
  .grid::-webkit-scrollbar-thumb:hover { background: rgba(255, 255, 255, 0.2); background-clip: padding-box; }

  .cell {
    position: relative; display: flex; flex-direction: column; gap: 5px;
    border: 2px solid transparent; border-radius: 10px; padding: 4px; background: transparent;
    transition: transform 0.16s ease, border-color 0.16s ease, background 0.16s ease;
  }
  .cell:hover { transform: translateY(-2px); border-color: var(--glass-brd); }
  .cell.on { border-color: var(--accent); background: rgba(224, 52, 52, 0.14); }
  .cell img { width: 100%; aspect-ratio: 1; object-fit: contain; border-radius: 6px; background: #000; }
  .name { font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cell.on .name { color: var(--text); }
  .check {
    position: absolute; top: 7px; right: 7px; width: 19px; height: 19px;
    display: grid; place-items: center; border-radius: 50%;
    background: var(--accent); color: #fff; font-size: 11px;
    box-shadow: 0 2px 8px rgba(224, 52, 52, 0.55);
    animation: pop 0.2s cubic-bezier(0.34, 1.56, 0.5, 1);
  }
  @keyframes pop { from { transform: scale(0); } to { transform: scale(1); } }
  .empty { grid-column: 1 / -1; color: var(--text-dim); place-self: center; padding: 28px; }

  .format {
    display: flex; flex-direction: column; gap: 14px;
    border-top: 1px solid var(--glass-brd); padding-top: 14px;
    transition: opacity 0.2s;
  }
  .format.busy { opacity: 0.5; pointer-events: none; }
  .opts { display: flex; flex-direction: column; gap: 14px; }
  .field { display: flex; flex-direction: column; gap: 7px; }
  .flabel { font-size: 11px; font-weight: 600; letter-spacing: 0.4px; text-transform: uppercase;
    color: var(--text-faint); }
  .flabel b { color: var(--text); font-weight: 600; letter-spacing: 0; text-transform: none;
    margin-left: 4px; }

  /* Segmented control with a sliding accent pill */
  .seg {
    position: relative; display: grid; grid-template-columns: repeat(var(--n), 1fr);
    padding: 3px; gap: 0; border-radius: 10px;
    background: var(--glass-hi); border: 1px solid var(--glass-brd);
    max-width: 360px;
  }
  .seg button {
    position: relative; z-index: 1; background: transparent; border: 0;
    padding: 8px 10px; border-radius: 7px;
    font-size: 12px; font-weight: 600; color: var(--text-dim);
    transition: color 0.2s ease;
  }
  .seg button.active { color: #fff; }
  .seg-ind {
    position: absolute; z-index: 0; top: 3px; bottom: 3px; left: 3px;
    width: calc((100% - 6px) / var(--n));
    border-radius: 7px; background: var(--accent);
    box-shadow: 0 2px 12px rgba(224, 52, 52, 0.45);
    transform: translateX(calc(var(--i) * 100%));
    transition: transform 0.3s cubic-bezier(0.34, 1.3, 0.5, 1);
  }

  /* Accent-filled range slider */
  .range {
    -webkit-appearance: none; appearance: none; width: 100%; height: 6px;
    border-radius: 999px; outline: none; cursor: pointer;
    background: linear-gradient(to right, var(--accent) var(--pct, 50%), var(--glass-hi) var(--pct, 50%));
  }
  .range::-webkit-slider-thumb {
    -webkit-appearance: none; appearance: none; width: 16px; height: 16px; border-radius: 50%;
    background: #fff; border: 2px solid var(--accent);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.45); transition: transform 0.12s;
  }
  .range::-webkit-slider-thumb:hover { transform: scale(1.12); }

  footer { display: flex; align-items: center; gap: 12px; }
  .msg { font-size: 12px; color: var(--text-dim); margin-right: auto; display: flex; align-items: center; gap: 8px; }
  .msg.ok { color: #6fd08c; }
  .msg.warn { color: #e0a23a; }
  .actions { display: flex; align-items: center; gap: 10px; margin-left: auto; }
  .spinner {
    width: 13px; height: 13px; border-radius: 50%;
    border: 2px solid var(--glass-brd); border-top-color: var(--accent);
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .ghost {
    padding: 9px 15px; border-radius: 10px; font-weight: 600; font-size: 13px;
    background: var(--glass-hi); border: 1px solid var(--glass-brd); color: var(--text);
    transition: background 0.15s, border-color 0.15s;
  }
  .ghost:hover { background: rgba(255, 255, 255, 0.08); border-color: rgba(255, 255, 255, 0.18); }
  .primary {
    padding: 9px 18px; border: 0; border-radius: 10px; font-weight: 600; font-size: 13px;
    background: var(--accent); color: #fff; cursor: pointer;
    box-shadow: 0 4px 16px rgba(224, 52, 52, 0.32); transition: filter 0.15s, box-shadow 0.15s;
  }
  .primary:hover:not(:disabled) { filter: brightness(1.08); }
  .primary:disabled { opacity: 0.45; cursor: default; box-shadow: none; }
</style>
