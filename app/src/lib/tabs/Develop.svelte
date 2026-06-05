<script lang="ts">
  import { save } from "@tauri-apps/plugin-dialog";
  import { activeId, params, images, tool, cropById, activeCrop, dustById, activeDust } from "../store";
  import { api } from "../api";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";
  import QualityMenu from "../viewport/QualityMenu.svelte";
  import Histogram from "../viewport/Histogram.svelte";
  import Toolbar from "../develop/Toolbar.svelte";
  import Basic from "../develop/Basic.svelte";
  import GlassPanel from "../glass/GlassPanel.svelte";
  import CropView from "../crop/CropView.svelte";
  import CropPanel from "../crop/CropPanel.svelte";
  import EraserPanel from "../develop/EraserPanel.svelte";
  import { addStroke, undoStroke, resetDust, emptyDust, type DustStroke, type DustEdits } from "../develop/dust";
  import type { Rect, CropRect } from "../crop/types";
  import { default80, conform, constrainToRotated } from "../crop/cropMath";
  import { presetNormAspect } from "../crop/presets";
  import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV, orientDims } from "../crop/transforms";

  $: active = $images.find((i) => i.id === $activeId);
  $: origW = active?.metadata.width ?? 0;
  $: origH = active?.metadata.height ?? 0;

  // ---- Crop draft state (only while tool === "crop") ----
  let rect: Rect = { x: 0.1, y: 0.1, w: 0.8, h: 0.8 };
  let aspect = "original";
  let orientation: "landscape" | "portrait" = "landscape";
  let rot90 = 0, flipH = false, flipV = false, angle = 0;
  let cropInit = false;

  $: [oW, oH] = orientDims(origW, origH, rot90);
  $: orientedRatio = oH > 0 ? oW / oH : 1;

  function startCrop() {
    const c = $activeCrop;
    if (c) {
      rect = { ...c.rect }; aspect = c.aspect; orientation = c.orientation;
      rot90 = c.rot90; flipH = c.flipH; flipV = c.flipV; angle = c.angle;
    } else {
      rect = default80(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait";
      rot90 = 0; flipH = false; flipV = false; angle = 0;
    }
    cropInit = true;
  }
  function draftCrop(): CropRect { return { rect, aspect, orientation, rot90: rot90 as 0 | 1 | 2 | 3, flipH, flipV, angle }; }
  function commitCrop() {
    const id = $activeId; if (!id || !cropInit) return;
    cropById.update((m) => ({ ...m, [id]: draftCrop() }));
  }
  function discardCrop() {
    const c = $activeCrop;
    if (c) { rect = { ...c.rect }; aspect = c.aspect; orientation = c.orientation; rot90 = c.rot90; flipH = c.flipH; flipV = c.flipV; angle = c.angle; }
    else { rect = default80(); aspect = "original"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  }
  function onPreset(id: string) { aspect = id; rect = conform(rect, presetNormAspect(id, orientedRatio, orientation)); }
  function onSwap() { orientation = orientation === "landscape" ? "portrait" : "landscape"; rect = conform(rect, presetNormAspect(aspect, orientedRatio, orientation)); }
  function onReset() { rect = default80(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  function onRotate(dir: number) {
    if (dir > 0) { rot90 = (rot90 + 1) % 4; rect = rotateRectCW(rect); }
    else { rot90 = (rot90 + 3) % 4; rect = rotateRectCCW(rect); }
  }
  function onFlip(axis: "h" | "v") {
    if (axis === "h") { flipH = !flipH; rect = flipRectH(rect); } else { flipV = !flipV; rect = flipRectV(rect); }
    angle = -angle;
  }
  function onStraighten(v: number) { angle = Math.max(-45, Math.min(45, v)); }

  $: lockRatio = presetNormAspect(aspect, orientedRatio, orientation);
  // Keep the crop inside the rotated image (constrainToRotated is idempotent → no loop).
  $: if (angle !== 0) rect = constrainToRotated(rect, angle, oW, oH);

  let prevTool = $tool;
  $: {
    if ($tool === "crop" && prevTool !== "crop") startCrop();
    if ($tool !== "crop" && prevTool === "crop") { commitCrop(); cropInit = false; }
    prevTool = $tool;
  }

  function rotateCommitted(dir: number) {
    const id = $activeId; if (!id) return;
    const base: CropRect = $activeCrop ?? { rect: { x: 0, y: 0, w: 1, h: 1 }, aspect: "custom", orientation: origW >= origH ? "landscape" : "portrait", rot90: 0, flipH: false, flipV: false, angle: 0 };
    const nr = dir > 0 ? rotateRectCW(base.rect) : rotateRectCCW(base.rect);
    const nrot = ((base.rot90 + (dir > 0 ? 1 : 3)) % 4) as 0 | 1 | 2 | 3;
    cropById.update((m) => ({ ...m, [id]: { ...base, rect: nr, rot90: nrot } }));
  }
  function onKey(e: KeyboardEvent) {
    const meta = e.metaKey || e.ctrlKey;
    if ($tool === "eraser" && meta && (e.key === "z" || e.key === "Z")) {
      e.preventDefault(); undoDust(); return;
    }
    if (meta && (e.key === "]" || e.key === "[")) {
      e.preventDefault();
      const dir = e.key === "]" ? 1 : -1;
      if ($tool === "crop") onRotate(dir); else rotateCommitted(dir);
      return;
    }
    if ($tool !== "crop") return;
    if (e.key === "Enter") { commitCrop(); tool.set("edit"); }
    else if (e.key === "Escape") { discardCrop(); }
    else if (e.key === "x" || e.key === "X") { onSwap(); }
  }

  // Committed crop → effective dims + image_crop for the normal Viewport.
  $: committed = $activeCrop;
  $: cRot = committed?.rot90 ?? 0;
  $: [coW, coH] = orientDims(origW, origH, cRot);
  $: effW = committed ? Math.max(1, Math.round(committed.rect.w * coW)) : coW;
  $: effH = committed ? Math.max(1, Math.round(committed.rect.h * coH)) : coH;
  $: imageCrop = committed ? [committed.rect.x, committed.rect.y, committed.rect.w, committed.rect.h] as [number, number, number, number] : null;

  let thumbTimer: ReturnType<typeof setTimeout> | null = null;
  function refreshThumb() {
    if (thumbTimer) clearTimeout(thumbTimer);
    const id = $activeId;
    if (!id) return;
    thumbTimer = setTimeout(async () => {
      try {
        const t = await api.thumbnail(id, $params);
        images.update((xs) => xs.map((i) => (i.id === id ? { ...i, thumbnail: t } : i)));
      } catch { /* ignore */ }
    }, 400);
  }
  $: $params, $activeId, refreshThumb();

  let brush = 0.03;            // normalized-to-width brush radius
  let dustRev = 0;            // bumped on any dust change to force Viewport re-render
  $: dust = $activeDust;

  // Apply a reducer to the active image's dust edits and force a Viewport re-render.
  function updateDust(fn: (d: DustEdits) => DustEdits) {
    const id = $activeId; if (!id) return;
    dustById.update((m) => ({ ...m, [id]: fn(m[id] ?? emptyDust()) }));
    dustRev++;
  }
  const commitStroke = (s: DustStroke) => updateDust((d) => addStroke(d, s));
  const undoDust = () => updateDust((d) => undoStroke(d));
  const resetDustEdits = () => updateDust((d) => resetDust(d));

  let menu: { x: number; y: number } | null = null;
  function onContext(e: MouseEvent) { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }

  let exporting = false, msg = "";
  async function exportTiff() {
    if (!$activeId) return;
    const out = await save({ defaultPath: "redroom-export.tiff", filters: [{ name: "TIFF", extensions: ["tiff"] }] });
    if (!out) return;
    exporting = true; msg = "";
    try {
      await api.exportImage($activeId, $params, out, imageCrop, {
        rot90: committed?.rot90 ?? 0, flip_h: committed?.flipH ?? false,
        flip_v: committed?.flipV ?? false, angle: committed?.angle ?? 0,
      }, dust.strokes);
      msg = "Exported ✓";
    } catch (e) { msg = "Error: " + e; }
    exporting = false;
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="layout" on:contextmenu={onContext}>
  <section class="center">
    {#if active?.developed}
      {#if $tool === "crop"}
        <CropView id={$activeId} params={$params} imgW={oW} imgH={oH}
                  bind:rect {lockRatio} {rot90} {flipH} {flipV} {angle}
                  on:custom={() => (aspect = "custom")} on:straighten={(e) => onStraighten(e.detail)} />
      {:else}
        <Viewport id={$activeId} params={$params} imgW={effW} imgH={effH} imageCrop={imageCrop}
                  rot90={cRot} flipH={committed?.flipH ?? false} flipV={committed?.flipV ?? false} angle={committed?.angle ?? 0}
                  eraser={$tool === "eraser"} {brush} dust={dust.strokes} {dustRev}
                  on:stroke={(e) => commitStroke(e.detail)} on:brush={(e) => (brush = e.detail)} />
      {/if}
    {:else}<div class="hint">Not developed yet</div>{/if}
  </section>

  <aside class="right">
    <GlassPanel>
      <Histogram />
      <Toolbar />
      {#if $tool === "edit"}
        <Basic />
      {:else if $tool === "crop"}
        <CropPanel bind:aspect bind:orientation bind:angle
                   on:preset={(e) => onPreset(e.detail)} on:swap={onSwap} on:reset={onReset}
                   on:rotate={(e) => onRotate(e.detail)} on:flip={(e) => onFlip(e.detail)} />
      {:else if $tool === "eraser"}
        <EraserPanel bind:brush on:reset={resetDustEdits} />
      {/if}
      <button class="export" on:click={exportTiff} disabled={exporting || !$activeId}>
        {exporting ? "Exporting…" : "Export 16-bit TIFF"}
      </button>
      {#if msg}<div class="msg">{msg}</div>{/if}
    </GlassPanel>
  </aside>

  <footer class="bottom"><Filmstrip /></footer>
</div>
{#if menu}<QualityMenu x={menu.x} y={menu.y} on:close={() => (menu = null)} />{/if}

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 1fr 300px; grid-template-rows: 1fr 88px;
    grid-template-areas: "center right" "bottom bottom"; }
  .right { grid-area: right; min-height: 0; overflow-y: auto; }
  .center { grid-area: center; min-height: 0; display: grid; place-items: center; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; }
  .export { width: 100%; margin-top: 12px; padding: 10px; border: 0; border-radius: 10px;
    background: var(--accent); color: white; font-weight: 600; cursor: pointer; }
  .export:disabled { opacity: 0.5; }
  .msg { margin-top: 8px; color: var(--text-dim); font-size: 12px; }
</style>
