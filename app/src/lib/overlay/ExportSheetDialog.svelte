<script lang="ts">
  import { createEventDispatcher } from "svelte";
  import { fade, scale as scaleT } from "svelte/transition";
  import { t } from "$lib/i18n";
  import type { ExportSheetOpts } from "$lib/roll/exportSheet";
  import type { PaperSize } from "$lib/roll/printLayout";
  import {
    sheetHeaderPhotographer, sheetHeaderCamera, sheetHeaderFilm, sheetHeaderDate,
  } from "$lib/store";

  const dispatch = createEventDispatcher<{ cancel: void; confirm: ExportSheetOpts }>();

  // Resolution presets: `scale` enlarges the whole sheet; `edge` is the per-frame
  // render long-edge cap so tiles stay sharp at the larger size. On paper sizes
  // the same scale doubles as print density (150/300/600 dpi) — issue #24 (upstream).
  type Res = "standard" | "high" | "print";
  const RES: Record<Res, { scale: number; edge: number; labelKey: string }> = {
    standard: { scale: 1, edge: 320,  labelKey: "roll.export.resStandard" },
    high:     { scale: 2, edge: 720,  labelKey: "roll.export.resHigh" },
    print:    { scale: 4, edge: 1280, labelKey: "roll.export.resPrint" },
  };
  let res: Res = "high";

  // Paper size: "strip" keeps the classic content-hugging canvas; A4/US Letter
  // produce paginated print pages with a header (page 1) + footer.
  const PAPERS: Array<{ id: PaperSize; labelKey: string }> = [
    { id: "strip",  labelKey: "roll.export.paperStrip" },
    { id: "a4",     labelKey: "roll.export.paperA4" },
    { id: "letter", labelKey: "roll.export.paperLetter" },
  ];
  let paper: PaperSize = "strip";

  type Fmt = "jpeg" | "png";
  let fmt: Fmt = "jpeg";
  let quality = 92;

  // TODO(issue #24): optional PNG logo/signature picker for the header. Needs a
  // byte-accurate local-file read the webview doesn't have yet (no fs plugin /
  // asset protocol; reference_thumb is a lossy 160px JPEG) — no Rust changes here.

  function go() {
    const r = RES[res];
    const format = fmt === "jpeg"
      ? { kind: "jpeg" as const, quality }
      : { kind: "png" as const, bitDepth: 8 as const };
    dispatch("confirm", { scale: r.scale, thumbEdge: r.edge, format, paper });
  }
</script>

