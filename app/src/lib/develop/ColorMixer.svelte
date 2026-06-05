<script lang="ts">
  import { t } from "$lib/i18n";
  import { params } from "../store";
  import { defaultParams, CM_BANDS, type PointColorSample } from "../api";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import { signed } from "./gradients";
  import { CM_HUE_GRADIENTS, CM_SAT_GRADIENTS, CM_LUM_GRADIENT } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  // Set by Task 13 (Develop.svelte passes a callback to arm the viewport dropper).
  export let onPick: (() => void) | null = null;
  export let picking = false;

  let open = true;
  type Tab = "mixer" | "point";
  let tab: Tab = "mixer";
  type Adjust = "hue" | "saturation" | "luminance" | "all";
  let adjust: Adjust = "hue";

  const ADJ: { id: Adjust; key: string }[] = [
    { id: "hue", key: "colorMixer.adjust.hue" },
    { id: "saturation", key: "colorMixer.adjust.saturation" },
    { id: "luminance", key: "colorMixer.adjust.luminance" },
    { id: "all", key: "colorMixer.adjust.all" },
  ];

  // Map adjust → field suffix.
  const suffix = (a: Exclude<Adjust, "all">) =>
    a === "hue" ? "hue" : a === "saturation" ? "sat" : "lum";

  // Index-typed accessor for the cm_* flat fields (InvertParams has no index signature).
  $: P = $params as unknown as Record<string, number>;
  function setField(key: string, v: number) {
    params.update((p) => ({ ...p, [key]: v }) as typeof p);
  }

  function resetMixer() {
    const d = defaultParams() as unknown as Record<string, number>;
    params.update((p) => {
      const next = { ...p } as unknown as Record<string, unknown>;
      for (const b of CM_BANDS) for (const s of ["hue", "sat", "lum"])
        next[`cm_${b}_${s}`] = d[`cm_${b}_${s}`];
      return next as unknown as typeof p;
    });
  }
  function resetPoint() {
    params.update((p) => ({ ...p, pc_samples: [] }));
  }

  // --- Point Color sample editing ---
  let selected = 0;
  $: samples = $params.pc_samples ?? [];
  $: sel = samples[selected] as PointColorSample | undefined;

  function updateSample(patch: Partial<PointColorSample>) {
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      if (!arr[selected]) return p;
      arr[selected] = { ...arr[selected], ...patch };
      return { ...p, pc_samples: arr };
    });
  }
  function removeSample(i: number) {
    params.update((p) => {
      const arr = (p.pc_samples ?? []).slice();
      arr.splice(i, 1);
      return { ...p, pc_samples: arr };
    });
    if (selected >= samples.length - 1) selected = Math.max(0, samples.length - 2);
  }
  // CSS color for a sample swatch (its sampled HSL).
  const swatch = (s: PointColorSample) => `hsl(${s.hue} ${Math.round(s.sat * 100)}% ${Math.round(s.lum * 100)}%)`;
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>{$t('colorMixer.title')}</span>
    </button>
    <button class="reset" on:click={() => (tab === "mixer" ? resetMixer() : resetPoint())}>
      {$t('colorMixer.reset')}
    </button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <div class="tabs">
        <button class:on={tab === "mixer"} on:click={() => (tab = "mixer")}>{$t('colorMixer.tab.mixer')}</button>
        <button class:on={tab === "point"} on:click={() => (tab = "point")}>{$t('colorMixer.tab.point')}</button>
      </div>

      {#if tab === "mixer"}
        <div class="modes">
          {#each ADJ as a}
            <button class:on={adjust === a.id} on:click={() => (adjust = a.id)}>{$t(a.key)}</button>
          {/each}
        </div>

        {#if adjust === "all"}
          {#each CM_BANDS as b}
            <div class="bandgroup">
              <div class="bandname">{$t(`colorMixer.band.${b}`)}</div>
              <Slider label={$t('colorMixer.adjust.hue')} min={-100} max={100}
                value={P[`cm_${b}_hue`]} def={0} format={signed} gradient={CM_HUE_GRADIENTS[b]}
                on:input={(e) => setField(`cm_${b}_hue`, +(e.target as HTMLInputElement).value)} />
              <Slider label={$t('colorMixer.adjust.saturation')} min={-100} max={100}
                value={P[`cm_${b}_sat`]} def={0} format={signed} gradient={CM_SAT_GRADIENTS[b]}
                on:input={(e) => setField(`cm_${b}_sat`, +(e.target as HTMLInputElement).value)} />
              <Slider label={$t('colorMixer.adjust.luminance')} min={-100} max={100}
                value={P[`cm_${b}_lum`]} def={0} format={signed} gradient={CM_LUM_GRADIENT}
                on:input={(e) => setField(`cm_${b}_lum`, +(e.target as HTMLInputElement).value)} />
            </div>
          {/each}
        {:else}
          {#each CM_BANDS as b}
            <Slider label={$t(`colorMixer.band.${b}`)} min={-100} max={100}
              value={P[`cm_${b}_${suffix(adjust as Exclude<Adjust, "all">)}`]} def={0} format={signed}
              gradient={adjust === "hue" ? CM_HUE_GRADIENTS[b] : adjust === "saturation" ? CM_SAT_GRADIENTS[b] : CM_LUM_GRADIENT}
              on:input={(e) => setField(`cm_${b}_${suffix(adjust as Exclude<Adjust, "all">)}`, +(e.target as HTMLInputElement).value)} />
          {/each}
        {/if}
      {:else}
        <div class="point">
          <button class="dropper" class:on={picking} on:click={() => onPick?.()}>
            <Icon name="pipette" size={14} />
            <span>{$t('colorMixer.point.dropper')}</span>
          </button>

          {#if samples.length === 0}
            <p class="hint">{$t('colorMixer.point.hint')}</p>
          {:else}
            <div class="swatches">
              {#each samples as s, i}
                <button class="sw" class:sel={i === selected} style="background:{swatch(s)}"
                  on:click={() => (selected = i)} title={`${Math.round(s.hue)}°`}>
                  {#if i === selected}
                    <span class="rm" role="button" tabindex="-1"
                      on:click|stopPropagation={() => removeSample(i)}
                      on:keydown|stopPropagation={() => removeSample(i)}
                      title={$t('colorMixer.point.delete')}>×</span>
                  {/if}
                </button>
              {/each}
            </div>

            {#if sel}
              <Slider label={$t('colorMixer.point.hueShift')} min={-100} max={100}
                value={sel.hue_shift} def={0} format={signed}
                on:input={(e) => updateSample({ hue_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.satShift')} min={-100} max={100}
                value={sel.sat_shift} def={0} format={signed}
                on:input={(e) => updateSample({ sat_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.lumShift')} min={-100} max={100}
                value={sel.lum_shift} def={0} format={signed}
                on:input={(e) => updateSample({ lum_shift: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.variance')} min={-100} max={100}
                value={sel.variance} def={0} format={signed}
                on:input={(e) => updateSample({ variance: +(e.target as HTMLInputElement).value })} />
              <Slider label={$t('colorMixer.point.range')} min={0} max={100}
                value={sel.range} def={50}
                on:input={(e) => updateSample({ range: +(e.target as HTMLInputElement).value })} />
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { display: flex; align-items: center; justify-content: space-between; width: 100%; padding: 4px 0; }
  .toggle { display: flex; align-items: center; gap: 6px; background: transparent; border: 0;
    color: var(--text); font-weight: 600; padding: 0; cursor: pointer; }
  .reset { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .tabs { display: flex; gap: 4px; margin: 6px 0 8px; }
  .tabs button { flex: 1; background: var(--bg-1); border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 5px 2px; font-size: 11px; cursor: pointer; }
  .tabs button.on { color: var(--text); border-color: var(--accent); }
  .modes { display: flex; gap: 4px; margin: 4px 0 10px; }
  .modes button { flex: 1; background: var(--bg-1); border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 4px 2px; font-size: 10px; cursor: pointer; }
  .modes button.on { color: var(--text); border-color: var(--accent); }
  .bandgroup { margin-bottom: 10px; }
  .bandname { font-size: 11px; color: var(--text); margin: 6px 0 2px; }
  .point { margin-top: 6px; }
  .dropper { display: inline-flex; align-items: center; gap: 6px; background: var(--bg-1);
    border: 1px solid var(--glass-brd); color: var(--text-dim); border-radius: 6px;
    padding: 5px 10px; font-size: 11px; cursor: pointer; }
  .dropper.on { color: var(--text); border-color: var(--accent); }
  .hint { color: var(--text-dim); font-size: 11px; margin: 10px 2px; }
  .swatches { display: flex; flex-wrap: wrap; gap: 6px; margin: 10px 0; }
  .sw { width: 26px; height: 26px; border-radius: 6px; border: 1px solid var(--glass-brd);
    position: relative; cursor: pointer; padding: 0; }
  .sw.sel { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
  .rm { position: absolute; top: -6px; right: -6px; width: 14px; height: 14px; line-height: 13px;
    text-align: center; font-size: 11px; border-radius: 50%; background: #222; color: #fff;
    border: 1px solid var(--glass-brd); }
</style>
