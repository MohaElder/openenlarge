<!-- app/src/lib/tabs/Roll.svelte -->
<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { fade } from "svelte/transition";
  import { get } from "svelte/store";
  import { t } from "$lib/i18n";
  import { developedFolderImages } from "$lib/export/eligible";
  import { editsById, cropById, images, activeId, setActive, deleteTarget, selectedFolder, folderImages } from "$lib/store";
  import { rollReferenceId, enterRollDraft, rollDraft, rollDraftTouched, rollApplied } from "$lib/roll/draft";
  import RollAdjust from "$lib/roll/RollAdjust.svelte";
  import { applyBaseToAll, applyWhitePointToAll, applyCropToAll, applyRollDelta, rollPreviewFrame } from "$lib/roll/apply";
  import { draftThumbView } from "$lib/roll/livePreview";
  import { withEffectiveBase, setFolderBase } from "$lib/develop/base";
  import { reseedRollProtectedFree } from "$lib/roll/rollBase";
  import { imageDir } from "$lib/library/folderScope";
  import { api, defaultParams } from "$lib/api";
  import { debounce } from "$lib/catalog";
  import { rollFilmEdge, rollEdgeText } from "$lib/store";
  import FramePreview from "$lib/roll/FramePreview.svelte";
  import BaseView from "$lib/develop/BaseView.svelte";
  import { exportContactSheet, type ExportSheetOpts } from "$lib/roll/exportSheet";
  import ExportSheetDialog from "$lib/overlay/ExportSheetDialog.svelte";
  import { pickTileAspect } from "$lib/roll/contactSheet";
  import Viewport from "$lib/viewport/Viewport.svelte";
  import CropView from "$lib/crop/CropView.svelte";
  import CropPanel from "$lib/crop/CropPanel.svelte";
  import { orientDims, rotateRectCW, rotateRectCCW, flipRectH, flipRectV, flipOrient } from "$lib/crop/transforms";
  import { defaultFull, conform, constrainToRotated } from "$lib/crop/cropMath";
  import { presetNormAspect } from "$lib/crop/presets";
  import type { Rect, CropRect } from "$lib/crop/types";
  import { emptyDust } from "$lib/develop/dust";
  import { markThumbsStale } from "$lib/develop/thumbRegen";
  import { proofMode, proofRev, proofOverlay, ensureProofSolves, proofEnterFolder, proofInvalidate, type ProofMode } from "$lib/roll/proof";
  import { developRev, dustById } from "$lib/store";
  import Icon from "$lib/icons/Icon.svelte";
  import QualityMenu from "$lib/viewport/QualityMenu.svelte";

  // Fresh roll draft each time the section opens (seed from defaults per spec).
  onMount(() => {
    // Keep the roll-wide slider values across re-entry to the same roll (start fresh on a
    // folder change); inert until the user edits, so it never re-applies/reverts per-frame edits.
    enterRollDraft(get(selectedFolder));
    proofEnterFolder(get(selectedFolder));
    // No entry overwrite-confirm: roll slider edits apply RELATIVELY (applyRollDelta — each
    // scalar is a delta on top of the frame's own value, preserving per-frame tweaks), so
    // entering a roll with already-edited frames never silently replaces them. Per-frame
    // tunes still surface in the contact-sheet preview without moving the roll sliders.
  });

  function openFrame(id: string) {
    setActive(id);
    rollReferenceId.set(id);
  }

  // --- Film-strip contact sheet helpers -------------------------------------
  const STRIP_SIZE = 6;

  // Tile aspect adapts to the roll's actual frame shape (one camera → one aspect),
  // so landscape frames fill their tile edge-to-edge with no gaps. Computed from
  // each frame's effective dims (metadata + rot90 + crop rect).
  $: tileAspect = (() => {
    const crops = $cropById;
    const aspects = $developedFolderImages.map((img) => {
      const w = img.metadata?.width ?? 0;
      const h = img.metadata?.height ?? 0;
      if (!w || !h) return 0;
      const c = crops[img.id] ?? null;
      const [ow, oh] = orientDims(w, h, c?.rot90 ?? 0);
      const rw = c ? c.rect.w : 1;
      const rh = c ? c.rect.h : 1;
      return oh * rh > 0 ? (ow * rw) / (oh * rh) : 0;
    });
    return pickTileAspect(aspects);
  })();

  // Chunk images into strips of STRIP_SIZE, computing flat sequential frame numbers.
  $: strips = (() => {
    const imgs = $developedFolderImages;
    const result: { frames: { img: typeof imgs[0]; num: string; idx: number }[]; padCount: number }[] = [];
    for (let i = 0; i < imgs.length; i += STRIP_SIZE) {
      const slice = imgs.slice(i, i + STRIP_SIZE);
      const frames = slice.map((img, j) => ({
        img,
        num: String(i + j + 1).padStart(2, "0"),
        idx: i + j,
      }));
      result.push({ frames, padCount: STRIP_SIZE - frames.length });
    }
    return result;
  })();

  // The film-edge marking is edited via the labeled field in the toolbar (so it's
  // clearly editable and applies to the whole roll); the strip renders it read-only.
  // Each strip's bottom rebate shows the marking REPEATS times, evenly distributed.
  const EDGE_REPEATS = 3;

  // --- Live apply -------------------------------------------------------------
  // Component-local map of id → data-URL for draft-look thumbnails (shown in the grid).
  let previewMap: Record<string, string> = {};
  // Monotonically increasing token; used to discard stale async batches.
  let previewToken = 0;

  const editsEntry = (id: string) => get(editsById)[id] ?? defaultParams();

  let destroyed = false;
  onDestroy(() => {
    schedulePersist.flush();   // flush while destroyed is still false, so the final look persists
    destroyed = true;
    if (previewRafHandle !== null) {
      cancelAnimationFrame(previewRafHandle);
      previewRafHandle = null;
    }
  });

  // --- Tiny concurrency pool: run `fns` with at most `concurrency` in-flight at once.
  async function pooled<T>(fns: (() => Promise<T>)[], concurrency: number): Promise<T[]> {
    const results: T[] = new Array(fns.length);
    let next = 0;
    async function worker() {
      while (next < fns.length) {
        const i = next++;
        results[i] = await fns[i]();
      }
    }
    const workers = Array.from({ length: Math.min(concurrency, fns.length) }, worker);
    await Promise.all(workers);
    return results;
  }

  // --- PASS 1: PREVIEW — cheap, frequent, display-only (120 ms debounce) --------
  // Renders draft thumbnails into previewMap. Does NOT write editsById or saveThumbnail.

  // Single-in-flight guard: only one preview batch runs at a time.
  let previewRunning = false;
  let previewPending: typeof $rollDraft | null = null;
  // rAF handle for the coalescing loop (cancelled on batch end / component destroy).
  let previewRafHandle: number | null = null;

  async function runPreviewBatch(draft: typeof $rollDraft): Promise<void> {
    previewRunning = true;
    try {
      const token = ++previewToken;
      const frames = get(developedFolderImages);
      // Read touched flag once at batch start — consistent across all frames in this batch.
      const touched = get(rollDraftTouched);
      // Proof layer (#18) read once per batch: when on, each frame's OWN params are
      // overlaid with its solved auto exposure / auto color BEFORE the roll delta,
      // so roll sliders offset the auto baselines. Display-only — never persisted.
      const pm = get(proofMode);

      // Accumulator: start from current previewMap so unrendered frames keep their old preview.
      const acc: Record<string, string> = { ...previewMap };
      let dirty = false;

      // Start a single rAF coalescing loop — commits previewMap at most ~60/sec.
      if (previewRafHandle !== null) cancelAnimationFrame(previewRafHandle);
      if (typeof requestAnimationFrame !== "undefined") {
        const rafLoop = () => {
          if (dirty && previewToken === token && !destroyed) {
            previewMap = { ...acc };
            dirty = false;
          }
          // Keep looping until this batch is superseded or the component is destroyed.
          if (previewToken === token && !destroyed) {
            previewRafHandle = requestAnimationFrame(rafLoop);
          } else {
            previewRafHandle = null;
          }
        };
        previewRafHandle = requestAnimationFrame(rafLoop);
      }

      // Build render tasks in folder order so the first (visible) frames update first.
      const tasks = frames.map((frame) => async () => {
        const own = proofOverlay(frame.id, editsEntry(frame.id), pm);
        // When untouched: render each frame with its own stored edits + its own stored crop
        // (the roll as-is — no revert, no reset). When touched: apply the draft look to all,
        // and use the draft crop if set (else fall back to each frame's own crop).
        const baseParams = touched
          ? {
              // Relative roll adjust: scalar sliders (incl. exposure) offset each frame's own
              // value (preserve per-frame diffs); the structured look broadcasts absolute.
              ...rollPreviewFrame(own, draft.params, get(rollApplied)),
              // Base / white-point stay absolute roll-wide overrides when the user set one.
              ...(draft.params.base_override != null ? { base_override: draft.params.base_override } : {}),
              ...(draft.params.d_max_override != null ? { d_max_override: draft.params.d_max_override } : {}),
            }
          : own;
        const params = withEffectiveBase(baseParams, imageDir(frame));
        // When touched: use the draft crop if set, else fall back to the frame's own crop.
        // When untouched: always use the frame's own stored crop (no draft applied).
        const frameCrop = touched
          ? (draft.crop != null ? draft.crop : (get(cropById)[frame.id] ?? null))
          : (get(cropById)[frame.id] ?? null);
        const view = draftThumbView(frameCrop);
        const dataUrl = await api.thumbnail(frame.id, params, view);
        // Write into accumulator (plain property set — no Svelte reactivity triggered here).
        if (previewToken === token && !destroyed) {
          acc[frame.id] = dataUrl;
          dirty = true;
        }
        return { id: frame.id, dataUrl };
      });

      // Limit to 5 concurrent backend renders.
      await pooled(tasks, 5);

      // Batch complete: cancel the rAF loop and do one final commit (covers SSR/test envs too).
      if (previewRafHandle !== null) {
        cancelAnimationFrame(previewRafHandle);
        previewRafHandle = null;
      }
      if (previewToken === token && !destroyed) {
        previewMap = { ...acc };
      }
    } finally {
      previewRunning = false;
      // If a newer draft arrived while we were running, render it now.
      if (previewPending !== null) {
        const next = previewPending;
        previewPending = null;
        runPreviewBatch(next);
      }
    }
  }

  const schedulePreview = debounce((draft: typeof $rollDraft) => {
    if (!get(rollDraftTouched)) return; // entry/re-entry: show cached thumbnails, don't re-render
    if (previewRunning) {
      // Queue the latest draft; an in-flight batch will pick it up on completion.
      previewPending = draft;
      return;
    }
    runPreviewBatch(draft);
  }, 120);

  // --- PASS 2: PERSIST — heavy, deferred, fires once after drag settles (600 ms) --
  // Writes editsById (triggers catalog write-through) + saveThumbnail for each frame.
  // Reuses whatever is already in previewMap — does NOT re-render.
  let persistToken = 0;
  const schedulePersist = debounce(async (draft: typeof $rollDraft) => {
    const token = ++persistToken;
    if (destroyed) return;
    // Guard: if the user hasn't touched any control yet (fresh/re-entry), don't write
    // anything — prevents resetting each frame's look/crop on every Develop tab visit.
    if (!get(rollDraftTouched)) return;

    const frames = get(developedFolderImages);
    const ids = frames.map((f) => f.id);

    // Compute the next edits. Scalar sliders (incl. exposure) fold in RELATIVELY — the delta
    // since the last applied offset, on top of each frame's own value — so per-frame
    // differences survive. The structured look broadcasts absolute.
    const applied = get(rollApplied);
    let nextEdits = applyRollDelta(get(editsById), ids, draft.params, applied);
    // Base / white-point stay absolute roll-wide overrides when set (null-guarded).
    if (draft.params.base_override != null) nextEdits = applyBaseToAll(nextEdits, ids, draft.params.base_override);
    if (draft.params.d_max_override != null) nextEdits = applyWhitePointToAll(nextEdits, ids, draft.params.d_max_override);

    // Guard: discard stale batch BEFORE writing stores so a stale run doesn't clobber newer state.
    if (persistToken !== token || destroyed) return;

    // Write the folded look into every frame's edits (persisted via write-through), then
    // advance the applied offset so the same draft won't re-apply (idempotent, no compounding).
    editsById.set(nextEdits);
    rollApplied.set(JSON.parse(JSON.stringify(draft.params)));

    // Persist crop to all frames when the draft has a crop set (null-guarded).
    if (draft.crop != null) cropById.set(applyCropToAll(get(cropById), ids, draft.crop));

    // Persist thumbnail for each frame — reuse previewMap renders (no re-render); for
    // any frame without a preview yet, mark it stale so the shared worker rebakes it.
    // While the proof layer (#18) is on, previewMap renders carry the DISPLAY-ONLY
    // auto overlays — persisting them would bake a look that isn't in editsById — so
    // every frame takes the stale path and the worker rebakes from the stored edits.
    const proofOn = get(proofMode).on;
    const snap = previewMap;
    const missing: string[] = [];
    for (const id of ids) {
      const url = snap[id];
      if (url && !proofOn) {
        images.update((xs) => xs.map((i) => i.id === id ? { ...i, thumbnail: url, thumb_stale: false } : i));
        api.saveThumbnail(id, url);
      } else {
        missing.push(id);
      }
    }
    if (missing.length) markThumbsStale(missing, { persist: true });
  }, 600);

  // Mirror every rollDraft change (tone/color/base/dmax/crop) to all frames. Inert until
  // the user actually edits a roll slider (schedulePersist no-ops while !rollDraftTouched),
  // so simply opening the roll never re-applies or reverts existing per-frame edits.
  $: { schedulePreview($rollDraft); schedulePersist($rollDraft); }

  // Proof layer (#18): re-render the sheet whenever the toggle state changes or new
  // solves land (proofRev). Renders immediately with whatever is cached (progressive:
  // unsolved frames keep their stored look), then solves the missing frames in the
  // background — the rev bump re-triggers this block for the final render. The
  // `lastProofKey` guard keeps plain Roll entry render-free (139762e) unless the
  // proof toggle was left on, in which case restoring its look on entry is the point.
  let lastProofKey = "";
  $: {
    const key = JSON.stringify([$proofMode, $proofRev]);
    if (key !== lastProofKey) {
      const first = lastProofKey === "";
      lastProofKey = key;
      if (!first || $proofMode.on) onProofChange($proofMode);
    }
  }
  function onProofChange(pm: ProofMode) {
    if (destroyed) return;
    runPreviewBatch(get(rollDraft));
    if (pm.on) void ensureProofSolves(get(developedFolderImages), editsEntry);
  }

  // --- Reference-edit mode (crop / base) ---------------------------------------
  type EditMode = "none" | "crop" | "base";
  let editMode: EditMode = "none";
  let refId: string | null = null;

  // Reference frame metadata (for the crop state machine).
  $: refFrame = refId ? $images.find((i) => i.id === refId) ?? null : null;
  $: origW = refFrame?.metadata.width ?? 0;
  $: origH = refFrame?.metadata.height ?? 0;
  $: refDir = refFrame ? imageDir(refFrame) : "";

  // Reference frame's own committed crop (for the Viewport view).
  // In crop mode we show the draft crop, not the committed one.
  $: refCommitted = refId ? ($cropById[refId] ?? null) : null;
  $: cRot = editMode === "crop" ? rot90 : (refCommitted?.rot90 ?? 0);
  $: [coW, coH] = orientDims(origW, origH, cRot);
  $: effW = editMode === "crop"
    ? coW
    : (refCommitted ? Math.max(1, Math.round(refCommitted.rect.w * coW)) : coW);
  $: effH = editMode === "crop"
    ? coH
    : (refCommitted ? Math.max(1, Math.round(refCommitted.rect.h * coH)) : coH);
  $: imageCrop = (editMode !== "crop" && refCommitted)
    ? ([refCommitted.rect.x, refCommitted.rect.y, refCommitted.rect.w, refCommitted.rect.h] as [number, number, number, number])
    : null;

  // Reference frame render params: use its own stored edits + effective base.
  $: refFrameParams = refId ? ($editsById[refId] ?? defaultParams()) : defaultParams();
  $: refEffParams = withEffectiveBase(refFrameParams, refDir);

  // Reference frame dust edits (view-only in crop mode).
  $: refDust = refId ? ($dustById[refId] ?? emptyDust()) : emptyDust();

  // --- Crop state machine (ported from Develop.svelte lines 88-145) -----------
  let rect: Rect = { x: 0, y: 0, w: 1, h: 1 };
  let aspect = "original";
  let orientation: "landscape" | "portrait" = "landscape";
  let rot90 = 0, flipH = false, flipV = false, angle = 0;
  let cropInit = false;

  $: [oW, oH] = orientDims(origW, origH, rot90);
  $: orientedRatio = oH > 0 ? oW / oH : 1;
  $: lockRatio = presetNormAspect(aspect, orientedRatio, orientation);
  // Keep the crop inside the rotated image (constrainToRotated is idempotent → no loop).
  $: if (angle !== 0) rect = constrainToRotated(rect, angle, oW, oH);

  function startCrop() {
    // Seed from the draft crop if a roll crop is already in progress, else from the
    // reference frame's COMMITTED crop (so orientation/flip/rotation + the crop box
    // persist when re-entering crop), else full frame. (refCommitted is stale here
    // because enterCropMode sets refId synchronously just before calling this.)
    const c = $rollDraft.crop ?? (refId ? get(cropById)[refId] ?? null : null);
    if (c) {
      rect = { ...c.rect }; aspect = c.aspect; orientation = c.orientation;
      rot90 = c.rot90; flipH = c.flipH; flipV = c.flipV; angle = c.angle;
    } else {
      rect = defaultFull(); aspect = "original";
      orientation = origW >= origH ? "landscape" : "portrait";
      rot90 = 0; flipH = false; flipV = false; angle = 0;
    }
    cropInit = true;
  }

  function draftCrop(): CropRect {
    return { rect, aspect, orientation, rot90: rot90 as 0 | 1 | 2 | 3, flipH, flipV, angle };
  }

  function commitCropToDraft() {
    if (!cropInit) return;
    rollDraft.update((d) => ({ ...d, crop: draftCrop() }));
    rollDraftTouched.set(true);
    // The live-apply mirror ($: if mirrorEnabled) will pick this up and call
    // applyCropToAll on the next scheduleLiveApply tick.
  }

  function onPreset(presetId: string) {
    aspect = presetId;
    rect = conform(rect, presetNormAspect(presetId, orientedRatio, orientation));
  }
  function onSwap() {
    orientation = orientation === "landscape" ? "portrait" : "landscape";
    rect = conform(rect, presetNormAspect(aspect, orientedRatio, orientation));
  }
  function onReset() {
    rect = defaultFull(); aspect = "original";
    orientation = origW >= origH ? "landscape" : "portrait";
    rot90 = 0; flipH = false; flipV = false; angle = 0;
  }
  function onRotate(dir: number) {
    if (dir > 0) { rot90 = (rot90 + 1) % 4; rect = rotateRectCW(rect); }
    else { rot90 = (rot90 + 3) % 4; rect = rotateRectCCW(rect); }
  }
  function onFlip(axis: "h" | "v") {
    ({ rot90, flipH, flipV } = flipOrient({ rot90, flipH, flipV }, axis));
    rect = axis === "h" ? flipRectH(rect) : flipRectV(rect);
    angle = -angle;
  }
  function onStraighten(v: number) { angle = Math.max(-45, Math.min(45, v)); }

  function enterCropMode() {
    const id = get(activeId) ?? $developedFolderImages[0]?.id ?? null;
    if (!id) return;
    refId = id;
    setActive(id);
    editMode = "crop";
    startCrop();
  }

  // Film Base tool: pick a clean rebate on the reference frame to recalibrate the
  // WHOLE roll — sets the folder's stored base + reseeds every protected-free frame
  // (see onBaseSampled). The same BaseView picker backs it.
  function enterBaseMode() {
    const id = get(activeId) ?? $developedFolderImages[0]?.id ?? null;
    if (!id) return;
    refId = id;
    setActive(id);
    editMode = "base";
  }

  async function onBaseSampled(e: CustomEvent<[number, number, number]>) {
    // Whole-roll recalibrate: BaseView already sampled the rebate via sampleBaseAt,
    // so e.detail IS the measured base. Set it as the roll/folder default and reseed
    // every protected-free frame (frames with base_override / wb_manual are skipped).
    const dir = get(selectedFolder);
    const base = e.detail;
    exitEditMode();
    if (!dir) return;
    setFolderBase(dir, base);
    // The proof solves (#18) were metered against the old base — drop them so the
    // proof layer re-solves against the recalibrated roll.
    proofInvalidate();
    await reseedRollProtectedFree(get(folderImages));
    // Base sampling changes editsById + folderBaseByPath but NOT rollDraft, so the
    // reactive preview trigger ($: schedulePreview($rollDraft)) never fires and the
    // contact-sheet cells keep showing stale previewMap entries (which shadow the freshly
    // baked thumbnails). Rebuild the preview explicitly so colors update immediately —
    // without this you'd only see the new base after a crop nudged rollDraft.
    runPreviewBatch(get(rollDraft));
  }


  /** Apply the current crop draft to all frames, then leave crop mode.
   *  If no manual WP pick has been made, re-derives D-max from the reference
   *  frame's new crop and applies that one value roll-wide. */
  async function applyCropAndExit() {
    commitCropToDraft();
    // Capture ref info before clearing refId.
    const id = refId;
    const crop = get(rollDraft).crop;
    const frame = id ? get(images).find((i) => i.id === id) : null;
    const dir = frame ? imageDir(frame) : "";
    // Exit crop mode immediately so the UI returns to the sheet.
    cropInit = false;
    editMode = "none";
    refId = null;
    // Re-analyze D-max from the new crop.
    if (id && crop) {
      try {
        const imageCrop: [number, number, number, number] = [crop.rect.x, crop.rect.y, crop.rect.w, crop.rect.h];
        const geom = { rot90: crop.rot90, flip_h: crop.flipH, flip_v: crop.flipV, angle: crop.angle };
        const { d_max } = await api.analyze(id, withEffectiveBase(get(rollDraft).params, dir), imageCrop, geom);
        rollDraft.update((d) => ({ ...d, params: { ...d.params, d_max_override: d_max } }));
        rollDraftTouched.set(true);
      } catch { /* not developed / analyze failed — leave d_max as-is */ }
    }
    // Auto exposure/WB are metered inside the crop — the proof solves (#18) are
    // stale the moment the roll crop moves.
    proofInvalidate();
  }

  /** Leave crop mode WITHOUT committing — prior roll crop is preserved. */
  function discardCropAndExit() {
    cropInit = false;
    editMode = "none";
    refId = null;
  }

  function exitEditMode() {
    // base: draft writes already mirrored live — nothing extra to commit.
    // NOTE: do NOT call this from crop mode — use applyCropAndExit or discardCropAndExit.
    editMode = "none";
    refId = null;
  }

  function onStripClick(id: string) {
    refId = id;
    setActive(id);
    // Re-seed crop with same draft (don't reset — the crop is roll-wide).
  }

  /** True while a form control has focus (INPUT/TEXTAREA/SELECT), so typing in the
   *  straighten field or edge-text input isn't hijacked by crop shortcuts. */
  function formFocused(): boolean {
    const a = document.activeElement;
    const tag = a?.tagName;
    return tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA";
  }

  /** Context menu state for the crop-mode right-click flip menu. */
  let menu: { x: number; y: number } | null = null;

  function onCropContextMenu(e: MouseEvent) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY };
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (editMode === "crop") {
      const meta = e.metaKey || e.ctrlKey;
      if (e.key === "Enter" && !formFocused()) {
        e.preventDefault();
        applyCropAndExit();
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        discardCropAndExit();
        return;
      }
      if ((e.key === "x" || e.key === "X") && !formFocused() && !meta) {
        e.preventDefault();
        onSwap();
        return;
      }
      if (meta && (e.key === "]" || e.key === "[")) {
        e.preventDefault();
        onRotate(e.key === "]" ? 1 : -1);
        return;
      }
    } else if (e.key === "Escape" && editMode !== "none") {
      exitEditMode();
    }
  }

  // Contact-sheet export: open the resolution/format dialog, then render on confirm.
  let exportOpen = false;
  function onExportConfirm(opts: ExportSheetOpts) {
    exportOpen = false;
    void exportContactSheet(opts);
  }

