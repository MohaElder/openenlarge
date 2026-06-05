<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { api, type InvertParams } from "../api";
  import { previewSrc } from "../store";
  import { FinishRenderer, webgl2Available } from "./gl/renderer";
  import { finishUniforms } from "./gl/uniforms";
  import SpinOverlay from "./SpinOverlay.svelte";
  import { spinGeometry } from "./spin";
  import { screenRadius, type DustStroke } from "../develop/dust";

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
  /** Brush radius normalized to image width. */
  export let brush = 0.03;
  /** Committed strokes for this image (rendered by the backend). */
  export let dust: DustStroke[] = [];
  /** Bumped by the parent on any dust change to force a re-render. */
  export let dustRev = 0;

  const dispatch = createEventDispatcher<{ stroke: DustStroke; brush: number }>();

  const CAP = 5000;
  const PAD = 60;

  let el: HTMLDivElement;
  let canvas: HTMLCanvasElement | null = null;
  let renderer: FinishRenderer | null = null;
  // GPU path: only the interactive, non-raw develop canvas, when WebGL2 exists.
  const useGL = interactive && !raw && webgl2Available();

  let spinOverlay: SpinOverlay;
  let prevRot90 = rot90;

  let src = "";
  let vpW = 0, vpH = 0;
  let scale = 0;
  let cx = 0, cy = 0;
  let prevId: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let histTimer: ReturnType<typeof setTimeout> | null = null;
  let animating = false;
  let animTimer: ReturnType<typeof setTimeout> | null = null;

  $: ready = imgW > 0 && imgH > 0 && vpW > 0 && vpH > 0;
  $: pad = interactive ? PAD : 0;
  $: avW = Math.max(1, vpW - 2 * pad);
  $: avH = Math.max(1, vpH - 2 * pad);
  $: fit = ready ? Math.min(avW / imgW, avH / imgH) : 0;
  $: eff = interactive ? (scale > 0 ? scale : fit) : fit;
  $: zoomed = interactive && eff > fit + 1e-6;
  $: label = eff <= fit + 1e-6 ? "Fit" : Math.round(eff * 100) + "%";

  // Animate a single 90° turn at Fit (skip while zoomed). Snapshot = GL canvas or img src.
  $: if (rot90 !== prevRot90) {
    const atFit = eff <= fit + 1e-6;
    const snap = useGL && canvas ? canvas.toDataURL("image/jpeg", 0.9) : src;
    const g = atFit && snap ? spinGeometry(prevRot90, rot90, imgW, imgH, vpW, vpH, PAD) : null;
    if (g && spinOverlay) spinOverlay.spin(snap, g.rect, g.dir, g.k);
    prevRot90 = rot90;
  }

  function clampCenter() {
    const halfW = avW / 2 / eff, halfH = avH / 2 / eff;
    cx = imgW * eff <= avW ? imgW / 2 : Math.max(halfW, Math.min(imgW - halfW, cx));
    cy = imgH * eff <= avH ? imgH / 2 : Math.max(halfH, Math.min(imgH - halfH, cy));
  }

  $: dispW = imgW * eff;
  $: dispH = imgH * eff;
  $: left = vpW / 2 - cx * eff;
  $: top = vpH / 2 - cy * eff;

  function measure() {
    if (!el) return;
    vpW = el.clientWidth; vpH = el.clientHeight;
  }
  onMount(() => {
    measure();
    if (useGL && canvas) {
      const r = new FinishRenderer(canvas);
      if (r.available) renderer = r;
    }
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    return () => ro.disconnect();
  });

  $: if (id !== prevId) { prevId = id; scale = 0; cx = imgW / 2; cy = imgH / 2; }
  $: if (interactive && scale === 0 && fit > 0) scale = fit;

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
    if (!id || !imgW || !vpW) { src = ""; return; }
    const rscale = Math.min(eff, CAP / Math.max(imgW, imgH));
    const out_w = Math.max(1, Math.round(imgW * rscale));
    const out_h = Math.max(1, Math.round(imgH * rscale));
    try {
      const data = await api.renderView(id, params, {
        crop: [0, 0, imgW, imgH], out_w, out_h, raw, finish: !(useGL && renderer),
        image_crop: imageCrop, rot90, flip_h: flipH, flip_v: flipV, angle, dust,
      });
      if (useGL && renderer) {
        const im = await loadImage(data);
        renderer.setSource(im, out_w, out_h);
        drawGL();
      } else {
        src = data;
        if (interactive && !raw) previewSrc.set(src);
      }
    } catch { /* keep previous frame */ }
  }

  function drawGL() {
    if (!renderer) return;
    renderer.setUniforms(finishUniforms(params));
    renderer.draw();
    // Publish a snapshot for the histogram (debounced; toDataURL is cheap-ish).
    if (canvas) {
      if (histTimer) clearTimeout(histTimer);
      const cv = canvas;
      histTimer = setTimeout(() => previewSrc.set(cv.toDataURL("image/jpeg", 0.8)), 120);
    }
  }

  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(render, 80); }
  function scheduleIfReady() { if (id && vpW && imgW) { clampCenter(); schedule(); } }

  // Re-fetch the SOURCE only when the inversion / zoom / view changes. In Plan 2A
  // exposure/temp/tint are still baked by the backend, so they live in this key.
  $: srcKey = `${id}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}|${dustRev}`;
  $: srcKey, imgW, imgH, scheduleIfReady();

  // Finishing-only change → GPU redraw, no backend fetch.
  $: finishKey = `${params.contrast}|${params.highlights}|${params.shadows}|${params.whites}|${params.blacks}|${params.texture}|${params.vibrance}|${params.saturation}`;
  $: if (useGL) { finishKey; if (renderer) drawGL(); }

  function imgPoint(e: { clientX: number; clientY: number }): [number, number] {
    const rect = el.getBoundingClientRect();
    return [(e.clientX - rect.left - left) / eff, (e.clientY - rect.top - top) / eff];
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

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
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
    const ns = Math.min(8, Math.max(fit, eff * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (eff / ns);
    cy = iy + (cy - iy) * (eff / ns);
    scale = ns;
  }

  let lastX = 0, lastY = 0, downX = 0, downY = 0, moved = false, panning = false;

  // Eraser: live cursor position (element coords) + the in-progress stroke (normalized).
  let curX = -100, curY = -100, hovering = false;
  let painting = false;
  let pending: { x: number; y: number }[] = [];
  $: cursorR = screenRadius(brush, imgW, eff);

  function normPoint(e: { clientX: number; clientY: number }): { x: number; y: number } {
    const [ix, iy] = imgPoint(e);
    return { x: ix / imgW, y: iy / imgH };
  }
  function onEraserMove(e: PointerEvent) {
    const rect = el.getBoundingClientRect();
    curX = e.clientX - rect.left;
    curY = e.clientY - rect.top;
    if (painting) pending = [...pending, normPoint(e)];
  }
  function onEnter() { if (eraser) hovering = true; }
  function onLeave() { hovering = false; painting = false; pending = []; }

  function onDown(e: PointerEvent) {
    if (!interactive) return;
    if (eraser) {
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
    if (eraser) { onEraserMove(e); return; }
    if (!(e.buttons & 1)) return;
    if (Math.abs(e.clientX - downX) > 3 || Math.abs(e.clientY - downY) > 3) moved = true;
    if (panning && moved) {
      cx -= (e.clientX - lastX) / eff;
      cy -= (e.clientY - lastY) / eff;
      clampCenter();
    }
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp(e: PointerEvent) {
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
  function onCancel() { panning = false; moved = false; }
</script>

<div
  class="vp" class:interactive class:zoomed class:erasing={eraser}
  bind:this={el}
  on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onCancel}
  on:pointerenter={onEnter} on:pointerleave={onLeave}
>
  {#if useGL}
    <canvas
      bind:this={canvas} class:anim={animating}
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
    ></canvas>
    {#if !id}<div class="hint">…</div>{/if}
  {:else if src}
    <img
      {src} alt="preview" draggable="false" class:anim={animating}
      style="position:absolute; width:{dispW}px; height:{dispH}px; left:{left}px; top:{top}px;"
    />
  {:else}<div class="hint">…</div>{/if}
  <SpinOverlay bind:this={spinOverlay} />
  {#if eraser && hovering}
    <div class="brush" style="left:{curX}px; top:{curY}px; width:{cursorR * 2}px; height:{cursorR * 2}px;"></div>
  {/if}
  {#if id && interactive}<div class="zoom">{label}</div>{/if}
</div>

<style>
  .vp { position: relative; width: 100%; height: 100%; overflow: hidden; user-select: none;
    border-radius: 10px; }
  .vp.interactive { cursor: zoom-in; }
  .vp.zoomed { cursor: grab; }
  .vp.zoomed:active { cursor: grabbing; }
  img, canvas { display: block; will-change: left, top, width, height; }
  img.anim, canvas.anim { transition: left 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    top 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    width 180ms cubic-bezier(0.22, 0.61, 0.36, 1),
    height 180ms cubic-bezier(0.22, 0.61, 0.36, 1); }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
  .zoom { position: absolute; bottom: 8px; right: 10px; font-size: 11px; color: var(--text-dim);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; z-index: 2; }
  .vp.erasing { cursor: none; }
  .brush { position: absolute; border-radius: 50%; pointer-events: none; z-index: 3;
    transform: translate(-50%, -50%); border: 1.5px solid rgba(255,255,255,0.9);
    box-shadow: 0 0 0 1px rgba(0,0,0,0.5), inset 0 0 0 1px rgba(0,0,0,0.4); }
</style>
