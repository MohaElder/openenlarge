<script lang="ts">
  import { get } from "svelte/store";
  import { t } from "$lib/i18n";
  import { params, activeId } from "../store";
  import { api, defaultParams } from "../api";
  import { reseedActive } from "./historyStore";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT, signed, ev, kelvin } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  let open = true;

  // Seed Temp/Tint from the estimated as-shot white point when the image OR the
  // film profile changes. The estimate runs against the current stock/mode, so
  // switching a profile re-balances to its colour space (prevents the cast that
  // appears when a profile's M_post rotates colours under a stale white balance).
  let seededFor: string | null = null;
  async function seed(id: string | null, stock: string) {
    const key = id ? `${id}:${stock}` : null;
    if (!key || seededFor === key) return;
    seededFor = key;
    try {
      const wb = await api.asShotWb(id!, get(params));
      params.update((p) => ({ ...p, temp: wb.temp, tint: wb.tint }));
      reseedActive();
    } catch { /* not developed yet */ }
  }
  $: seed($activeId, $params.stock);

  function autoWb() { seededFor = null; seed($activeId, $params.stock); }

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
      <span>{$t('basic.title')}</span>
    </button>
    <button class="reset" on:click={resetBasic}>{$t('basic.reset')}</button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <!-- Film Profile -->
      <div class="sub">{$t('basic.filmProfile')}</div>
      <select bind:value={$params.stock}>
        <option value="none">{$t('basic.noFilmProfile')}</option>
        <option value="portra400">{$t('basic.stock.portra400')}</option>
        <option value="fujic200">{$t('basic.stock.fujic200')}</option>
      </select>

      <!-- White Balance -->
      <div class="sub">{$t('basic.whiteBalance')}</div>
      <div class="wbhead">
        <span>{$t('basic.tempTint')}</span>
        <button class="auto" on:click={autoWb}>{$t('basic.auto')}</button>
      </div>
      <Slider label={$t('basic.temp')} min={2000} max={50000} step={50}
        bind:value={$params.temp} def={5500} gradient={TEMP_GRADIENT} format={kelvin} />
      <Slider label={$t('basic.tint')} min={-150} max={150} step={1}
        bind:value={$params.tint} def={0} gradient={TINT_GRADIENT} format={signed} />

      <!-- Tone -->
      <div class="sub">{$t('basic.tone')}</div>
      <Slider label={$t('basic.exposure')} min={-5} max={5} step={0.05} bind:value={$params.exposure} def={0} format={ev} />
      <Slider label={$t('basic.contrast')} min={-100} max={100} bind:value={$params.contrast} def={0} format={signed} />
      <Slider label={$t('basic.highlights')} min={-100} max={100} bind:value={$params.highlights} def={0} format={signed} />
      <Slider label={$t('basic.shadows')} min={-100} max={100} bind:value={$params.shadows} def={0} format={signed} />
      <Slider label={$t('basic.whites')} min={-100} max={100} bind:value={$params.whites} def={0} format={signed} />
      <Slider label={$t('basic.blacks')} min={-100} max={100} bind:value={$params.blacks} def={0} format={signed} />

      <!-- Presence -->
      <div class="sub">{$t('basic.presence')}</div>
      <Slider label={$t('basic.texture')} min={-100} max={100} bind:value={$params.texture} def={0} format={signed} />
      <Slider label={$t('basic.vibrance')} min={-100} max={100} bind:value={$params.vibrance} def={0} gradient={SAT_GRADIENT} format={signed} />
      <Slider label={$t('basic.saturation')} min={-100} max={100} bind:value={$params.saturation} def={0} gradient={SAT_GRADIENT} format={signed} />
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
