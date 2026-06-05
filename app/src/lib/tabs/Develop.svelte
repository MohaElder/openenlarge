<script lang="ts">
  import { t } from "$lib/i18n";
  import { activeId, params, images, folderImages, tool, cropById, activeCrop, dustById, activeDust, deleteTarget, dustRev, folderBaseByPath } from "../store";
  import { imageDir } from "../library/folderScope";
  import { withEffectiveBase } from "../develop/base";
  import { api } from "../api";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import Viewport from "../viewport/Viewport.svelte";
  import QualityMenu from "../viewport/QualityMenu.svelte";
  import Histogram from "../viewport/Histogram.svelte";
  import Toolbar from "../develop/Toolbar.svelte";
  import Basic from "../develop/Basic.svelte";
  import TonalCurve from "../develop/TonalCurve.svelte";
  import ColorGrading from "../develop/ColorGrading.svelte";
  import ColorMixer from "../develop/ColorMixer.svelte";
  import GlassPanel from "../glass/GlassPanel.svelte";
  import CropView from "../crop/CropView.svelte";
  import CropPanel from "../crop/CropPanel.svelte";
  import BaseView from "../develop/BaseView.svelte";
  import BasePanel from "../develop/BasePanel.svelte";
  import EraserPanel from "../develop/EraserPanel.svelte";
  import { setFolderBase, clearFolderBase } from "../develop/base";
  import { addStroke, resetDust, emptyDust, setIrEnabled, setIrSensitivity, type DustStroke, type DustEdits } from "../develop/dust";
  import type { Rect, CropRect } from "../crop/types";
  import { default80, conform, constrainToRotated } from "../crop/cropMath";
  import { presetNormAspect } from "../crop/presets";
  import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV, orientDims } from "../crop/transforms";
  import { commitActive } from "../develop/historyStore";
  import { rgbToHslSample } from "../develop/colorPick";

  $: active = $images.find((i) => i.id === $activeId);
  $: origW = active?.metadata.width ?? 0;
  $: origH = active?.metadata.height ?? 0;
  $: dir = active ? imageDir(active) : "";
  // eslint-disable-next-line @typescript-eslint/no-unused-expressions
  $: { void $folderBaseByPath; effParams = withEffectiveBase($params, dir); }
  let effParams = withEffectiveBase($params, dir);

  // ---- Base picker state ----
  let sampledBase: [number, number, number] | null = null;
  // eslint-disable-next-line @typescript-eslint/no-unused-expressions
  $: { $activeId; sampledBase = null; }

  function applyBaseRoll() {
    if (!sampledBase || !dir) return;
    setFolderBase(dir, sampledBase);
  }
  function applyBaseThisImage() {
    if (!sampledBase) return;
    params.update((p) => ({ ...p, base_override: sampledBase }));
    commitActive();
  }
  function resetBase() {
    // Clear the per-image override first; if none, clear the folder default.
    if ($params.base_override) {
      params.update((p) => ({ ...p, base_override: null }));
      commitActive();
    } else if (dir) clearFolderBase(dir);
  }
  $: baseScope = ($params.base_override ? "override" : (dir && $folderBaseByPath[dir] ? "folder" : "auto")) as "override" | "folder" | "auto";

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
    commitActive();
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
    commitActive();
  }
  // True while a form control has focus, so its own arrow-key behaviour wins
  // (e.g. nudging a slider) instead of stepping the image.
  function formFocused(): boolean {
    const a = document.activeElement;
    const tag = a?.tagName;
    return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
  }

  // Arrow keys step through images from anywhere in Develop (not just the filmstrip).
  function navImages(e: KeyboardEvent): boolean {
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    const arrows = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];
    if (!arrows.includes(e.key) || formFocused()) return false;
    const list = $folderImages;
    if (list.length === 0) return false;
    let idx = list.findIndex((i) => i.id === $activeId);
    if (idx < 0) idx = 0;
    if (e.key === "ArrowLeft") idx = Math.max(0, idx - 1);
    else if (e.key === "ArrowRight") idx = Math.min(list.length - 1, idx + 1);
    else if (e.key === "ArrowUp") idx = 0;
    else idx = list.length - 1;
    e.preventDefault();
    activeId.set(list[idx].id);
    return true;
  }

  function onKey(e: KeyboardEvent) {
    const meta = e.metaKey || e.ctrlKey;
    if (meta && e.key === "Backspace") {
      e.preventDefault();
      if ($activeId && !formFocused()) deleteTarget.set($activeId);
      return;
    }
    if (meta && (e.key === "]" || e.key === "[")) {
      e.preventDefault();
      const dir = e.key === "]" ? 1 : -1;
      if ($tool === "crop") onRotate(dir); else rotateCommitted(dir);
      return;
    }
    if (navImages(e)) return;
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
    const c = $activeCrop;
    const d = $activeDust;
    const view = {
      image_crop: c ? [c.rect.x, c.rect.y, c.rect.w, c.rect.h] as [number, number, number, number] : null,
      rot90: c?.rot90 ?? 0, flip_h: c?.flipH ?? false, flip_v: c?.flipV ?? false, angle: c?.angle ?? 0,
      dust: d.strokes, ir_removal: d.irRemoval,
    };
    thumbTimer = setTimeout(async () => {
      try {
        const t = await api.thumbnail(id, effParams, view);
        images.update((xs) => xs.map((i) => (i.id === id ? { ...i, thumbnail: t } : i)));
      } catch { /* ignore */ }
    }, 400);
  }
  $: $params, $activeId, $activeCrop, $activeDust, $folderBaseByPath, refreshThumb();

  let brush = 0.03;            // normalized-to-width brush radius
  $: dust = $activeDust;

  // Apply a reducer to the active image's dust edits and force a Viewport re-render.
  function updateDust(fn: (d: DustEdits) => DustEdits) {
    const id = $activeId; if (!id) return;
    dustById.update((m) => ({ ...m, [id]: fn(m[id] ?? emptyDust()) }));
    dustRev.update((n) => n + 1);
  }
  const commitStroke = (s: DustStroke) => updateDust((d) => addStroke(d, s));
  const resetDustEdits = () => updateDust((d) => resetDust(d));
  function setIrOn(on: boolean) { updateDust((d) => setIrEnabled(d, on)); }
  function setIrSens(v: number) { updateDust((d) => setIrSensitivity(d, v)); }

  $: hasIr = active?.has_ir ?? false;

  let menu: { x: number; y: number } | null = null;
  function onContext(e: MouseEvent) { e.preventDefault(); menu = { x: e.clientX, y: e.clientY }; }

  // ---- Point Color eyedropper state ----
  let pointPicking = false;
  function togglePointPick() { pointPicking = !pointPicking; }
  function onPointPick(e: CustomEvent<{ r: number; g: number; b: number }>) {
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      if (arr.length >= 8) return p; // cap at 8
      arr.push(rgbToHslSample(e.detail.r, e.detail.g, e.detail.b));
      return { ...p, pc_samples: arr };
    });
    pointPicking = false;
  }
