<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { sampleCurve } from "./curve";
  import type { CurvePoint } from "../api";
  import { previewSrc } from "../store";
  import { binPixels, channelPath } from "../viewport/histogram";

  /** Control points in [0,1]×[0,1] (input → output), endpoints at x=0 and x=1. */
  export let points: CurvePoint[];
  /** Stroke color of the curve line. */
  export let color = "#e8e8e8";
  /** Which histogram channels to draw behind the curve. */
  export let hist: ("r" | "g" | "b")[] = ["r", "g", "b"];

  const dispatch = createEventDispatcher<{ change: CurvePoint[] }>();
  const S = 256; // viewBox size (square)
  const HIT_PX = 9; // grab radius in screen pixels (size-independent; ~dot radius)

  let svgEl: SVGSVGElement;
  let dragIdx = -1;
  let moved = false;

  // Local working copy; resync from the prop whenever we're not dragging.
  let pts: CurvePoint[] = points;
  $: if (dragIdx < 0) pts = points.map((p) => [...p] as CurvePoint);

  const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);
  const sx = (x: number) => x * S;
  const sy = (y: number) => (1 - y) * S;

  $: curveD = (() => {
    let d = "";
    const N = 72;
    for (let i = 0; i <= N; i++) {
      const x = i / N;
      const y = sampleCurve(pts, x);
      d += (i === 0 ? "M" : "L") + sx(x).toFixed(1) + " " + sy(y).toFixed(1);
    }
    return d;
  })();

  // --- Histogram behind the grid (decoded from the live preview). ---
  let rPath = "", gPath = "", bPath = "";
  let htimer: ReturnType<typeof setTimeout> | null = null;
  const hcv = typeof document !== "undefined" ? document.createElement("canvas") : null;
  function computeHist(src: string) {
    if (!src || !hcv) { rPath = gPath = bPath = ""; return; }
    const img = new Image();
    img.onload = () => {
      const w = 256, h = Math.max(1, Math.round((img.height / img.width) * 256));
      hcv.width = w; hcv.height = h;
      const ctx = hcv.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(img, 0, 0, w, h);
      const bins = binPixels(ctx.getImageData(0, 0, w, h).data);
      rPath = channelPath(bins.r, S, S);
      gPath = channelPath(bins.g, S, S);
      bPath = channelPath(bins.b, S, S);
    };
    img.src = src;
  }
  $: { const s = $previewSrc; if (htimer) clearTimeout(htimer); htimer = setTimeout(() => computeHist(s), 140); }

  function toLocal(e: PointerEvent): CurvePoint {
    const r = svgEl.getBoundingClientRect();
    return [clamp01((e.clientX - r.left) / r.width), clamp01(1 - (e.clientY - r.top) / r.height)];
  }
  /** Nearest control point within HIT_PX screen pixels, or -1 to add a new one. */
  function hitIndex(p: CurvePoint, rect: DOMRect): number {
    let best = -1, bd = HIT_PX;
    for (let i = 0; i < pts.length; i++) {
      const d = Math.hypot((pts[i][0] - p[0]) * rect.width, (pts[i][1] - p[1]) * rect.height);
      if (d < bd) { bd = d; best = i; }
    }
    return best;
  }

  function commit() { dispatch("change", pts.map((p) => [...p] as CurvePoint)); }

  function onDown(e: PointerEvent) {
    const p = toLocal(e);
    let idx = hitIndex(p, svgEl.getBoundingClientRect());
    if (idx < 0) {
      // Insert a new interior point, keep sorted by x.
      pts = [...pts, p].sort((a, b) => a[0] - b[0]);
      idx = pts.findIndex((q) => q[0] === p[0] && q[1] === p[1]);
    }
    dragIdx = idx;
    moved = false;
    svgEl.setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (dragIdx < 0) return;
    moved = true;
    const [nx, ny] = toLocal(e);
    const last = pts.length - 1;
    const isEnd = dragIdx === 0 || dragIdx === last;
    let x = pts[dragIdx][0];
    if (!isEnd) {
      const lo = pts[dragIdx - 1][0] + 1e-3;
      const hi = pts[dragIdx + 1][0] - 1e-3;
      x = Math.min(hi, Math.max(lo, nx));
    }
    pts[dragIdx] = [x, ny];
    pts = pts; // trigger reactivity
    commit();
  }
  function onUp(e: PointerEvent) {
    if (dragIdx < 0) return;
    const last = pts.length - 1;
    const isEnd = dragIdx === 0 || dragIdx === last;
    const [, ly] = toLocal(e);
    const r = svgEl.getBoundingClientRect();
    const outY = (e.clientY < r.top - 16) || (e.clientY > r.bottom + 16);
    void ly;
    // Drag an interior point off the top/bottom to delete it.
    if (!isEnd && moved && outY && pts.length > 2) {
      pts = pts.filter((_, i) => i !== dragIdx);
      commit();
    }
    dragIdx = -1;
  }
  function onDblPoint(i: number) {
    const last = pts.length - 1;
    if (i === 0 || i === last || pts.length <= 2) return; // keep endpoints
    pts = pts.filter((_, idx) => idx !== i);
    commit();
  }
</script>

<svg
  bind:this={svgEl} class="curve" viewBox="0 0 {S} {S}" preserveAspectRatio="none"
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onUp}
  role="application" aria-label="Tone curve editor"
>
  <!-- grid -->
  {#each [0.25, 0.5, 0.75] as g}
    <line x1={g * S} y1="0" x2={g * S} y2={S} class="grid" />
    <line x1="0" y1={g * S} x2={S} y2={g * S} class="grid" />
  {/each}
  <line x1="0" y1={S} x2={S} y2="0" class="diag" />
  <!-- histogram -->
  {#if hist.includes("r")}<polyline points={rPath} class="hr" />{/if}
  {#if hist.includes("g")}<polyline points={gPath} class="hg" />{/if}
  {#if hist.includes("b")}<polyline points={bPath} class="hb" />{/if}
  <!-- curve -->
  <path d={curveD} class="line" style="stroke:{color}" />
  {#each pts as p, i}
    <circle cx={sx(p[0])} cy={sy(p[1])} r="5" class="pt" style="fill:{color}"
      on:dblclick={() => onDblPoint(i)} role="button" tabindex="-1" aria-label="curve point" />
  {/each}
</svg>

<style>
  .curve { width: 100%; aspect-ratio: 1 / 1; display: block; border-radius: 8px;
    background: rgba(0, 0, 0, 0.35); touch-action: none; cursor: crosshair; }
  .grid { stroke: rgba(255, 255, 255, 0.08); stroke-width: 1; }
  .diag { stroke: rgba(255, 255, 255, 0.14); stroke-width: 1; }
  .line { fill: none; stroke-width: 2; }
  polyline { fill: none; stroke-width: 1; mix-blend-mode: screen; opacity: 0.5; }
  .hr { stroke: #ff5a5a; } .hg { stroke: #5aff7a; } .hb { stroke: #5a9cff; }
  .pt { stroke: rgba(0, 0, 0, 0.5); stroke-width: 1; cursor: grab; }
  .pt:active { cursor: grabbing; }
</style>
