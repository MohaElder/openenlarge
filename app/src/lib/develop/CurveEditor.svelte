<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { t } from "$lib/i18n";
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
  let moved = false;
  let active = false; // true once a press becomes a real drag (point or segment)
  // Gesture state: a press arms a candidate; it only becomes a drag once the
  // pointer moves past DRAG_PX. A press that never moves is a tap (adds a point).
  const DRAG_PX = 4;
  let downPt: CurvePoint | null = null;
  let downClientX = 0, downClientY = 0;
  let hadHit = false;
  // Drag target. "point": move a single control point (you grabbed it directly).
  // "segment": move both control points bounding the grabbed span, weighted by t.
  let dragMode: "point" | "segment" = "point";
  let dragIdx = -1;            // point mode: which point
  let grabOffset: CurvePoint = [0, 0]; // point mode: press − point, so drags don't teleport
  let segL = 0, segR = 1, segT = 0;    // segment mode: bounding indices + position along
  let startYL = 0, startYR = 0, startCy = 0; // segment mode: y's at grab + grab cursor y

  // Local working copy; resync from the prop whenever we're not actively dragging.
  let pts: CurvePoint[] = points;
  $: if (!active) pts = points.map((p) => [...p] as CurvePoint);

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
    const hit = hitIndex(p, svgEl.getBoundingClientRect());
    hadHit = hit >= 0;
    if (hadHit) {
      // Grabbed a control point directly → move just that point.
      dragMode = "point";
      dragIdx = hit;
      grabOffset = [p[0] - pts[hit][0], p[1] - pts[hit][1]];
    } else {
      // Grabbed the curve between points → move the whole bounding segment.
      dragMode = "segment";
      let i = 0;
      while (i < pts.length - 2 && p[0] > pts[i + 1][0]) i++;
      segL = i; segR = i + 1;
      const span = pts[segR][0] - pts[segL][0];
      segT = span > 1e-6 ? (p[0] - pts[segL][0]) / span : 0.5;
      startYL = pts[segL][1]; startYR = pts[segR][1]; startCy = p[1];
    }
    downPt = p;
    downClientX = e.clientX; downClientY = e.clientY;
    moved = false;
    active = false; // not a drag until the pointer moves past the threshold
    svgEl.setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!downPt) return;
    if (!moved) {
      if (Math.hypot(e.clientX - downClientX, e.clientY - downClientY) < DRAG_PX) return;
      moved = true; active = true;
    }
    const [cx, cy] = toLocal(e);
    if (dragMode === "point") {
      // Apply the grab offset so the point tracks the cursor without jumping to it.
      const nx = clamp01(cx - grabOffset[0]);
      const ny = clamp01(cy - grabOffset[1]);
      const last = pts.length - 1;
      const isEnd = dragIdx === 0 || dragIdx === last;
      let x = pts[dragIdx][0];
      if (!isEnd) {
        const lo = pts[dragIdx - 1][0] + 1e-3;
        const hi = pts[dragIdx + 1][0] - 1e-3;
        x = Math.min(hi, Math.max(lo, nx));
      }
      pts[dragIdx] = [x, ny];
    } else {
      // Move both bounding points in y, weighted by where along the segment we grabbed
      // (nearer end moves more), so the segment translates instead of pivoting.
      const dy = cy - startCy;
      pts[segL] = [pts[segL][0], clamp01(startYL + dy * (1 - segT))];
      pts[segR] = [pts[segR][0], clamp01(startYR + dy * segT)];
    }
    pts = pts; // trigger reactivity
    commit();
  }
  function onUp(e: PointerEvent) {
    if (!downPt) return;
    if (!moved) {
      // Tap (no drag): add a new control point — unless the tap landed on one.
      if (!hadHit) {
        pts = [...pts, downPt].sort((a, b) => a[0] - b[0]);
        commit();
      }
    } else if (dragMode === "point") {
      // Drag an interior point off the top/bottom to delete it.
      const last = pts.length - 1;
      const isEnd = dragIdx === 0 || dragIdx === last;
      const r = svgEl.getBoundingClientRect();
      const outY = (e.clientY < r.top - 16) || (e.clientY > r.bottom + 16);
      if (!isEnd && outY && pts.length > 2) {
        pts = pts.filter((_, i) => i !== dragIdx);
        commit();
      }
    }
    downPt = null; dragIdx = -1; moved = false; active = false;
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
  role="application" aria-label={$t('curve.editorAriaLabel')}
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
      on:dblclick={() => onDblPoint(i)} role="button" tabindex="-1" aria-label={$t('curve.pointAriaLabel')} />
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
