<script lang="ts">
  import { get } from "svelte/store";
  import { t } from "$lib/i18n";
  import { params, activeId, images, cropById, folderBaseByPath, baseSampling, sampledBase, sampledDmax, whitePointPinned, hdrMode } from "../store";
  import { developedFolderImages } from "../export/eligible";
  import { applySelectedTo } from "./copySettings";
  import type { GroupSelection, SettingsSnapshot } from "../roll/apply";
  import ConfirmApplySettings from "../overlay/ConfirmApplySettings.svelte";
  import { api, defaultParams } from "../api";
  import { reseedActive, commitActive } from "./historyStore";
  import { createSeedGuard } from "./seedGuard";
  import { withEffectiveBase } from "./base";
  import { applyAsShotWb } from "./wb";
  import { imageDir } from "../library/folderScope";
  import Icon from "../icons/Icon.svelte";
  import HelpDot from "./HelpDot.svelte";
  import Slider from "./Slider.svelte";
  import { TEMP_GRADIENT, TINT_GRADIENT, SAT_GRADIENT, signed, ev, relKelvin } from "./gradients";
  import { slide } from "svelte/transition";
  import { cubicInOut } from "svelte/easing";

  // Gray-point WB picker: parent (Develop) arms the viewport crosshair and routes the
  // sampled pixel; this component only toggles it and reflects the active state.
  export let onWbPick: (() => void) | null = null;
  export let wbPicking = false;
  // The persistent normalized image crop [x,y,w,h] (null = full frame). Analysis
  // (base / D_max / WB) runs against this so black scan borders don't skew it.
  export let imageCrop: [number, number, number, number] | null = null;
  // Orientation/straighten of the active image. The crop is expressed in oriented
  // space, so analysis must apply the same geometry before sampling (a horizontal
  // flip otherwise re-analyzes the wrong region and shifts brightness).
  export let geom: { rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number } = {};

  let open = true;

  $: activeImg = $images.find((i) => i.id === $activeId);
  $: dir = activeImg ? imageDir(activeImg) : "";

  // The backend auto-detects a film base per developed image. Surface it so the
  // swatch isn't empty when nothing is manually set (the "auto" scope).
  // Mirror crates/film-core/src/calibrate.rs REBATE_CONFIDENCE (keep in sync).
  const REBATE_CONF_UI = 0.12;
  let autoBase: { base: [number, number, number]; confidence: number } | null = null;
  // Only the (image, developed-state) pair should drive a refetch. `activeImg` gets a
  // fresh object reference on every `images` mutation (e.g. the live thumbnail write-back
  // in Develop.svelte), so keying the reactive on the object alone refetches constantly —
  // and since the auto base wobbles between develops, that re-arms the WB auto-seed guard
  // (effBase changes → new key) into an unbounded seed→params→thumbnail→images→base loop.
  let autoBaseKey = "";
  async function loadAutoBase(id: string | null, developed?: boolean) {
    const key = `${id ?? ""}|${developed ? 1 : 0}`;
    if (key === autoBaseKey) return; // unrelated `images` churn — base is unchanged
    autoBaseKey = key;
    if (!id) { autoBase = null; return; }
    try { autoBase = await api.autoBaseInfo(id); }
    catch { autoBase = null; autoBaseKey = ""; } // not developed yet — allow a retry
  }
  // `activeImg?.developed` is a pure reactive trigger: on the first develop of a
  // not-yet-developed image it flips false->true and re-fetches so the swatch fills.
  $: loadAutoBase($activeId, activeImg?.developed);

  $: effBase = $params.base_override ?? (dir ? $folderBaseByPath[dir] : null) ?? autoBase?.base ?? null;

  // Reset any in-progress sampling when the active image changes.
  // eslint-disable-next-line @typescript-eslint/no-unused-expressions
  $: { $activeId; sampledBase.set(null); baseSampling.set(false); sampledDmax.set(null); }
  // 8-bit swatch preview of a linear base (display gamma ~1/2.2).
  const baseCss = (b: [number, number, number] | null) =>
    b ? `rgb(${b.map((v) => Math.round(255 * Math.min(1, Math.max(0, v ** (1 / 2.2))))).join(",")})` : "transparent";
  // Hint to repoint only when the base actually in use is the auto one and its
  // detection confidence is low (no override / folder base to mask it).
  $: lowConfBase =
    !$params.base_override && !(dir && $folderBaseByPath[dir]) &&
    autoBase != null && autoBase.confidence < REBATE_CONF_UI;

  // Tapping the swatch arms the rebate picker (BaseView overlay on the negative).
  function toggleRecalibrate() { baseSampling.update((v) => !v); }

  function isPinned(id: string | null): boolean {
    return !!id && get(whitePointPinned).has(id);
  }
  function setPinned(id: string, on: boolean) {
    whitePointPinned.update((s) => {
      const n = new Set(s);
      if (on) n.add(id); else n.delete(id);
      return n;
    });
  }

  // Apply a freshly measured white-point D_max (from the Tone-section picker in the
  // parent viewport): pin it, override, reseed WB. Handed off via the sampledDmax store.
  function applyWhitePointDmax(d: number) {
    const id = get(activeId); if (!id) { sampledDmax.set(null); return; }
    setPinned(id, true);
    params.update((p) => ({ ...p, d_max_override: d }));
    commitActive();
    autoWb();
    sampledDmax.set(null);
  }
  $: if ($sampledDmax != null) applyWhitePointDmax($sampledDmax);

  // Picking a point auto-applies that base to the ACTIVE image; commitActive puts it
  // in the Cmd+Z / Cmd+Shift+Z undo scope (replacing the old apply / reset buttons).
  function applyBaseThisImage() {
    const s = get(sampledBase);
    if (!s) return;
    params.update((p) => ({ ...p, base_override: s }));
    commitActive();
    sampledBase.set(null); baseSampling.set(false);
  }
  $: if ($sampledBase) applyBaseThisImage();

  // Seed Temp/Tint from the estimated as-shot white point when the image OR the
  // film profile changes (or the effective base changes). The estimate runs
  // against the effective base and stock/mode, so switching a profile or
  // applying a roll calibration re-balances to the correct neutral point.
  // Guard remembers every (image, profile, base) it has seeded — not just the
  // last — so revisiting an image never re-runs the auto seed and clobbers the
  // manual Temp/Tint the user set on it. `force` (Auto button) re-seeds anyway.
  const shouldSeed = createSeedGuard();
  async function seed(id: string | null, stock: string, baseKey: string, force = false) {
    // A deliberate WB (gray-point pick) is sticky: don't let the base/profile-change
    // auto-reseed clobber it. The Auto button (force) overrides and re-takes control.
    if (!force && get(params).wb_manual) return;
    const key = id ? `${id}:${stock}:${baseKey}` : null;
    if (!shouldSeed(key, force)) return;
    try {
      const wb = await api.asShotWb(id!, withEffectiveBase(get(params), dir), imageCrop, geom);
      params.update((p) => ({ ...applyAsShotWb(p, wb), wb_manual: false }));
      // Seed per-zone WB gains from the freshly balanced params so the initial
      // develop has residual-cast corrections applied from the start.
      const pz = await api.perZoneWb(id!, get(params), imageCrop, geom);
      params.update((p) => ({ ...p, pz_sh: pz.sh, pz_mid: pz.mid, pz_hi: pz.hi }));
      // Auto-exposure on initial develop: fold a highlight-preserving exposure into the
      // baseline (after WB, so it measures the balanced positive) the first time a frame
      // is shown — so a fresh inversion opens at a sensible brightness, like auto-WB.
      await seedExposure(id!);
      reseedActive();
    } catch { /* not developed yet */ }
  }
  $: seed($activeId, $params.stock, JSON.stringify(effBase));

  // One-shot-per-image auto-exposure. Only runs while exposure is untouched (default),
  // so it never clobbers a saved or hand-set value; marked seen only on success so a
  // not-yet-developed frame retries on the next seed pass. Folded into the baseline by
  // seed()'s reseedActive — it's part of the initial look, not a separate undo step.
  const expSeeded = new Set<string>();
  async function seedExposure(id: string) {
    if (expSeeded.has(id)) return;
    if (get(params).exposure !== defaultParams().exposure) { expSeeded.add(id); return; }
    const { exposure } = await api.autoBrightness(id, withEffectiveBase(get(params), dir), imageCrop, geom);
    params.update((p) => ({ ...p, exposure }));
    expSeeded.add(id);
  }

  function autoWb() { seed($activeId, $params.stock, JSON.stringify(effBase), true); }

  // Manual auto-exposure: unlike seedExposure (one-shot on first develop), this re-runs
  // the auto-brightness estimate on demand and writes the slider every time the user asks.
  async function autoExposure() {
    const id = $activeId; if (!id) return;
    try {
      const { exposure } = await api.autoBrightness(id, withEffectiveBase(get(params), dir), imageCrop, geom);
      params.update((p) => ({ ...p, exposure }));
      commitActive();
    } catch { /* not developed yet */ }
  }

  // Temp is shown as a signed ± offset from this fixed neutral. Under re-zero
  // (Phase 1), params.temp=5500 always means "no trim / neutral" — the per-image
  // auto-WB correction lives in the hidden wb_baseline gains param — so no
  // per-image as-shot fetch is needed to display or copy the slider value.
  const TEMP_NEUTRAL = 5500;

  // ---- Crop-aware D_max analysis ----
  // Re-derive D_max from the image area (the persistent crop) and apply it to THIS
  // image, then reseed WB so the white point matches the new dynamic range. Fires
  // automatically when the crop's coverage changes (see the reactive block below).
  async function reanalyze(softWb = false) {
    const id = get(activeId); if (!id) return;
    try {
      const { d_max } = await api.analyze(id, withEffectiveBase(get(params), dir), imageCrop, geom);
      params.update((p) => ({ ...p, d_max_override: d_max }));
      commitActive();
      // softWb: re-meter paths (positive flip / border change) must not clobber a
      // deliberate gray-point WB (spec §6). Skip autoWb when wb_manual is set.
      if (!softWb || !get(params).wb_manual) {
        autoWb();
      }
    } catch { /* not developed yet */ }
  }

  // Re-derive D_max only when the crop CHANGES on the current image — not on image
  // switch (the stored develop-time d_max already covers that). Switching no longer
  // recomputes analysis or adds an undo step.
  let lastCrop = { id: "", key: "", orient: "" };
  $: {
    const id = $activeId ?? "";
    const key = JSON.stringify(imageCrop);
    // rot90 / flips / straighten merely RE-ORIENT the same pixels, so D_max (and the
    // WB re-seed reanalyze() does) must not move. Rotating the crop rect changes
    // `imageCrop`, which previously refit the white point and visibly brightened the
    // frame ("exposure jumps on rotate"). Re-derive ONLY when the crop's coverage
    // changes (resize/move) WITHOUT a reorientation.
    const orient = `${geom.rot90 ?? 0}|${geom.flip_h ? 1 : 0}|${geom.flip_v ? 1 : 0}|${geom.angle ?? 0}`;
    if (id && id === lastCrop.id && key !== lastCrop.key && orient === lastCrop.orient && !isPinned(id)) {
      reanalyze();
    }
    lastCrop = { id, key, orient };
  }

  // A user drag of Temp/Tint makes WB user-controlled (sticky vs the base-change
  // auto-reseed). Fires only on real input events, not programmatic seed updates.
  function markWbManual() {
    params.update((p) => (p.wb_manual ? p : { ...p, wb_manual: true }));
  }

  // Re-estimate per-zone WB gains when pz_strength changes. Debounced so rapid
  // slider sweeps don't flood the backend. Uses the same imageCrop/geom/id the
  // main seed call uses (no special guard — the result just overwrites the stored
  // gains; if it fails the existing gains stay in place).
  let pzDebounce: ReturnType<typeof setTimeout> | null = null;
  function reestimatePerZoneWb() {
    const id = get(activeId); if (!id) return;
    if (pzDebounce) clearTimeout(pzDebounce);
    pzDebounce = setTimeout(async () => {
      try {
        const pz = await api.perZoneWb(id, get(params), imageCrop, geom);
        params.update((p) => ({ ...p, pz_sh: pz.sh, pz_mid: pz.mid, pz_hi: pz.hi }));
      } catch { /* not developed yet */ }
    }, 200);
  }

  // Reset every Basic-section control to its default, leaving all other develop
  // state (mode, base_override, black/gamma, tone curve, color grading) untouched.
  // Temp/Tint are re-seeded to the as-shot white point rather than the hard
  // slider defaults, matching the Auto button.
  function resetBasic() {
    const d = defaultParams();
    params.update((p) => ({
      ...p,
      stock: d.stock,
      exposure: d.exposure, brightness: d.brightness,
      contrast: d.contrast, highlights: d.highlights, shadows: d.shadows,
      whites: d.whites, blacks: d.blacks,
      texture: d.texture, vibrance: d.vibrance, saturation: d.saturation,
    }));
    autoWb();
  }

  // Flip this image between negative (Cineon inversion) and positive (passthrough).
  // Re-renders live; analysis (base/D_max) already ran at develop time so the flip
  // is instant in both directions. Undoable.
  function togglePositive() {
    params.update((p) => ({ ...p, positive: !p.positive }));
    commitActive();
    remeterActiveExposure(false);
  }

  // Re-derive metering (D_max + WB + exposure) under the current border/positive
  // state. WB reseeds only when wb_manual is unset (spec §6: a deliberate
  // gray-point WB must survive a positive flip or border change). Exposure
  // re-seeds only when untouched unless `force` (an explicit metering choice).
  function remeterActiveExposure(force: boolean) {
    const id = get(activeId); if (!id) return;
    reanalyze(/*softWb*/ true); // re-derive D_max; WB respects wb_manual (spec §6)
    if (force || get(params).exposure === defaultParams().exposure) {
      autoExposure();
    }
  }

  function setMeterBorder(mode: "auto" | "exclude" | "include") {
    params.update((p) => ({ ...p, meter_border: mode }));
    commitActive();
    remeterActiveExposure(true);
  }

  function setMeterAnchor(anchor: "highlight" | "midgray") {
    params.update((p) => ({ ...p, meter_anchor: anchor }));
    commitActive();
    remeterActiveExposure(true);
  }

  // "Apply to whole roll": open the shared picker, then push the active frame's
  // selected settings onto every developed frame in the current folder.
  let rollApplyIds: string[] | null = null;
  function openRollApply() {
    const ids = get(developedFolderImages).map((f) => f.id);
    if (ids.length) rollApplyIds = ids;
  }
  function confirmRollApply(groups: GroupSelection) {
    const ids = rollApplyIds;
    rollApplyIds = null;
    const id = get(activeId);
    if (!ids || !id) return;
    // Carry Temp as an offset from TEMP_NEUTRAL (5500 K) — the fixed universal
    // neutral under re-zero — so the same relative ± lands on every target frame
    // regardless of per-frame auto-WB. No per-image as-shot fetch needed.
    const src: SettingsSnapshot = {
      params: get(params), crop: get(cropById)[id] ?? null,
      tempOffset: get(params).temp - TEMP_NEUTRAL,
    };
    void applySelectedTo(ids, src, groups);
  }
