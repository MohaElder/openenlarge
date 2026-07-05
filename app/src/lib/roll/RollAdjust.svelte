<script lang="ts">
  import { t } from "$lib/i18n";
  import Slider from "$lib/develop/Slider.svelte";
  import TonalCurve from "$lib/develop/TonalCurve.svelte";
  import { signed, ev, relKelvin, TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT } from "$lib/develop/gradients";
  import { draftParamsStore } from "./draftParams";
  import { defaultParams } from "$lib/api";
  import { proofMode, proofSolving } from "./proof";

  const ps = draftParamsStore();

  function markWbManual() { ps.update((p) => ({ ...p, wb_manual: true })); }
  function resetLook() {
    ps.update((p) => ({ ...defaultParams(), base_override: p.base_override, d_max_override: p.d_max_override }));
  }
</script>

<div class="adjust">
  <div class="head">
    <h3>{$t('roll.adjust.heading')}</h3>
    <button class="reset" on:click={resetLook}>{$t('basic.reset')}</button>
  </div>
  <slot />

  <!-- Temporary contact sheet (#18): a display-only proof layer. When on, every
       frame previews at its OWN solved auto exposure / auto color, and the roll
       sliders below apply RELATIVE to those baselines. Stored per-frame edits are
       never modified; solves are cached for the session (see roll/proof.ts). -->
  <div class="proof">
    <button class="ptoggle" class:on={$proofMode.on} aria-pressed={$proofMode.on}
            title={$t('roll.proof.title')}
            on:click={() => proofMode.update((m) => ({ ...m, on: !m.on }))}>
      {$t('roll.proof.label')}
    </button>
    {#if $proofMode.on}
      <span class="proofsubs">
        <button class="popt" class:on={$proofMode.autoExposure} aria-pressed={$proofMode.autoExposure}
                on:click={() => proofMode.update((m) => ({ ...m, autoExposure: !m.autoExposure }))}
                >{$t('roll.proof.autoExposure')}</button>
        <button class="popt" class:on={$proofMode.autoColor} aria-pressed={$proofMode.autoColor}
                on:click={() => proofMode.update((m) => ({ ...m, autoColor: !m.autoColor }))}
                >{$t('roll.proof.autoColor')}</button>
        {#if $proofSolving}<span class="solving">{$t('roll.proof.solving')}</span>{/if}
      </span>
    {/if}
  </div>

  <!-- These rows are copied VERBATIM from Basic.svelte (lines ~248-272), with only
       `$params` swapped for `$ps`. Same label keys / min / max / step / scale /
       gradient / format so the roll look matches per-image Tune exactly. -->
  <Slider label={$t('basic.temp')} min={2000} max={25000} step={0.5} scale="reciprocalCentered" scrubStep={10}
    bind:value={$ps.temp} def={5500} gradient={TEMP_GRADIENT} format={(v) => relKelvin(v - 5500)} on:input={markWbManual} />
  <Slider label={$t('basic.tint')} min={-150} max={150} step={1}
    bind:value={$ps.tint} def={0} gradient={TINT_GRADIENT} format={signed} on:input={markWbManual} />
  <Slider label={$t('basic.exposure')} min={-5} max={5} step={0.01} bind:value={$ps.exposure} def={0} format={ev} />
  <Slider label={$t('basic.contrast')} min={-100} max={100} bind:value={$ps.contrast} def={0} format={signed} />
  <Slider label={$t('basic.highlights')} min={-100} max={100} bind:value={$ps.highlights} def={0} format={signed} />
  <Slider label={$t('basic.shadows')} min={-100} max={100} bind:value={$ps.shadows} def={0} format={signed} />
  <Slider label={$t('basic.whites')} min={-100} max={100} bind:value={$ps.whites} def={0} format={signed} />
  <Slider label={$t('basic.blacks')} min={-100} max={100} bind:value={$ps.blacks} def={0} format={signed} />
  <Slider label={$t('basic.vibrance')} min={-100} max={100} bind:value={$ps.vibrance} def={0} gradient={SAT_GRADIENT} format={signed} />
  <Slider label={$t('basic.saturation')} min={-100} max={100} bind:value={$ps.saturation} def={0} gradient={SAT_GRADIENT} format={signed} />

  <TonalCurve paramsStore={ps} />
</div>

<style>
  .adjust { display: flex; flex-direction: column; gap: 8px; }
  .head { display: flex; align-items: center; justify-content: space-between; }
  h3 { margin: 0 0 4px; font-size: 13px; color: var(--text); }
  .reset { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .reset:hover { color: var(--text); }
  .proof { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .ptoggle { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 3px 10px; font-size: 11px; cursor: pointer; }
  .ptoggle.on { color: var(--text); border-color: rgba(244,157,78,0.5); background: rgba(244,157,78,0.12); }
  .proofsubs { display: inline-flex; align-items: center; gap: 6px; }
  .popt { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .popt.on { color: var(--text); border-color: rgba(244,157,78,0.5); background: rgba(244,157,78,0.12); }
  .solving { color: var(--text-dim); font-size: 11px; }
</style>
