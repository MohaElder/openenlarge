<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { t } from "$lib/i18n";
  import { PRESETS } from "./presets";
  import Icon from "../icons/Icon.svelte";

  export let aspect: string;
  export let orientation: "landscape" | "portrait";
  export let angle: number;
  const dispatch = createEventDispatcher<{ preset: string; swap: void; reset: void; rotate: number; flip: "h" | "v" }>();
</script>

<div class="section">
  <div class="head"><span>{$t('crop.title')}</span></div>

  <div class="sub">{$t('crop.aspectRatio')}</div>
  <select value={aspect} on:change={(e) => dispatch("preset", (e.target as HTMLSelectElement).value)}>
    {#if aspect === "custom"}<option value="custom">{$t('crop.custom')}</option>{/if}
    {#each PRESETS as p}<option value={p.id}>{$t(p.label)}</option>{/each}
  </select>

  <div class="sub">{$t('crop.orientation')}</div>
  <button class="orient" title={$t('crop.toggleOrientation')} aria-label={$t('crop.orientationAriaLabel')}
          on:click={() => dispatch("swap")}>
    <Icon name={orientation === "landscape" ? "landscape" : "portrait"} size={20} />
  </button>

  <div class="sub">{$t('crop.transform')}</div>
  <div class="btns">
    <button title={$t('crop.rotateLeft')} on:click={() => dispatch("rotate", -1)}><Icon name="rotate-ccw" size={16} /></button>
    <button title={$t('crop.rotateRight')} on:click={() => dispatch("rotate", 1)}><Icon name="rotate-cw" size={16} /></button>
    <button title={$t('crop.flipHorizontal')} on:click={() => dispatch("flip", "h")}><Icon name="flip-h" size={16} /></button>
    <button title={$t('crop.flipVertical')} on:click={() => dispatch("flip", "v")}><Icon name="flip-v" size={16} /></button>
  </div>

  <div class="sub">{$t('crop.straighten')}</div>
  <div class="slrow">
    <input type="range" min="-45" max="45" step="0.1" bind:value={angle} on:dblclick={() => (angle = 0)} />
    <span class="val">{angle.toFixed(1)}°</span>
  </div>

  <button class="row" on:click={() => dispatch("reset")}>{$t('crop.reset')}</button>
  <div class="hint">{$t('crop.hint')}</div>
</div>

<style>
  .section { margin-bottom: 12px; }
  .head { color: var(--text); font-weight: 600; padding: 4px 0; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  select { width: 100%; padding: 10px 12px; border-radius: 9px; background: var(--bg-1);
    color: var(--text); border: 1px solid var(--glass-brd); font-size: 13px; }
  .orient { display: grid; place-items: center; width: 44px; height: 38px; padding: 0;
    border-radius: 9px; border: 1px solid var(--glass-brd); background: transparent;
    color: var(--text); cursor: pointer; transition: background 0.12s, border-color 0.12s; }
  .orient:hover { background: var(--glass-hi); border-color: rgba(255,255,255,0.18); }
  .row { width: 100%; display: flex; justify-content: space-between; align-items: center;
    padding: 7px 10px; margin: 6px 0; border-radius: 8px; border: 1px solid var(--glass-brd);
    background: transparent; color: var(--text); cursor: pointer; }
  .btns { display: flex; gap: 6px; }
  .btns button { flex: 1; display: grid; place-items: center; padding: 8px 0; border-radius: 8px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text); cursor: pointer; }
  .slrow { display: flex; align-items: center; gap: 8px; }
  .slrow input[type="range"] { flex: 1; accent-color: var(--accent); }
  .val { font-size: 12px; color: var(--text); width: 44px; text-align: right; font-variant-numeric: tabular-nums; }
  .hint { font-size: 11px; color: var(--text-dim); margin-top: 8px; line-height: 1.5; }
</style>
