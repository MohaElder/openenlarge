<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { PRESETS, labelFor } from "./presets";

  export let aspect: string;                        // bound preset id or "custom"
  export let orientation: "landscape" | "portrait"; // bound
  const dispatch = createEventDispatcher<{ preset: string; swap: void; reset: void }>();
</script>

<div class="section">
  <div class="head"><span>Crop</span></div>

  <div class="sub">Aspect ratio</div>
  <select value={aspect} on:change={(e) => dispatch("preset", (e.target as HTMLSelectElement).value)}>
    {#if aspect === "custom"}<option value="custom">Custom</option>{/if}
    {#each PRESETS as p}<option value={p.id}>{p.label}</option>{/each}
  </select>
  <div class="current">{labelFor(aspect)}</div>

  <button class="row" on:click={() => dispatch("swap")}>
    Orientation: {orientation === "landscape" ? "Landscape" : "Portrait"} <span class="key">X</span>
  </button>
  <button class="row" on:click={() => dispatch("reset")}>Reset</button>

  <div class="hint">Enter to apply · Esc to discard · Shift locks the ratio</div>
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { color: var(--text); font-weight: 600; padding: 4px 0; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  select { width: 100%; padding: 6px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); }
  .current { font-size: 12px; color: var(--text-dim); margin: 4px 0 8px; }
  .row { width: 100%; display: flex; justify-content: space-between; align-items: center;
    padding: 7px 10px; margin-bottom: 6px; border-radius: 8px; border: 1px solid var(--glass-brd);
    background: transparent; color: var(--text); cursor: pointer; }
  .key { font-size: 10px; border: 1px solid var(--glass-brd); border-radius: 4px; padding: 0 5px;
    color: var(--text-dim); }
  .hint { font-size: 11px; color: var(--text-dim); margin-top: 8px; line-height: 1.5; }
</style>
