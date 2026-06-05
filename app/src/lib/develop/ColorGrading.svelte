<script lang="ts">
  import { t } from "$lib/i18n";
  import { params } from "../store";
  import { defaultParams } from "../api";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import ColorWheel from "./ColorWheel.svelte";
  import { signed } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  let open = true;

  type Mode = "3way" | "sh" | "mid" | "hi" | "glob";
  let mode: Mode = "3way";

  const MODES: { id: Mode; labelKey: string }[] = [
    { id: "3way", labelKey: "colorGrading.mode.threeWay" },
    { id: "sh", labelKey: "colorGrading.mode.shadows" },
    { id: "mid", labelKey: "colorGrading.mode.midtones" },
    { id: "hi", labelKey: "colorGrading.mode.highlights" },
    { id: "glob", labelKey: "colorGrading.mode.global" },
  ];

  const KEYS = {
    sh: ["cg_sh_hue", "cg_sh_sat", "cg_sh_lum"],
    mid: ["cg_mid_hue", "cg_mid_sat", "cg_mid_lum"],
    hi: ["cg_hi_hue", "cg_hi_sat", "cg_hi_lum"],
    glob: ["cg_glob_hue", "cg_glob_sat", "cg_glob_lum"],
  } as const;

  type Region = keyof typeof KEYS;
  function setWheel(region: Region, d: { hue: number; sat: number; lum: number }) {
    const [h, s, l] = KEYS[region];
    params.update((p) => ({ ...p, [h]: d.hue, [s]: d.sat, [l]: d.lum }));
  }

  // Reset every Color Grading control to its default on the active image,
  // leaving all other develop state untouched. The view-mode selector is local
  // UI state (which wheel is shown) and is intentionally left unchanged.
  function resetColorGrading() {
    const d = defaultParams();
    params.update((p) => ({
      ...p,
      cg_sh_hue: d.cg_sh_hue, cg_sh_sat: d.cg_sh_sat, cg_sh_lum: d.cg_sh_lum,
      cg_mid_hue: d.cg_mid_hue, cg_mid_sat: d.cg_mid_sat, cg_mid_lum: d.cg_mid_lum,
      cg_hi_hue: d.cg_hi_hue, cg_hi_sat: d.cg_hi_sat, cg_hi_lum: d.cg_hi_lum,
      cg_glob_hue: d.cg_glob_hue, cg_glob_sat: d.cg_glob_sat, cg_glob_lum: d.cg_glob_lum,
      cg_blending: d.cg_blending, cg_balance: d.cg_balance,
    }));
  }
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>{$t('colorGrading.title')}</span>
    </button>
    <button class="reset" on:click={resetColorGrading}>{$t('colorGrading.reset')}</button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <div class="modes">
        {#each MODES as m}
          <button class:on={mode === m.id} on:click={() => (mode = m.id)}>{$t(m.labelKey)}</button>
        {/each}
      </div>

      {#if mode === "3way"}
        <div class="mid-row">
          <ColorWheel label={$t('colorGrading.wheel.midtones')} hue={$params.cg_mid_hue} sat={$params.cg_mid_sat} lum={$params.cg_mid_lum}
            on:change={(e) => setWheel("mid", e.detail)} />
        </div>
        <div class="pair">
          <ColorWheel label={$t('colorGrading.wheel.shadows')} hue={$params.cg_sh_hue} sat={$params.cg_sh_sat} lum={$params.cg_sh_lum}
            on:change={(e) => setWheel("sh", e.detail)} />
          <ColorWheel label={$t('colorGrading.wheel.highlights')} hue={$params.cg_hi_hue} sat={$params.cg_hi_sat} lum={$params.cg_hi_lum}
            on:change={(e) => setWheel("hi", e.detail)} />
        </div>
      {:else if mode === "sh"}
        <div class="single"><ColorWheel label={$t('colorGrading.wheel.shadows')} hue={$params.cg_sh_hue} sat={$params.cg_sh_sat} lum={$params.cg_sh_lum} on:change={(e) => setWheel("sh", e.detail)} /></div>
      {:else if mode === "mid"}
        <div class="single"><ColorWheel label={$t('colorGrading.wheel.midtones')} hue={$params.cg_mid_hue} sat={$params.cg_mid_sat} lum={$params.cg_mid_lum} on:change={(e) => setWheel("mid", e.detail)} /></div>
      {:else if mode === "hi"}
        <div class="single"><ColorWheel label={$t('colorGrading.wheel.highlights')} hue={$params.cg_hi_hue} sat={$params.cg_hi_sat} lum={$params.cg_hi_lum} on:change={(e) => setWheel("hi", e.detail)} /></div>
      {:else}
        <div class="single"><ColorWheel label={$t('colorGrading.wheel.global')} hue={$params.cg_glob_hue} sat={$params.cg_glob_sat} lum={$params.cg_glob_lum} on:change={(e) => setWheel("glob", e.detail)} /></div>
      {/if}

      <div class="sliders">
        <Slider label={$t('colorGrading.blending')} min={0} max={100} bind:value={$params.cg_blending} def={50} />
        <Slider label={$t('colorGrading.balance')} min={-100} max={100} bind:value={$params.cg_balance} def={0} format={signed} />
      </div>
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
  .modes { display: flex; gap: 4px; margin: 6px 0 10px; }
  .modes button { flex: 1; background: var(--bg-1); border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 4px 2px; font-size: 10px; cursor: pointer; }
  .modes button.on { color: var(--text); border-color: var(--accent); }
  .mid-row { display: flex; justify-content: center; margin-bottom: 12px; }
  .pair { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .single { display: flex; justify-content: center; }
  .sliders { margin-top: 12px; }
</style>
