<script lang="ts">
  import { t } from "$lib/i18n";
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { activeId, params, images, folderImages, tool, cropById, activeCrop, dustById, activeDust, deleteTarget, dustRev, developRev, folderBaseByPath, baseSampling, sampledBase, selectAll, deleteSelectionIds, setActive, previewSrc, clipWarn, hotkeyBindings, autodustSpotsById, activeAutodustSpots, selectedSpot, editPage, type EditPage } from "../store";
  import { get } from "svelte/store";
  import { onMount } from "svelte";
  import { createPreviewPrefetcher } from "../develop/previewPrefetch";
  import { imageDir } from "../library/folderScope";
  import { withEffectiveBase } from "../develop/base";
  import { applyAsShotWb } from "../develop/wb";
  import { mergeEnsured } from "../workflow";
  import { api } from "../api";
  import Filmstrip from "../panels/Filmstrip.svelte";
  import ImageContextMenu from "../overlay/ImageContextMenu.svelte";
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
  import EraserPanel from "../develop/EraserPanel.svelte";
  import AutoDustPanel from "../develop/AutoDustPanel.svelte";
  import AiEnhancePanel from "../develop/AiEnhancePanel.svelte";
  import { addStroke, resetDust, emptyDust, setIrEnabled, setIrSensitivity, setAutoDustEnabled, setAutoDustSensitivity, setBrushMigan, setAiApplied, removeStrokeAt, addExclusion, setShowSpots, type DustStroke, type DustEdits } from "../develop/dust";
  import { listen } from "@tauri-apps/api/event";
  import type { Rect, CropRect } from "../crop/types";
  import { defaultFull, conform, constrainToRotated } from "../crop/cropMath";
  import { presetNormAspect } from "../crop/presets";
  import { rotateRectCW, rotateRectCCW, flipRectH, flipRectV, flipOrient, orientDims } from "../crop/transforms";
  import { commitActive, reseedActive } from "../develop/historyStore";
  import { copyDevelopSettings, pasteDevelopSettings } from "../develop/copySettings";
  import { rgbToHslSample } from "../develop/colorPick";
  import { inTextField, isRangeFocused } from "../keymap/focus";
  import { matchCombo, selectorParam, normKey, type AdjustParam } from "../keymap/hotkeys";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

  async function revealImage(id: string | null) {
    const img = $images.find((i) => i.id === id);
    if (img) { try { await revealItemInDir(img.path); } catch (e) { console.error("reveal failed", e); } }
  }

  // Warm ~1080p previews for nearby developed images while idle, so a first click on a
  // filmstrip thumbnail shows the developed look instantly (the Viewport switch overlay
  // reads previewById). Cancels on any interaction; developed images only.
  onMount(() => {
    const prefetcher = createPreviewPrefetcher();
    let un: (() => void) | null = null;
    listen<{ id: string; count: number; spots: [number, number][] }>("autodust://result", (e) => {
      const { id, spots } = e.payload;
      autodustSpotsById.update((m) => ({ ...m, [id]: (spots ?? []).map(([x, y]) => ({ x, y })) }));
    }).then((u) => { un = u; });
    return () => { prefetcher.stop(); un?.(); };
  });

  $: active = $images.find((i) => i.id === $activeId);
  $: origW = active?.metadata.width ?? 0;
  $: origH = active?.metadata.height ?? 0;
  $: dir = active ? imageDir(active) : "";
  // eslint-disable-next-line @typescript-eslint/no-unused-expressions
  $: { void $folderBaseByPath; effParams = withEffectiveBase($params, dir); }
  let effParams = withEffectiveBase($params, dir);

  // Lazily upgrade the selected image to the current quality. No-op on the backend
  // when the resident buffer already satisfies it (Performance, or already full-res),
  // so this is cheap on every navigation. The stale guard drops results when the
  // user has already moved on (rapid arrow-key stepping in Quality mode).
  let lastEnsured: string | null = null;
  async function ensureActiveDeveloped(id: string | null) {
    if (!id || id === lastEnsured) return;
    lastEnsured = id;
    try {
      const updated = await api.ensureDeveloped(id);
      if (get(activeId) !== id) return; // navigated away mid-decode
      // Keep the live (edited-look) thumbnail; ensureDeveloped returns the develop-time
      // default-params render, which would otherwise flash the filmstrip back to the
      // un-adjusted look for ~400ms (until refreshThumb) on every navigation.
      images.update((list) => list.map((i) => (i.id === id ? mergeEnsured(i, updated) : i)));
      developRev.update((n) => n + 1);
    } catch (e) {
      console.error("ensureDeveloped failed", id, e);
    }
  }
  $: ensureActiveDeveloped($activeId);

  // ---- Base recalibration (armed from Basic > Film Base) ----
  // The Film Base tools live in Basic.svelte; here we only render the sampling
  // overlay while armed and disarm it whenever we leave the edit tool.
  $: if ($tool !== "edit" && $baseSampling) baseSampling.set(false);

  // ---- Crop draft state (only while tool === "crop") ----
  let rect: Rect = { x: 0, y: 0, w: 1, h: 1 };
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
      rect = defaultFull(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait";
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
    else { rect = defaultFull(); aspect = "original"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  }
  function onPreset(id: string) { aspect = id; rect = conform(rect, presetNormAspect(id, orientedRatio, orientation)); }
  function onSwap() { orientation = orientation === "landscape" ? "portrait" : "landscape"; rect = conform(rect, presetNormAspect(aspect, orientedRatio, orientation)); }
  function onReset() { rect = defaultFull(); aspect = "original"; orientation = origW >= origH ? "landscape" : "portrait"; rot90 = 0; flipH = false; flipV = false; angle = 0; }
  function onRotate(dir: number) {
    if (dir > 0) { rot90 = (rot90 + 1) % 4; rect = rotateRectCW(rect); }
    else { rot90 = (rot90 + 3) % 4; rect = rotateRectCCW(rect); }
  }
  function onFlip(axis: "h" | "v") {
    // Flip the *displayed* image: the backend flips before rot90, so for odd
    // quarter-turns flipOrient negates rot90 to keep H/H and V/V (see transforms.ts).
    ({ rot90, flipH, flipV } = flipOrient({ rot90, flipH, flipV }, axis));
    rect = axis === "h" ? flipRectH(rect) : flipRectV(rect);
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

  // Flip the committed image (mirrors onFlip's draft logic) — used by the develop
  // context menus when a single image is selected.
  function flipCommitted(axis: "h" | "v") {
    const id = $activeId; if (!id) return;
    const base: CropRect = $activeCrop ?? { rect: { x: 0, y: 0, w: 1, h: 1 }, aspect: "custom", orientation: origW >= origH ? "landscape" : "portrait", rot90: 0, flipH: false, flipV: false, angle: 0 };
    const o = flipOrient({ rot90: base.rot90, flipH: base.flipH, flipV: base.flipV }, axis);
    const nr = axis === "h" ? flipRectH(base.rect) : flipRectV(base.rect);
    cropById.update((m) => ({ ...m, [id]: { ...base, rot90: o.rot90 as 0 | 1 | 2 | 3, flipH: o.flipH, flipV: o.flipV, rect: nr, angle: -base.angle } }));
    commitActive();
  }
  // ---- Keyboard adjustment nudges (hold-selector + ←/→) ----
  // A held selector key picks a parameter; ← lowers, → raises it. Each press is a
  // coarse step; holding Shift gives the 1/10 fine step. Steps mirror the retuned I2
  // slider ranges. Temp is nudged in mireds (1e6/K) so a press moves the white point
  // by an even perceptual amount across the reciprocal track, then clamped to the same
  // 2000–25000 K range as the slider. WB nudges mark wb_manual (so the auto-reseed
  // won't clobber them); every nudge commits one undo step. See lib/keymap/hotkeys.ts
  // for the default key→parameter map (1·temp, 2·tint, q·exposure, w·contrast,
  // a·highlights, s·shadows, z·whites, x·blacks) and the user's rebindings.
  const TEMP_MIN = 2000, TEMP_MAX = 25000;
  // Coarse step per parameter (Shift → ×0.1). Temp is in mireds; the ±100 tone
  // sliders share one step; exposure (±5) is finer.
  // Temp/tint steps are perceptually balanced (issue #20): on a mid-gray through
  // the engine's Planckian WB mapping, one press ≈ 0.65 ΔE₇₆ for BOTH axes.
  // The old 5-mired/2-unit pair was ~4× stronger on temp (1.07 vs 0.27 ΔE):
  // one tint unit is only a 1/300 green shift (tint/150 · 0.5), so it needs the
  // larger raw step, not the smaller one.
  const NUDGE: Record<AdjustParam, number> = {
    temp: 3, tint: 5, exposure: 0.1,
    contrast: 2, highlights: 2, shadows: 2, whites: 2, blacks: 2,
  };
  const clampN = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

  function adjTemp(miredStep: number) {
    // +mired = lower Kelvin (warmer); -mired = higher Kelvin (cooler).
    params.update((p) => ({
      ...p,
      temp: clampN(1e6 / (1e6 / p.temp + miredStep), TEMP_MIN, TEMP_MAX),
      wb_manual: true,
    }));
    commitActive();
  }
  function adjTint(delta: number) {
    params.update((p) => ({
      ...p,
      tint: clampN(Math.round((p.tint + delta) * 10) / 10, -100, 100),
      wb_manual: true,
    }));
    commitActive();
  }
  function adjExposure(delta: number) {
    params.update((p) => ({
      ...p,
      exposure: clampN(Math.round((p.exposure + delta) * 100) / 100, -5, 5),
    }));
    commitActive();
  }
  // The ±100 tone sliders (contrast/highlights/shadows/whites/blacks).
  function adjLinear(key: "contrast" | "highlights" | "shadows" | "whites" | "blacks", delta: number) {
    params.update((p) => ({ ...p, [key]: clampN(Math.round((p[key] + delta) * 10) / 10, -100, 100) }));
    commitActive();
  }

  /** Nudge a develop parameter. `dir` = +1 for → (raise), −1 for ← (lower). */
  function adjustParam(p: AdjustParam, dir: number, fine: boolean) {
    const step = NUDGE[p] * (fine ? 0.1 : 1) * dir;
    if (p === "temp") adjTemp(-step);       // → raises Kelvin ⇒ negative mired
    else if (p === "tint") adjTint(step);
    else if (p === "exposure") adjExposure(step);
    else adjLinear(p, step);
  }

  // ---- Held-key tracking for the adjustment chords ----
  // A keyup or window blur clears the set so a missed keyup can't leave a selector
  // "stuck" armed. Typing into a text field never arms a selector.
  let heldKeys = new Set<string>();
  function onKeyUp(e: KeyboardEvent) { heldKeys.delete(normKey(e.key)); }
  function clearHeld() { heldKeys.clear(); }
  function heldAdjustParam(): AdjustParam | null {
    const ov = get(hotkeyBindings);
    for (const k of heldKeys) { const p = selectorParam(k, ov); if (p) return p; }
    return null;
  }

  // Flip the active image — draft flip in the crop tool, committed flip elsewhere.
  function flipActive(axis: "h" | "v") { if ($tool === "crop") onFlip(axis); else flipCommitted(axis); }

  // Step the active image within the folder. Plain arrows defer to a focused
  // text field / range slider so their native arrow behaviour wins.
  function stepNav(target: number | "first" | "last", e: KeyboardEvent): boolean {
    if (inTextField() || isRangeFocused()) return false;
    const list = $folderImages;
    if (list.length === 0) return false;
    let idx = list.findIndex((i) => i.id === $activeId);
    if (idx < 0) idx = 0;
    if (target === "first") idx = 0;
    else if (target === "last") idx = list.length - 1;
    else idx = Math.max(0, Math.min(list.length - 1, idx + target));
    e.preventDefault();
    setActive(list[idx].id);
    return true;
  }

  /** Run a resolved combo action. Returns true if it consumed the key. */
  function runCombo(id: string, e: KeyboardEvent): boolean {
    const inCrop = $tool === "crop";
    switch (id) {
      case "select.all":
        if (inTextField()) return false;
        e.preventDefault(); selectAll(); return true;
      case "nav.delete": {
        if (inTextField()) return false;
        // In the eraser tool, the delete hotkey removes the selected heal spot
        // (not the image).
        if ($tool === "eraser" && get(selectedSpot)) {
          e.preventDefault();
          removeSpot(get(selectedSpot)!);
          return true;
        }
        e.preventDefault();
        const ids = deleteSelectionIds(); if (ids.length) deleteTarget.set(ids);
        return true;
      }
      case "edit.copySettings":
        if (inTextField()) return false; // let native text copy win
        e.preventDefault(); copyDevelopSettings(); return true;
      case "edit.pasteSettings":
        if (inTextField()) return false; // let native text paste win
        e.preventDefault(); pasteDevelopSettings(); return true;
      case "edit.rotateCCW":
        e.preventDefault(); if (inCrop) onRotate(-1); else rotateCommitted(-1); return true;
      case "edit.rotateCW":
        e.preventDefault(); if (inCrop) onRotate(1); else rotateCommitted(1); return true;
      case "edit.flipV": e.preventDefault(); flipActive("v"); return true;
      case "edit.flipH": e.preventDefault(); flipActive("h"); return true;
      case "nav.prev":  return stepNav(-1, e);
      case "nav.next":  return stepNav(1, e);
      case "nav.first": return stepNav("first", e);
      case "nav.last":  return stepNav("last", e);
      case "crop.commit":  e.preventDefault(); commitCrop(); tool.set("edit"); return true;
      case "crop.discard": e.preventDefault(); discardCrop(); return true;
      case "crop.swap":    e.preventDefault(); onSwap(); return true;
      // edit.undo / edit.redo are handled globally in +page.svelte.
    }
    return false;
  }

  function onKey(e: KeyboardEvent) {
    // Arm the selector set (never while typing, so "1" stays a literal in a field).
    if (!inTextField()) heldKeys.add(normKey(e.key));

    const inCrop = $tool === "crop";

    // 1) Adjustment chord: a held selector + ←/→ nudges its parameter. Checked
    //    before combos so a focused slider's native arrow stepping is suppressed.
    if (!inTextField() && !inCrop && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      const p = heldAdjustParam();
      if (p) { adjustParam(p, e.key === "ArrowRight" ? 1 : -1, e.shiftKey); e.preventDefault(); return; }
    }

    // 2) Registered combos (nav / rotate / flip / copy / paste / crop).
    const id = matchCombo(e, get(hotkeyBindings), inCrop);
    if (id && runCombo(id, e)) return;

    // 3) Escape cancels an in-progress eyedropper pick.
    if (e.key === "Escape" && pickTarget) { pickTarget = ""; e.preventDefault(); }
  }

  // Committed crop → effective dims + image_crop for the normal Viewport.
  $: committed = $activeCrop;
  $: cRot = committed?.rot90 ?? 0;
  $: [coW, coH] = orientDims(origW, origH, cRot);
  $: effW = committed ? Math.max(1, Math.round(committed.rect.w * coW)) : coW;
  $: effH = committed ? Math.max(1, Math.round(committed.rect.h * coH)) : coH;
  $: imageCrop = committed ? [committed.rect.x, committed.rect.y, committed.rect.w, committed.rect.h] as [number, number, number, number] : null;
  // The raw catalog thumbnail is native-oriented + full-frame, so it only matches the
  // developed view (and is safe as a placeholder) when no crop/rotation is committed.
  $: thumbMatchesView = !committed || (cRot === 0 && !committed.flipH && !committed.flipV && !committed.angle && committed.rect.x === 0 && committed.rect.y === 0 && committed.rect.w === 1 && committed.rect.h === 1);

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
        // Persist the edited-look thumbnail so the filmstrip keeps the user's edits
        // across relaunch instead of reverting to the develop-time default render.
        api.saveThumbnail(id, t).catch(() => { /* best-effort */ });
      } catch { /* ignore */ }
    }, 400);
  }
  $: $params, $activeId, $activeCrop, $activeDust, $folderBaseByPath, refreshThumb();

  let brush = 0.03;            // normalized-to-width brush radius
  let zoomMarquee = false; // eraser marquee-zoom armed
  let viewZoomed = false;  // eraser viewport currently magnified
  let vp: import("$lib/viewport/Viewport.svelte").default; // Viewport instance for resetZoom()
  // Disarm the marquee when the eraser tool isn't active so it can't leak into
  // another tool or persist across a Viewport remount.
  $: if ($tool !== "eraser") zoomMarquee = false;
  $: dust = $activeDust;

  // Apply a reducer to the active image's dust edits and force a Viewport re-render.
  function updateDust(fn: (d: DustEdits) => DustEdits) {
    const id = $activeId; if (!id) return;
    dustById.update((m) => ({ ...m, [id]: fn(m[id] ?? emptyDust()) }));
    dustRev.update((n) => n + 1);
  }
  // showSpots is a view-only overlay flag — update the store (so it persists and the
  // markers re-render via the prop) WITHOUT bumping dustRev, so toggling it never
  // triggers a re-bake (which would re-run auto-dust and fire the "N spots removed"
  // toast).
  function setShowSpotsEdit(on: boolean) {
    const id = $activeId; if (!id) return;
    dustById.update((m) => ({ ...m, [id]: setShowSpots(m[id] ?? emptyDust(), on) }));
  }
  // Clear any selection when leaving the eraser tool or switching image.
  $: if ($tool !== "eraser") selectedSpot.set(null);
  $: { $activeId; selectedSpot.set(null); }

  /** Remove a heal spot: a brush stroke, or a global spot (kept via exclusion). */
  function removeSpot(sel: import("../store").SpotSel) {
    if (sel.kind === "stroke") {
      updateDust((d) => removeStrokeAt(d, sel.index));
    } else {
      const c = $activeAutodustSpots[sel.index];
      if (c) updateDust((d) => addExclusion(d, c));
    }
    selectedSpot.set(null);
  }

  const commitStroke = (s: DustStroke) => updateDust((d) => addStroke(d, s));
  const resetDustEdits = () => updateDust((d) => resetDust(d));
  function setIrOn(on: boolean) { updateDust((d) => setIrEnabled(d, on)); }
  function setIrSens(v: number) { updateDust((d) => setIrSensitivity(d, v)); }
  function setAutoSens(v: number) { if (dust.autoDust.enabled) autoBusy = true; updateDust((d) => setAutoDustSensitivity(d, v)); }
  function setAutoOn(on: boolean) { autoBusy = on; updateDust((d) => setAutoDustEnabled(d, on)); }
  function setBrushAi(on: boolean) { updateDust((d) => setBrushMigan(d, on)); }
  let aiBusy = false;
  // Spinner for the AI auto-dust toggle: set the instant the user taps (proves the
  // tap registered), cleared when the Viewport's bake completes (`autodusted`).
  let autoBusy = false;
  function aiErase() { aiBusy = true; updateDust((d) => setAiApplied(d, true)); }
  // Never let the erase spinner outlive the active image.
  $: { $activeId; aiBusy = false; }

  $: hasIr = active?.has_ir ?? false;

  // Right-click on a filmstrip thumbnail opens the image Delete menu (acting on the
  // whole selection); right-click anywhere else in Develop opens the quality menu.
  let menu: { x: number; y: number } | null = null;
  let thumbMenu: { x: number; y: number; id: string } | null = null;
  function onContext(e: MouseEvent) {
    e.preventDefault();
    const onThumb = (e.target as HTMLElement).closest("[data-id]");
    const id = onThumb?.getAttribute("data-id");
    if (onThumb && id) { thumbMenu = { x: e.clientX, y: e.clientY, id }; menu = null; }
    else { menu = { x: e.clientX, y: e.clientY }; thumbMenu = null; }
  }

  // ---- Eyedropper state ----
  // One crosshair, two consumers: 'pc' = ColorMixer point-colour sample, 'wb' = gray-point
  // white balance. The target string routes the single pointpick event to the right place.
  let pickTarget: "" | "pc" | "wb" = "";
  function togglePcPick() { pickTarget = pickTarget === "pc" ? "" : "pc"; }
  function toggleWbPick() { pickTarget = pickTarget === "wb" ? "" : "wb"; }
  async function onPointPick(e: CustomEvent<{ r: number; g: number; b: number; u: number; v: number; rr: number; rg: number; rb: number }>) {
    const { r, g, b, u, v, rr, rg, rb } = e.detail;
    const target = pickTarget;
    pickTarget = "";
    if (target === "wb") {
      if (!$activeId) return;
      // Use the grain-robust window median (not the single pixel) so a gray-point
      // pick over grainy film lands a stable Temp/Tint instead of an extreme (D).
      const wb = await api.grayPointWb(get(params), [rr, rg, rb]);
      // Mark WB user-controlled so a later base/profile change won't auto-reseed over it.
      params.update((p) => ({ ...applyAsShotWb(p, wb), wb_manual: true }));
      reseedActive();
    } else if (target === "pc") {
      params.update((p) => {
        const arr = (p.pc_samples ?? []).slice();
        if (arr.length >= 8) return p; // cap at 8
        arr.push(rgbToHslSample(r, g, b));
        return { ...p, pc_samples: arr };
      });
    }
  }
