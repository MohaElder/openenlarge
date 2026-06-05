<script lang="ts">
  import { t } from "$lib/i18n";
  import { params } from "../store";
  import Icon from "../icons/Icon.svelte";
  import Slider from "./Slider.svelte";
  import CurveEditor from "./CurveEditor.svelte";
  import { signed } from "./gradients";
  import { IDENTITY_CURVE, type CurvePoint } from "../api";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  let open = true;

  type Mode = "master" | "r" | "g" | "b";
  let mode: Mode = "master";

  const KEY = { master: "tc_curve", r: "tc_red", g: "tc_green", b: "tc_blue" } as const;
  const COLOR = { master: "#e8e8e8", r: "#ff6b6b", g: "#6bff8b", b: "#6ba8ff" };
  const HIST = { master: ["r", "g", "b"], r: ["r"], g: ["g"], b: ["b"] } as const;

  $: key = KEY[mode];
  $: points = $params[key] as CurvePoint[];

  function onCurve(e: CustomEvent<CurvePoint[]>) {
    params.update((p) => ({ ...p, [key]: e.detail }));
  }
  // Reset the entire Tone Curve section on the active image: all four curves
  // (master + R/G/B) back to identity and every region slider back to 0.
  function resetTone() {
    const identity = () => IDENTITY_CURVE.map((q) => [...q] as CurvePoint);
    params.update((p) => ({
      ...p,
      tc_curve: identity(), tc_red: identity(), tc_green: identity(), tc_blue: identity(),
      tc_highlights: 0, tc_lights: 0, tc_darks: 0, tc_shadows: 0,
    }));
  }
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>{$t('curve.title')}</span>
    </button>
    <button class="reset" on:click={resetTone}>{$t('curve.reset')}</button>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <div class="adjust">
        <span class="adjlabel">{$t('curve.adjust')}</span>
        <div class="dots">
          <button class="dot m" class:on={mode === "master"} on:click={() => (mode = "master")} title={$t('curve.master.title')} aria-label={$t('curve.master.ariaLabel')}></button>
          <button class="dot r" class:on={mode === "r"} on:click={() => (mode = "r")} title={$t('curve.red.title')} aria-label={$t('curve.red.ariaLabel')}></button>
          <button class="dot g" class:on={mode === "g"} on:click={() => (mode = "g")} title={$t('curve.green.title')} aria-label={$t('curve.green.ariaLabel')}></button>
          <button class="dot b" class:on={mode === "b"} on:click={() => (mode = "b")} title={$t('curve.blue.title')} aria-label={$t('curve.blue.ariaLabel')}></button>
        </div>
      </div>

      <CurveEditor {points} color={COLOR[mode]} hist={[...HIST[mode]]} on:change={onCurve} />

      <div class="sub">{$t('curve.region')}</div>
      <Slider label={$t('curve.highlights')} min={-100} max={100} bind:value={$params.tc_highlights} def={0} format={signed} />
      <Slider label={$t('curve.lights')} min={-100} max={100} bind:value={$params.tc_lights} def={0} format={signed} />
      <Slider label={$t('curve.darks')} min={-100} max={100} bind:value={$params.tc_darks} def={0} format={signed} />
      <Slider label={$t('curve.shadows')} min={-100} max={100} bind:value={$params.tc_shadows} def={0} format={signed} />
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
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  .adjust { display: flex; align-items: center; gap: 10px; margin: 6px 0 8px; }
  .adjlabel { font-size: 11px; color: var(--text-dim); }
  .dots { display: flex; gap: 8px; }
  .dot { width: 16px; height: 16px; border-radius: 50%; cursor: pointer;
    border: 1px solid rgba(255, 255, 255, 0.25); padding: 0; opacity: 0.55; }
  .dot.on { opacity: 1; box-shadow: 0 0 0 2px var(--accent); }
  .dot.m { background: #cfcfcf; } .dot.r { background: #ff6b6b; }
  .dot.g { background: #6bff8b; } .dot.b { background: #6ba8ff; }
  .reset { background: transparent; border: 1px solid var(--glass-brd);
    color: var(--text-dim); border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
</style>
