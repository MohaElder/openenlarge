<script lang="ts">
  import { onMount } from "svelte";
  import { api, type InvertParams } from "../api";
  import { deriveView, fitScale } from "./view";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;
  export let imgH = 0;
  export let raw = false;
  export let interactive = true;

  let el: HTMLDivElement;
  let src = "";
  let vpW = 0, vpH = 0;
  let scale = 0;
  let cx = 0, cy = 0;
  let prevId: string | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  $: fit = fitScale(imgW, imgH, vpW, vpH);
  $: zoomed = interactive && scale > fit + 1e-6;
  $: label = scale <= fit + 1e-6 ? "Fit" : Math.round(scale * 100) + "%";

  function measure() {
    if (!el) return;
    vpW = el.clientWidth; vpH = el.clientHeight;
    if (scale === 0 && imgW) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
  }

  onMount(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    return () => ro.disconnect();
  });

  $: if (id !== prevId) { prevId = id; scale = fit || 1; cx = imgW / 2; cy = imgH / 2; }

  async function render() {
    if (!id || !imgW || !vpW) { src = ""; return; }
    const v = deriveView(interactive ? scale : fit, cx, cy, imgW, imgH, vpW, vpH, raw);
    try { src = await api.renderView(id, params, v); } catch { /* keep previous frame */ }
  }
  function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(render, 100); }

  function maybeRender() {
    if (id && vpW && imgW) schedule();
  }
  // Re-render whenever any of these change (listed as a reactive dependency
  // sequence so their *values* aren't used as a gating condition).
  $: id, vpW, imgH, imgW, params, scale, cx, cy, raw, maybeRender();

  function imgPoint(e: { clientX: number; clientY: number }): [number, number] {
    const v = deriveView(scale, cx, cy, imgW, imgH, vpW, vpH);
    const rect = el.getBoundingClientRect();
    const offX = (vpW - v.out_w) / 2, offY = (vpH - v.out_h) / 2;
    const px = (e.clientX - rect.left - offX) / v.out_w;
    const py = (e.clientY - rect.top - offY) / v.out_h;
    return [v.crop[0] + px * v.crop[2], v.crop[1] + py * v.crop[3]];
  }

  function onClick(e: MouseEvent) {
    if (!interactive) return;
    const [ix, iy] = imgPoint(e);
    if (zoomed) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
    else { scale = 1.0; cx = ix; cy = iy; }
  }

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
    e.preventDefault();
    const [ix, iy] = imgPoint(e);
    const ns = Math.min(8, Math.max(fit, scale * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (scale / ns);
    cy = iy + (cy - iy) * (scale / ns);
    scale = ns;
  }

  let dragging = false, lastX = 0, lastY = 0;
  function onDown(e: PointerEvent) {
    if (!zoomed) return;
    dragging = true; lastX = e.clientX; lastY = e.clientY;
    (e.target as Element).setPointerCapture?.(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!dragging) return;
    cx -= (e.clientX - lastX) / scale;
    cy -= (e.clientY - lastY) / scale;
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp() { dragging = false; }
</script>

<div
  class="vp" class:interactive class:zoomed
  bind:this={el}
  on:click={onClick} on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointerleave={onUp}
>
  {#if src}<img {src} alt="preview" draggable="false" />{:else}<div class="hint">…</div>{/if}
  {#if id && interactive}<div class="zoom">{label}</div>{/if}
</div>

<style>
  .vp { position: relative; width: 100%; height: 100%; display: grid; place-items: center;
    overflow: hidden; user-select: none; }
  .vp.interactive { cursor: zoom-in; }
  .vp.zoomed { cursor: grab; }
  .vp.zoomed:active { cursor: grabbing; }
  img { max-width: 100%; max-height: 100%; object-fit: contain; border-radius: 10px; display: block; }
  .hint { color: var(--text-dim); }
  .zoom { position: absolute; bottom: 8px; right: 10px; font-size: 11px; color: var(--text-dim);
    background: rgba(0,0,0,0.45); padding: 2px 8px; border-radius: 6px; }
</style>
