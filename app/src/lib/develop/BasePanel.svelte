<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { t } from "$lib/i18n";

  export let sampled: [number, number, number] | null = null;
  export let scope: "override" | "folder" | "auto" = "auto";

  const dispatch = createEventDispatcher<{ applyRoll: void; thisImage: void; reset: void }>();
  // 8-bit swatch preview of the linear base (display-only, gamma ~1/2.2).
  $: css = sampled
    ? `rgb(${sampled.map((v) => Math.round(255 * Math.min(1, Math.max(0, v ** (1 / 2.2)))) ).join(",")})`
    : "transparent";
  const scopeKey = { override: "base.scopeOverride", folder: "base.scopeFolder", auto: "base.scopeAuto" } as const;
</script>

<div class="sec">
  <div class="sub">{$t("base.title")}</div>
  <p class="hint">{$t("base.hint")}</p>
  <div class="swatch-row">
    <div class="swatch" style="background:{css}"></div>
    <span class="vals">{sampled ? sampled.map((v) => v.toFixed(3)).join(", ") : "—"}</span>
  </div>
  <div class="btns">
    <button disabled={!sampled} on:click={() => dispatch("applyRoll")}>{$t("base.applyRoll")}</button>
    <button disabled={!sampled} on:click={() => dispatch("thisImage")}>{$t("base.thisImage")}</button>
  </div>
  <button class="reset" on:click={() => dispatch("reset")}>{$t("base.reset")}</button>
  <p class="scope">{$t(scopeKey[scope])}</p>
</div>

<style>
  .sec { padding: 4px 2px; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-dim); margin-bottom: 6px; }
  .hint { font-size: 11px; color: var(--text-faint); margin: 0 0 10px; }
  .swatch-row { display: flex; align-items: center; gap: 8px; margin-bottom: 10px; }
  .swatch { width: 40px; height: 40px; border-radius: 6px; border: 1px solid var(--glass-brd); }
  .vals { font-size: 11px; color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .btns { display: flex; gap: 6px; margin-bottom: 8px; }
  .btns button { flex: 1; padding: 7px; border-radius: 8px; font-size: 12px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text); }
  .btns button:disabled { opacity: 0.4; }
  .reset { width: 100%; padding: 6px; border-radius: 8px; font-size: 12px;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text-dim); }
  .scope { font-size: 11px; color: var(--text-faint); margin: 8px 0 0; }
</style>