</script>

<svelte:window on:keydown={onWindowKeydown} />

{#if editMode === "none"}
  <!-- ===== Default contact-sheet view ===== -->
  <div class="roll">
    <div class="sheet-col">
      <div class="sheet-toolbar">
        <!-- Toggle + export grouped: the bottom stroke spans only this group (to
             the end of the export button), with a gap before the right panel. -->
        <div class="toolbar-actions">
          <!-- Editable film-edge marking for the whole roll -->
          {#if $rollFilmEdge}
            <label class="edge-field">
              <span class="edge-field-label">{$t('roll.edgeText')}</span>
              <input
                type="text"
                value={$rollEdgeText}
                on:input={(e) => rollEdgeText.set(e.currentTarget.value)}
                placeholder={$t('roll.edgeTextPlaceholder')}
                aria-label={$t('roll.edgeText')}
              />
            </label>
          {/if}
          <!-- Film edge toggle (left of export button) -->
          <button class="film-edge-toggle" on:click={() => rollFilmEdge.update(b => !b)}
                  aria-label={$t('roll.filmEdge')} aria-pressed={$rollFilmEdge}>
            <span class="film-edge-label">{$t('roll.filmEdge')}</span>
            <span class="pill" class:pill-on={$rollFilmEdge}>
              <span class="knob"></span>
            </span>
          </button>
          <button class="export-btn" on:click={() => (exportOpen = true)} disabled={$developedFolderImages.length === 0}>
            {$t('roll.export.button')}
          </button>
        </div>
      </div>
      <div class="sheet">
      {#if $developedFolderImages.length === 0}
        <div class="empty">{$t('roll.empty')}</div>
      {:else}
        {#key $rollFilmEdge}
          <div class="sheet-anim" in:fade={{ duration: 180 }}>
            <div class="strips-container" style="--tile-aspect: {tileAspect}">
              {#if $rollFilmEdge}
                <!-- ===== FILM-EDGE ON: filmstrip with rebates ===== -->
                {#each strips as strip, stripIndex}
                  <div class="filmstrip-strip">
                    <!-- Top rebate -->
                    <div class="rebate rebate-top">
                      <div class="sprocket-holes"></div>
                      <div class="frame-numbers">
                        {#each strip.frames as f}
                          <div class="frame-num">{f.num}</div>
                        {/each}
                        {#each Array(strip.padCount) as _}
                          <div class="frame-num frame-pad"></div>
                        {/each}
                      </div>
                    </div>
                    <!-- Frames row -->
                    <div class="frames-row">
                      {#each strip.frames as f}
                        <button class="frame-cell" data-id={f.img.id}
                                on:click={() => openFrame(f.img.id)}>
                          <img src={previewMap[f.img.id] ?? f.img.thumbnail}
                               alt={f.img.file_name} draggable="false" />
                        </button>
                      {/each}
                      {#each Array(strip.padCount) as _}
                        <div class="frame-cell frame-cell-pad"></div>
                      {/each}
                    </div>
                    <!-- Bottom rebate -->
                    <div class="rebate rebate-bottom">
                      <div class="rebate-info-row">
                        <div class="barcode"></div>
                        <div class="edge-track">
                          {#each Array(EDGE_REPEATS) as _}
                            <span class="edge-text">{$rollEdgeText}</span>
                          {/each}
                        </div>
                        <span class="edge-arrow">→</span>
                      </div>
                      <div class="sprocket-holes"></div>
                    </div>
                  </div>
                {/each}
              {:else}
                <!-- ===== FILM-EDGE OFF: proof grid ===== -->
                {#each strips as strip}
                  <div class="proof-strip">
                    {#each strip.frames as f}
                      <div class="proof-cell">
                        <button class="proof-frame" data-id={f.img.id}
                                on:click={() => openFrame(f.img.id)}>
                          <img src={previewMap[f.img.id] ?? f.img.thumbnail}
                               alt={f.img.file_name} draggable="false" />
                        </button>
                        <div class="proof-caption">{f.num}</div>
                      </div>
                    {/each}
                    {#each Array(strip.padCount) as _}
                      <div class="proof-cell proof-cell-pad"></div>
                    {/each}
                  </div>
                {/each}
              {/if}
            </div>
          </div>
        {/key}
      {/if}
      </div>
    </div>

    <aside class="panel">
      <RollAdjust>
        <!-- Change A+D: tool row rendered in slot between heading and sliders -->
        <div class="tool-row">
          <!-- Film Base comes before Crop: calibrate the base first (no rotation), then
               crop+rotate last so re-entering an un-rotated base view never disorients. -->
          <div class="tool">
            <button class="tool-btn" class:on={(editMode as string) === "base"}
                    on:click={enterBaseMode} disabled={$developedFolderImages.length === 0}
                    aria-label={$t('roll.base.heading')}>
              <Icon name="droplet" size={20} />
            </button>
            <span class="tool-label">{$t('roll.base.heading')}</span>
          </div>
          <div class="tool">
            <button class="tool-btn" on:click={enterCropMode} disabled={$developedFolderImages.length === 0}
                    aria-label={$t('roll.crop.tool')}>
              <Icon name="crop" size={20} />
            </button>
            <span class="tool-label">{$t('roll.crop.tool')}</span>
          </div>
        </div>
      </RollAdjust>
    </aside>
  </div>

  {#if $rollReferenceId}
    <FramePreview />
  {/if}

{:else if editMode === "crop"}
  <!-- ===== Reference-edit crop layout ===== -->
  <div class="ref-layout">
    <!-- Center: CropView on the reference frame -->
    <div class="ref-center" on:contextmenu={onCropContextMenu}>
      {#if refFrame?.developed && refId}
        <CropView id={refId} params={refEffParams} imgW={oW} imgH={oH}
                  bind:rect {lockRatio} {rot90} {flipH} {flipV} {angle}
                  on:custom={() => (aspect = "custom")}
                  on:straighten={(e) => onStraighten(e.detail)} />
      {:else}
        <div class="ref-hint">{$t('develop.notDevelopedYet')}</div>
      {/if}
    </div>

    <!-- Right panel: CropPanel + Apply button -->
    <aside class="ref-panel">
      <div class="ref-panel-inner">
        <CropPanel bind:aspect bind:orientation bind:angle
                   on:preset={(e) => onPreset(e.detail)}
                   on:swap={onSwap}
                   on:reset={onReset}
                   on:rotate={(e) => onRotate(e.detail)}
                   on:flip={(e) => onFlip(e.detail)} />
        <button class="done-btn" on:click={applyCropAndExit}>
          {$t('roll.crop.tool')}
        </button>
      </div>
    </aside>

    <!-- Bottom strip: contact-sheet thumbnails (short) -->
    <div class="ref-strip">
      {#each $developedFolderImages as img (img.id)}
        <button class="strip-cell" class:strip-active={img.id === refId}
                data-id={img.id} on:click={() => onStripClick(img.id)}>
          <img src={previewMap[img.id] ?? img.thumbnail} alt={img.file_name} draggable="false" />
        </button>
      {/each}
    </div>
  </div>

{:else if editMode === "base"}
  <!-- ===== Reference-edit base layout ===== -->
  <div class="ref-layout">
    <!-- Center: BaseView fills the area — it owns all pointer events (no overlay, no z-index obstruction).
         This is the fix for feedback #2: the previous approach layered BaseView on top of a Viewport
         inside .base-overlay with position:absolute, risking z-index conflicts and click interception
         by the Viewport's own interactive layer. Here BaseView IS the center content, unobstructed. -->
    <div class="ref-center">
      {#if refFrame?.developed && refId}
        <BaseView id={refId} params={refEffParams} imgW={origW} imgH={origH}
                  on:sampled={onBaseSampled} />
      {:else}
        <div class="ref-hint">{$t('develop.notDevelopedYet')}</div>
      {/if}
    </div>

    <!-- Right panel: empty (picking auto-closes; press Escape to cancel) -->
    <aside class="ref-panel">
      <div class="ref-panel-inner"></div>
    </aside>

    <!-- Bottom strip -->
    <div class="ref-strip">
      {#each $developedFolderImages as img (img.id)}
        <button class="strip-cell" class:strip-active={img.id === refId}
                data-id={img.id} on:click={() => onStripClick(img.id)}>
          <img src={previewMap[img.id] ?? img.thumbnail} alt={img.file_name} draggable="false" />
        </button>
      {/each}
    </div>
  </div>

{/if}

{#if exportOpen}
  <ExportSheetDialog on:cancel={() => (exportOpen = false)}
                     on:confirm={(e) => onExportConfirm(e.detail)} />
{/if}

{#if menu && editMode === "crop"}
  <QualityMenu x={menu.x} y={menu.y} showFlip={true} showReveal={false}
    on:flipH={() => { onFlip("h"); menu = null; }}
    on:flipV={() => { onFlip("v"); menu = null; }}
    on:delete={() => { if (refId) deleteTarget.set([refId]); menu = null; }}
    on:close={() => (menu = null)} />
{/if}

<style>
  /* ===== Contact-sheet layout ===== */
  .roll { height: 100%; min-height: 0; display: grid; grid-template-columns: 1fr 320px; }
  .sheet-col { display: flex; flex-direction: column; min-height: 0; overflow: hidden; }
  .sheet-toolbar { display: flex; align-items: center; justify-content: flex-end;
    padding: 6px 8px; flex: none; background: transparent; }
  /* Toggle + export grouped at the right, with a gap before the panel. */
  .toolbar-actions { display: flex; align-items: center; gap: 10px; margin-right: 12px; }

  /* Editable film-edge marking field in the toolbar */
  .edge-field { display: flex; align-items: center; gap: 8px; margin-right: 4px; }
  .edge-field-label { font-size: 12px; color: var(--text-faint); white-space: nowrap;
    text-transform: uppercase; letter-spacing: .4px; }
  .edge-field input {
    width: 220px; padding: 6px 10px; border-radius: 8px;
    background: var(--glass-hi); border: 1px solid var(--glass-brd); color: var(--text);
    font: 600 12px 'Spline Sans Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    letter-spacing: .04em; outline: none; transition: border-color 0.15s, background 0.15s; }
  .edge-field input:focus { border-color: #cf9152; background: rgba(255,255,255,0.06); }
  .edge-field input::placeholder { color: var(--text-faint); letter-spacing: 0; }
  .sheet { flex: 1; overflow-y: auto; padding: 0; background: #111111; display: flex; flex-direction: column; }
  .empty { color: var(--text-faint); padding: 16px; }

  /* ===== Film-edge toggle ===== */
  .film-edge-toggle { display: flex; align-items: center; gap: 10px; cursor: pointer;
    background: transparent; border: none; padding: 0; margin-right: auto; }
  .film-edge-label { font-size: 13px; color: #b6b5b9; }
  .pill { position: relative; width: 38px; height: 21px; border-radius: 11px;
    background: #34343a; transition: background 0.2s; flex: none; }
  .pill-on { background: #cf9152; }
  .knob { position: absolute; top: 2px; width: 17px; height: 17px; border-radius: 50%;
    transition: left 0.2s ease, background 0.2s ease; }
  .pill-on .knob { left: 19px; background: #fff; }
  .pill:not(.pill-on) .knob { left: 2px; background: #a6a6ab; }

  /* ===== Strip container ===== */
  .strips-container { padding: 24px 26px 30px; display: flex; flex-direction: column; }

  /* ===== Filmstrip (film-edge ON) ===== */
  .filmstrip-strip { margin-bottom: 16px; }

  .rebate { background: #131210; }
  .rebate-top { border-radius: 1px 1px 0 0; }
  .rebate-bottom { border-radius: 0 0 1px 1px; }

  .sprocket-holes { height: 8px;
    background: repeating-linear-gradient(90deg,
      rgba(216,207,184,0) 0 9px,
      rgba(216,207,184,.16) 9px 15px,
      rgba(216,207,184,0) 15px 20px); }

  .frame-numbers { display: flex; height: 24px; align-items: center; }
  .frame-num { flex: 1; text-align: center;
    font: 600 14px 'Spline Sans Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    color: #a39a82; letter-spacing: .14em; }
  .frame-num.frame-pad { visibility: hidden; }

  .frames-row { display: flex; gap: 7px; background: #000; padding: 0 6px; align-items: flex-start; }
  /* Tiles share one landscape aspect derived from the roll (--tile-aspect, set on
     .strips-container; fallback 3:2). Landscape frames fill the tile; portrait crops
     fit INSIDE via object-fit:contain — uniform row height, Lightroom-style. */
  .frame-cell { flex: 1; aspect-ratio: var(--tile-aspect, 3 / 2); position: relative; background: #000;
    overflow: hidden; padding: 0; border: none; cursor: pointer; display: block;
    appearance: none; -webkit-appearance: none; }
  .frame-cell img { width: 100%; height: 100%; object-fit: contain; object-position: left center; display: block; }
  .frame-cell-pad { flex: 1; aspect-ratio: var(--tile-aspect, 3 / 2); background: transparent; cursor: default; }

  .rebate-info-row { display: flex; align-items: center; gap: 14px; height: 24px; padding: 0 12px; }
  .barcode { width: 34px; height: 11px; flex: none;
    background: repeating-linear-gradient(90deg,
      #c9c3b0 0 1px, transparent 1px 3px,
      #c9c3b0 3px 4px, transparent 4px 6px,
      #c9c3b0 6px 8px, transparent 8px 11px,
      #c9c3b0 11px 12px, transparent 12px 15px,
      #c9c3b0 15px 17px, transparent 17px 19px); }
  /* Edge marking repeated and evenly distributed across the strip width */
  .edge-track { flex: 1; min-width: 0; display: flex; align-items: center; gap: 14px; }
  .edge-text { flex: 1; min-width: 0; text-align: center;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font: 600 12px 'Spline Sans Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    color: #968f7c; letter-spacing: .24em; }
  .edge-arrow { font: 600 13px 'Spline Sans Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    color: #7a7464; letter-spacing: .24em; flex: none; }

  /* ===== Proof grid (film-edge OFF) ===== */
  .proof-strip { display: flex; gap: 16px; padding: 0 0 16px; align-items: flex-start; }
  .proof-cell { flex: 1; display: flex; flex-direction: column; gap: 8px; }
  .proof-cell-pad { flex: 1; }
  /* Roll-derived landscape tile (--tile-aspect; fallback 3:2); content letterboxes
     inside, so portrait frames don't push the row taller. */
  .proof-frame { background: #d8d3c4; padding: 3px; overflow: hidden;
    box-shadow: 0 1px 3px rgba(0,0,0,.5); border: none; cursor: pointer;
    display: block; width: 100%; aspect-ratio: var(--tile-aspect, 3 / 2); box-sizing: border-box;
    appearance: none; -webkit-appearance: none; }
  .proof-frame img { width: 100%; height: 100%; object-fit: contain; object-position: left center; display: block; }
  .proof-caption { text-align: center;
    font: 600 10px 'Spline Sans Mono', ui-monospace, 'SF Mono', Menlo, monospace;
    color: #6f6a5e; letter-spacing: .12em; }
  .panel { border-left: 1px solid var(--glass-brd); display: flex; flex-direction: column;
    gap: 8px; padding: 12px; overflow-y: auto; }
  .export-btn { padding: 8px 16px; border-radius: 9px; font-weight: 600; font-size: 12px;
    background: var(--glass-hi); border: 1px solid var(--glass-brd); color: var(--text);
    transition: background 0.15s, border-color 0.15s; cursor: pointer; }
  .export-btn:hover:not(:disabled) { background: rgba(255,255,255,0.08); border-color: rgba(255,255,255,0.18); }
  .export-btn:disabled { opacity: 0.45; cursor: default; }

  /* ===== Square icon tool buttons (Change D) ===== */
  .tool-row { display: flex; flex-direction: row; gap: 6px; margin: 10px 0; }
  .tool { flex: 1; display: flex; flex-direction: column; align-items: center; gap: 4px; }
  .tool-btn { width: 46px; height: 46px; border-radius: 10px; border: 1px solid var(--glass-brd);
    background: var(--glass-hi); color: var(--text-dim); cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    transition: background 0.15s, border-color 0.15s, box-shadow 0.15s; }
  .tool-btn:hover:not(:disabled) { background: rgba(255,255,255,0.08); border-color: rgba(255,255,255,0.18); }
  .tool-btn:disabled { opacity: 0.45; cursor: default; }
  .tool-btn.on { background: rgba(244,157,78,0.14); box-shadow: inset 0 0 0 1px rgba(244,157,78,0.4);
    border-color: rgba(244,157,78,0.4); color: var(--text); }
  .tool-label { font-size: 11px; color: var(--text-dim); text-align: center; line-height: 1.2; }

  /* ===== Reference-edit (crop) layout ===== */
  .ref-layout {
    height: 100%;
    min-height: 0;
    display: grid;
    grid-template-columns: 1fr 280px;
    grid-template-rows: 1fr 110px;
    grid-template-areas:
      "center panel"
      "strip  panel";
    background: #0d0d0f;
  }
  .ref-center {
    grid-area: center;
    min-height: 0;
    display: grid;
    place-items: center;
    position: relative;
    background: #111;
  }
  .ref-hint { color: var(--text-dim, #888); }
  .ref-panel {
    grid-area: panel;
    border-left: 1px solid var(--glass-brd);
    overflow-y: auto;
    background: #111;
  }
  .ref-panel-inner {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    height: 100%;
    box-sizing: border-box;
  }
  .done-btn {
    margin-top: auto;
    padding: 8px 16px;
    border-radius: 9px;
    font-weight: 600;
    font-size: 12px;
    background: rgba(244,157,78,0.18);
    border: 1px solid rgba(244,157,78,0.5);
    color: #fff;
    cursor: pointer;
    transition: background 0.15s, border-color 0.15s;
  }
  .done-btn:hover { background: rgba(244,157,78,0.28); border-color: rgba(244,157,78,0.7); }
  .ref-strip {
    grid-area: strip;
    display: flex;
    flex-direction: row;
    overflow-x: auto;
    overflow-y: hidden;
    background: #0d0d0f;
    border-top: 1px solid #222;
    padding: 4px 4px;
    gap: 2px;
    align-items: center;
  }
  .strip-cell {
    flex: 0 0 auto;
    width: 120px;
    height: 90px;
    padding: 0;
    border: 2px solid transparent;
    border-radius: 3px;
    overflow: hidden;
    background: #0d0d0f;
    cursor: pointer;
    transition: border-color 0.1s;
  }
  .strip-cell img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .strip-cell:hover { border-color: rgba(255,255,255,0.25); }
  .strip-cell.strip-active { border-color: rgba(255,255,255,0.7); }

</style>