<div class="scrim" on:click|self={() => dispatch("cancel")} transition:fade={{ duration: 150 }}>
  <div class="card" transition:scaleT={{ duration: 160, start: 0.96, opacity: 0 }}>
    <div class="title">{$t('roll.export.title')}</div>

    <div class="field-label">{$t('roll.export.resolution')}</div>
    <div class="opts">
      {#each (Object.keys(RES) as Res[]) as key (key)}
        <label class="opt" class:on={res === key}>
          <input type="radio" name="res" value={key} bind:group={res} />
          <span>{$t(RES[key].labelKey)}</span>
        </label>
      {/each}
    </div>

    <div class="field-label">{$t('roll.export.paper')}</div>
    <div class="opts">
      {#each PAPERS as p (p.id)}
        <label class="opt" class:on={paper === p.id}>
          <input type="radio" name="paper" value={p.id} bind:group={paper} />
          <span>{$t(p.labelKey)}</span>
        </label>
      {/each}
    </div>

    {#if paper !== 'strip'}
      <!-- Print header fields (page 1 of the paper export). Bound straight to
           the prefs-backed stores so they persist across sessions — issue #24
           (upstream). Free text; empty fields drop out of the header. -->
      <div class="field-label">{$t('roll.export.header')}</div>
      <div class="hdr-grid">
        <input type="text" bind:value={$sheetHeaderPhotographer}
               placeholder={$t('roll.export.photographer')}
               aria-label={$t('roll.export.photographer')} />
        <input type="text" bind:value={$sheetHeaderCamera}
               placeholder={$t('roll.export.camera')}
               aria-label={$t('roll.export.camera')} />
        <input type="text" bind:value={$sheetHeaderFilm}
               placeholder={$t('roll.export.filmStock')}
               aria-label={$t('roll.export.filmStock')} />
        <input type="text" bind:value={$sheetHeaderDate}
               placeholder={$t('roll.export.date')}
               aria-label={$t('roll.export.date')} />
      </div>
    {/if}

    <div class="field-label">{$t('roll.export.format')}</div>
    <div class="opts">
      <label class="opt" class:on={fmt === 'jpeg'}>
        <input type="radio" name="fmt" value="jpeg" bind:group={fmt} />
        <span>{$t('roll.export.formatJpeg')}</span>
      </label>
      <label class="opt" class:on={fmt === 'png'}>
        <input type="radio" name="fmt" value="png" bind:group={fmt} />
        <span>{$t('roll.export.formatPng')}</span>
      </label>
    </div>

    {#if fmt === 'jpeg'}
      <div class="quality">
        <span class="field-label q-label">{$t('roll.export.quality')}</span>
        <input type="range" min="60" max="100" step="1" bind:value={quality} />
        <span class="q-val">{quality}</span>
      </div>
    {/if}

    <div class="row">
      <button class="ghost" on:click={() => dispatch("cancel")}>{$t('confirmApply.cancel')}</button>
      <button class="go" on:click={go}>{$t('roll.export.confirm')}</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; background: rgba(0,0,0,0.5); backdrop-filter: blur(6px);
    display: grid; place-items: center; z-index: 60; }
  .card { background: var(--glass-bg); border: 1px solid var(--glass-brd); border-radius: 14px;
    padding: 22px; min-width: 340px; box-shadow: 0 20px 60px rgba(0,0,0,0.5); }
  .title { font-weight: 600; margin-bottom: 16px; }
  .field-label { font-size: 11px; color: var(--text-faint); text-transform: uppercase;
    letter-spacing: .4px; margin-bottom: 8px; }
  .opts { display: flex; gap: 8px; margin-bottom: 16px; }
  .opt { flex: 1; display: flex; align-items: center; justify-content: center; gap: 7px;
    padding: 9px 8px; border-radius: 9px; border: 1px solid var(--glass-brd);
    background: var(--glass-hi); cursor: pointer; font-size: 12px; transition: border-color .15s, background .15s; }
  .opt:hover { background: rgba(255,255,255,0.06); }
  .opt.on { border-color: rgba(244,157,78,0.6); background: rgba(244,157,78,0.12); }
  .opt input { accent-color: var(--accent, #f49d4e); width: 14px; height: 14px; cursor: pointer; }
  .hdr-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-bottom: 16px; }
  .hdr-grid input { padding: 7px 10px; border-radius: 8px;
    background: var(--glass-hi); border: 1px solid var(--glass-brd); color: var(--text);
    font: 600 12px 'Spline Sans Mono', ui-monospace, monospace; outline: none;
    transition: border-color 0.15s, background 0.15s; min-width: 0; }
  .hdr-grid input:focus { border-color: #cf9152; background: rgba(255,255,255,0.06); }
  .hdr-grid input::placeholder { color: var(--text-faint); font-weight: 400; }
  .quality { display: flex; align-items: center; gap: 10px; margin-bottom: 18px; }
  .q-label { margin-bottom: 0; }
  .quality input[type="range"] { flex: 1; accent-color: var(--accent, #f49d4e); }
  .q-val { font: 600 12px 'Spline Sans Mono', ui-monospace, monospace; color: var(--text);
    min-width: 26px; text-align: right; }
  .row { display: flex; gap: 10px; justify-content: flex-end; }
  button { padding: 8px 14px; border-radius: 9px; border: 1px solid var(--glass-brd); background: transparent; }
  .go { background: var(--accent-grad); color: white; border: 0; font-weight: 600; }
</style>
