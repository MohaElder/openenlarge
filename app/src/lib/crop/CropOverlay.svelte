<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import type { Rect, Handle } from "./types";
  import { toScreen, handleAt, applyDrag, type ScreenRect } from "./cropMath";

  export let rect: Rect;             // bound by the parent (draft)
  export let img: ScreenRect;        // displayed image rect, container px
  export let lockRatio: number;      // effective w/h; used when Shift is held

  const dispatch = createEventDispatcher<{ custom: void }>();

  let host: HTMLDivElement;
  let active: Handle = null;
  let startRect: Rect = rect;
  let startX = 0, startY = 0;
  let hover: Handle = null;

  $: box = toScreen(rect, img);
  $: vx = [box.left + box.width / 3, box.left + (2 * box.width) / 3];
  $: hy = [box.top + box.height / 3, box.top + (2 * box.height) / 3];

  const CURSOR: Record<string, string> = {
    move: "move", n: "ns-resize", s: "ns-resize", e: "ew-resize", w: "ew-resize",
    nw: "nwse-resize", se: "nwse-resize", ne: "nesw-resize", sw: "nesw-resize",
  };
  $: cursor = active ? CURSOR[active] : (hover ? CURSOR[hover] : "default");

  function localXY(e: PointerEvent): [number, number] {
    const r = host.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  }

  function onMove(e: PointerEvent) {
    const [px, py] = localXY(e);
    if (!active) { hover = handleAt(px, py, box, 12); return; }
    const dnx = (px - startX) / Math.max(1, img.width);
    const dny = (py - startY) / Math.max(1, img.height);
    const lock = e.shiftKey ? lockRatio : null;
    rect = applyDrag(active, startRect, dnx, dny, lock);
    if (active !== "move" && lock == null) dispatch("custom");
  }
  function onDown(e: PointerEvent) {
    const [px, py] = localXY(e);
    const h = handleAt(px, py, box, 12);
    if (!h) return;
    active = h; startRect = rect; startX = px; startY = py;
    host.setPointerCapture(e.pointerId);
  }
  function onUp() { active = null; }
</script>

<div
  bind:this={host} class="overlay" style="cursor:{cursor}"
  on:pointerdown={onDown} on:pointermove={onMove} on:pointerup={onUp} on:pointercancel={onUp}
>
  <div class="scrim" style="left:0; top:0; right:0; height:{box.top}px"></div>
  <div class="scrim" style="left:0; top:{box.top + box.height}px; right:0; bottom:0"></div>
  <div class="scrim" style="left:0; top:{box.top}px; width:{box.left}px; height:{box.height}px"></div>
  <div class="scrim" style="left:{box.left + box.width}px; top:{box.top}px; right:0; height:{box.height}px"></div>

  <div class="frame" style="left:{box.left}px; top:{box.top}px; width:{box.width}px; height:{box.height}px"></div>
  {#each vx as x}<div class="grid v" style="left:{x}px; top:{box.top}px; height:{box.height}px"></div>{/each}
  {#each hy as y}<div class="grid h" style="top:{y}px; left:{box.left}px; width:{box.width}px"></div>{/each}

  {#each [["nw",box.left,box.top],["ne",box.left+box.width,box.top],["sw",box.left,box.top+box.height],["se",box.left+box.width,box.top+box.height],["n",box.left+box.width/2,box.top],["s",box.left+box.width/2,box.top+box.height],["w",box.left,box.top+box.height/2],["e",box.left+box.width,box.top+box.height/2]] as b}
    <div class="bracket" style="left:{b[1]}px; top:{b[2]}px"></div>
  {/each}
</div>

<style>
  .overlay { position: absolute; inset: 0; user-select: none; touch-action: none; }
  .scrim { position: absolute; background: rgba(0,0,0,0.5); }
  .frame { position: absolute; border: 1px solid rgba(255,255,255,0.9); box-sizing: border-box; }
  .grid { position: absolute; background: rgba(255,255,255,0.3); }
  .grid.v { width: 1px; } .grid.h { height: 1px; }
  .bracket { position: absolute; width: 12px; height: 12px; transform: translate(-50%,-50%);
    border-radius: 2px; background: rgba(230,230,230,0.95); box-shadow: 0 0 2px rgba(0,0,0,0.6); }
</style>
