<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { quality } from "../store";
  import { api, type Quality } from "../api";
  import { developAll, markAllUndeveloped } from "../workflow";
  export let x = 0;
  export let y = 0;
  const dispatch = createEventDispatcher();

  async function pick(q: Quality) {
    if (q !== $quality) {
      quality.set(q);
      await api.setQuality(q);
      markAllUndeveloped();
      dispatch("close");
      await developAll();
    } else {
      dispatch("close");
    }
  }
</script>

<div class="menu" style="left:{x}px; top:{y}px" on:pointerleave={() => dispatch("close")}>
  <div class="head">Preview quality</div>
  <button class:on={$quality === "performance"} on:click={() => pick("performance")}>Performance · 4K</button>
  <button class:on={$quality === "quality"} on:click={() => pick("quality")}>Quality · full res</button>
</div>

<style>
  .menu { position: fixed; z-index: 70; background: var(--glass-bg); border: 1px solid var(--glass-brd);
    border-radius: 10px; padding: 6px; min-width: 180px; backdrop-filter: blur(20px);
    box-shadow: 0 12px 40px rgba(0,0,0,0.5); }
  .head { font-size: 11px; color: var(--text-dim); padding: 4px 8px; }
  button { display: block; width: 100%; text-align: left; padding: 7px 8px; border: 0;
    background: transparent; border-radius: 7px; color: var(--text-dim); }
  button.on { color: var(--text); background: rgba(244,157,78,0.16); }
</style>