</script>

<div class="section">
  <div class="head">
    <button class="toggle" on:click={() => (open = !open)}>
      <Icon name={open ? "chevron-down" : "chevron-right"} size={14} />
      <span>{$t('basic.title')}</span>
    </button>
    <span class="headbtns">
      <!-- Negative↔Positive: a two-state segment showing the current classification.
           Clicking the inactive side flips via togglePositive (same as the old Inverse
           button, but it shows state and re-meters). -->
      <span class="negpos">
        <button class="hdrtoggle" class:on={!$params.positive} disabled={!$activeId}
                aria-pressed={!$params.positive}
                on:click={() => { if ($params.positive) togglePositive(); }}>{$t('basic.negative')}</button>
        <button class="hdrtoggle" class:on={$params.positive} disabled={!$activeId}
                aria-pressed={$params.positive}
                on:click={() => { if (!$params.positive) togglePositive(); }}>{$t('basic.positive')}</button>
      </span>
      {#if $hdrMode !== "hidden"}
        <button class="hdrtoggle" class:on={$params.hdr}
                title={$t('basic.hdrTitle')} aria-pressed={$params.hdr}
                on:click={() => { params.update((p) => ({ ...p, hdr: !p.hdr })); commitActive(); }}>
          {$t('basic.hdr')}
        </button>
      {/if}
      <button class="reset" on:click={resetBasic}>{$t('basic.reset')}</button>
    </span>
  </div>

  {#if open}
    <div class="body" transition:slide={{ duration: 280, easing: cubicInOut }}>
      <!-- Metering: Auto (confidence-gated spoke exclusion) is the applied default and has
           no button. The two pills are overrides ON TOP of Auto — clicking the active one
           returns to Auto. Shown for both neg/pos (exposure + WB consume the mask). -->
      <div class="wbhead metahead" title={$t('basic.meterBorderTitle')}>
        <span>{$t('basic.meterBorder')}</span>
        <span class="wbbtns">
          <button class="auto" class:on={$params.meter_border === "exclude"}
                  on:click={() => setMeterBorder($params.meter_border === "exclude" ? "auto" : "exclude")}
                  >{$t('basic.meterBorder.exclude')}</button>
          <button class="auto" class:on={$params.meter_border === "include"}
                  on:click={() => setMeterBorder($params.meter_border === "include" ? "auto" : "include")}
                  >{$t('basic.meterBorder.include')}</button>
          <!-- Exposure anchor override: auto-exposure normalizes the median to 18% gray
               instead of driving the highlights to 0.80 (issue #29). Toggle off = default. -->
          <button class="auto" class:on={$params.meter_anchor === "midgray"}
                  title={$t('basic.meterAnchorTitle')}
                  on:click={() => setMeterAnchor($params.meter_anchor === "midgray" ? "highlight" : "midgray")}
                  >{$t('basic.meterAnchor.midgray')}</button>
        </span>
      </div>

      <!-- Inversion-specific controls only apply to negatives. -->
      {#if !$params.positive}
        <!-- Film Base: tap the swatch to pick the rebate; the pick auto-applies to this image -->
        <div class="sub">{$t('base.title')}</div>
        <button class="baseswatch" class:on={$baseSampling} on:click={toggleRecalibrate}
                title={$t('base.recalibrate')} aria-label={$t('base.recalibrate')}>
          <span class="cube big" style="background:{baseCss(effBase)}"></span>
          <span class="pick"><Icon name="pipette" size={18} /></span>
        </button>
        {#if lowConfBase}
          <p class="lowconf">{$t('base.lowConfidence')}</p>
        {/if}
      {/if}

      <!-- Sync this frame's chosen settings across every frame in the roll. -->
      <button class="recal rollapply" on:click={openRollApply}
              disabled={!$activeId}>{$t('basic.applyToRoll')}</button>

      <!-- White Balance -->
      <div class="sub">{$t('basic.whiteBalance')}</div>
      <div class="wbhead">
        <span>{$t('basic.tempTint')}</span>
        <span class="wbbtns">
          <!-- Manual white-point picker: target/crosshair icon (distinct from the
               Auto WB button so the two are no longer confused). -->
          <button class="wbdrop" class:on={wbPicking} title={$t('basic.grayPick')}
                  aria-label={$t('basic.grayPick')} on:click={() => onWbPick?.()}>
            <Icon name="target" size={14} />
          </button>
          <HelpDot text={$t('basic.grayPickHelp')} />
          <!-- One-click Auto WB (Lightroom-style): sparkles + label, calls as_shot_wb. -->
          <button class="auto autowb" title={$t('basic.autoWbTitle')} on:click={autoWb}>
            <Icon name="sparkles" size={12} />{$t('basic.auto')}
          </button>
        </span>
      </div>
      <!-- Temp: 2000–15000 K on the reciprocal track — matches the engine's auto-WB
           CCT bounds (gains_to_cct, wb.rs) so any Auto/gray-pick estimate is always
           representable and fine-tunable on the slider (a narrower track pinned the
           thumb at its bound and snapped away on the first drag — 意见1). The reciprocal
           scale keeps perceptual travel even; neutral 5500 K sits ~73% along (warm now
           extends to 2000 K, far past the old 3793). Label shown as ± offset from 5500.
           Tint: range trimmed to ±100 and stepped finely (0.1) to kill the
           banding a coarse 1-unit step produced across a sweep (I2). -->
      <Slider label={$t('basic.temp')} min={2000} max={15000} step={0.5} scale="reciprocal" scrubStep={10}
        bind:value={$params.temp} def={TEMP_NEUTRAL} gradient={TEMP_GRADIENT} format={(v) => relKelvin(v - TEMP_NEUTRAL)} on:input={markWbManual} />
      <Slider label={$t('basic.tint')} min={-100} max={100} step={0.1}
        bind:value={$params.tint} def={0} gradient={TINT_GRADIENT} format={signed} on:input={markWbManual} />

      <!-- Per-zone WB: toggle enable + strength slider -->
      <div class="wbhead pzhead">
        <span>{$t('basic.perZone')}</span>
        <button class="auto" class:on={$params.pz_enabled}
                title={$t('basic.perZone')} aria-pressed={$params.pz_enabled}
                on:click={() => { params.update((p) => ({ ...p, pz_enabled: !p.pz_enabled })); commitActive(); }}>
          {$params.pz_enabled ? $t('basic.auto') : $t('basic.off')}
        </button>
      </div>
      {#if $params.pz_enabled}
        <Slider label={$t('basic.perZoneStrength')} min={0} max={1} step={0.01}
          bind:value={$params.pz_strength} def={0.7}
          format={(v) => Math.round(v * 100) + '%'}
          on:input={reestimatePerZoneWb} />
      {/if}

      <!-- Tone -->
      <div class="sub tonehead">
        <span>{$t('basic.tone')}</span>
      </div>
      <Slider label={$t('basic.exposure')} min={-5} max={5} step={0.01} bind:value={$params.exposure} def={0} format={ev}>
        <button slot="label-extra" class="auto autoexp" title={$t('basic.autoExpTitle')} on:click={autoExposure}>{$t('basic.autoExp')}</button>
      </Slider>
      <Slider label={$t('basic.brightness')} min={-100} max={100} bind:value={$params.brightness} def={0} format={signed} />
      <Slider label={$t('basic.contrast')} min={-100} max={100} bind:value={$params.contrast} def={0} format={signed} />
      <Slider label={$t('basic.highlights')} min={-100} max={100} bind:value={$params.highlights} def={0} format={signed} />
      <Slider label={$t('basic.shadows')} min={-100} max={100} bind:value={$params.shadows} def={0} format={signed} />
      <Slider label={$t('basic.whites')} min={-100} max={100} bind:value={$params.whites} def={0} format={signed} />
      <Slider label={$t('basic.blacks')} min={-100} max={100} bind:value={$params.blacks} def={0} format={signed} />

      <!-- Presence -->
      <!-- Texture (spatial USM) is not applied to the HDR rendition, so hide it while HDR
           is on to avoid a control that bakes edge halos into the gain map (see finish_from). -->
      {#if !$params.hdr}
        <Slider label={$t('basic.texture')} min={-100} max={100} bind:value={$params.texture} def={0} format={signed} />
      {/if}
      <Slider label={$t('basic.vibrance')} min={-100} max={100} bind:value={$params.vibrance} def={0} gradient={SAT_GRADIENT} format={signed} />
      <Slider label={$t('basic.saturation')} min={-100} max={100} bind:value={$params.saturation} def={0} gradient={SAT_GRADIENT} format={signed} />
    </div>
  {/if}
</div>

{#if rollApplyIds}
  <ConfirmApplySettings
    title={$t('confirmApply.rollTitle', { count: rollApplyIds.length })}
    sub={$t('confirmApply.rollSub')}
    defaults={{ toneColor: true, crop: true, base: true, exposure: false }}
    on:confirm={(e) => confirmRollApply(e.detail.groups)}
    on:cancel={() => (rollApplyIds = null)} />
{/if}

<style>
  .section { margin-bottom: 12px; }
  .head { display: flex; align-items: center; justify-content: space-between;
    width: 100%; padding: 4px 0; }
  .toggle { display: flex; align-items: center; gap: 6px;
    background: transparent; border: 0; color: var(--text); font-weight: 600;
    padding: 0; cursor: pointer; }
  .reset { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  .headbtns { display: inline-flex; align-items: center; gap: 6px; }
  .hdrtoggle { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; font-weight: 600; }
  .hdrtoggle.on { color: #fff; border-color: var(--accent); background: rgba(244,157,78,0.18); }
  .hdrtoggle:disabled { opacity: 0.4; cursor: not-allowed; }
  /* Neg|Pos two-state segment in the header (reuses .hdrtoggle styling per side). */
  .negpos { display: inline-flex; gap: 4px; }
  .sub { font-size: 11px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-dim); margin: 12px 0 4px; }
  .wbhead { display: flex; justify-content: space-between; align-items: center;
    font-size: 11px; color: var(--text-dim); margin: 4px 0; }
  .auto { background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 8px; font-size: 11px; cursor: pointer; }
  /* Auto WB: icon + label, accent on hover so it reads as the primary one-click action. */
  .autowb { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px 2px 6px; }
  .autowb:hover { color: var(--text); border-color: var(--accent); background: rgba(244,157,78,0.12); }
  /* Toggle-on state for .auto buttons (e.g. color-head toggle). */
  .auto.on { color: #fff; border-color: var(--accent); background: rgba(244,157,78,0.18); }
  .wbbtns { display: inline-flex; align-items: center; gap: 6px; }
  /* Metering label row: label left, the two override pills right. */
  .metahead { margin-top: 12px; }
  .tonehead { display: flex; justify-content: space-between; align-items: center; }
  .wbdrop { display: inline-flex; align-items: center; justify-content: center;
    background: transparent; border: 1px solid var(--glass-brd); color: var(--text-dim);
    border-radius: 6px; padding: 2px 6px; cursor: pointer; }
  .wbdrop.on { color: var(--text); border-color: var(--accent); }

  /* Film Base */
  .cube { width: 16px; height: 16px; border-radius: 4px; border: 1px solid var(--glass-brd);
    flex: none; }
  /* Full-width rectangle swatch — match the re-analysis button's radius + height. */
  .cube.big { width: 100%; height: 30px; border-radius: 8px; box-sizing: border-box; }
  /* The swatch IS the picker: hover (or armed) reveals the pipette overlay.
     7px vertical margin matches the slider rhythm so spacing aligns with others. */
  .baseswatch { position: relative; display: flex; width: 100%; padding: 0; border: 0;
    background: transparent; cursor: pointer; margin: 7px 0; }
  .baseswatch .pick { position: absolute; inset: 0; display: flex; align-items: center;
    justify-content: center; color: #fff; background: rgba(0,0,0,0.4); border-radius: 8px;
    opacity: 0; transition: opacity 120ms; }
  .baseswatch:hover .pick, .baseswatch.on .pick { opacity: 1; }
  .baseswatch.on .cube.big { box-shadow: 0 0 0 2px rgba(244,157,78,0.7); }
  .recal { width: 100%; padding: 7px; border-radius: 8px; font-size: 12px; cursor: pointer;
    border: 1px solid var(--glass-brd); background: transparent; color: var(--text); margin-bottom: 8px;
    transition: border-color 120ms, background 120ms; }
  .recal:hover { border-color: var(--accent); background: rgba(244,157,78,0.12); }
  .lowconf { font-size: 11px; color: rgba(244,157,78,0.9); margin: 6px 0 0; }
  /* "Apply to whole roll" separates the per-image film-base block from WB. */
  .rollapply { margin: 14px 0 16px; }
  .recal:disabled { opacity: 0.4; cursor: not-allowed; }
  .recal:disabled:hover { border-color: var(--glass-brd); background: transparent; }
</style>
