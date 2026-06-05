<script lang="ts">
  export let label: string;
  export let min: number;
  export let max: number;
  export let step = 1;
  export let value: number;
  export let def = 0;                 // double-click reset target
  export let gradient = "";           // CSS background for the track
  export let format: (v: number) => string = (v) => `${Math.round(v)}`;
</script>

<div class="slider">
  <div class="row">
    <span class="label" on:dblclick={() => (value = def)}>{label}</span>
    <span class="val">{format(value)}</span>
  </div>
  <input
    type="range" {min} {max} {step} bind:value
    class:grad={!!gradient}
    style={gradient ? `--track:${gradient}` : ""}
    on:dblclick={() => (value = def)}
    on:input
  />
</div>

<style>
  .slider { margin: 7px 0; }
  .row { display: flex; justify-content: space-between; font-size: 11px;
    color: var(--text-dim); margin-bottom: 2px; }
  .val { color: var(--text); font-variant-numeric: tabular-nums; }
  .label { cursor: default; }
  input[type="range"] { width: 100%; height: 3px; border-radius: 3px;
    -webkit-appearance: none; appearance: none; background: var(--glass-brd);
    accent-color: var(--accent); }
  input.grad { background: var(--track); }
  input[type="range"]::-webkit-slider-thumb { -webkit-appearance: none;
    width: 12px; height: 12px; border-radius: 50%; background: #fff;
    border: 1px solid rgba(0,0,0,0.3); box-shadow: 0 1px 3px rgba(0,0,0,0.4); cursor: grab; }
  input[type="range"]:active::-webkit-slider-thumb { cursor: grabbing; }
</style>
