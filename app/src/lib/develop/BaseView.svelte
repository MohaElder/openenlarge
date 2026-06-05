<script lang="ts">
  import { onMount, createEventDispatcher } from "svelte";
  import { api, type InvertParams } from "../api";
  import type { ScreenRect } from "../crop/cropMath";

  export let id: string | null;
  export let params: InvertParams;
  export let imgW = 0;   // working-image dims (uncropped, oriented identity)
  export let imgH = 0;

  const dispatch = createEventDispatcher<{ sampled: [number, number, number] }>();
  const PAD = 60, CAP = 4000;
  // Sampled patch as a fraction of image WIDTH (~1%); height matched for square px.
  // Click samples a small averaged patch (robust to grain/dust) rather than one pixel.
  const PATCH = 0.01;
  let el: HTMLDivElement;
  let src = "";
  let vpW = 0, vpH = 0;
  let mark: { x: number; y: number } | null = null; // last sampled point (normalized)

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

  const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

  // Sample a small averaged patch centered on a normalized point in the raw negative.
  async function sampleAt(nx: number, ny: number) {
    if (!id) return;
    const w = PATCH;
    const h = imgH > 0 ? PATCH * (imgW / imgH) : PATCH; // square in pixels
    const x = clamp01(nx - w / 2);
    const y = clamp01(ny - h / 2);
    try {
      const b = await api.sampleBaseAt(id, [x, y, Math.min(w, 1 - x), Math.min(h, 1 - y)]);
      dispatch("sampled", b);
    } catch { /* ignore */ }
  }

  function onClick(e: MouseEvent) {
    if (!dispW || !dispH) return;
    const r = el.getBoundingClientRect();
    const nx = (e.clientX - r.left - imgScreen.left) / dispW;
    const ny = (e.clientY - r.top - imgScreen.top) / dispH;
    if (nx < 0 || nx > 1 || ny < 0 || ny > 1) return; // clicked outside the negative
    mark = { x: nx, y: ny };
    sampleAt(nx, ny);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions a11y_click_events_have_key_events -->
<div class="basevp" bind:this={el} on:click={onClick}>
  {#if src}
    <img {src} alt="negative" draggable="false"
      style="position:absolute; left:{imgScreen.left}px; top:{imgScreen.top}px; width:{dispW}px; height:{dispH}px;" />
    {#if mark}
      <div class="mark" style="left:{imgScreen.left + mark.x * dispW}px; top:{imgScreen.top + mark.y * dispH}px;"></div>
    {/if}
  {:else}<div class="hint">…</div>{/if}
</div>

<style>
  .basevp { position: relative; width: 100%; height: 100%; overflow: hidden;
    border-radius: 10px; user-select: none; cursor: crosshair; }
  .hint { color: var(--text-dim); position: absolute; inset: 0; display: grid; place-items: center; }
  .mark { position: absolute; width: 14px; height: 14px; transform: translate(-50%, -50%);
    border: 2px solid rgba(120,220,255,0.95); border-radius: 50%;
    box-shadow: 0 0 0 1px rgba(0,0,0,0.5); pointer-events: none; }
</style>
