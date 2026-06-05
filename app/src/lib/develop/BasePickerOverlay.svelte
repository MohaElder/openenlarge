<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import type { Rect, Handle } from "../crop/types";
  import { toScreen, handleAt, applyDrag, type ScreenRect } from "../crop/cropMath";

  export let rect: Rect;       // bound draft, normalized to the displayed image
  export let img: ScreenRect;  // displayed image rect, container px

  const dispatch = createEventDispatcher<{ change: Rect }>();
  let host: HTMLDivElement;
  let active: Handle = null;
  let startRect: Rect = rect;
  let startX = 0, startY = 0;

  $: box = toScreen(rect, img);

  function localXY(e: PointerEvent): [number, number] {
    const r = host.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }
  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const [px, py] = localXY(e);
    const h = handleAt(px, py, box, 12);
    const inside = px > box.left && px < box.left + box.width && py > box.top && py < box.top + box.height;
    active = h ?? (inside ? "move" : null);
    if (!active) return;
    startRect = rect; startX = px; startY = py;
    host.setPointerCapture(e.pointerId);
  }
  function onMove(e: PointerEvent) {
    if (!active) return;
    const [px, py] = localXY(e);
    const dnx = (px - startX) / Math.max(1, img.width);
    const dny = (py - startY) / Math.max(1, img.height);
    rect = applyDrag(active, startRect, dnx, dny, null);
    dispatch("change", rect);
  }
  function onUp() { if (active) { active = null; dispatch("change", rect); } }
</script>

<div bind:this={host} class="overlay"
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onUp}>
  <div class="frame" style="left:{box.left}px; top:{box.top}px; width:{box.width}px; height:{box.height}px"></div>
  {#each [["nw",box.left,box.top],["ne",box.left+box.width,box.top],["sw",box.left,box.top+box.height],["se",box.left+box.width,box.top+box.height]] as b}
    <div class="bracket" style="left:{b[1]}px; top:{b[2]}px"></div>
  {/each}
</div>

<style>
  .overlay { position: absolute; inset: 0; user-select: none; touch-action: none; cursor: crosshair; }
  .frame { position: absolute; border: 1.5px solid rgba(120,220,255,0.95);
    box-shadow: 0 0 0 1px rgba(0,0,0,0.5); box-sizing: border-box; }
  .bracket { position: absolute; width: 12px; height: 12px; transform: translate(-50%,-50%);
    border-radius: 2px; background: rgba(120,220,255,0.95); box-shadow: 0 0 2px rgba(0,0,0,0.6); }
</style>
