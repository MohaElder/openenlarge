<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { api, type InvertParams } from "../api";
  import type { Rect } from "../crop/types";
  import type { ScreenRect } from "../crop/cropMath";
  import BasePickerOverlay from "./BasePickerOverlay.svelte";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;   // working-image dims (uncropped, oriented identity)
  export let imgH = 0;

  const dispatch = createEventDispatcher<{ sampled: [number, number, number] }>();
  const PAD = 60, CAP = 4000;
  let el: HTMLDivElement;
  let src = "";
  let vpW = 0, vpH = 0;
  let rect: Rect = { x: 0.02, y: 0.02, w: 0.14, h: 0.14 };

  function measure() { if (el) { vpW = el.clientWidth; vpH = el.clientHeight; } }
  onMount(() => { measure(); const ro = new ResizeObserver(measure); if (el) ro.observe(el); return () => ro.disconnect(); });

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
        crop: [0, 0, imgW, imgH], out_w, out_h, raw: true, finish: false,
        image_crop: null, rot90: 0, flip_h: false, flip_v: false, angle: 0,
      });
    } catch { /* keep last */ }
  }
  $: key = `${id}|${vpW}|${vpH}|${imgW}|${imgH}`;
  $: if (key !== lastKey) { lastKey = key; render(); }

  let timer: ReturnType<typeof setTimeout> | null = null;
  async function sample() {
    if (!id) return;
    try {
      const b = await api.sampleBaseAt(id, [rect.x, rect.y, rect.w, rect.h]);
      dispatch("sampled", b);
    } catch { /* ignore */ }
  }
  function onChange() { if (timer) clearTimeout(timer); timer = setTimeout(sample, 120); }
  onMount(() => { sample(); });
</script>

<div class="basevp" bind:this={el}>
  {#if src}
    <img {src} alt="negative" draggable="false"
      style="position:absolute; left:{imgScreen.left}px; top:{imgScreen.top}px; width:{dispW}px; height:{dispH}px;" />
    <BasePickerOverlay bind:rect img={imgScreen} on:change={onChange} />
  {:else}<div class="hint">…</div>{/if}
</div>

<style>
  .basevp { position: relative; width: 100%; height: 100%; overflow: hidden;
    border-radius: 10px; user-select: none; }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
</style>