</script>

<svelte:window on:keydown={onKey} on:keyup={onKeyUp} on:blur={clearHeld} />

<div class="layout" on:contextmenu={onContext}>
  <section class="center">
    {#if active?.developed}
      {#if $tool === "crop"}
        <CropView id={$activeId} params={effParams} imgW={oW} imgH={oH}
                  bind:rect {lockRatio} {rot90} {flipH} {flipV} {angle}
                  on:custom={() => (aspect = "custom")} on:straighten={(e) => onStraighten(e.detail)} />
      {:else}
        <!-- Cold-start placeholder: the in-memory catalog thumbnail fills the main view
             instantly while the Viewport decodes the full working buffer from cache on
             first launch (`previewSrc` is empty until the first real frame is rendered).
             object-fit:contain matches the Viewport's fit, so the real frame paints over
             it seamlessly. -->
        {#if active?.thumbnail && !$previewSrc && thumbMatchesView}
          <img class="cold-thumb" src={active.thumbnail} alt="" draggable="false" />
        {/if}
        <!-- The Viewport stays mounted while the film-base picker is armed; BaseView
             overlays it. Unmounting the Viewport tears down its GPU context and forces
             a full working-buffer re-fetch + re-upload on dismiss (a multi-second blank),
             so we keep it alive and just cover it. -->
        <Viewport bind:this={vp} allowHdrSurface={true} id={$activeId} params={effParams} imgW={effW} imgH={effH} imageCrop={imageCrop} fallbackThumb={active?.thumbnail ?? ""}
                  rot90={cRot} flipH={committed?.flipH ?? false} flipV={committed?.flipV ?? false} angle={committed?.angle ?? 0}
                  eraser={$tool === "eraser"} marquee={zoomMarquee} {brush} dust={dust.strokes} irRemoval={dust.irRemoval} dustRev={$dustRev} developRev={$developRev}
                  brushMigan={dust.brushMigan} aiApplied={dust.aiApplied}
                  autoDustEnabled={dust.autoDust.enabled} autoDustSensitivity={dust.autoDust.sensitivity}
                  showSpots={dust.showSpots} autoSpots={$activeAutodustSpots}
                  autoExclusions={dust.autoDustExclusions} selectedSpot={$selectedSpot}
                  pointPick={pickTarget !== ""}
                  clipHigh={$clipWarn.high} clipLow={$clipWarn.low} clipStrict={$clipWarn.strict}
                  on:stroke={(e) => commitStroke(e.detail)} on:brush={(e) => (brush = e.detail)}
                  on:aierased={() => (aiBusy = false)}
                  on:autodusted={() => (autoBusy = false)}
                  on:zoomchange={(e) => (viewZoomed = e.detail)}
                  on:marqueedone={() => (zoomMarquee = false)}
                  on:selectspot={(e) => selectedSpot.set(e.detail)}
                  on:removespot={(e) => removeSpot(e.detail)}
                  on:pointpick={onPointPick} />
        {#if $baseSampling}
          <div class="picker-overlay">
            <BaseView id={$activeId} params={effParams} imgW={origW} imgH={origH}
                      on:sampled={(e) => sampledBase.set(e.detail)} />
          </div>
        {/if}
      {/if}
    {:else}<div class="hint">{$t('develop.notDevelopedYet')}</div>{/if}
  </section>

  <aside class="right editzone">
    <GlassPanel>
      <Histogram />
      <Toolbar />
      {#key $tool}
        <div class="toolpane" in:fade={{ duration: 160, easing: cubicOut }}>
          {#if $tool === "edit"}
            <!-- Workflow pages (upstream #22): edit panels grouped by editing order —
                 Basic (correction + exposure/tone) → Curves → Color — with "All"
                 keeping the classic stacked layout. Selection persists via prefs. -->
            <div class="pagestrip" role="tablist">
              {#each ["all", "basic", "curves", "color"] as pg}
                <button class="page" class:on={$editPage === pg} role="tab"
                        aria-selected={$editPage === pg}
                        on:click={() => editPage.set(pg as EditPage)}
                        >{$t(`develop.page.${pg}`)}</button>
              {/each}
            </div>
            {#if $editPage === "all" || $editPage === "basic"}
              <Basic onWbPick={toggleWbPick} wbPicking={pickTarget === "wb"} imageCrop={imageCrop}
                     geom={{ rot90: cRot, flip_h: committed?.flipH ?? false, flip_v: committed?.flipV ?? false, angle: committed?.angle ?? 0 }} />
            {/if}
            {#if $editPage === "all" || $editPage === "curves"}
              <TonalCurve />
            {/if}
            {#if $editPage === "all" || $editPage === "color"}
              <ColorGrading />
              <ColorMixer onPick={togglePcPick} picking={pickTarget === "pc"} />
            {/if}
          {:else if $tool === "crop"}
            <CropPanel bind:aspect bind:orientation bind:angle
                       on:preset={(e) => onPreset(e.detail)} on:swap={onSwap} on:reset={onReset}
                       on:rotate={(e) => onRotate(e.detail)} on:flip={(e) => onFlip(e.detail)} />
          {:else if $tool === "eraser"}
            <EraserPanel bind:brush {hasIr} zoomed={viewZoomed} marqueeArmed={zoomMarquee}
                         irEnabled={dust.irRemoval.enabled} irSensitivity={dust.irRemoval.sensitivity}
                         brushMigan={dust.brushMigan} aiApplied={dust.aiApplied}
                         strokeCount={dust.strokes.length} aiBusy={aiBusy}
                         showSpots={dust.showSpots}
                         on:reset={resetDustEdits}
                         on:irEnabled={(e) => setIrOn(e.detail)}
                         on:irSensitivity={(e) => setIrSens(e.detail)}
                         on:brushMigan={(e) => setBrushAi(e.detail)}
                         on:aiErase={aiErase}
                         on:zoomArea={() => (zoomMarquee = true)}
                         on:resetView={() => { vp?.resetZoom(); zoomMarquee = false; }}
                         on:showSpots={(e) => setShowSpotsEdit(e.detail)} />
            <AutoDustPanel id={$activeId}
                           enabled={dust.autoDust.enabled}
                           busy={autoBusy}
                           sensitivity={dust.autoDust.sensitivity}
                           on:toggle={(e) => setAutoOn(e.detail)}
                           on:sensitivity={(e) => setAutoSens(e.detail)} />
          {:else if $tool === "enhance"}
            <AiEnhancePanel effParams={effParams} imageCrop={imageCrop}
                            geom={{ rot90: cRot, flip_h: committed?.flipH ?? false, flip_v: committed?.flipV ?? false, angle: committed?.angle ?? 0 }} />
          {/if}
        </div>
      {/key}
    </GlassPanel>
    <!-- Top/bottom shadow gradients pinned to the panel edges so scrolled content
         fades out at the boundaries (infinity-scroll feel). -->
    <div class="edge-fade top" aria-hidden="true"></div>
    <div class="edge-fade bottom" aria-hidden="true"></div>
  </aside>

  <footer class="bottom"><Filmstrip /></footer>
</div>
{#if menu}<QualityMenu x={menu.x} y={menu.y} showFlip={deleteSelectionIds().length === 1} showReveal={true}
  on:flipH={() => { flipCommitted("h"); menu = null; }}
  on:flipV={() => { flipCommitted("v"); menu = null; }}
  on:reveal={() => { revealImage($activeId); menu = null; }}
  on:delete={() => { const ids = deleteSelectionIds(); if (ids.length) deleteTarget.set(ids); menu = null; }}
  on:close={() => (menu = null)} />{/if}
{#if thumbMenu}<ImageContextMenu x={thumbMenu.x} y={thumbMenu.y} count={deleteSelectionIds().length}
  showFlip={deleteSelectionIds().length === 1} showReveal={true}
  on:flipH={() => { flipCommitted("h"); thumbMenu = null; }}
  on:flipV={() => { flipCommitted("v"); thumbMenu = null; }}
  on:reveal={() => { if (thumbMenu) revealImage(thumbMenu.id); thumbMenu = null; }}
  on:delete={() => { const ids = deleteSelectionIds(); if (ids.length) deleteTarget.set(ids); thumbMenu = null; }}
  on:close={() => (thumbMenu = null)} />{/if}

<style>
  .layout { display: grid; height: 100%; gap: 12px;
    grid-template-columns: 1fr 300px; grid-template-rows: 1fr 88px;
    grid-template-areas: "center right" "bottom right"; }
  /* Workflow page strip (#22): compact tabs over the edit panels. */
  .pagestrip { display: flex; gap: 4px; padding: 8px 10px 0; }
  .page { flex: 1; background: transparent; border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 3px 0; font-size: 11px; cursor: pointer; }
  .page.on { color: var(--text); border-color: rgba(244,157,78,0.5); background: rgba(244,157,78,0.12); }
  .right { grid-area: right; min-height: 0; position: relative; overflow-y: auto;
    scrollbar-width: none; -ms-overflow-style: none; }
  .right::-webkit-scrollbar { width: 0; height: 0; }
  /* Blend the panel into the page: match the body background (#111111), drop the
     floating drop-shadow and the now-pointless backdrop blur, and hide its
     scrollbar. Keep the inset top highlight that defines the glass edge. */
  .right :global(.glass) { background: #111111; box-shadow: inset 0 1px 0 var(--glass-hi);
    backdrop-filter: none; -webkit-backdrop-filter: none;
    scrollbar-width: none; -ms-overflow-style: none; }
  .right :global(.glass)::-webkit-scrollbar { width: 0; height: 0; }
  /* Shadow gradients at the panel's top/bottom edges. Inset 1px to sit inside the
     GlassPanel border and rounded to match its corners; non-interactive. */
  .edge-fade { position: absolute; left: 1px; right: 1px; height: 26px;
    pointer-events: none; z-index: 3; }
  .edge-fade.top { top: 1px; border-radius: var(--radius) var(--radius) 0 0;
    background: linear-gradient(to bottom, rgba(0,0,0,0.55), rgba(0,0,0,0)); }
  .edge-fade.bottom { bottom: 1px; border-radius: 0 0 var(--radius) var(--radius);
    background: linear-gradient(to top, rgba(0,0,0,0.55), rgba(0,0,0,0)); }
  .center { grid-area: center; min-height: 0; display: grid; place-items: center; position: relative; }
  /* Film-base picker overlay: covers the still-mounted Viewport (opaque so it doesn't
     bleed through) while keeping the Viewport's GPU texture alive for an instant dismiss. */
  .picker-overlay { position: absolute; inset: 0; z-index: 6; background: #111111; }
  /* Cold-start thumbnail placeholder: sits behind the Viewport, fit to the same padded
     box so the first real frame paints over it without a jump. */
  .cold-thumb { position: absolute; inset: 60px; width: calc(100% - 120px); height: calc(100% - 120px);
    object-fit: contain; z-index: 0; pointer-events: none; border-radius: 10px; }
  .hint { color: var(--text-dim); }
  .bottom { grid-area: bottom; min-width: 0; overflow: hidden; }
</style>
