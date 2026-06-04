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
    try {
      src = await api.renderView(id, params, v);
      // New bitmap matches the committed view → drop the live pan transform.
      tx = 0; ty = 0;
    } catch { /* keep previous frame */ }
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

  function onWheel(e: WheelEvent) {
    if (!interactive) return;
    e.preventDefault();
    const [ix, iy] = imgPoint(e);
    const ns = Math.min(8, Math.max(fit, scale * Math.exp(-e.deltaY * 0.0015)));
    cx = ix + (cx - ix) * (scale / ns);
    cy = iy + (cy - iy) * (scale / ns);
    scale = ns;
  }

  // Unified pointer gesture: a tap toggles zoom, a drag pans. During a pan we
  // apply an instant CSS translate (tx,ty) to the current bitmap for smooth
  // feedback, then commit to cx/cy + render on release (transform cleared when
  // the new bitmap arrives, so there's no jump).
  let lastX = 0, lastY = 0, downX = 0, downY = 0, moved = false, panning = false;
  let tx = 0, ty = 0;
  function onDown(e: PointerEvent) {
    if (!interactive) return;
    downX = lastX = e.clientX; downY = lastY = e.clientY;
    moved = false;
    panning = zoomed; // only pan when already zoomed in
    (e.target as Element).setPointerCapture?.(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!interactive || !(e.buttons & 1)) return;
    if (Math.abs(e.clientX - downX) > 3 || Math.abs(e.clientY - downY) > 3) moved = true;
    if (panning && moved) {
      tx += e.clientX - lastX;
      ty += e.clientY - lastY;
    }
    lastX = e.clientX; lastY = e.clientY;
  }
  function onUp(e: PointerEvent) {
    if (!interactive) return;
    if (panning && moved) {
      // commit the live translate into the image-space centre, keep tx/ty until
      // the re-render lands (render() clears them) to avoid a flash-back.
      cx -= tx / scale;
      cy -= ty / scale;
    } else if (!moved) {
      const [ix, iy] = imgPoint(e);
      if (zoomed) { scale = fit; cx = imgW / 2; cy = imgH / 2; }
      else { scale = 1.0; cx = ix; cy = iy; }
    }
    panning = false; moved = false;
  }
  function onCancel() { tx = 0; ty = 0; panning = false; moved = false; }
</script>

<div
  class="vp" class:interactive class:zoomed
  bind:this={el}
  on:wheel={onWheel}
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onCancel}
>
  {#if src}<img {src} alt="preview" draggable="false" style="transform: translate({tx}px, {ty}px)" />{:else}<div class="hint">…</div>{/if}
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
