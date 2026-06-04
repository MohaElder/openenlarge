<script lang="ts">
  import { onMount } from "svelte";
  import { api, type InvertParams } from "../api";
  import type { Rect } from "./types";
  import CropOverlay from "./CropOverlay.svelte";
  import type { ScreenRect } from "./cropMath";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;
  export let imgH = 0;
  export let rect: Rect;          // bound draft rect
  export let lockRatio: number;

  const PAD = 60;
  const CAP = 5000;
  let el: HTMLDivElement;
  let src = "";
  let vpW = 0, vpH = 0;

  function measure() { if (el) { vpW = el.clientWidth; vpH = el.clientHeight; } }
  onMount(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (el) ro.observe(el);
    return () => ro.disconnect();
  });

  $: avW = Math.max(1, vpW - 2 * PAD);
  $: avH = Math.max(1, vpH - 2 * PAD);
  $: fit = imgW > 0 && imgH > 0 && vpW > 0 ? Math.min(avW / imgW, avH / imgH) : 0;
  $: dispW = imgW * fit;
  $: dispH = imgH * fit;
  $: imgScreen = { left: (vpW - dispW) / 2, top: (vpH - dispH) / 2, width: dispW, height: dispH } as ScreenRect;

  let lastKey = "";
  async function render() {
    if (!id || !imgW || !vpW) return;
    const rscale = Math.min(fit, CAP / Math.max(imgW, imgH));
    const out_w = Math.max(1, Math.round(imgW * rscale));
    const out_h = Math.max(1, Math.round(imgH * rscale));
    try {
      src = await api.renderView(id, params, {
        crop: [0, 0, imgW, imgH], out_w, out_h, raw: false, finish: true, image_crop: null,
      });
    } catch { /* keep last */ }
  }
  $: key = `${id}|${vpW}|${vpH}|${imgW}|${imgH}|${params.mode}|${params.stock}|${params.exposure}|${params.temp}|${params.tint}|${params.contrast}|${params.highlights}|${params.shadows}|${params.whites}|${params.blacks}|${params.texture}|${params.vibrance}|${params.saturation}`;
  $: if (key !== lastKey) { lastKey = key; render(); }
</script>

<div class="cropvp" bind:this={el}>
  {#if src}
    <img {src} alt="crop" draggable="false"
      style="position:absolute; left:{imgScreen.left}px; top:{imgScreen.top}px; width:{dispW}px; height:{dispH}px;" />
    <CropOverlay bind:rect img={imgScreen} {lockRatio} on:custom />
  {:else}<div class="hint">…</div>{/if}
</div>

<style>
  .cropvp { position: relative; width: 100%; height: 100%; overflow: hidden;
    border-radius: 10px; user-select: none; }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
</style>
