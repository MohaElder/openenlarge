<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { t } from "$lib/i18n";

  export let label = "";
  export let hue = 0;   // 0..360
  export let sat = 0;   // 0..100
  export let lum = 0;   // −100..100

  const dispatch = createEventDispatcher<{ change: { hue: number; sat: number; lum: number } }>();

  let disc: HTMLDivElement;
  let dragging = false;

  // Wheel uses CSS conic-gradient(from 90deg) which runs clockwise (y-down) from
  // the right. Stored hue is standard HSV (0=red, 120=green, 240=blue), so the
  // gradient shows hue = (360 − g). Invert both ways to keep thumb ↔ color aligned.
  const toAngle = (h: number) => (((360 - h) % 360) * Math.PI) / 180; // radians, y-down
  function fromAngle(rad: number): number {
    let g = (rad * 180) / Math.PI;
    g = ((g % 360) + 360) % 360;
    return (360 - g) % 360;
  }

  $: a = toAngle(hue);
  $: tx = 50 + Math.cos(a) * (sat / 100) * 50; // % of disc
  $: ty = 50 + Math.sin(a) * (sat / 100) * 50;
  $: thumbColor = `hsl(${hue}, ${sat}%, 50%)`;

  function pick(e: PointerEvent) {
    const r = disc.getBoundingClientRect();
    const dx = (e.clientX - r.left) / r.width - 0.5;
    const dy = (e.clientY - r.top) / r.height - 0.5;
    const dist = Math.hypot(dx, dy) * 2; // 0..~1 across the radius
    hue = fromAngle(Math.atan2(dy, dx));
    sat = Math.min(1, dist) * 100;
    dispatch("change", { hue, sat, lum });
  }
  function onDown(e: PointerEvent) { dragging = true; disc.setPointerCapture(e.pointerId); pick(e); }
  function onMove(e: PointerEvent) { if (dragging) pick(e); }
  function onUp() { dragging = false; }
  function reset() { hue = 0; sat = 0; dispatch("change", { hue, sat, lum }); }

  function onLum(e: Event) {
    lum = Number((e.target as HTMLInputElement).value);
    dispatch("change", { hue, sat, lum });
  }
  function resetLum() { lum = 0; dispatch("change", { hue, sat, lum }); }
</script>

<div class="wheel">
  {#if label}<div class="title">{label}</div>{/if}
  <div
    class="disc" bind:this={disc}
    on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onUp}
    on:dblclick={reset}
    role="application" aria-label={$t('colorWheel.hueSatAriaLabel', { label, hue: Math.round(hue), sat: Math.round(sat) })}
  >
    <div class="thumb" style="left:{tx}%; top:{ty}%; background:{thumbColor}"></div>
  </div>
  <input class="lum" type="range" min="-100" max="100" step="1"
    value={lum} on:input={onLum} on:dblclick={resetLum} aria-label={$t('colorWheel.luminanceAriaLabel', { label })} />
</div>

<style>
  .wheel { display: flex; flex-direction: column; align-items: center; gap: 6px; }
  .title { font-size: 11px; color: var(--text-dim); }
  .disc { position: relative; width: 100%; max-width: 120px; aspect-ratio: 1 / 1;
    border-radius: 50%; cursor: crosshair; touch-action: none;
    background:
      radial-gradient(circle at center, #fff 0%, rgba(255, 255, 255, 0) 70%),
      conic-gradient(from 90deg, #f00, #f0f, #00f, #0ff, #0f0, #ff0, #f00);
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.4); }
  .thumb { position: absolute; width: 12px; height: 12px; border-radius: 50%;
    transform: translate(-50%, -50%); border: 2px solid #fff;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6); pointer-events: none; }
  .lum { width: 100%; max-width: 120px; height: 3px; border-radius: 3px;
    -webkit-appearance: none; appearance: none; background: var(--glass-brd); accent-color: var(--accent); }
  .lum::-webkit-slider-thumb { -webkit-appearance: none; width: 11px; height: 11px;
    border-radius: 50%; background: #fff; border: 1px solid rgba(0, 0, 0, 0.3); cursor: grab; }
</style>
