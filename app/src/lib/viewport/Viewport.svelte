<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { api, type InvertParams } from "../api";
  import type { IrRemoval } from "../api";
  import { previewSrc } from "../store";
  import { FinishRenderer, webgl2Available, float16RenderTargetSupported } from "./gl/renderer";
  import { finishUniforms } from "./gl/uniforms";
  import { toInversionUniforms } from "./gl/invert";
  import { toneLutBytes, colorGrade } from "../develop/finish";
  import { screenRadius, type DustStroke } from "../develop/dust";
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
  /** Brush radius normalized to image width. */
  export let brush = 0.03;
  /** Committed strokes for this image (rendered by the backend). */
  export let dust: DustStroke[] = [];
  /** Bumped by the parent on any dust change to force a re-render. */
  export let dustRev = 0;
  export let irRemoval: IrRemoval = { enabled: false, sensitivity: 50 };

  const dispatch = createEventDispatcher<{ stroke: DustStroke; brush: number }>();

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

  // GPU upload state. The working buffer is uploaded to the GPU as a float texture;
  // inversion + geometry then update via uniforms. In bake mode the upload re-fires
  // when strokes/geometry change (keyed by uploadKey); otherwise once per image.
  let uploadKey = "";
  let texW = 0, texH = 0;

  $: ready = imgW > 0 && imgH > 0 && vpW > 0 && vpH > 0;
  $: pad = interactive ? PAD : 0;
  $: avW = Math.max(1, vpW - 2 * pad);
  $: avH = Math.max(1, vpH - 2 * pad);
  $: fit = ready ? Math.min(avW / imgW, avH / imgH) : 0;
  $: eff = interactive ? (scale > 0 ? scale : fit) : fit;
  $: zoomed = interactive && eff > fit + 1e-6;
  $: label = eff <= fit + 1e-6 ? $t("viewport.fit") : $t("viewport.zoomPercent", { percent: Math.round(eff * 100) });

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
    const v = float16RenderTargetSupported();
    console.log(`[SPIKE float16] ok=${v.ok} reason="${v.reason}"`);
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
    if (gpuEligible) return; // GPU path owns eligible images; this is the CPU fallback only
    if (!id || !imgW || !vpW) { src = ""; return; }
    const rscale = Math.min(eff, CAP / Math.max(imgW, imgH));
    const out_w = Math.max(1, Math.round(imgW * rscale));
    const out_h = Math.max(1, Math.round(imgH * rscale));
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
    }
  }

  function drawGL() {
    if (!renderer) return;
    renderer.setUniforms(finishUniforms(params));
    renderer.setLut(toneLutBytes(params));
    renderer.setColorGrade(colorGrade(params));
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

  // GPU path: WebGL2 available + non-raw. Dust/IR now stay on the GPU via a baked
  // (geometry + pre-invert heal) working texture; only raw/no-WebGL2 fall to CPU.
  $: gpuEligible = !!(useGL && renderer && !raw);
  // Bake mode: dust/IR active → request the baked working texture + identity geometry.
  $: bakeMode = dust.length > 0 || irRemoval.enabled;

  // Key the uploaded working texture. In bake mode it depends on dust strokes + the
  // baked geometry (re-bake on commit/geometry change); else just the image id.
  // Contract: the parent must bump `dustRev` on any change to `dust` (it's the proxy
  // for stroke changes here — the dust array itself is not in the key).
  function currentUploadKey(): string {
    if (bakeMode) {
      return `bake|${id}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
    }
    return `raw|${id}`;
  }

  // Upload the working float texture to the GPU. In bake mode, fetch the BAKED
  // (geometry + pre-invert heal) buffer; else the raw working buffer. Sets uniforms
  // + draws after upload. Re-fetches only when the upload key changes.
  async function uploadWorking() {
    if (!gpuEligible || !id || !renderer) return;
    const key = currentUploadKey();
    if (uploadKey === key) return; // already on the GPU for these inputs
    const k = key;
    if (bakeMode) {
      const spec = {
        rot90, flip_h: flipH, flip_v: flipV, angle,
        image_crop: imageCrop, dust, ir_removal: irRemoval,
      };
      const info = await api.workingBakedInfo(id, spec);
      const buf = await api.workingBakedPixels(id, spec);
      if (!renderer || currentUploadKey() !== k) return; // stale (params changed mid-fetch)
      renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h);
      texW = info.w; texH = info.h;
    } else {
      const info = await api.workingInfo(id);
      const buf = await api.workingPixels(id);
      if (!renderer || currentUploadKey() !== k) return; // image changed mid-fetch
      renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h);
      texW = info.w; texH = info.h;
    }
    uploadKey = k;
    await refreshInversion();
    applyGeometryAndDraw();
  }

  // Resolve inversion params (+ sampled base) into GPU uniforms — no fetch of pixels.
  async function refreshInversion() {
    if (!gpuEligible || !id || !renderer) return;
    const curId = id;
    const res = await api.resolvedInversion(id, params);
    if (id !== curId || !renderer) return;
    renderer.setInversion(toInversionUniforms(res));
  }

  // Map orient/flip/straighten/persistent-crop into GPU geometry uniforms, then draw.
  function applyGeometryAndDraw() {
    if (!gpuEligible || !renderer || texW === 0) return; // no texture uploaded yet
    // Bake mode: geometry is already baked into the texture → identity + baked dims.
    if (bakeMode) {
      renderer.setGeometry({
        crop_off: [0, 0], crop_scale: [1, 1], angle: 0,
        orient: [1, 0, 0, 1], raw, outW: texW, outH: texH,
      });
      drawGL();
      return;
    }
    // orient as a 2x2 on centred UV: rot90 (clockwise) + flips.
    const ang = (rot90 % 4) * Math.PI / 2;
    const s = Math.sin(ang), c = Math.cos(ang);
    let o = [c, -s, s, c]; // rotation
    const fx = flipH ? -1 : 1, fy = flipV ? -1 : 1;
    o = [o[0] * fx, o[1] * fy, o[2] * fx, o[3] * fy];
    // persistent crop in source UV (imageCrop is normalized [x,y,w,h] or null).
    const [cropX, cropY, cropW, cropH] = imageCrop ?? [0, 0, 1, 1];
    // output canvas = oriented+cropped aspect; for odd rot90, swap dims.
    const baseW = texW * cropW, baseH = texH * cropH;
    const swap = (rot90 % 2) === 1;
    const outW = Math.max(1, Math.round(swap ? baseH : baseW));
    const outH = Math.max(1, Math.round(swap ? baseW : baseH));
    renderer.setGeometry({
      crop_off: [cropX, cropY], crop_scale: [cropW, cropH],
      angle: (angle * Math.PI) / 180, orient: o as [number, number, number, number],
      raw, outW, outH,
    });
    drawGL();
  }

  // Upload the working float texture. Re-fires when the image changes or, in bake
  // mode, when strokes/IR/geometry change (currentUploadKey dedupes redundant runs).
  $: if (gpuEligible) { id; dustRev; irRemoval.enabled; irRemoval.sensitivity; imageCrop; rot90; flipH; flipV; angle; uploadWorking(); }
  $: if (!gpuEligible) uploadKey = "";

  // Inversion params now drive GPU uniforms (no backend pixel fetch) when eligible.
  $: invKey = `${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${params.black}|${params.gamma}`;
  $: if (gpuEligible) { invKey; refreshInversion().then(applyGeometryAndDraw); }

  // Geometry also drives GPU uniforms (no fetch) when eligible.
  $: geomKey = `${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
  // Raw-mode GPU geometry only: in bake mode geometry is baked into the texture and
  // the upload trigger handles re-draws, so this would otherwise double-draw.
  $: if (gpuEligible && !bakeMode) { geomKey; applyGeometryAndDraw(); }

  // CPU fallback path: re-fetch the SOURCE from the backend only when NOT eligible
  // (dust/IR active, raw view, or no WebGL2). Reuses the existing render()/schedule.
  $: cpuKey = gpuEligible ? '' :
    `${id}|${raw}|${eff}|${vpW}|${vpH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}`;
  $: cpuKey, imgW, imgH, scheduleIfReady();

  // Finishing-only change → GPU redraw, no backend fetch. Tone curve + color
  // grading are all finishing-layer controls, so they live here too.
  $: finishKey = [
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
  ].join("|");
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
    if (e.button !== 0) return; // ignore right/middle click — let the context menu open
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
    if (e.button !== 0) return; // right/middle click never triggers tap-to-zoom
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
  function onCancel() { painting = false; pending = []; panning = false; moved = false; }
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