</script>

<svelte:window on:keydown={onKey} />

<div class="layout" on:contextmenu={onContext}>
  <section class="center">
    {#if active?.developed}
      {#if $tool === "crop"}
        <CropView id={$activeId} params={effParams} imgW={oW} imgH={oH}
                  bind:rect {lockRatio} {rot90} {flipH} {flipV} {angle}
                  on:custom={() => (aspect = "custom")} on:straighten={(e) => onStraighten(e.detail)} />
      {:else if $tool === "base_picker"}
        <BaseView id={$activeId} params={effParams} imgW={origW} imgH={origH}
                  on:sampled={(e) => (sampledBase = e.detail)} />
      {:else}
        <Viewport id={$activeId} params={effParams} imgW={effW} imgH={effH} imageCrop={imageCrop}
                  rot90={cRot} flipH={committed?.flipH ?? false} flipV={committed?.flipV ?? false} angle={committed?.angle ?? 0}
                  eraser={$tool === "eraser"} {brush} dust={dust.strokes} irRemoval={dust.irRemoval} dustRev={$dustRev}
                  pointPick={pointPicking}
                  on:stroke={(e) => commitStroke(e.detail)} on:brush={(e) => (brush = e.detail)}
                  on:pointpick={onPointPick} />
      {/if}
    {:else}<div class="hint">{$t('develop.notDevelopedYet')}</div>{/if}
  </section>

  <aside class="right">
    <GlassPanel>
      <Histogram />
      <Toolbar />
      {#if $tool === "edit"}
        <Basic />
        <TonalCurve />
        <ColorGrading />
        <ColorMixer onPick={togglePointPick} picking={pointPicking} />
      {:else if $tool === "crop"}
        <CropPanel bind:aspect bind:orientation bind:angle
                   on:preset={(e) => onPreset(e.detail)} on:swap={onSwap} on:reset={onReset}
                   on:rotate={(e) => onRotate(e.detail)} on:flip={(e) => onFlip(e.detail)} />
      {:else if $tool === "eraser"}
        <EraserPanel bind:brush {hasIr}
                     irEnabled={dust.irRemoval.enabled} irSensitivity={dust.irRemoval.sensitivity}
                     on:reset={resetDustEdits}
                     on:irEnabled={(e) => setIrOn(e.detail)}
                     on:irSensitivity={(e) => setIrSens(e.detail)} />
      {:else if $tool === "base_picker"}
        <BasePanel sampled={sampledBase} scope={baseScope}
                   on:applyRoll={applyBaseRoll} on:thisImage={applyBaseThisImage} on:reset={resetBase} />
      {/if}
    </GlassPanel>
  </aside>

  <footer class="bottom"><Filmstrip /></footer>
</div>
{#if menu}<QualityMenu x={menu.x} y={menu.y}
  on:delete={() => { if ($activeId) deleteTarget.set($activeId); menu = null; }}
  on:close={() => (menu = null)} />{/if}

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 1fr 300px; grid-template-rows: 1fr 88px;
    grid-template-areas: "center right" "bottom right"; }
  .right { grid-area: right; min-height: 0; overflow-y: auto;
    scrollbar-width: none; -ms-overflow-style: none; }
  .right::-webkit-scrollbar { width: 0; height: 0; }
  .center { grid-area: center; min-height: 0; display: grid; place-items: center; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; min-width: 0; overflow: hidden; }
</style>
