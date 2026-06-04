<script lang="ts">
  import { params, activeId } from "../store";
  import { api } from "../api";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT, signed, ev, kelvin } from "./gradients";

  let open = true;

  // Seed Temp/Tint from the estimated as-shot white point when the image changes.
  let seededFor: string | null = null;
  async function seed(id: string | null) {
    if (!id || seededFor === id) return;
    seededFor = id;
    try {
      const wb = await api.asShotWb(id);
      params.update((p) => ({ ...p, temp: wb.temp, tint: wb.tint }));
    } catch { /* not developed yet */ }
  }
  $: seed($activeId);

  function autoWb() { seededFor = null; seed($activeId); }
</script>

<div class="section">
  <button class="head" on:click={() => (open = !open)}>
    <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
    <span>Basic</span>
  </button>

  {#if open}
    <div class="body">
      <!-- White Balance -->
      <div class="sub">White Balance</div>
      <div class="seg">
        <button class:on={$params.mode === "b"} on:click={() => params.update((p) => ({ ...p, mode: "b" }))}>B · density</button>
        <button class:on={$params.mode === "c"} on:click={() => params.update((p) => ({ ...p, mode: "c" }))}>C · per-chan</button>
      </div>
      <select bind:value={$params.stock}>
        <option value="none">No film profile</option>
        <option value="portra400">Kodak Portra 400</option>
        <option value="fujic200">Fuji C200</option>
      </select>
      <div class="wbhead">
        <span>Temp / Tint</span>
        <button class="auto" on:click={autoWb}>Auto</button>
      </div>
      <Slider label="Temp" min={2000} max={50000} step={50}
        bind:value={$params.temp} def={5500} gradient={TEMP_GRADIENT} format={kelvin} />
      <Slider label="Tint" min={-150} max={150} step={1}
        bind:value={$params.tint} def={0} gradient={TINT_GRADIENT} format={signed} />

      <!-- Tone -->
      <div class="sub">Tone</div>
      <Slider label="Exposure" min={-5} max={5} step={0.05} bind:value={$params.exposure} def={0} format={ev} />
      <Slider label="Contrast" min={-100} max={100} bind:value={$params.contrast} def={0} format={signed} />
      <Slider label="Highlights" min={-100} max={100} bind:value={$params.highlights} def={0} format={signed} />
      <Slider label="Shadows" min={-100} max={100} bind:value={$params.shadows} def={0} format={signed} />
      <Slider label="Whites" min={-100} max={100} bind:value={$params.whites} def={0} format={signed} />
      <Slider label="Blacks" min={-100} max={100} bind:value={$params.blacks} def={0} format={signed} />

      <!-- Presence -->
      <div class="sub">Presence</div>
      <Slider label="Texture" min={-100} max={100} bind:value={$params.texture} def={0} format={signed} />
      <Slider label="Vibrance" min={-100} max={100} bind:value={$params.vibrance} def={0} gradient={SAT_GRADIENT} format={signed} />
      <Slider label="Saturation" min={-100} max={100} bind:value={$params.saturation} def={0} gradient={SAT_GRADIENT} format={signed} />
    </div>
  {/if}
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { display: flex; align-items: center; gap: 6px; width: 100%;
    background: transparent; border: 0; color: var(--text); font-weight: 600;
    padding: 4px 0; cursor: pointer; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  .seg { display: flex; gap: 6px; margin-bottom: 8px; }
  .seg button { flex: 1; padding: 6px; border-radius: 8px; font-size: 12px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text-dim); }
  .seg button.on { color: #fff; background: rgba(224,52,52,0.18); border-color: rgba(224,52,52,0.5); }
  select { width: 100%; padding: 6px; border-radius: 8px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); margin-bottom: 8px; }
  .wbhead { display: flex; justify-content: space-between; align-items: center;
    font-size: 11px; color: var(--text-dim); margin: 4px 0; }
  .auto { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
</style>
