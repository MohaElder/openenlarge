<script context="module" lang="ts">
  // Gray-point WB sampling-ring diameter in CSS px, remembered across Viewport
  // remounts within a session (e.g. a trip through the crop tool, which swaps in
  // CropView and back). `null` = use the per-canvas default until the user first
  // resizes the ring.
  let savedWbRing: number | null = null;
</script>

<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { api, type InvertParams } from "../api";
  import type { IrRemoval } from "../api";
  import { previewSrc, previewById, cachePreview, hdrMode } from "../store";
  import { FinishRenderer, webgl2Available, float16RenderTargetSupported } from "./gl/renderer";
  import { finishUniforms } from "./gl/uniforms";
  import { toInversionUniforms } from "./gl/invert";
  import { clipUniforms } from "./gl/clip";
  import { toneLutBytes, colorGrade, colorMix, perZoneWb } from "../develop/finish";
  import { screenRadius, strokeCentroid, type DustStroke } from "../develop/dust";
  import { marqueeZoom } from "./marquee";
  import { hiTierAction } from "./hiTier";
  import { pickPixel, sampleRobust, nextWbRing } from "../develop/colorPick";
  import { orientUVMatrix, displayToSourceUV } from "../crop/transforms";
  import { viewWindow, deriveView, type ViewWindow } from "./view";
  import { t } from "$lib/i18n";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;
  export let imgH = 0;
  export let raw = false;
  export let interactive = true;
  export let imageCrop: [number, number, number, number] | null = null;
  export let rot90 = 0;
  export let flipH = false;
  export let flipV = false;
  export let angle = 0;
  export let eraser = false;
  /** Opt-in: this mount may drive the macOS native EDR compositing surface in
   *  live-edr mode (hide SDR canvas + punch a transparent body hole). Only the
   *  develop viewport sets this true; other mounts (e.g. the Roll "view frame"
   *  in FramePreview, which has its own opaque #111 overlay ancestor that would
   *  block the layer) keep the default and fall through to the gain-map <img>. */
  export let allowHdrSurface = false;
  /** Marquee-zoom armed: the next drag draws a zoom rectangle instead of painting. */
  export let marquee = false;
  export let pointPick = false;
  /** Brush radius normalized to image width. */
  export let brush = 0.03;
  /** Committed strokes for this image (rendered by the backend). */
  export let dust: DustStroke[] = [];
  /** Bumped by the parent on any dust change to force a re-render. */
  export let dustRev = 0;
  export let developRev = 0;
  export let irRemoval: IrRemoval = { enabled: false, sensitivity: 50 };
  /** AI-fill mode: show strokes as a red mask overlay (unhealed) until applied. */
  export let brushMigan = false;
  /** Whether the strokes have been MI-GAN-applied (heal baked) vs shown as overlay. */
  export let aiApplied = false;
  /** Heal-spot markers: show toggle, global spot centroids, kept-spot exclusions. */
  export let showSpots = true;
  export let autoSpots: { x: number; y: number }[] = [];
  export let autoExclusions: { x: number; y: number }[] = [];
  export let selectedSpot: import("../store").SpotSel | null = null;
  /** AI auto-dust: detector-driven defect heal, live on the main display. */
  export let autoDustEnabled = false;
  export let autoDustSensitivity = 50;
  /** Clipping-warning overlay toggles (GPU path only). */
  export let clipHigh = false;
  export let clipLow = false;
  export let clipStrict = false;
  /** Catalog thumbnail data-URL for the active image — shown as the switch-gap
   *  overlay when this image has no cached fit-view preview yet (first view). */
  export let fallbackThumb = "";

  const dispatch = createEventDispatcher<{ stroke: DustStroke; brush: number; pointpick: { r: number; g: number; b: number; u: number; v: number; rr: number; rg: number; rb: number }; aierased: void; autodusted: void; zoomchange: boolean; marqueedone: void; selectspot: import("../store").SpotSel; removespot: import("../store").SpotSel }>();

  const CAP = 5000;
  const PAD = 60;

  let el: HTMLDivElement;
  let canvas: HTMLCanvasElement | null = null;
  let renderer: FinishRenderer | null = null;
  // GPU path: only the interactive, non-raw develop canvas, when WebGL2 exists.
  const useGL = interactive && !raw && webgl2Available();

  let src = "";
  let vpW = 0, vpH = 0;
  let scale = 0;
  let cx = 0, cy = 0;
  let prevId: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let histTimer: ReturnType<typeof setTimeout> | null = null;
  let animating = false;
  let animTimer: ReturnType<typeof setTimeout> | null = null;

  // ---- Render-in-flight indicator -----------------------------------------
  // `busy` drives a spinner overlay while an async backend render/upload is in
  // flight (image open, dust/IR bake, hi-res deep-zoom decode). Finishing-layer
  // edits are instant GPU redraws and never set it. A short show-delay keeps the
  // spinner from flashing on fast proxy uploads.
  let inflight = 0;
  let busy = false;
  let busyTimer: ReturnType<typeof setTimeout> | null = null;
  const BUSY_DELAY_MS = 180;
  function enterRender() {
    inflight++;
    if (inflight === 1 && !busyTimer && !busy) {
      busyTimer = setTimeout(() => { busyTimer = null; busy = inflight > 0; }, BUSY_DELAY_MS);
    }
  }
  function exitRender() {
    inflight = Math.max(0, inflight - 1);
    if (inflight === 0) {
      if (busyTimer) { clearTimeout(busyTimer); busyTimer = null; }
      busy = false;
    }
  }

  // GPU upload state. The working buffer is uploaded to the GPU as a float texture;
  // inversion + geometry then update via uniforms. In bake mode the upload re-fires
  // when strokes/geometry change (keyed by uploadKey); otherwise once per image.
  let uploadKey = "";
  let texW = 0, texH = 0;
  // True once the NEW image's texture + inversion are uploaded and a correct frame
  // has been drawn. Reset on image switch so the stale-frame draws (finishKey below,
  // and geomKey/invKey via the texW===0 guard) are suppressed until that single
  // correct draw — `preserveDrawingBuffer` keeps the prior frame visible meanwhile.
  // Also gates hiding the per-image preview overlay.
  let frameReady = false;

  $: ready = imgW > 0 && imgH > 0 && vpW > 0 && vpH > 0;
  $: pad = interactive ? PAD : 0;
  $: avW = Math.max(1, vpW - 2 * pad);
  $: avH = Math.max(1, vpH - 2 * pad);
  $: fit = ready ? Math.min(avW / imgW, avH / imgH) : 0;
  $: eff = interactive ? (scale > 0 ? scale : fit) : fit;
  $: zoomed = interactive && eff > fit + 1e-6;
  $: label = eff <= fit + 1e-6 ? $t("viewport.fit") : $t("viewport.zoomPercent", { percent: Math.round(eff * 100) });

  // Deep-zoom tier: when the on-screen (device-pixel) long edge exceeds the proxy's
  // native pixels AND the source has more resolution to offer, request the high-res
  // texture. Hysteresis (enter > PROXY_EDGE, leave < 0.9×) avoids thrash at the edge.
  const PROXY_EDGE = 2560;
  // Canvas backing never exceeds this — keeps the composited layer well under the
  // WebKit/WebGL limits regardless of zoom. The visible window is rendered into it.
  const MAX_BACKING = 4096;
  // Hard upper zoom limit (200%). Beyond ~211% the CSS-scaled canvas/overlays exceed
  // WebKit's compositor layer size and the preview corrupts ("flashing"); cap here.
  const MAX_ZOOM = 2.0;
  // Settle delay before a hi-res upgrade actually fires. The hi-res decode+upload is
  // heavy, so we never kick it off mid-gesture — only once zoom/pan has been quiet
  // for this long. Restarted by every wheel/pan event (see armHiTier).
  const HI_SETTLE_MS = 220;
  // `wantHi` is the *desired* tier from the live zoom math; `hiTier` is the *committed*
  // tier that actually drives the upload (currentUploadKey / uploadWorking). Splitting
  // them lets us defer the expensive upgrade until the gesture settles while the current
  // proxy frame keeps showing (frameReady stays true → no drop to thumbnail).
  let wantHi = false;
  let hiTier = false;
  let hiTimer: ReturnType<typeof setTimeout> | null = null;
  // Set true (per image) once a hi-res upload fails to allocate on this GPU, so we
  // stop trying to upgrade and never flip-flop proxy↔hi-res. Reset on image switch.
  let hiUnavailable = false;
  $: {
    const srcLong = Math.max(imgW, imgH);
    const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
    const dispDevice = eff * srcLong * dpr;
    if (srcLong <= PROXY_EDGE) wantHi = false; // nothing sharper than the proxy to fetch
    else if (!wantHi && dispDevice > PROXY_EDGE) wantHi = true;
    else if (wantHi && dispDevice < PROXY_EDGE * 0.9) wantHi = false;
  }

  // Commit `wantHi` → `hiTier` only after the gesture settles. Upgrades to hi-res are
  // deferred behind a restartable timer (each wheel/pan event re-arms it, so a held or
  // continuous gesture never triggers the decode until it stops). Downgrades to the
  // proxy apply immediately — the proxy is always resident, so it's instant and any
  // pending upgrade is cancelled.
  function armHiTier() {
    if (hiTimer) { clearTimeout(hiTimer); hiTimer = null; }
    // Once hi-res has proven unallocatable for this image, never upgrade again —
    // treat the desired tier as proxy so we don't loop fetch→fail→fallback→fetch.
    const want = wantHi && !hiUnavailable;
    switch (hiTierAction(want, hiTier)) {
      case "downgrade": hiTier = false; return; // proxy resident → instant; drop pending upgrade
      case "noop": return;                       // already at hi-res
      case "arm": hiTimer = setTimeout(() => { hiTimer = null; hiTier = true; }, HI_SETTLE_MS); return;
    }
  }
  // Re-arm whenever the desired tier changes (e.g. a tap-to-100% animation crosses the
  // threshold). Gesture handlers (onWheel/onMove) also re-arm to keep deferring.
  $: { wantHi; armHiTier(); }

  // `e` defaults to the current effective scale, but callers that have just
  // reassigned `scale` must pass the new value: `eff` is a reactive derived
  // value that hasn't recomputed yet within the same tick, so clamping against
  // the stale `eff` (still fit) would wrongly re-center the view.
  function clampCenter(e = eff) {
    const halfW = avW / 2 / e, halfH = avH / 2 / e;
    cx = imgW * e <= avW ? imgW / 2 : Math.max(halfW, Math.min(imgW - halfW, cx));
    cy = imgH * e <= avH ? imgH / 2 : Math.max(halfH, Math.min(imgH - halfH, cy));
  }

  $: dispW = imgW * eff;
  $: dispH = imgH * eff;
  $: left = vpW / 2 - cx * eff;
  $: top = vpH / 2 - cy * eff;
  $: dpr = typeof window !== "undefined" ? (window.devicePixelRatio || 1) : 1;
  $: vw0 = (gpuEligible && !bakeMode && imgW > 0 && imgH > 0 && vpW > 0 && vpH > 0)
    ? viewWindow(eff, cx, cy, imgW, imgH, vpW, vpH, dpr, MAX_BACKING)
    : null;

  function measure() {
    if (!el) return;
    vpW = el.clientWidth; vpH = el.clientHeight;
  }
  onMount(() => {
    const v = float16RenderTargetSupported();
    console.log(`[SPIKE float16] ok=${v.ok} reason="${v.reason}"`);
    measure();
    if (useGL && canvas) {
      const r = new FinishRenderer(canvas);
      if (r.available) renderer = r;
    }
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    // Keep the native EDR surface's rect glued to the WebGL canvas's on-screen box.
    // A ResizeObserver fires AFTER layout, so canvasRect() reads the correct
    // post-resize box — unlike the geomKey-driven push, which can run before the DOM
    // has resized the canvas. This fixes zoom-OUT to fit, where the canvas shrinks
    // from the viewport-sized (zoomed) window back to the letterboxed dispW×dispH
    // box but the surface kept the stale viewport-sized rect ("fills the view"). The
    // canvas element is stable (useGL is const; the {#if useGL} block never toggles),
    // so a single observe suffices. No-op unless the surface is actually showing.
    const canvasRO = new ResizeObserver(() => {
      if (hdrUsesSurface && hdrShown) api.hdrSurfaceSetRect(canvasRect()).catch(() => {});
    });
    if (canvas) canvasRO.observe(canvas);
    // macOS WebKit delivers a trackpad PINCH as non-standard gesture* events (not
    // ctrl+wheel), so onWheel never sees it. While the WB picker is armed, capture
    // the pinch and map it to the same ring resize (and own the gesture so the
    // webview doesn't page-zoom). Outside picking, ignore it (existing behaviour).
    const opts = { passive: false } as AddEventListenerOptions;
    let gestureRing = 0;
    const onGestureStart = (ev: Event) => {
      if (!pointPick) return;
      ev.preventDefault();
      gestureRing = wbRing ?? wbRingDefault;
    };
    const onGestureChange = (ev: Event) => {
      if (!pointPick) return;
      ev.preventDefault();
      const scale = (ev as unknown as { scale?: number }).scale ?? 1;
      wbRing = Math.min(wbRingMax, Math.max(WB_RING_MIN, gestureRing * scale));
      savedWbRing = wbRing;
    };
    el?.addEventListener("gesturestart", onGestureStart, opts);
    el?.addEventListener("gesturechange", onGestureChange, opts);
    return () => {
      ro.disconnect();
      canvasRO.disconnect();
      el?.removeEventListener("gesturestart", onGestureStart);
      el?.removeEventListener("gesturechange", onGestureChange);
      if (hiTimer) { clearTimeout(hiTimer); hiTimer = null; }
      if (busyTimer) { clearTimeout(busyTimer); busyTimer = null; }
      // Free the WebGL context on unmount — otherwise every remount leaks one and
      // WebKit stalls the app once it caps out (~16 contexts).
      renderer?.dispose();
      renderer = null;
      // live-edr: never leave the native surface showing (or the body transparent)
      // behind an unmounted Viewport — e.g. navigating away from Develop mid-HDR.
      // Also cancels any pending per-frame uniforms rAF.
      hideHdrSurface();
      if (typeof document !== "undefined") document.body.classList.remove("hdr-edr-hole");
    };
  });

  $: if (id !== prevId) {
    prevId = id; scale = 0; cx = imgW / 2; cy = imgH / 2;
    // Invalidate GPU readiness synchronously so this flush's stale-texture redraws
    // (geomKey/invKey/finishKey, fired by the new image's params) are blocked until
    // uploadWorking binds the new texture and draws once. uploadKey reset forces the
    // re-upload (its key includes id, but resetting is explicit).
    texW = 0; texH = 0; uploadKey = ""; frameReady = false;
    hiUnavailable = false; // re-evaluate hi-res capacity for the new image
  }
  $: if (interactive && scale === 0 && fit > 0) scale = fit;

  // Image geometry changed without an id switch — a 90° rotation swaps width/height,
  // and applying/clearing a crop changes the effective dims. The old pan/zoom no
  // longer fits the new aspect, leaving the frame off-centre until a manual "reset
  // view". Re-fit + re-centre exactly like the id-switch reset above (scale=0 re-arms
  // the fit on the next flush). Skipped on the very first sizing (prevImg* still 0),
  // where the id-switch block already centred.
  let prevImgW = 0, prevImgH = 0;
  $: if (interactive && (imgW !== prevImgW || imgH !== prevImgH)) {
    if (prevImgW !== 0 && imgW > 0 && imgH > 0) { scale = 0; cx = imgW / 2; cy = imgH / 2; }
    prevImgW = imgW; prevImgH = imgH;
  }

  // Decode a JPEG data-URL to an <img> we can upload as a texture.
  function loadImage(url: string): Promise<HTMLImageElement> {
    return new Promise((resolve, reject) => {
      const im = new Image();
      im.onload = () => resolve(im);
      im.onerror = reject;
      im.src = url;
    });
  }

  // Fetch the source preview. With GL, request the PRE-FINISH image (finish:false)
  // and apply finishing in the shader; otherwise fetch the finished image.
  async function render() {
    if (gpuEligible) return; // GPU path owns eligible images; this is the CPU fallback only
    if (!id || !imgW || !vpW) { src = ""; return; }
    const rscale = Math.min(eff, CAP / Math.max(imgW, imgH));
    const out_w = Math.max(1, Math.round(imgW * rscale));
    const out_h = Math.max(1, Math.round(imgH * rscale));
    enterRender();
    try {
      const data = await api.renderView(id, params, {
        crop: [0, 0, imgW, imgH], out_w, out_h, raw, finish: !(useGL && renderer),
        image_crop: imageCrop, rot90, flip_h: flipH, flip_v: flipV, angle, dust, ir_removal: irRemoval,
      });
      if (useGL && renderer) {
        const im = await loadImage(data);
        renderer.setSource(im, out_w, out_h);
        drawGL();
      } else {
        src = data;
        if (interactive && !raw) previewSrc.set(src);
      }
    } catch (e) {
      // "not developed" is expected for thumbnails of not-yet-developed images;
      // anything else is a real error worth surfacing (don't swallow silently).
      if (!(typeof e === "string" && e === "not developed")) console.error("renderView failed", e);
      /* keep previous frame */
    } finally {
      exitRender();
    }
  }

  function drawGL() {
    if (!renderer) return;
    renderer.setUniforms(finishUniforms(params));
    renderer.setLut(toneLutBytes(params));
    renderer.setColorGrade(colorGrade(params));
    renderer.setPerZoneWb(perZoneWb(params));
    renderer.setColorMix(colorMix(params));
    renderer.setClip(clipUniforms({ high: clipHigh, low: clipLow, strict: clipStrict }));
    renderer.draw();
    // Publish a snapshot for the histogram (debounced; toDataURL is cheap-ish). Also
    // stash it as this image's fit-view preview so a later switch back shows it
    // instantly (skip while zoomed — a zoomed crop is a poor switch-in preview).
    if (canvas) {
      if (histTimer) clearTimeout(histTimer);
      const cv = canvas;
      const capId = id;
      const cache = gpuEligible && !zoomed;
      histTimer = setTimeout(() => {
        const url = cv.toDataURL("image/jpeg", 0.8);
        previewSrc.set(url);
        if (cache && capId) cachePreview(capId, url);
      }, 120);
    }
  }

  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(render, 80); }
  function scheduleIfReady() { if (id && vpW && imgW) { clampCenter(); schedule(); } }

  // ---- HDR overlay (settle mode) --------------------------------------------
  // When params.hdr is on, once an edit settles we show a full-frame HDR preview
  // over the live SDR canvas. Two backends, chosen by $hdrMode:
  //  - "gainmap-fallback": render a gain-map JPEG via api.encodeHdr and crossfade
  //    it in as an <img> overlay, SETTLE-debounced; during an active gesture the
  //    overlay hides so the live SDR shows through, then fades back in on settle.
  //    (Unchanged from before live-edr existed.)
  //  - "live-edr" (macOS): drive the native EDR compositing surface PER-FRAME —
  //    no settle, no gesture crutch. A rare SOURCE upload (raw-negative pixels)
  //    on image/develop/tier changes, then per-frame UNIFORMS pushes (throttled
  //    to rAF) on every edit; the full invert→finish→HDR pipeline runs on the
  //    native surface. The surface sits BEHIND the webview, so the SDR canvas is
  //    hidden (`edrhole` class below) and a transparent hole is punched through
  //    the DOM down to it (see the `hdr-edr-hole` body class + comment in the
  //    style block below). The SDR WebGL canvas KEEPS rendering (for the
  //    histogram's toDataURL) even while hidden.
  let hdrSrc = "";
  let hdrShown = false;
  let hdrTimer: ReturnType<typeof setTimeout> | null = null;
  let hdrPrevId: string | null = null;
  // live-edr for THIS mount: opt-in + capable + toggled on. Gates the per-frame
  // source/uniforms drivers; the gain-map settle path runs when this is false.
  $: liveEdr = allowHdrSurface && $hdrMode === "live-edr" && !!canvas;
  // True once the live-edr native surface (not the gain-map <img>) is the active
  // overlay — gates the canvas-hiding CSS class, the body transparent-hole class,
  // and rect-sync, so a gainmap-fallback frame (or a live-edr frame that fell back
  // because there's no GL canvas) never hides the SDR canvas under it.
  let hdrUsesSurface = false;

  // Build the ViewSpec matching EXACTLY what the SDR canvas is currently showing
  // (geometry, persistent crop, dust/IR), at every zoom level — so the EDR texture
  // is the on-screen view, not the whole image stretched to fill the canvas box.
  function hdrViewSpec(): import("../api").ViewSpec {
    const geom = {
      image_crop: imageCrop, rot90, flip_h: flipH, flip_v: flipV, angle,
      dust, ir_removal: irRemoval,
    };
    // Deep-zoom windowed path: the SDR canvas renders only the visible crop-window
    // into a viewport-sized box (`vw0`). Mirror it via deriveView (same visible-
    // region math as viewWindow) so `crop` matches the on-screen window; the surface
    // is positioned at that same canvas box (canvasRect), so the EDR aligns with the
    // SDR at any zoom. At fit deriveView yields the whole image ([0,0,imgW,imgH]) —
    // identical to the pre-zoom framing.
    //   Resolution: render at `vw0.backing` (= the SDR GL canvas's device-pixel
    // backing: displaySize × dpr, MAX_BACKING-capped), NOT deriveView's CSS-px
    // out_w/out_h. The native Metal drawable is sized rect.w*dpr × rect.h*dpr, so
    // a CSS-px texture would upload at ~1/dpr resolution and upscale to fill it
    // (soft). Matching vw0.backing makes the texture 1:1 with the drawable and the
    // SDR — same sharpness (and the same proxy/hi-res cap the SDR already honors).
    if (vw0) {
      const v = deriveView(eff, cx, cy, imgW, imgH, vpW, vpH, raw);
      return { ...v, out_w: vw0.backing.w, out_h: vw0.backing.h, finish: true, ...geom };
    }
    // No windowed render (bake mode / non-GPU): the canvas shows the full image at
    // its display box, so render the whole image at device-pixel resolution
    // (display size × dpr, uniformly capped to MAX_BACKING like the SDR backing).
    const dispDevW = imgW * eff * dpr;
    const dispDevH = imgH * eff * dpr;
    const k = Math.min(1, MAX_BACKING / Math.max(dispDevW, dispDevH, 1));
    const out_w = Math.max(1, Math.round(dispDevW * k));
    const out_h = Math.max(1, Math.round(dispDevH * k));
    return { crop: [0, 0, imgW, imgH], out_w, out_h, raw, finish: true, ...geom };
  }

  // The native surface is positioned at the WebGL canvas's on-screen box (CSS px,
  // top-left origin) + devicePixelRatio; the backend Y-flips it against the
  // content view, which the webview fills.
  function canvasRect(): { x: number; y: number; w: number; h: number; dpr: number } {
    if (!canvas) return { x: 0, y: 0, w: 0, h: 0, dpr: 1 };
    const r = canvas.getBoundingClientRect();
    return { x: r.left, y: r.top, w: r.width, h: r.height, dpr };
  }

  // ---- gain-map fallback path (settle-debounced <img>) ----------------------
  // ONLY used when !liveEdr (gainmap-fallback mode, or a non-opt-in mount like
  // FramePreview). live-edr never runs this — it drives the surface per-frame.
  async function encodeHdr() {
    if (!params.hdr || !id || !imgW || !vpW || liveEdr) return;
    const curId = id;
    try {
      const data = await api.encodeHdr(id, params, hdrViewSpec());
      if (id !== curId || !params.hdr) return; // image switched or toggled off mid-encode
      hdrUsesSurface = false;
      hdrSrc = data;
      hdrShown = true;
    } catch (e) {
      // "not developed" etc. are expected; swallow like seed()/reanalyze().
      if (!(typeof e === "string" && e === "not developed")) console.error("encodeHdr failed", e);
    }
  }

  // Any edit hides the gain-map overlay immediately (live SDR shows through), then
  // debounces an encode that fades HDR back in once things settle. Fallback only.
  function scheduleHdr() {
    hdrShown = false; // live SDR while dragging
    if (hdrTimer) clearTimeout(hdrTimer);
    if (!params.hdr || !id) return;
    hdrTimer = setTimeout(encodeHdr, 200);
  }

  // ---- live-edr path (per-frame native surface) -----------------------------
  // Split the drivers into a RARE source upload and a per-frame uniforms push.
  //  - Source (raw-negative pixels): changes only on image id / develop revision /
  //    proxy↔hi-res tier — the exact inputs the WebGL source upload keys off
  //    (`currentUploadKey` uses id|developRev|tier for the non-bake path). set_source
  //    also resolves + renders the current uniforms, so it produces a correct first
  //    frame on its own.
  //  - Uniforms (per-frame): invert + geometry (crop/zoom/pan) + finish/tone/color +
  //    clip toggles — reuses invKey|geomKey|finishKey. Throttled to rAF so multiple
  //    param changes in a frame coalesce into a single IPC call.
  let hdrRaf: number | null = null;
  let sourceSeq = 0;
  // Monotonic "last-issued-wins" epoch sent with every surface command. Bumped at
  // ISSUE time (synchronously, before any await) so a hide issued after a uniforms
  // push always carries a strictly larger epoch; the backend drops any command whose
  // epoch is older than the last one it acted on (fixes the stale set_uniforms
  // un-hiding the surface after a hide).
  let hdrEpoch = 0;
  // Guards against re-uploading the source on every edit: the source reactive block
  // re-runs whenever `params` is reassigned (its `params.hdr` gate), but the pixels
  // only change when `hdrSourceKey` does. Set optimistically BEFORE the await so
  // rapid edits within one upload don't double-fire; reset to "" on hide/failure so
  // re-enabling HDR (or a retry after "not developed") re-uploads.
  let lastHdrSourceKey = "";
  function cancelHdrRaf() {
    if (hdrRaf !== null) { cancelAnimationFrame(hdrRaf); hdrRaf = null; }
  }
  function clipArg() { return { high: clipHigh, low: clipLow, strict: clipStrict }; }
  // The SAME geometry object handed to the WebGL renderer (buildGeometry → the 7
  // view-shaping fields), so the surface's MSL invert reproduces the identical view
  // by construction — orient/crop/straighten + the deep-zoom window (view_off/
  // view_scale). At fit that's the identity crop + [0,0]/[1,1] window; when zoomed
  // it's the vw0 window. Before the working texture is uploaded (buildGeometry null)
  // fall back to identity; the next push (keyed on geomKey) corrects it.
  function hdrGeom(): {
    crop_off: [number, number]; crop_scale: [number, number];
    view_off: [number, number]; view_scale: [number, number];
    angle: number; aspect: number; orient: [number, number, number, number];
  } {
    const g = buildGeometry();
    if (!g) return {
      crop_off: [0, 0], crop_scale: [1, 1], view_off: [0, 0], view_scale: [1, 1],
      angle: 0, aspect: 1, orient: [1, 0, 0, 1],
    };
    return {
      crop_off: g.crop_off, crop_scale: g.crop_scale,
      view_off: g.view_off, view_scale: g.view_scale,
      angle: g.angle, aspect: g.aspect, orient: g.orient,
    };
  }

  // Upload (or re-upload) the raw-negative source; on success mark the surface live
  // and push one uniforms update to reconcile any edits made during the async gap.
  async function pushHdrSource() {
    if (!liveEdr || !params.hdr || !id || !imgW || !vpW) return;
    if (hdrSourceKey === lastHdrSourceKey) return; // source pixels unchanged
    lastHdrSourceKey = hdrSourceKey;
    const curId = id;
    const myseq = ++sourceSeq;
    try {
      await api.hdrSurfaceSetSource(id, params, hdrViewSpec(), canvasRect(), hdrGeom(), ++hdrEpoch);
      if (myseq !== sourceSeq || id !== curId || !params.hdr || !liveEdr) return; // superseded
      hdrUsesSurface = true;
      hdrShown = true; // hides the SDR canvas via the `edrhole` class below
      // The source render used params-at-call-time; sync to the latest uniforms in
      // case the user kept editing while the pixels were packing/uploading.
      pushHdrUniforms();
    } catch (e) {
      lastHdrSourceKey = ""; // allow a retry (e.g. transient "not developed")
      if (!(typeof e === "string" && e === "not developed")) console.error("hdrSurfaceSetSource failed", e);
    }
  }

  // Fire one uniforms update now (no throttle). No-op on the backend until a source
  // exists, so a stray early call is harmless.
  function pushHdrUniforms() {
    if (!liveEdr || !params.hdr || !id || !imgW || !vpW) return;
    api.hdrSurfaceSetUniforms(id, params, hdrViewSpec(), clipArg(), canvasRect(), hdrGeom(), ++hdrEpoch).catch((e) => {
      if (!(typeof e === "string" && e === "not developed")) console.error("hdrSurfaceSetUniforms failed", e);
    });
  }

  // Per-frame uniforms push, coalesced to one call per animation frame. A newer
  // edit within the same frame cancels the pending rAF and reschedules.
  function scheduleHdrUniforms() {
    if (!liveEdr || !params.hdr || !id) return;
    cancelHdrRaf();
    hdrRaf = requestAnimationFrame(() => { hdrRaf = null; pushHdrUniforms(); });
  }

  // Hide the native surface + tear down live-edr driving state (shared by image
  // switch, HDR-off, and unmount). Safe to call when no surface is active.
  function hideHdrSurface() {
    cancelHdrRaf();
    lastHdrSourceKey = ""; // re-enabling / next image must re-upload the source
    if (hdrUsesSurface) { api.hdrSurfaceHide(++hdrEpoch).catch(() => {}); hdrUsesSurface = false; }
  }

  // Clear the overlay on image switch so a stale HDR frame never shows for the wrong
  // photo. (The live-edr source key includes `id`, so a new source uploads next.)
  $: if (id !== hdrPrevId) {
    hdrPrevId = id; hdrSrc = ""; hdrShown = false;
    if (hdrTimer) clearTimeout(hdrTimer);
    hideHdrSurface();
  }

  // ---- reactive triggers ----------------------------------------------------
  // Fallback: one settle-debounced encode on any frame-affecting change.
  $: hdrKey = `${id}|${params.hdr}|${developRev}|${invKey}|${finishKey}|${geomKey}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}|${vpW > 0}`;
  $: if (params.hdr && !liveEdr) { hdrKey; if (id && vpW && imgW) scheduleHdr(); }

  // live-edr: source key (rare) and uniforms key (per-frame). The uniforms key is
  // the invert + geometry + finish/clip composite (finishKey already folds in the
  // clip toggles). vpW>0 guards against firing before the canvas is laid out.
  $: hdrSourceKey = `${id}|${developRev}|${hiTier ? 'hi' : 'lo'}`;
  $: hdrUniformsKey = `${invKey}|${geomKey}|${finishKey}`;
  $: if (params.hdr && liveEdr && id && imgW && vpW > 0) { hdrSourceKey; pushHdrSource(); }
  $: if (params.hdr && liveEdr && id && imgW && vpW > 0) { hdrUniformsKey; scheduleHdrUniforms(); }

  // HDR toggled off: hide the surface (the native CAMetalLayer would otherwise stay
  // shown with a frozen frame; the CSS self-heals off hdrShown) and clear fallback.
  $: if (!params.hdr) {
    hdrShown = false; if (hdrTimer) clearTimeout(hdrTimer);
    hideHdrSurface();
  }

  // NOTE (Sub-project B/C): in live-edr mode the GPU-drawn clipping warning
  // (clip.ts) and on-image dust markers are baked into the now-hidden SDR canvas,
  // so they aren't visible over the native layer at rest. They reappear during an
  // active gesture, when scheduleHdr() hides the surface and the live SDR canvas
  // (with those overlays) shows through again. Not fixed here — deferred until the
  // surface itself can render overlays.

  // ---- The transparent hole (live-edr only) ---------------------------------
  // The native CAMetalLayer is composited BEHIND the WKWebView and shows only
  // through pixels the webview itself doesn't paint. The Rust side already turns
  // off the webview's own background fill (`drawsBackground = false`, set once
  // when the surface is first created) — so the only thing left painting opaque
  // pixels over the image region is <body>'s solid theme color (#111111, see
  // styles/theme.css); `.vp`/`.center`/`.layout` have no background of their own
  // and the canvas itself is hidden via the `edrhole` class below. Toggling body
  // transparent for the whole document (scoped to exactly this state) punches the
  // hole. This is the part most likely to need Task-9 visual tuning: it also
  // makes the ~12px `.layout` grid gaps and any outer page margin transparent,
  // since nothing else paints those — the `.right`/`.bottom` chrome panels keep
  // their own opaque backgrounds (GlassPanel etc.) so they should be unaffected.
  // If a gap/margin shows an unwanted sliver through to whatever is behind the
  // window, give that ancestor (e.g. `.layout`) its own unconditional opaque
  // background instead of relying on body's.
  $: if (typeof document !== "undefined") {
    document.body.classList.toggle("hdr-edr-hole", hdrUsesSurface && hdrShown);
  }

  // GPU path: WebGL2 available + non-raw. Dust/IR now stay on the GPU via a baked
  // (geometry + pre-invert heal) working texture; only raw/no-WebGL2 fall to CPU.
  $: gpuEligible = !!(useGL && renderer && !raw);
  // Bake mode: dust/IR active → request the baked working texture + identity geometry.
  $: bakeMode = dust.length > 0 || irRemoval.enabled || autoDustEnabled;

  // Key the uploaded working texture. In bake mode it depends on dust strokes + the
  // baked geometry (re-bake on commit/geometry change); else just the image id.
  // Contract: the parent must bump `dustRev` on any change to `dust` (it's the proxy
  // for stroke changes here — the dust array itself is not in the key).
  function currentUploadKey(): string {
    const tier = hiTier ? 'hi' : 'lo';
    if (bakeMode) {
      return `bake|${tier}|${id}|${developRev}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}|${brushMigan}|${aiApplied}|${autoDustEnabled}|${autoDustSensitivity}|${autoExclusions.map((p) => p.x.toFixed(4) + ',' + p.y.toFixed(4)).join(';')}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
    }
    return `raw|${tier}|${id}|${developRev}`;
  }

  // Upload the working float texture to the GPU. In bake mode, fetch the BAKED
  // (geometry + pre-invert heal) buffer; else the raw working buffer. Sets uniforms
  // + draws after upload. Re-fetches only when the upload key changes.
  // A tier this GPU can't allocate (size/VRAM) must not render black. When a hi-res
  // upgrade fails, drop back to the resident proxy — which is still on the canvas —
  // and stop retrying hi-res for this image (the hiTier change re-triggers a proxy
  // upload). uploadKey is left unset so the proxy actually re-uploads.
  function dropHiTier() {
    if (hiTier) {
      hiUnavailable = true;
      hiTier = false;
      // The failed hi-res upload left the source/intermediate textures at the wrong
      // (hi-res) size; clear uploadKey so the hiTier change re-runs uploadWorking and
      // actually re-uploads the proxy (its key now differs), restoring a consistent
      // state — otherwise the dedupe (uploadKey === key) would skip the re-upload.
      uploadKey = "";
    }
  }

  async function uploadWorking() {
    if (!gpuEligible || !id || !renderer) return;
    const key = currentUploadKey();
    if (uploadKey === key) return; // already on the GPU for these inputs
    const k = key;
    enterRender();
    try {
      if (bakeMode) {
        const spec = {
          rot90, flip_h: flipH, flip_v: flipV, angle,
          image_crop: imageCrop, dust, ir_removal: irRemoval,
          migan: brushMigan && aiApplied,
          skip_dust_heal: brushMigan && !aiApplied,
          auto_dust: { enabled: autoDustEnabled, sensitivity: autoDustSensitivity },
          auto_dust_exclusions: autoExclusions.map((p) => [p.x, p.y] as [number, number]),
        };
        const info = await api.workingBakedInfo(id, spec, hiTier);
        const buf = await api.workingBakedPixels(id, spec, params, hiTier);
        if (!renderer || currentUploadKey() !== k) return; // stale (params changed mid-fetch)
        if (!renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h)) { dropHiTier(); return; }
        texW = info.w; texH = info.h;
        if (spec.migan) dispatch("aierased"); // MI-GAN apply bake finished → clear the button spinner
        if (spec.auto_dust.enabled) dispatch("autodusted"); // auto-dust heal bake finished → clear toggle spinner
      } else {
        const info = await api.workingInfo(id, hiTier);
        const buf = await api.workingPixels(id, hiTier);
        if (!renderer || currentUploadKey() !== k) return; // image changed mid-fetch
        if (!renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h)) { dropHiTier(); return; }
        texW = info.w; texH = info.h;
      }
      uploadKey = k;
      await refreshInversion();
      applyGeometryAndDraw();
    } catch (e) {
      // Expected when the target image isn't developed/cached yet (matches render()'s
      // CPU-path handling). Leave uploadKey unset so a later trigger (developRev bump,
      // or any structural change) retries; frameReady stays false so the cached-preview
      // / thumbnail overlay keeps covering the canvas instead of a stuck stale frame.
      if (!(typeof e === "string" && e === "not developed")) console.error("uploadWorking failed", e);
    } finally {
      exitRender();
    }
  }

  // Resolve inversion params (+ sampled base) into GPU uniforms — no fetch of pixels.
  // Latest-wins: each call takes a monotonic token; a slower stale resolve (e.g. the
  // mount-time default-WB call, which is slow on first entry because the backend must
  // ensure_resident/reload the freshly-developed image) must NOT overwrite the gains
  // from a newer call that resolved first. Without this the default temp/tint (neutral
  // wb = [1,1,1]) lands last and leaves the frame on a blue cast until a re-render.
  let invSeq = 0;
  async function refreshInversion() {
    if (!gpuEligible || !id || !renderer) return;
    const curId = id;
    const myseq = ++invSeq;
    const res = await api.resolvedInversion(id, params);
    if (myseq !== invSeq || id !== curId || !renderer) return; // superseded by a newer resolve
    renderer.setInversion(toInversionUniforms(res));
  }

  // The EXACT geometry object handed to the WebGL renderer for the current view —
  // orient/flip/straighten/persistent-crop + deep-zoom window, or bake-mode identity
  // (geometry pre-baked into the texture). Single source of truth so the WebGL draw
  // (below) and the EDR-surface push (`hdrGeom`) can never diverge. Returns null
  // before the working texture is uploaded (texW===0).
  function buildGeometry(): {
    crop_off: [number, number]; crop_scale: [number, number];
    angle: number; aspect: number; orient: [number, number, number, number];
    raw: boolean; outW: number; outH: number;
    view_off: [number, number]; view_scale: [number, number];
  } | null {
    if (texW === 0) return null; // no texture uploaded yet
    // Bake mode: geometry is already baked into the texture → identity + baked dims.
    if (bakeMode) {
      return {
        crop_off: [0, 0], crop_scale: [1, 1], angle: 0, aspect: 1,
        orient: [1, 0, 0, 1], raw, outW: texW, outH: texH,
        view_off: [0, 0], view_scale: [1, 1],
      };
    }
    // u_orient: oriented-UV → source-UV (undoes rot90/flip). Crop is in oriented UV.
    const o = orientUVMatrix(rot90, flipH, flipV);
    const [cropX, cropY, cropW, cropH] = imageCrop ?? [0, 0, 1, 1];
    // Oriented (pre-crop) pixel dims; for odd rot90 the source dims swap.
    const swap = (rot90 % 2) === 1;
    const oW = swap ? texH : texW, oH = swap ? texW : texH;
    // Output canvas = the crop window of the oriented image (used as fallback backing).
    const outW = Math.max(1, Math.round(oW * cropW));
    const outH = Math.max(1, Math.round(oH * cropH));
    // If a view window is available, render only the visible sub-region into a
    // viewport-sized backing; otherwise fall back to the full-image dims.
    const win = vw0 ?? { off: [0, 0] as [number, number], scale: [1, 1] as [number, number],
      backing: { w: outW, h: outH }, css: { left, top, width: dispW, height: dispH } };
    return {
      crop_off: [cropX, cropY], crop_scale: [cropW, cropH],
      angle: (angle * Math.PI) / 180, aspect: oH / oW, orient: o,
      raw, outW: win.backing.w, outH: win.backing.h,
      view_off: win.off, view_scale: win.scale,
    };
  }

  // Map orient/flip/straighten/persistent-crop into GPU geometry uniforms, then draw.
  function applyGeometryAndDraw() {
    if (!gpuEligible || !renderer) return;
    const g = buildGeometry();
    if (!g) return; // no texture uploaded yet
    renderer.setGeometry(g);
    drawGL();
    frameReady = true; // correct frame for the current image is now on screen
  }

  // Upload the working float texture. Re-fires when the image changes or, in bake
  // mode, when strokes/IR/geometry change (currentUploadKey dedupes redundant runs).
  $: if (gpuEligible) { id; developRev; dustRev; irRemoval.enabled; irRemoval.sensitivity; brushMigan; aiApplied; autoDustEnabled; autoDustSensitivity; autoExclusions; imageCrop; rot90; flipH; flipV; angle; hiTier; uploadWorking(); }
  $: if (!gpuEligible) uploadKey = "";

  // Inversion params now drive GPU uniforms (no backend pixel fetch) when eligible.
  $: invKey = `${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${params.black}|${params.gamma}|${params.positive}|${JSON.stringify(params.base_override)}|${JSON.stringify(params.wb_baseline)}`;
  $: if (gpuEligible) { invKey; refreshInversion().then(applyGeometryAndDraw).catch((e) => {
    if (!(typeof e === "string" && e === "not developed")) console.error("refreshInversion failed", e);
  }); }

  // Geometry also drives GPU uniforms (no fetch) when eligible.
  $: geomKey = `${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}|${eff}|${cx}|${cy}|${vpW}|${vpH}`;
  // Raw-mode GPU geometry only: in bake mode geometry is baked into the texture and
  // the upload trigger handles re-draws, so this would otherwise double-draw.
  $: if (gpuEligible && !bakeMode) { geomKey; applyGeometryAndDraw(); }

  // CPU fallback path: re-fetch the SOURCE from the backend only when NOT eligible
  // (dust/IR active, raw view, or no WebGL2). Reuses the existing render()/schedule.
  $: cpuKey = gpuEligible ? '' :
    `${id}|${developRev}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}|${JSON.stringify(params.base_override)}`;
  $: cpuKey, imgW, imgH, scheduleIfReady();

  // Finishing-only change → GPU redraw, no backend fetch. Tone curve + color
  // grading are all finishing-layer controls, so they live here too.
  $: finishKey = [
    params.brightness,
    params.contrast, params.highlights, params.shadows, params.whites, params.blacks,
    params.texture, params.vibrance, params.saturation,
    params.tc_highlights, params.tc_lights, params.tc_darks, params.tc_shadows,
    JSON.stringify(params.tc_curve), JSON.stringify(params.tc_red),
    JSON.stringify(params.tc_green), JSON.stringify(params.tc_blue),
    params.cg_sh_hue, params.cg_sh_sat, params.cg_sh_lum,
    params.cg_mid_hue, params.cg_mid_sat, params.cg_mid_lum,
    params.cg_hi_hue, params.cg_hi_sat, params.cg_hi_lum,
    params.cg_glob_hue, params.cg_glob_sat, params.cg_glob_lum,
    params.cg_blending, params.cg_balance,
    params.cm_red_hue, params.cm_red_sat, params.cm_red_lum,
    params.cm_orange_hue, params.cm_orange_sat, params.cm_orange_lum,
    params.cm_yellow_hue, params.cm_yellow_sat, params.cm_yellow_lum,
    params.cm_green_hue, params.cm_green_sat, params.cm_green_lum,
    params.cm_aqua_hue, params.cm_aqua_sat, params.cm_aqua_lum,
    params.cm_blue_hue, params.cm_blue_sat, params.cm_blue_lum,
    params.cm_purple_hue, params.cm_purple_sat, params.cm_purple_lum,
    params.cm_magenta_hue, params.cm_magenta_sat, params.cm_magenta_lum,
    JSON.stringify(params.pc_samples),
    params.pz_enabled,
    JSON.stringify(params.pz_sh), JSON.stringify(params.pz_mid), JSON.stringify(params.pz_hi),
    clipHigh, clipLow, clipStrict,
  ].join("|");
  // On the GPU develop path, suppress finishing redraws until the new image's frame
  // is ready (else this fires synchronously on switch with the OLD texture bound —
  // the worst flash). Raw/CPU-with-GL keeps its prior behavior (render() drives it).
  $: if (useGL) { finishKey; if (renderer && (!gpuEligible || frameReady)) drawGL(); }

  function imgPoint(e: { clientX: number; clientY: number }): [number, number] {
    const rect = el.getBoundingClientRect();
    return [(e.clientX - rect.left - left) / eff, (e.clientY - rect.top - top) / eff];
  }

  // ---- RGB densitometer ----------------------------------------------------
  // Read the displayed pixel under the cursor as sRGB 8-bit (0..255) — what the
  // eye sees and what exports. GPU path only: readCanvasPixel needs the WebGL2
  // backbuffer (preserveDrawingBuffer). On the CPU <img> fallback it returns null
  // and the badge stays hidden. Suppressed in eraser/point-pick modes (own overlays).
  let hoverRGB: [number, number, number] | null = null;
  $: readoutActive = interactive && useGL && !!id && !eraser && !pointPick;

  // This image's cached fit-view preview, shown as an overlay during the switch gap
  // (until frameReady) so the NEW image appears instantly instead of holding the old.
  // Cached previews (prefetched render_view / viewed-canvas snapshots) are always
  // oriented to the committed geometry, so they're safe to show as-is.
  $: cachedPreview = useGL && id ? ($previewById[id] ?? "") : "";
  // The raw catalog thumbnail is at the image's NATIVE orientation and is the FULL
  // frame, so it only matches the developed view when there's no committed crop /
  // rotation / flip / straighten. Otherwise showing it would flash a wrong-oriented
  // or wrongly-framed image, so fall back to holding the previous frame instead.
  $: thumbMatchesView = !imageCrop && rot90 === 0 && !flipH && !flipV && (angle ?? 0) === 0;
  $: switchPreview = !frameReady ? (cachedPreview || (thumbMatchesView ? fallbackThumb : "")) : "";
  function sampleHover(e: { clientX: number; clientY: number }) {
    if (!readoutActive || !canvas) { hoverRGB = null; return; }
    const rect = canvas.getBoundingClientRect();
    // Pick CLEAN image color via the renderer (no clip-warning overlay baked in).
    hoverRGB = pickPixel(renderer, canvas, e.clientX - rect.left, e.clientY - rect.top);
  }

  // Notify the parent whenever the zoom state flips so it can swap the toolbar button.
  // Push the zoom state to the parent on every flip — and once the viewport is
  // ready, even if it matches the initial `false`. A fresh Viewport instance
  // (e.g. remounted after a trip through the crop tool, which swaps in CropView)
  // starts un-zoomed; without this first emit the parent would keep a stale
  // `viewZoomed = true` from the previous instance and the button would stick.
  let prevZoomed: boolean | null = null;
  $: if (ready && zoomed !== prevZoomed) { prevZoomed = zoomed; dispatch("zoomchange", zoomed); }

  /** Animate back to fit-to-view. Called by the parent via bind:this. */
  export function resetZoom() {
    startAnim();
    scale = fit; cx = imgW / 2; cy = imgH / 2;
  }

  /** Animate to 1:1 (100%), centered. Crosses the hi-res zoom threshold so
   *  resolution-dependent effects (Texture) preview truthfully. Parent via bind:this. */
  export function zoomTo100() {
    startAnim();
    scale = 1.0; cx = imgW / 2; cy = imgH / 2;
  }

  function startAnim() {
    animating = true;
    if (animTimer) clearTimeout(animTimer);
    animTimer = setTimeout(() => (animating = false), 200);
  }
  function stopAnim() {
    if (animTimer) { clearTimeout(animTimer); animTimer = null; }
    animating = false;
  }

  // Resize the WB ring one step and remember it across remounts. `deltaSign < 0`
  // grows, > 0 shrinks (matches wheel deltaY and our pinch mapping).
  function resizeWbRing(deltaSign: number) {
    wbRing = nextWbRing(wbRing ?? wbRingDefault, deltaSign, WB_RING_MIN, wbRingMax);
    savedWbRing = wbRing;
  }

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
    // Gray-point WB picker: the wheel resizes the sampling ring instead of zooming,
    // so the user can dial the averaged area on the fly (like the film-base picker).
    if (pointPick) {
      e.preventDefault();
      resizeWbRing(e.deltaY);
      return;
    }
    // Eraser mode: the wheel resizes the brush; a trackpad PINCH (ctrlKey) still zooms.
    if (eraser && !e.ctrlKey) {
      e.preventDefault();
      const next = Math.min(0.2, Math.max(0.005, brush * Math.exp(-e.deltaY * 0.0015)));
      dispatch("brush", next);
      return;
    }
    stopAnim();
    e.preventDefault();
    const [ix, iy] = imgPoint(e);
    const ns = Math.min(Math.max(MAX_ZOOM, fit), Math.max(fit, eff * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (eff / ns);
    cy = iy + (cy - iy) * (eff / ns);
    scale = ns;
    // Each wheel tick restarts the settle countdown so a continuous zoom never fires
    // the hi-res decode until the wheel stops (the wantHi reactive only re-arms on a
    // tier *flip*, not on every same-tier tick).
    armHiTier();
  }

  let lastX = 0, lastY = 0, downX = 0, downY = 0, moved = false, panning = false;

  // Eraser: live cursor position (element coords) + the in-progress stroke (normalized).
  let curX = -100, curY = -100, hovering = false;
  let painting = false;
  let pending: { x: number; y: number }[] = [];
  // Marquee zoom: drag-in-progress flag, start corner in element coords (for drawing)
  // and image coords (for the zoom math), and the live corner in element coords.
  let mqActive = false;
  let mqSX = 0, mqSY = 0;
  let mqStartImg: [number, number] = [0, 0];
  let mqCX = 0, mqCY = 0;
  $: cursorR = screenRadius(brush, imgW, eff);

  // ---- Gray-point WB sampling ring -----------------------------------------
  // The picker medians a window of canvas pixels (sampleRobust). Make that window
  // user-sizable and visible: a ring on the live preview the user grows/shrinks
  // (scroll or trackpad pinch) to dial how much area gets averaged — bigger = more
  // grain stability. Size is in CSS px (an on-screen aiming target); converted to
  // backbuffer pixels via the canvas DPR at sample time.
  const WB_RING_MIN = 12;       // px (CSS)
  let wbRing: number | null = savedWbRing; // CSS-px diameter; null until defaulted
  let pickHovering = false;     // cursor is over the preview while the picker is armed
  // Canvas CSS dimensions (matches the inline canvas box; vw0 is the windowed view).
  $: canvasCssW = vw0 ? vw0.css.width : dispW;
  $: canvasCssH = vw0 ? vw0.css.height : dispH;
  $: canvasShort = Math.max(1, Math.min(canvasCssW, canvasCssH));
  // Default ≈ today's fixed 4%-of-short-edge window; max ≈ 40% so the ring stays
  // usable but can cover a generous flat patch.
  $: wbRingMax = Math.max(40, canvasShort * 0.4);
  $: wbRingDefault = Math.min(wbRingMax, Math.max(WB_RING_MIN, canvasShort * 0.04));
  // Fill the default once the canvas is sized; clamp a remembered value to this canvas.
  $: if (pointPick && canvasShort > 1) {
    wbRing = wbRing == null ? wbRingDefault : Math.min(wbRingMax, Math.max(WB_RING_MIN, wbRing));
  }

  // Marker anchors in normalized [0,1] space, both kinds, for rendering + hit-test.
  $: spotMarkers = [
    ...dust.map((s, index) => ({ kind: "stroke" as const, index, c: strokeCentroid(s) })),
    ...autoSpots.map((c, index) => ({ kind: "auto" as const, index, c })),
  ];
  // Markers are only interactive in the eraser tool while the toggle is on.
  $: spotsVisible = eraser && showSpots && spotMarkers.length > 0;
  const HIT_PX = 13; // tap radius (element px) to grab a marker instead of painting

  /** Nearest marker within HIT_PX of the event, or null. */
  function hitTestSpot(e: { clientX: number; clientY: number }): import("../store").SpotSel | null {
    const rect = el.getBoundingClientRect();
    const ex = e.clientX - rect.left, ey = e.clientY - rect.top;
    let best: import("../store").SpotSel | null = null;
    let bestD = HIT_PX * HIT_PX;
    for (const m of spotMarkers) {
      const mx = left + m.c.x * dispW, my = top + m.c.y * dispH;
      const d = (mx - ex) * (mx - ex) + (my - ey) * (my - ey);
      if (d <= bestD) { bestD = d; best = { kind: m.kind, index: m.index }; }
    }
    return best;
  }
  // Double-tap tracking: a second tap on the same marker within 300ms removes it.
  let lastTap: { kind: "stroke" | "auto"; index: number; t: number } | null = null;
  $: if (!spotsVisible) lastTap = null;

  // SVG path for a stroke's polyline in display px (normalized → dispW/dispH).
  // A single dab becomes "M p L p" so a round cap renders it as a dot.
  function strokeD(pts: { x: number; y: number }[], w: number, h: number): string {
    if (!pts.length) return "";
    const p = (q: { x: number; y: number }) => `${(q.x * w).toFixed(1)} ${(q.y * h).toFixed(1)}`;
    if (pts.length === 1) return `M ${p(pts[0])} L ${p(pts[0])}`;
    return "M " + pts.map(p).join(" L ");
  }
  // Show committed + in-progress strokes as a mask while AI-fill is pending.
  $: showMask = eraser && brushMigan && !aiApplied;

  function normPoint(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const [ix, iy] = imgPoint(e);
    return { x: ix / imgW, y: iy / imgH };
  }
  // True while the cursor is over a heal-spot marker → show the tap/pointer cursor
  // (and hide the brush circle) so the spot reads as clickable.
  let overSpot = false;
  function onEraserMove(e: PointerEvent) {
    const rect = el.getBoundingClientRect();
    curX = e.clientX - rect.left;
    curY = e.clientY - rect.top;
    overSpot = spotsVisible && !painting ? !!hitTestSpot(e) : false;
    if (painting) pending = [...pending, normPoint(e)];
  }
  function onEnter() { if (eraser) hovering = true; }
  function onLeave() { hovering = false; painting = false; pending = []; mqActive = false; hoverRGB = null; overSpot = false; pickHovering = false; }

  function onDown(e: PointerEvent) {
    if (!interactive) return;
    if (e.button !== 0) return; // ignore right/middle click — let the context menu open
    if (pointPick) {
      // Mark as "moved" so the upcoming pointerup is not treated as a tap-to-zoom.
      // The pick dispatch flips pointPick off synchronously (the parent clears its
      // picking flag), so onUp can't rely on `pointPick` still being true.
      moved = true;
      if (canvas) {
        const rect = canvas.getBoundingClientRect();
        const px = e.clientX - rect.left, py = e.clientY - rect.top;
        // Canvas covers only the visible window; map the in-canvas fraction to the
        // displayed-image UV (window off + frac*scale) before crop/orient inversion.
        const du = vw0 ? vw0.off[0] + (px / rect.width) * vw0.scale[0] : px / rect.width;
        const dv = vw0 ? vw0.off[1] + (py / rect.height) * vw0.scale[1] : py / rect.height;
        const [u, v] = displayToSourceUV(du, dv, imageCrop, rot90, flipH, flipV);
        // Clean image color (no clip overlay) so a point pick over a clipped
        // region samples the real pixel, not the warning color (B2). Alongside the
        // single center pixel we pass a grain-robust median over a small window
        // (~4% of the shorter display edge, odd) — gray-point WB uses the robust
        // value so film grain can't throw Temp/Tint to an extreme (D).
        const rgb = pickPixel(renderer, canvas, px, py);
        if (rgb) {
          // The visible ring is a CSS-px diameter; convert to backbuffer pixels via
          // the canvas DPR so sampleRobust medians exactly the area the user framed.
          const dpr = rect.width > 0 ? canvas.width / rect.width : 1;
          const ringCss = wbRing ?? wbRingDefault;
          const win = Math.max(15, Math.round(ringCss * dpr)) | 1;
          const rob = sampleRobust(renderer, canvas, px, py, win) ?? rgb;
          dispatch("pointpick", { r: rgb[0], g: rgb[1], b: rgb[2], u, v, rr: rob[0], rg: rob[1], rb: rob[2] });
        }
      }
      return;
    }
    if (eraser && marquee) {
      const rect = el.getBoundingClientRect();
      mqActive = true;
      mqSX = mqCX = e.clientX - rect.left;
      mqSY = mqCY = e.clientY - rect.top;
      mqStartImg = imgPoint(e);
      (e.target as Element).setPointerCapture?.(e.pointerId);
      return;
    }
    if (eraser) {
      if (spotsVisible) {
        const hit = hitTestSpot(e);
        if (hit) {
          const now = performance.now();
          const same = lastTap && lastTap.kind === hit.kind && lastTap.index === hit.index && now - lastTap.t < 300;
          if (same) { dispatch("removespot", hit); lastTap = null; }
          else { dispatch("selectspot", hit); lastTap = { ...hit, t: now }; }
          return; // grabbed a marker → do not paint
        }
        lastTap = null;
      }
      painting = true;
      pending = [normPoint(e)];
      (e.target as Element).setPointerCapture?.(e.pointerId);
      return;
    }
    stopAnim();
    downX = lastX = e.clientX; downY = lastY = e.clientY; moved = false;
    panning = zoomed;
    (e.target as Element).setPointerCapture?.(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!interactive) return;
    sampleHover(e);
    if (pointPick) {
      // Track the cursor so the sampling ring follows it.
      const rect = el.getBoundingClientRect();
      curX = e.clientX - rect.left;
      curY = e.clientY - rect.top;
      pickHovering = true;
      return;
    }
    if (eraser && marquee) {
      if (mqActive) {
        const rect = el.getBoundingClientRect();
        mqCX = e.clientX - rect.left;
        mqCY = e.clientY - rect.top;
      }
      return;
    }
    if (eraser) { onEraserMove(e); return; }
    if (!(e.buttons & 1)) return;
    if (Math.abs(e.clientX - downX) > 3 || Math.abs(e.clientY - downY) > 3) moved = true;
    if (panning && moved) {
      cx -= (e.clientX - lastX) / eff;
      cy -= (e.clientY - lastY) / eff;
      clampCenter();
      // Defer a not-yet-committed hi-res upgrade until panning stops, so dragging
      // never kicks off the heavy decode/upload mid-gesture.
      if (!hiTier && wantHi) armHiTier();
    }
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp(e: PointerEvent) {
    if (e.button !== 0) return; // right/middle click never triggers tap-to-zoom
    if (pointPick) return;
    if (eraser && marquee) {
      if (mqActive) {
        const dist = Math.hypot(mqCX - mqSX, mqCY - mqSY);
        if (dist >= 8) {
          const [bx, by] = imgPoint(e);
          const z = marqueeZoom(mqStartImg[0], mqStartImg[1], bx, by, avW, avH, fit, Math.max(MAX_ZOOM, fit));
          startAnim();
          scale = z.scale; cx = z.cx; cy = z.cy;
          clampCenter(z.scale);
          dispatch("marqueedone");
        }
        mqActive = false;
      }
      return;
    }
    if (eraser) {
      if (painting && pending.length > 0) dispatch("stroke", { points: pending, r: brush });
      painting = false; pending = [];
      return;
    }
    if (interactive && !moved) {
      const [ix, iy] = imgPoint(e);
      startAnim();
      if (zoomed) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
      else { scale = 1.0; cx = ix; cy = iy; }
    }
    panning = false; moved = false;
  }
  function onCancel() { painting = false; pending = []; panning = false; moved = false; mqActive = false; }
</script>

<div
  class="vp" class:interactive class:zoomed class:erasing={eraser} class:picking={pointPick} class:marqueearm={eraser && marquee} class:overspot={eraser && overSpot}
  bind:this={el}
  on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onCancel}
  on:pointerenter={onEnter} on:pointerleave={onLeave}
>
  {#if useGL}
    <canvas
      bind:this={canvas} class:anim={animating} class:edrhole={hdrUsesSurface && hdrShown}
      style="position:absolute; left:{vw0 ? vw0.css.left : left}px; top:{vw0 ? vw0.css.top : top}px; width:{vw0 ? vw0.css.width : dispW}px; height:{vw0 ? vw0.css.height : dispH}px;"
    ></canvas>
    {#if switchPreview}
      <img
        class="preview-cache" src={switchPreview} alt="" draggable="false" aria-hidden="true"
        style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
      />
    {/if}
    {#if !id}<div class="hint">…</div>{/if}
  {:else if src}
    <img
      {src} alt="preview" draggable="false" class:anim={animating}
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
    />
  {:else}<div class="hint">…</div>{/if}
  {#if params.hdr && hdrSrc}
    <img
      class="hdr-overlay" src={hdrSrc} alt="" draggable="false" aria-hidden="true"
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px; opacity:{hdrShown ? 1 : 0};"
    />
  {/if}
  {#if showMask}
    <svg class="maskov" aria-hidden="true"
         style="left:{left}px; top:{top}px; width:{dispW}px; height:{dispH}px;"
         viewBox="0 0 {Math.max(dispW, 1)} {Math.max(dispH, 1)}">
      {#each dust as s}
        <path d={strokeD(s.points, dispW, dispH)} stroke-width={s.r * 2 * dispW}
              fill="none" stroke-linecap="round" stroke-linejoin="round" />
      {/each}
      {#if painting && pending.length}
        <path d={strokeD(pending, dispW, dispH)} stroke-width={brush * 2 * dispW}
              fill="none" stroke-linecap="round" stroke-linejoin="round" />
      {/if}
    </svg>
  {/if}
  {#if spotsVisible}
    <div class="spots" style="left:{left}px; top:{top}px; width:{dispW}px; height:{dispH}px;" aria-hidden="true">
      {#each spotMarkers as m}
        <span class="spot" class:sel={selectedSpot && selectedSpot.kind === m.kind && selectedSpot.index === m.index}
              style="left:{m.c.x * dispW}px; top:{m.c.y * dispH}px;">
          <svg viewBox="0 0 24 24" width="14" height="14">
            <path d="M16.2 3.6 3.6 16.2a2 2 0 0 0 0 2.8l1.4 1.4a2 2 0 0 0 2.8 0L20.4 7.8a2 2 0 0 0 0-2.8l-1.4-1.4a2 2 0 0 0-2.8 0Z"
                  fill="currentColor" opacity="0.92" />
            <path d="M9 20.4h11" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
          </svg>
        </span>
      {/each}
    </div>
  {/if}
  {#if eraser && hovering && !marquee && !overSpot}
    <div class="brush" style="left:{curX}px; top:{curY}px; width:{cursorR * 2}px; height:{cursorR * 2}px;"></div>
  {/if}
  {#if pointPick && pickHovering && wbRing != null}
    <div class="wbring" style="left:{curX}px; top:{curY}px; width:{wbRing}px; height:{wbRing}px;" aria-hidden="true">
      <span class="wbdot"></span>
    </div>
  {/if}
  {#if eraser && marquee && mqActive}
    <div class="marquee" style="left:{Math.min(mqSX, mqCX)}px; top:{Math.min(mqSY, mqCY)}px; width:{Math.abs(mqCX - mqSX)}px; height:{Math.abs(mqCY - mqSY)}px;"></div>
  {/if}
  {#if id && interactive}<div class="zoom">{label}</div>{/if}
  {#if busy && interactive && !raw}
    <div class="rendering" aria-live="polite" aria-busy="true">
      <span class="spinner" aria-hidden="true"></span>
      <span class="rlabel">{$t("viewport.rendering")}</span>
    </div>
  {/if}
  {#if readoutActive && hoverRGB}
    <div class="readout" title={$t("viewport.rgbReadout")}>
      <span class="sw" style="background:rgb({hoverRGB[0]},{hoverRGB[1]},{hoverRGB[2]})"></span>
      <span>R {hoverRGB[0]}</span><span>G {hoverRGB[1]}</span><span>B {hoverRGB[2]}</span>
    </div>
  {/if}
</div>

<style>
  .vp { position: relative; width: 100%; height: 100%; overflow: hidden; user-select: none;
    border-radius: 10px; }
  .vp.interactive { cursor: zoom-in; }
  .vp.zoomed { cursor: grab; }
  .vp.zoomed:active { cursor: grabbing; }
  /* contain (not the default fill) so that during an image switch — when the canvas
     box is already sized to the NEW image but its buffer still holds the previous
     frame at a different aspect (e.g. a 90° rotation flips portrait↔landscape) — the
     stale frame is letterboxed rather than stretched. Once the correct frame draws,
     buffer and box share the same aspect, so contain fills exactly (a no-op). */
  img, canvas { display: block; object-fit: contain; will-change: left, top, width, height; }
  /* live-edr: the native EDR surface replaces the SDR canvas at rest — hide the
     canvas (not unmount it, so the GL context/texture stay live for an instant
     return on the next gesture) so it doesn't paint over the native layer. */
  canvas.edrhole { visibility: hidden; }
  /* See the "transparent hole" comment block in the script block above for what
     this punches through and what to adjust if Task-9 verification finds a gap. */
  :global(body.hdr-edr-hole) { background: transparent; }
  img.anim, canvas.anim { transition: left 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    top 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    width 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    height 180ms cubic-bezier(0.22, 0.61, 0.36, 1); }
  .preview-cache { object-fit: contain; pointer-events: none; z-index: 1; }
  .hdr-overlay { object-fit: contain; pointer-events: none; z-index: 1;
    /* Lift the HDR headroom clamp so the gain-map JPEG can exceed SDR white. */
    dynamic-range-limit: no-limit; transition: opacity 150ms; }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
  .zoom { position: absolute; bottom: 8px; right: 10px; font-size: 11px; color: var(--text-dim);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; z-index: 2; }
  /* Render-in-flight pill: centered, non-interactive, above the canvas + overlays. */
  .rendering { position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);
    display: flex; align-items: center; gap: 8px; z-index: 6; pointer-events: none;
    font-size: 11px; color: var(--text); background: rgba(0,0,0,0.55);
    padding: 4px 10px; border-radius: 999px; box-shadow: 0 1px 6px rgba(0,0,0,0.4); }
  .rendering .spinner { width: 12px; height: 12px; border-radius: 50%;
    border: 2px solid rgba(255,255,255,0.25); border-top-color: var(--accent);
    animation: vp-spin 0.7s linear infinite; }
  @keyframes vp-spin { to { transform: rotate(360deg); } }
  .readout { position: absolute; bottom: 32px; right: 10px; font-size: 11px; color: var(--text);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; z-index: 2;
    display: flex; align-items: center; gap: 6px; pointer-events: none;
    font-variant-numeric: tabular-nums; }
  .readout .sw { width: 10px; height: 10px; border-radius: 2px; flex: none;
    box-shadow: inset 0 0 0 1px rgba(255,255,255,0.35); }
  .vp.erasing { cursor: none; }
  .vp.erasing.overspot { cursor: pointer; }
  .vp.picking { cursor: crosshair; }
  .maskov { position: absolute; pointer-events: none; z-index: 2; overflow: visible; }
  .maskov path { stroke: rgba(244,70,70,0.55); }
  .brush { position: absolute; border-radius: 50%; pointer-events: none; z-index: 3;
    transform: translate(-50%, -50%); border: 1.5px solid rgba(255,255,255,0.9);
    box-shadow: 0 0 0 1px rgba(0,0,0,0.5), inset 0 0 0 1px rgba(0,0,0,0.4); }
  /* Gray-point WB sampling ring: shows the averaged area; scroll/pinch resizes it. */
  .wbring { position: absolute; border-radius: 50%; pointer-events: none; z-index: 3;
    transform: translate(-50%, -50%); display: grid; place-items: center;
    border: 1.5px solid rgba(120,220,255,0.95);
    box-shadow: 0 0 0 1px rgba(0,0,0,0.55), inset 0 0 0 1px rgba(0,0,0,0.45); }
  .wbring .wbdot { width: 3px; height: 3px; border-radius: 50%;
    background: rgba(120,220,255,0.95); box-shadow: 0 0 0 1px rgba(0,0,0,0.6); }
  .vp.marqueearm { cursor: crosshair; }
  .marquee { position: absolute; z-index: 5; pointer-events: none;
    border: 1px solid #fff; background: rgba(244,157,78,0.15);
    box-shadow: 0 0 0 1px rgba(0,0,0,0.4); }
  .spots { position: absolute; pointer-events: none; z-index: 4; overflow: visible; }
  .spot { position: absolute; transform: translate(-50%, -50%); display: grid; place-items: center;
    width: 20px; height: 20px; border-radius: 50%; color: #fff;
    background: rgba(0,0,0,0.45); box-shadow: 0 0 0 1px rgba(0,0,0,0.5), inset 0 0 0 1px rgba(255,255,255,0.35); }
  .spot.sel { color: #111; background: rgba(244,157,78,0.95);
    box-shadow: 0 0 0 2px rgba(244,157,78,0.6), 0 0 0 1px rgba(0,0,0,0.5); }
</style>
