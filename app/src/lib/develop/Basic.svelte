<script lang="ts">
  import { params, activeId } from "../store";
  import { api, defaultParams } from "../api";
  import { reseedActive } from "./historyStore";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT, signed, ev, kelvin } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  let open = true;

  // Seed Temp/Tint from the estimated as-shot white point when the image changes.
  let seededFor: string | null = null;
  async function seed(id: string | null) {
    if (!id || seededFor === id) return;
    seededFor = id;
    try {
      const wb = await api.asShotWb(id);
      params.update((p) => ({ ...p, temp: wb.temp, tint: wb.tint }));
      reseedActive();
    } catch { /* not developed yet */ }
  }
  $: seed($activeId);

  function autoWb() { seededFor = null; seed($activeId); }

  // Reset every Basic-section control to its default, leaving all other develop
  // state (mode, base_rect, black/gamma, tone curve, color grading) untouched.
  // Temp/Tint are re-seeded to the as-shot white point rather than the hard
  // slider defaults, matching the Auto button.
  function resetBasic() {
    const d = defaultParams();
    params.update((p) => ({
      ...p,
      stock: d.stock,
      exposure: d.exposure,
      contrast: d.contrast, highlights: d.highlights, shadows: d.shadows,
      whites: d.whites, blacks: d.blacks,
      texture: d.texture, vibrance: d.vibrance, saturation: d.saturation,
    }));
    autoWb();
  }
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>Basic</span>
    </button>
    <button class="reset" on:click={resetBasic}>Reset</button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <!-- Film Profile -->
      <div class="sub">Film Profile</div>
      <select bind:value={$params.stock}>
        <option value="none">No film profile</option>
        <option value="portra400">Kodak Portra 400</option>
        <option value="fujic200">Fuji C200</option>
      </select>

      <!-- White Balance -->
      <div class="sub">White Balance</div>
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
  .head { display: flex; align-items: center; justify-content: space-between;
    width: 100%; padding: 4px 0; }
  .toggle { display: flex; align-items: center; gap: 6px;
    background: transparent; border: 0; color: var(--text); font-weight: 600;
    padding: 0; cursor: pointer; }
  .reset { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  select { width: 100%; padding: 10px 12px; border-radius: 9px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); margin-bottom: 8px; font-size: 13px; }
  .wbhead { display: flex; justify-content: space-between; align-items: center;
    font-size: 11px; color: var(--text-dim); margin: 4px 0; }
  .auto { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
</style>
