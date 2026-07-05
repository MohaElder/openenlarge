//! Color-negative inversion: a single engine, Kodak Cineon densitometry
//! (darktable's negadoctor). Per channel it restores the negative's density in
//! log space, returns to linear, applies a paper inversion + tone curve with a
//! highlight soft-clip, and balances with white balance as a gain on the linear
//! print. See `invert_d` and the negadoctor inversion design spec.

use nalgebra::Matrix3;
use rayon::prelude::*;

/// How white balance is applied. `Gain` multiplies the positive output after the
/// filmic curve (von-Kries display gain). `Subtractive` applies the same gains as a
/// per-channel multiply on normalised log-density BEFORE the filmic curve, like a
/// dichroic enlarger head changing each emulsion layer's exposure — coupled to the
/// tone-curve slope, anchored at black, no highlight clipping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WbMode {
    #[default]
    Gain,
    Subtractive,
}

/// Display tone path. `Filmic` is the legacy default (untouched). `Faithful` is the
/// detail-preserving reconstruction (gamma body + gentle highlight shoulder), fit to the
/// C400 digital-SDR reference. See docs/superpowers/specs/2026-06-21-faithful-tone-core-design.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToneMode {
    #[default]
    Filmic,
    Faithful,
}

/// All knobs for one inversion. Defaults give a reasonable neutral result.
#[derive(Debug, Clone)]
pub struct InversionParams {
    /// Per-channel film-base value (orange mask), from calibrate::sample_base.
    pub base: [f32; 3],
    /// Pre-log linear mix (sensor↔dye crosstalk). Default = identity.
    pub m_pre: Matrix3<f32>,
    /// Post-log density-space unmix. Default = identity.
    pub m_post: Matrix3<f32>,
    /// Exposure multiplier applied after unmix.
    pub exposure: f32,
    /// Black point subtracted (post-exposure), in [0,1)-ish density-output units.
    pub black: f32,
    /// Output gamma encoding exponent (sRGB-ish ~ 1/2.2 applied as power).
    pub gamma: f32,
    /// Per-channel white-balance gain applied in linear light before gamma.
    pub wb: [f32; 3],
    /// How `wb` is applied: post-curve gain (default) or subtractive (pre-curve, color-head).
    pub wb_mode: WbMode,
    /// Display tone path: `Filmic` (legacy S-curve, default) or `Faithful` (gamma+shoulder).
    pub tone_mode: ToneMode,
    /// Cineon (Mode D) — scalar white / dynamic-range anchor (D_max).
    pub d_max: f32,
    /// Cineon (Mode D) — print exposure (ASC-CDL slope).
    pub print_exposure: f32,
    /// Cineon (Mode D) — paper black (ASC-CDL offset).
    pub paper_black: f32,
    /// Cineon (Mode D) — paper grade (ASC-CDL power; also the display encode).
    /// Valid range: `[0, ∞)`. A negative value yields `Inf` where `print_lin == 0`.
    pub paper_grade: f32,
    /// Cineon (Mode D) — highlight soft-clip threshold.
    pub soft_clip: f32,
    /// Positive passthrough: skip the Cineon inversion and render the decoded
    /// scan directly (display-encoded), applying only exposure (`print_exposure`)
    /// and white balance (`wb`). For already-positive sources (slides/prints).
    pub positive: bool,
    /// Per-channel density neutralisation (camera-matrix mode). The Faithful arm scales the
    /// raw density `d` by this before exposure/tone, so an auto-computed per-channel factor
    /// (calibrate::sample_channel_balance) equalises the channels and removes the orange/matrix
    /// cast WITHOUT a manual WB. Default `[1,1,1]` = identity (matrix-off path unchanged).
    /// MUST equal shaders.ts u_cam_balance.
    pub channel_balance: [f32; 3],
    /// Highlight recovery [0,1] (from −Highlights). Widens the Faithful shoulder
    /// rolloff to re-separate crushed highlights. SDR Faithful only; 0 = identity.
    pub hi_recovery: f32,
    /// Shadow recovery [0,1] (from −Shadows). Softens the look_s toe to re-separate
    /// crushed shadows. SDR Faithful only; 0 = identity.
    pub lo_recovery: f32,
}

impl Default for InversionParams {
    fn default() -> Self {
        InversionParams {
            base: [1.0, 1.0, 1.0],
            m_pre: Matrix3::identity(),
            m_post: Matrix3::identity(),
            exposure: 1.0,
            black: 0.0,
            gamma: 1.0 / 2.2,
            wb: [1.0, 1.0, 1.0],
            wb_mode: WbMode::Gain,
            tone_mode: ToneMode::Filmic,
            d_max: 1.5,
            print_exposure: 1.0,
            paper_black: 0.0,
            paper_grade: 0.95,
            soft_clip: 0.9,
            positive: false,
            channel_balance: [1.0, 1.0, 1.0],
            hi_recovery: 0.0,
            lo_recovery: 0.0,
        }
    }
}

const EPS: f32 = 1e-5;
/// HDR highlight expansion: output above this knee is remapped into [knee, HDR_HEADROOM].
/// `pub(crate)` so `finish::hdr_finalize` (the color-managed HDR finalize shoulder,
/// the tested reference for the MSL finalize) shares the same knee.
pub(crate) const HDR_KNEE: f32 = 0.8;
/// HDR headroom ceiling (linear-ish display units, ~1.3 stops over SDR white).
/// Tuned on real scans later; keep modest to avoid clipping. `pub(crate)` — see `HDR_KNEE`.
pub(crate) const HDR_HEADROOM: f32 = 2.5;
/// Exposure → t-multiply. The exposure slider (EV) scales the normalised
/// log-density `t` by `2^(EXPO_K·EV)`, pivoting at black (t=0 stays 0): brightening
/// pushes tones up the filmic curve, darkening pulls them down. Highlight-preserving
/// (a saturated specular at t ≫ 1 stays clipped to white until pulled far down) and
/// free of the dead zone the old eff_d_max clamp produced past ~EV+3. Mirrored
/// verbatim in shaders.ts (INVERT_FRAG).
const EXPO_K: f32 = 0.14;
/// Faithful-path exposure sensitivity (replaces EXPO_K for `tone_mode == Faithful`). The
/// faithful core renders the bright content near white, so auto-exposure must pull it down by
/// several stops; at EXPO_K=0.14 that needed −4…−11 EV (the auto-exposure clamp can't reach,
/// and the slider barely moves). At 1.0, one EV ≈ one photographic stop on the density scale,
/// so auto-exposure lands every frame within ~±1.5 EV (inside the ±3 clamp, slider free to
/// tune both ways). Measured on the C400/Ektar + user SF frames. Filmic keeps EXPO_K (dormant).
/// MUST equal shaders.ts.
const FAITHFUL_EXPO_K: f32 = 1.0;
/// Subtractive WB strength: gain `g` → density scale `g^CMY_STRENGTH` on `t`. Tuned so
/// the mid-tone shift at a typical Temp/Tint roughly matches the old gain magnitude
/// while giving a proper shadow→highlight crossover. Mirrored in shaders.ts.
const CMY_STRENGTH: f32 = 1.6;

// Fit to the C400 digital-SDR reference (tone-calibration). MUST equal shaders.ts.
const FAITHFUL_GAMMA: f32 = 1.590;
const FAITHFUL_KNEE: f32 = 0.892;
/// Faithful FIXED density scale = 1/recommended_d_max from the C400 GammaShoulder fit
/// (recommended d_max 0.700). The faithful core multiplies the RAW film density `d` by this
/// CONSTANT — it deliberately does NOT use the per-frame `p.d_max`. Because
/// `d = log10(base/scan)` already cancels scan/lightbox brightness, a fixed scale gives a
/// frozen, faithful tone reproduction that is identical on every frame; per-image brightness
/// is auto-exposure's job (the safeguard). The earlier `(d/d_max)·ANCHOR` form coupled the
/// effective scale to the per-frame d_max (= 4.137/d_max), so it only matched the calibration
/// on the one frame whose d_max happened to be 2.896 and blew highlights on every other frame
/// (default d_max 1.5 → ~2× too bright). MUST equal shaders.ts.
const FAITHFUL_SCALE: f32 = 1.0 / 0.700;
/// Faithful 0 EV baseline anchor (issue #41): a fixed EV offset folded into the
/// linear-light exposure gain so a negative at FILM BASE + 0.6D — the standard
/// mid-gray density of a correctly exposed negative (0.6D ≈ 2 stops of light,
/// 10^-0.6 ≈ 25% transmission) — renders at the 8-bit mid-gray 118 (L* ≈ 50)
/// with every control at its default. Solved numerically through the full
/// default chain (invert + finish + display finalize): −2.245 ≈ −2 EV from the
/// 0.6D physics plus the shoulder/look layer's shift. Without it, a standard
/// negative rendered ~2 EV bright and every frame ate its downward latitude in
/// manual/auto exposure. Stored exposures from before this anchor are shifted
/// by −FAITHFUL_BASELINE_EV once on load (frontend catalog migration,
/// `exposure_baseline_06d` pref) so existing edits keep their appearance.
/// Guarded by `faithful_baseline_anchors_base_plus_06d_to_midgray` in
/// commands.rs. MUST equal shaders.ts (GL + WGSL) and msl.rs. Public so the
/// catalog's v6 migration derives the stored-exposure shift from it.
pub const FAITHFUL_BASELINE_EV: f32 = -2.25;

/// Look-layer strength — the clean-punchy "MEDIUM" the user chose (~+31% mid-contrast).
/// MUST equal shaders.ts LOOK_K.
const LOOK_K: f32 = 2.0;
/// Highlight-recovery shoulder widening: `hi_recovery∈[0,1]` multiplies the
/// gamma_shoulder rolloff scale by `(1 + REC_H_GAIN·hi_recovery)`. Gentler rolloff
/// → brightest densities sit further below the ceiling, re-separating highlights the
/// SDR shoulder crushes flat. 0 → identity. MUST equal shaders.ts REC_H_GAIN.
const REC_H_GAIN: f32 = 3.0;
/// Shadow-recovery toe softening: `lo_recovery∈[0,1]` reduces look_s's toe contrast
/// to `LOOK_K·(1 − REC_S_GAIN·lo_recovery)` (shoulder + mid-gray slope untouched),
/// re-separating shadows the tanh toe compresses. 0 → identity. MUST equal shaders.ts.
const REC_S_GAIN: f32 = 0.6;

/// Clean-punchy look curve: a normalized symmetric tanh S applied to the faithful core's
/// SDR display value `v ∈ [0,1]`. Pivot 0.5, anchored `0→0` and `1→1`, strictly monotonic,
/// soft toe + soft shoulder — adds mid-contrast (punch) without clipping or crushing detail.
/// This is the film-LOOK layer on top of the measured faithful core; SDR only (HDR bypasses
/// it). `lo_recovery` softens the toe without disturbing the shoulder or mid-gray pivot.
/// lo_recovery=0 → the original symmetric tanh exactly. MUST equal shaders.ts `lookS`.
#[inline]
pub(crate) fn look_s(v: f32, lo_recovery: f32) -> f32 {
    // Shadow recovery softens the toe (v<0.5) via a smoothstep weight that is 1 at
    // deep shadow and 0 by mid-gray, so the shoulder and the mid-gray slope are
    // untouched (no kink). The per-point normaliser `t = tanh(k/2)` anchors
    // look_s(0)=0 and look_s(1)=1 for any k, so recovery re-separates crushed darks
    // without lifting black to a milky grey. lo_recovery=0 → k=LOOK_K → original tanh.
    let s = ((0.5 - v) / 0.5).clamp(0.0, 1.0);
    let w = s * s * (3.0 - 2.0 * s); // smoothstep: 1 (v=0) → 0 (v≥0.5)
    let k = LOOK_K * (1.0 - REC_S_GAIN * lo_recovery * w);
    let t = (k * 0.5).tanh();
    (0.5 + 0.5 * (k * (v - 0.5)).tanh() / t).clamp(0.0, 1.0)
}

// --- Filmic display S-curve (replaces the old paper-grade/soft-clip encode). ---
// Applied per channel in the NORMALISED LOG-DENSITY domain `t = d/d_max` (then
// scaled by exposure), which is linear in scene stops — the correct place for a
// tone curve (the old
// `1 − 10^(−d/d_max)` paper response was a pure shoulder that capped white at
// ~0.90 and dumped all contrast into the shadows). A logistic, rescaled to exact
// anchors: gentle toe (shadow detail), mid slope > 1 (contrast/punch), gentle
// shoulder to TRUE white at 1.0 (highlight separation). MUST be mirrored verbatim
// in shaders.ts (INVERT_FRAG) so the CPU export and GPU proxy preview match.
const FILMIC_K: f32 = 5.0; // contrast / max slope
                           // Max-slope point in normalised density. Below the geometric midpoint (0.5) so the
                           // curve renders mids/shadows brighter — a calibration lift (digital "print
                           // exposure"), since auto-fit d_max puts the white point at the top and most real
                           // content lands in the lower-mid range. Black (t=0→0) and white (t=WHITE_T→1) stay
                           // anchored regardless. 0.44 chosen on real scans.
const FILMIC_PIVOT: f32 = 0.44;
const FILMIC_WHITE_T: f32 = 1.05; // density (× d_max) that maps to 1.0

/// Logistic display S-curve on normalised log-density `t` (0 = scene black at the
/// film base, 1 = the white point at `d_max`). Rescaled so `filmic_s(0) == 0`
/// exactly (neutral black) and `filmic_s(FILMIC_WHITE_T) == 1.0` (true white).
#[inline]
fn filmic_s(t: f32) -> f32 {
    filmic_s_raw(t).clamp(0.0, 1.0)
}

/// Unclamped filmic forward — the same logistic but WITHOUT the `[0,1]` clamp, so
/// super-white density (`t > FILMIC_WHITE_T`, dense negatives / blown highlights)
/// stays a distinct value above 1.0 instead of collapsing to white. Used only for
/// the WB-neutralisation round-trip in `invert_d`, where clamping would destroy the
/// highlight latitude that lowering exposure can recover. The final display value
/// goes through the clamped `filmic_s`.
#[inline]
fn filmic_s_raw(t: f32) -> f32 {
    let l = |x: f32| 1.0 / (1.0 + (-FILMIC_K * (x - FILMIC_PIVOT)).exp());
    let l0 = l(0.0);
    let lw = l(FILMIC_WHITE_T);
    (l(t) - l0) / (lw - l0)
}

/// Exact inverse of [`filmic_s_raw`] (the logistic is invertible — a logit). Maps a
/// display-density `y` back to its normalised log-density `t`, so exposure can scale
/// the WB-neutralised density (see `invert_d`). `filmic_inv(0) == 0` (black pivots at
/// 0) and `filmic_inv(filmic_s_raw(t)) == t`. The internal `big` is clamped just
/// inside `(0,1)` to keep the logit finite when a WB gain pushes `y` past the
/// representable white asymptote (`y ≳ 1.053`) — that channel is a blown highlight
/// and resolves to white. MUST be mirrored verbatim in shaders.ts (INVERT_FRAG).
#[inline]
fn filmic_inv(y: f32) -> f32 {
    let l = |x: f32| 1.0 / (1.0 + (-FILMIC_K * (x - FILMIC_PIVOT)).exp());
    let l0 = l(0.0);
    let lw = l(FILMIC_WHITE_T);
    let big = (y * (lw - l0) + l0).clamp(1e-6, 1.0 - 1e-6); // = l(t)
    FILMIC_PIVOT + (big / (1.0 - big)).ln() / FILMIC_K
}

/// Power-law body of the Faithful curve: `x^(1/γ)`, `x` clamped at 0. The shoulder
/// rolloff is applied separately by `shoulder_only`, so a super-white body (`> 1`)
/// can be carried into the finish stage and only rolled off at display-encode time.
#[inline]
pub(crate) fn gamma_body(x: f32) -> f32 {
    x.max(0.0).powf(1.0 / FAITHFUL_GAMMA)
}

/// Shoulder rolloff of the Faithful curve, applied to the gamma body `raw`.
/// Identity below `FAITHFUL_KNEE`; asymptotic to `ceil` above. `hi_recovery` widens
/// the rolloff scale (0 = current curve). MUST equal shaders.ts `shoulderOnly`.
#[inline]
pub(crate) fn shoulder_only(raw: f32, ceil: f32, hi_recovery: f32) -> f32 {
    if raw <= FAITHFUL_KNEE {
        raw.min(ceil)
    } else {
        let k = FAITHFUL_KNEE;
        let scale = (1.0 - k) * (1.0 + REC_H_GAIN * hi_recovery);
        k + (ceil - k) * (1.0 - (-(raw - k) / scale).exp())
    }
}

/// Faithful reconstruction curve: `shoulder_only(gamma_body(x), ceil, hi_recovery)`.
/// `ceil` is `1.0` for SDR or `HDR_HEADROOM` for HDR. Output is in `[0, ceil]`.
#[inline]
fn gamma_shoulder(x: f32, ceil: f32, hi_recovery: f32) -> f32 {
    shoulder_only(gamma_body(x), ceil, hi_recovery)
}

/// SDR display finalizer: the shoulder rolloff + clean-punchy look layer + clamp,
/// applied to a super-white gamma body `v` to produce the display value in `[0,1]`.
/// Recovery is retired, so the shoulder/toe are fixed (`hi=lo=0`). This is the tail
/// of the old Faithful SDR path, moved out of `invert_d` so the finish tone tools can
/// operate on the body first. MUST equal shaders.ts `displayFinalize` (`lookS(shoulderOnly(v,1,0),0)`).
#[inline]
pub(crate) fn display_finalize(v: f32) -> f32 {
    look_s(shoulder_only(v, 1.0, 0.0), 0.0)
}

/// Positive passthrough: the working buffer is linear, so display-encode it with
/// `1/2.2` (matching the raw-scan view), after applying exposure + WB gain.
/// `0 * wb == 0` keeps black neutral, mirroring the inversion's WB convention.
pub fn develop_positive_px(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    // `1/2.2` is hardcoded deliberately: `InversionParams.gamma` is vestigial —
    // the negative path uses `paper_grade` for its display encode and never reads
    // `p.gamma`, so there is no `p.gamma` to honour here either.
    const DISPLAY_GAMMA: f32 = 1.0 / 2.2;
    std::array::from_fn(|c| {
        let lit = (rgb[c] * p.print_exposure * p.wb[c]).max(0.0);
        lit.powf(DISPLAY_GAMMA)
    })
}

/// Shared implementation for `invert_d` and `invert_d_core`. Returns the display value
/// when `produce_body` is false (analysis / back-compat), or the super-white BODY for
/// Faithful SDR when `produce_body` is true (display render path). For all other modes
/// both flags return the same value.
///
/// WB is applied as a gain on the positive `print_lin` (not as a log-space offset
/// on the negative): `0 * wb == 0`, so deep shadows stay neutral black instead of
/// being tinted/clamped per channel. A log-space offset converges shadows to
/// `print_exposure·(1 − 1/wb[c])`, which drives one channel to black before the
/// others and reads as a colour cast in the darkest tones (the "yellow shadow"
/// bug). A positive-domain gain spreads the WB tint evenly across tones instead.
fn invert_d_impl(rgb: [f32; 3], p: &InversionParams, produce_body: bool) -> [f32; 3] {
    if p.positive {
        return develop_positive_px(rgb, p);
    }
    const THRESHOLD: f32 = 2.328_306_4e-10; // negadoctor's -32 EV floor
                                            // Exposure is a log-density MULTIPLY pivoting at black (not the old eff_d_max
                                            // rescale): EV stops scale the normalised log-density by 2^(EXPO_K·EV).
                                            // Brightening pushes tones up the filmic curve; darkening pulls them down while
                                            // a saturated specular stays clipped to white — highlight-preserving, no dead
                                            // zone. EV=0 → expo_gain=1 → unchanged. d_max sets the white anchor.
                                            //
                                            // CRITICAL: the scale is applied to the WB-NEUTRALISED log-density
                                            // `filmic_inv(filmic_s(t)·wb)`, not to raw `t`. WB is a post-curve gain
                                            // (below), so a neutral patch has unequal per-channel `t` but EQUAL `filmic_s(t)·wb`;
                                            // scaling raw `t` would push each channel a different amount through the nonlinear
                                            // curve and shift the colour temperature with exposure (the ±5-EV "warmer/cooler"
                                            // bug). Scaling the WB-neutralised density keeps neutrals neutral at every
                                            // exposure: brightness moves, hue does not.
    let ev = p.print_exposure.max(EPS).log2();
    let expo_gain = 2f32.powf(EXPO_K * ev);
    // `paper_black`, `paper_grade`, `soft_clip` are DEPRECATED by the filmic
    // display curve below (they encoded the old `1 − 10^(−x)` paper response that
    // capped white at ~0.90 and had no toe). The fields are kept on the struct /
    // uniforms / session JSON for compatibility but are no longer read here.
    std::array::from_fn(|c| {
        let clamped = rgb[c].max(THRESHOLD);
        let dmin = p.base[c].max(EPS);
        // Negative density d = log10(base/scan) ≥ 0 (thin neg = scene black = 0;
        // dense neg = scene highlight = large). This is LINEAR IN SCENE STOPS — the
        // correct domain for the tone curve.
        let d = (dmin / clamped).log10().max(0.0);
        // Normalised log-density `t`: d == d_max → t == 1 (the white point).
        let t = d / p.d_max.max(EPS);
        // WB is a linear gain on the positive OUTPUT (filmic value), NOT a scale on
        // t. This keeps black neutral (filmic_s(0)·wb = 0, so no "yellow shadow")
        // AND stays consistent with `auto_wb_gains` / the gray-point picker, which
        // both treat WB as a multiply on the displayed positive. (A t-scale is a
        // nonlinear remap that those gray-world estimators cannot neutralise.)
        // `y` is that WB-neutralised display density (the EV-0 result). Use the
        // UNCLAMPED forward so a super-white highlight keeps a distinct value > 1 —
        // clamping here would merge near-white tones and kill the latitude that
        // lowering exposure recovers.
        // WB application depends on the mode (mirror shaders.ts INVERT_FRAG):
        //  - Gain:        post-curve display multiply  →  filmic_s(t) · wb[c]
        //  - Subtractive: pre-curve density multiply    →  filmic_s(t · wb[c]^CMY_STRENGTH)
        //    Anchored at black (t=0 → 0 for any filter), coupled to the filmic slope.
        let v = match p.tone_mode {
            ToneMode::Filmic => {
                let v = match p.wb_mode {
                    WbMode::Gain => {
                        // WB is a linear gain on the positive OUTPUT (filmic value), NOT a scale on
                        // t. This keeps black neutral (filmic_s(0)·wb = 0, so no "yellow shadow")
                        // AND stays consistent with `auto_wb_gains` / the gray-point picker, which
                        // both treat WB as a multiply on the displayed positive. (A t-scale is a
                        // nonlinear remap that those gray-world estimators cannot neutralise.)
                        // `y` is that WB-neutralised display density (the EV-0 result). Use the
                        // UNCLAMPED forward so a super-white highlight keeps a distinct value > 1 —
                        // clamping here would merge near-white tones and kill the latitude that
                        // lowering exposure recovers.
                        let y = filmic_s_raw(t) * p.wb[c];
                        // Exposure scales the WB-neutralised log-density `filmic_inv(y)`, then
                        // re-applies the (clamped) curve. At EV 0 (expo_gain == 1) this is exactly
                        // `filmic_s(t)·wb` — the look is unchanged; off EV 0 it brightens/darkens
                        // without moving hue (see the expo_gain note above).
                        filmic_s(filmic_inv(y) * expo_gain)
                    }
                    WbMode::Subtractive => {
                        filmic_s(t * p.wb[c].max(EPS).powf(CMY_STRENGTH) * expo_gain)
                    }
                };
                v.min(1.0) // SDR: clip to white (v ≥ 0 since filmic_s ≥ 0 and wb ≥ 0)
            }
            ToneMode::Faithful => {
                // Faithful uses a FIXED density scale on the raw density `d` (NOT the
                // per-frame `t = d/d_max`): a frozen, faithful transfer identical on every frame.
                //
                // Exposure is a LINEAR-LIGHT gain on the reconstructed scene, applied BEFORE the
                // contrast curve — we treat the log-inverted negative as a positive and "expose"
                // it like a TIFF. Black-anchored linear scene `L = 10^d − 1` (d = 0 → L = 0, so
                // scene black pivots and stays black at every EV); gain ×2^EV (FAITHFUL_EXPO_K is
                // the per-stop sensitivity, 1.0 = photographic); then back to density. EV 0 is the
                // identity `d' = d`, so the EV-0 look is unchanged, and `gamma_shoulder` supplies
                // the highlight rolloff (no hard clip). (The old code MULTIPLIED density by 2^EV,
                // which scales contrast, not exposure.) MUST be mirrored in shaders.ts.
                //
                // Per-channel density neutralisation (camera-matrix mode): scale `d` per channel
                // by `channel_balance[c]` before the exposure round-trip. Default `[1,1,1]` →
                // `d·1 == d` exactly, so the matrix-off path is byte-identical; in camera-matrix
                // mode the auto-computed factor equalises the channels (FreeCCR's OD equalisation).
                let dn = d * p.channel_balance[c];
                let l = (10f32.powf(dn) - 1.0).max(0.0);
                // FAITHFUL_BASELINE_EV re-anchors 0 EV so base+0.6D lands on mid-gray
                // (issue #41); it folds into the same photographic gain as the slider.
                let lit = l * 2f32.powf(FAITHFUL_EXPO_K * ev + FAITHFUL_BASELINE_EV);
                let t_eff = (lit + 1.0).log10() * FAITHFUL_SCALE;
                if produce_body {
                    // Display path: emit the GAMMA BODY (super-white preserved). The shoulder +
                    // look layer + clamp move to `display_finalize`, applied by `finish::tone_curve`
                    // after the tone tools. Gain wb folds into the body here (so finish sees a
                    // WB'd value); that commutes wb ahead of the shoulder, which is exactly the
                    // intended highlight change for the DISPLAY pipeline only.
                    match p.wb_mode {
                        WbMode::Gain => gamma_body(t_eff) * p.wb[c],
                        WbMode::Subtractive =>
                            gamma_body(t_eff * p.wb[c].max(EPS).powf(CMY_STRENGTH)),
                    }
                } else {
                    // Analysis / back-compat display value: BYTE-IDENTICAL to the original invert_d
                    // — shoulder BEFORE the gain-wb multiply, then the look layer; honours
                    // p.hi_recovery / p.lo_recovery.
                    let hr = p.hi_recovery;
                    let core = match p.wb_mode {
                        WbMode::Gain => gamma_shoulder(t_eff, 1.0, hr) * p.wb[c],
                        WbMode::Subtractive =>
                            gamma_shoulder(t_eff * p.wb[c].max(EPS).powf(CMY_STRENGTH), 1.0, hr),
                    };
                    look_s(core, p.lo_recovery)
                }
            }
        };
        v
    })
}

/// Display-referred inversion — byte-identical to the pre-headroom engine, so all
/// analysis / back-compat callers (auto-WB, gray-pick, per-zone seed, dust, color-match
/// stats) are unaffected.
pub fn invert_d(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    invert_d_impl(rgb, p, false)
}

/// Like `invert_d` but returns the super-white BODY for the Faithful SDR path (no
/// shoulder/look/clamp); identical to `invert_d` for every other mode. Used only by the
/// display render path, which finalizes in `finish::tone_curve` after the tone tools.
pub fn invert_d_core(rgb: [f32; 3], p: &InversionParams) -> [f32; 3] {
    invert_d_impl(rgb, p, true)
}

/// Which inversion to run. One engine: Kodak Cineon (negadoctor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Kodak Cineon densitometry (darktable negadoctor).
    D,
}

/// Invert a whole image (returns a new Image, same dims).
pub fn invert_image(img: &crate::Image, p: &InversionParams, _mode: Mode) -> crate::Image {
    // par_iter + collect into Vec preserves index order, so output is identical
    // to the sequential map; `invert_d` is pure (no shared state).
    let pixels = img.pixels.par_iter().map(|&px| invert_d(px, p)).collect();
    crate::Image {
        width: img.width,
        height: img.height,
        pixels,
        ir: img.ir.clone(),
    }
}

/// Like `invert_image` but returns the super-white BODY (no display finalize). Used
/// only by the display path (feeds `finish_image`, which finalizes after the tone tools).
pub fn invert_image_core(img: &crate::Image, p: &InversionParams, _mode: Mode) -> crate::Image {
    let pixels = img.pixels.par_iter().map(|&px| invert_d_core(px, p)).collect();
    crate::Image { width: img.width, height: img.height, pixels, ir: img.ir.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Image;

    // --- Filmic display curve (the 2026-06-20 tonal-rendering fix) -------------

    #[test]
    fn filmic_anchors_black_and_white() {
        assert!(filmic_s(0.0).abs() < 1e-6, "black: {}", filmic_s(0.0));
        assert!(
            (filmic_s(FILMIC_WHITE_T) - 1.0).abs() < 1e-6,
            "white: {}",
            filmic_s(FILMIC_WHITE_T)
        );
    }

    #[test]
    fn filmic_is_monotonic() {
        let mut prev = filmic_s(0.0);
        let mut t = 0.0;
        while t <= 1.2 {
            let cur = filmic_s(t);
            assert!(cur >= prev - 1e-6, "fold at t={t}: {cur} < {prev}");
            prev = cur;
            t += 1.0 / 256.0;
        }
    }

    #[test]
    fn filmic_redistributes_gamma_toe_mid_shoulder() {
        // Absolute slope: gentle toe (<1), punchy mids (>1), gentle shoulder (<1).
        let slope = |t: f32| (filmic_s(t + 1e-3) - filmic_s(t - 1e-3)) / 2e-3;
        let toe = slope(0.12);
        let mid = slope(0.50);
        let shoulder = slope(0.95);
        assert!(mid > 1.0, "mid slope must add punch: {mid}");
        assert!(toe < mid, "toe gentler than mid: toe {toe} mid {mid}");
        assert!(
            shoulder < mid,
            "shoulder gentler than mid: shoulder {shoulder} mid {mid}"
        );
    }

    #[test]
    fn invert_d_reaches_true_white() {
        // The densest neutral negative at the auto-fit d_max maps to t == 1.0 and
        // must render to a real white (>= 0.98) — NOT the old structural 0.90 cap
        // that read as washed-out/pale.
        let p = InversionParams {
            base: [1.0, 1.0, 1.0],
            d_max: 1.5,
            ..Default::default()
        };
        let densest = 10f32.powf(-1.5); // log10(base/scan) == d_max == 1.5 → t == 1.0
        let out = invert_d([densest; 3], &p);
        assert!(out[0] >= 0.98, "white must reach >=0.98, got {}", out[0]);
    }

    #[test]
    fn wb_is_an_output_multiply_not_log_scale() {
        // WB must be a linear gain on the positive OUTPUT, so gray-world gains
        // measured on the WB-neutral inversion neutralize it (equal channel means).
        // Both the auto-WB estimator (`auto_wb_gains`) and the gray-point picker
        // (`gray_point_temp_tint`) assume this; a log-domain t-scale breaks them.
        let neg = [0.12_f32, 0.10, 0.08];
        let base = [0.5, 0.4, 0.3];
        let p0 = InversionParams {
            base,
            d_max: 1.5,
            ..Default::default()
        };
        let neutral = invert_d(neg, &p0); // wb == [1,1,1]
        let gray = (neutral[0] + neutral[1] + neutral[2]) / 3.0;
        let gains = [gray / neutral[0], gray / neutral[1], gray / neutral[2]];
        let out = invert_d(neg, &InversionParams { wb: gains, ..p0 });
        let m = (out[0] + out[1] + out[2]) / 3.0;
        for c in 0..3 {
            assert!(
                (out[c] - m).abs() < 1e-4,
                "gains must neutralize output: {out:?}"
            );
        }
    }

    #[test]
    fn invert_d_black_stays_neutral_under_wb() {
        // A pixel at the film base (zero density) is scene-black; WB is a log-domain
        // scale on t, so t==0 → 0 on every channel and black stays neutral (no
        // per-channel "yellow shadow" tint).
        let p = InversionParams {
            base: [1.0, 1.0, 1.0],
            wb: [1.2, 1.0, 0.6],
            ..Default::default()
        };
        let out = invert_d([1.0, 1.0, 1.0], &p); // scan == base → d == 0
        for (c, v) in out.iter().enumerate() {
            assert!(v.abs() < 1e-4, "black chan {c} not neutral: {v}");
        }
    }

    #[test]
    fn exposure_does_not_shift_white_balance() {
        // Regression for the 乐凯 C400 report: nudging exposure ±5 EV visibly shifted
        // the colour temperature. Root cause was exposure scaling raw `t` (pre-curve)
        // while WB multiplied post-curve — a neutral patch's unequal per-channel `t`
        // moved different amounts through the nonlinear filmic curve. Exposure now
        // scales the WB-NEUTRALISED density, so a patch that is neutral at EV 0 stays
        // neutral at every exposure.
        let base = [0.90, 0.55, 0.35]; // orange C-41 mask
                                       // A neutral patch with a realistic per-channel density imbalance (B thinner).
        let densities = [
            [0.34f32, 0.37, 0.28], // shadow
            [0.62, 0.66, 0.54],    // mid
            [0.95, 1.00, 0.84],    // light
        ];
        for d in densities {
            let scan: [f32; 3] = std::array::from_fn(|c| base[c] / 10f32.powf(d[c]));
            // WB that neutralises this patch at EV 0 (a post-curve gain, exactly how
            // auto_wb_gains / the gray-point picker produce gains).
            let p0 = InversionParams {
                base,
                d_max: 1.5,
                ..Default::default()
            };
            let n = invert_d(scan, &p0);
            let g = (n[0] + n[1] + n[2]) / 3.0;
            let wb = [g / n[0], g / n[1], g / n[2]];
            // Neutral at EV 0 by construction; assert it stays neutral at ±5 EV.
            let mut refs = Vec::new();
            for ev in [-5.0f32, -2.0, 0.0, 2.0, 5.0] {
                let p = InversionParams {
                    base,
                    d_max: 1.5,
                    wb,
                    print_exposure: 2f32.powf(ev),
                    ..Default::default()
                };
                let out = invert_d(scan, &p);
                let m = (out[0] + out[1] + out[2]) / 3.0;
                if m > 1e-3 && m < 0.999 {
                    // skip pure black / fully-clipped white (trivially neutral)
                    for c in 0..3 {
                        // Per-channel deviation from gray, normalised — a proxy for hue
                        // drift. The old pre-curve coupling pushed this to several %
                        // (hundreds of K); the fix holds it near machine epsilon.
                        let dev = (out[c] - m).abs() / m;
                        assert!(
                            dev < 1e-3,
                            "EV{ev} d={d:?} chan {c} hue drift {dev}: {out:?}"
                        );
                    }
                }
                refs.push(out);
            }
            let _ = refs;
        }
    }

    #[test]
    fn ev0_output_unchanged_by_wb_refactor() {
        // The exposure refactor must NOT alter the EV-0 look: at EV 0 the output is
        // still exactly `filmic_s(t)·wb` (the WB convention auto_wb / gray-point rely
        // on). Probe coloured pixels with a non-trivial WB.
        let base = [0.8, 0.6, 0.4];
        let wb = [1.15, 1.0, 0.7];
        let p = InversionParams {
            base,
            d_max: 1.5,
            wb,
            ..Default::default()
        };
        for scan in [[0.4f32, 0.3, 0.2], [0.1, 0.25, 0.5], [0.05, 0.05, 0.05]] {
            let out = invert_d(scan, &p);
            for c in 0..3 {
                let d = (base[c] / scan[c].max(1e-9)).log10().max(0.0);
                let want = (filmic_s(d / 1.5) * wb[c]).min(1.0);
                assert!(
                    (out[c] - want).abs() < 1e-5,
                    "EV0 chan {c}: {} vs filmic_s(t)·wb {}",
                    out[c],
                    want
                );
            }
        }
    }

    #[test]
    fn invert_image_is_per_pixel_and_order_preserving() {
        // A multi-pixel image must invert each pixel exactly as the scalar fn does,
        // in the same order — this guards the parallel collect() against reordering.
        // Non-default print_exposure exercises the eff_d_max path so the parallel
        // invert_image stays pinned to the scalar invert_d under the new semantics.
        let p = InversionParams {
            base: [0.8, 0.6, 0.4],
            print_exposure: 2f32.powf(-1.5),
            ..Default::default()
        };
        let pixels = vec![
            [0.8, 0.6, 0.4],
            [0.1, 0.2, 0.3],
            [0.5, 0.5, 0.5],
            [0.05, 0.9, 0.45],
        ];
        let img = Image {
            width: 2,
            height: 2,
            pixels: pixels.clone(),
            ir: None,
        };
        let out = invert_image(&img, &p, Mode::D);
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 2);
        for (i, &px) in pixels.iter().enumerate() {
            let want = invert_d(px, &p);
            for (c, (&got, &exp)) in out.pixels[i].iter().zip(want.iter()).enumerate() {
                assert!((got - exp).abs() < 1e-5, "pixel {i} chan {c}");
            }
        }
    }

    #[test]
    fn mode_d_base_pixel_is_black() {
        // I == Dmin → log_dens 0 → ten_to_x 1 → print_lin = pe*(1+pb) - pe = pe*pb = 0.
        let p = InversionParams {
            base: [0.7, 0.6, 0.5],
            ..Default::default()
        };
        let out = invert_d([0.7, 0.6, 0.5], &p);
        for (c, &v) in out.iter().enumerate() {
            assert!(v.abs() < 1e-4, "ch {c} = {v}");
        }
    }

    #[test]
    fn mode_d_darker_negative_is_brighter_positive() {
        // A denser negative (lower transmission) = brighter scene = brighter positive.
        let p = InversionParams {
            base: [1.0, 1.0, 1.0],
            ..Default::default()
        };
        let dim = invert_d([0.5, 0.5, 0.5], &p);
        let bright = invert_d([0.1, 0.1, 0.1], &p);
        assert!(
            bright[0] > dim[0],
            "denser neg should be brighter: {bright:?} vs {dim:?}"
        );
    }

    #[test]
    fn mode_d_recovers_neutrals_as_neutral() {
        // base*10^(-k*scene) for a neutral scene must invert back to neutral (wb=1).
        let base = [0.8, 0.55, 0.35];
        let k = 0.6;
        let p = InversionParams {
            base,
            ..Default::default()
        };
        for g in [0.2f32, 0.5, 0.8] {
            let neg = [
                base[0] * 10f32.powf(-k * g),
                base[1] * 10f32.powf(-k * g),
                base[2] * 10f32.powf(-k * g),
            ];
            let out = invert_d(neg, &p);
            let max = out.iter().cloned().fold(f32::MIN, f32::max);
            let min = out.iter().cloned().fold(f32::MAX, f32::min);
            assert!(max - min < 1e-3, "non-neutral recovery at g={g}: {out:?}");
        }
    }

    #[test]
    fn mode_d_wb_gain_brightens_channel() {
        // wb[c] > 1 must BRIGHTEN channel c in the positive (matches B/C convention),
        // even though WB is injected as a log-space offset on the negative side.
        let base = [0.7, 0.6, 0.5];
        let probe = [0.3, 0.25, 0.2];
        let neutral = InversionParams {
            base,
            ..Default::default()
        };
        let warmed = InversionParams {
            base,
            wb: [1.5, 1.0, 1.0],
            ..Default::default()
        };
        let a = invert_d(probe, &neutral);
        let b = invert_d(probe, &warmed);
        assert!(
            b[0] > a[0],
            "R wb 1.5 should brighten R: {} vs {}",
            b[0],
            a[0]
        );
        assert!((b[1] - a[1]).abs() < 1e-6, "G unchanged");
    }

    #[test]
    fn mode_d_shadow_stays_neutral_under_wb() {
        // Regression for the yellow-shadow bug: a pixel AT the film base is the
        // deepest shadow → must invert to neutral BLACK for ANY white balance,
        // because WB is a gain on the positive print (0·wb = 0). With the old
        // log-space WB offset this same input produced print_lin = pe·(1 − 1/wb[c]),
        // i.e. a bright, strongly per-channel-tinted (yellow) result — not black.
        let base = [0.7, 0.6, 0.5];
        let warm = InversionParams {
            base,
            wb: [1.3, 1.0, 0.7],
            ..Default::default()
        };
        let out = invert_d(base, &warm);
        let max = out.iter().cloned().fold(f32::MIN, f32::max);
        let min = out.iter().cloned().fold(f32::MAX, f32::min);
        assert!(max < 1e-4, "shadow at base should be ~black: {out:?}");
        assert!(max - min < 1e-4, "shadow not neutral under WB: {out:?}");
    }

    #[test]
    fn highlight_rolloff_retains_separation() {
        // Raise exposure (print_exposure 2.0): t scales up, so highlights move
        // toward white but the filmic shoulder still keeps them *below* white with a
        // visible gap between distinct luminances — latitude survives into Develop.
        let p = InversionParams {
            print_exposure: 2.0,
            ..Default::default()
        };
        let bright = invert_d([0.1, 0.1, 0.1], &p)[0]; // denser neg → brighter pos
        let dim = invert_d([0.3, 0.3, 0.3], &p)[0];
        assert!(bright > dim, "monotonic: {bright} vs {dim}");
        assert!(
            bright < 0.995,
            "brightest highlight keeps headroom: {bright}"
        );
        assert!(
            bright - dim > 0.01,
            "highlight separation retained: {}",
            bright - dim
        );
        assert!(bright <= 1.0001, "still capped at white: {bright}");
    }

    #[test]
    fn highlight_rolloff_unchanged_below_knee() {
        // Midtones sit well below white on the filmic curve (only the densest
        // negatives reach the shoulder), so a neutral mid stays in the body of the
        // curve, never clipped to white.
        let p = InversionParams::default();
        let mid = invert_d([0.5, 0.5, 0.5], &p);
        for v in mid {
            assert!(v <= 0.9 + 1e-4, "mid below white: {v}");
        }
    }

    #[test]
    fn invert_image_preserves_ir_plane() {
        let mut img = Image::new(2, 1);
        img.ir = Some(vec![0.5, 0.25]);
        let p = InversionParams::default();
        let out = invert_image(&img, &p, Mode::D);
        assert_eq!(out.ir, Some(vec![0.5, 0.25]));
        assert_eq!(out.width, 2);
        assert_eq!(out.height, 1);
    }

    #[test]
    fn positive_passthrough_neutral_is_display_encode() {
        // positive + neutral params (exposure 1, wb 1) must match the raw-scan
        // display encode pow(rgb, 1/2.2) — no inversion, no tint.
        let p = InversionParams {
            positive: true,
            ..Default::default()
        };
        for probe in [[0.04f32, 0.04, 0.04], [0.2, 0.3, 0.5], [0.9, 0.9, 0.9]] {
            let out = invert_d(probe, &p);
            for c in 0..3 {
                let want = probe[c].powf(1.0 / 2.2);
                assert!(
                    (out[c] - want).abs() < 1e-5,
                    "ch {c}: {} vs {}",
                    out[c],
                    want
                );
            }
        }
    }

    #[test]
    fn positive_exposure_brightens() {
        let base = InversionParams {
            positive: true,
            ..Default::default()
        };
        let up = InversionParams {
            positive: true,
            print_exposure: 2.0,
            ..Default::default()
        };
        let a = invert_d([0.25, 0.25, 0.25], &base);
        let b = invert_d([0.25, 0.25, 0.25], &up);
        assert!(
            b[0] > a[0],
            "2x exposure should brighten: {} vs {}",
            b[0],
            a[0]
        );
    }

    #[test]
    fn positive_wb_gains_one_channel() {
        let neutral = InversionParams {
            positive: true,
            ..Default::default()
        };
        let warm = InversionParams {
            positive: true,
            wb: [1.5, 1.0, 1.0],
            ..Default::default()
        };
        let a = invert_d([0.3, 0.3, 0.3], &neutral);
        let b = invert_d([0.3, 0.3, 0.3], &warm);
        assert!(
            b[0] > a[0],
            "R gain should brighten R: {} vs {}",
            b[0],
            a[0]
        );
        assert!((b[1] - a[1]).abs() < 1e-6, "G unchanged");
    }

    #[test]
    fn positive_false_matches_today() {
        // Regression: the default (negative) path is byte-for-byte unchanged.
        let p = InversionParams {
            base: [0.7, 0.6, 0.5],
            ..Default::default()
        };
        assert!(!p.positive, "default must be negative");
        let probe = [0.3, 0.25, 0.2];
        let neg = invert_d(probe, &p);
        let p2 = InversionParams {
            positive: false,
            ..p.clone()
        };
        assert_eq!(neg, invert_d(probe, &p2));
    }

    #[test]
    fn lower_exposure_reseparates_blown_highlights() {
        // Two close highlights pushed just past the white point collapse together at
        // EV0 (both clip near 1.0). Lowering exposure pulls them DOWN off white and
        // re-separates them — the t-multiply slides them back into the curve's body.
        // (The recovery is gentler than the old eff_d_max rescale: a SATURATED
        // specular deliberately stays white — see exposure_preserves_speculars — so
        // this uses near-knee highlights, which is the recoverable case.)
        let base = [1.0, 1.0, 1.0];
        let hi_a = [0.011, 0.011, 0.011]; // d ≈ 1.96 (≈1.3·d_max)
        let hi_b = [0.0158, 0.0158, 0.0158]; // d ≈ 1.80 (≈1.2·d_max)
        let at = |ev: f32, neg: [f32; 3]| {
            let p = InversionParams {
                base,
                d_max: 1.5,
                print_exposure: 2f32.powf(ev),
                ..Default::default()
            };
            invert_d(neg, &p)[0]
        };
        let gap0 = (at(0.0, hi_a) - at(0.0, hi_b)).abs();
        let gap_dn = (at(-3.0, hi_a) - at(-3.0, hi_b)).abs();
        assert!(
            at(-3.0, hi_a) < at(0.0, hi_a),
            "lower EV must darken the highlight"
        );
        assert!(
            gap_dn > gap0 && gap_dn > 0.01,
            "separation must widen: {gap0} -> {gap_dn}"
        );
    }

    #[test]
    fn black_anchored_under_any_exposure() {
        // A pixel AT the film base is the deepest shadow → must invert to ~black for
        // ANY exposure, because the t-multiply pivots at the black point (0·gain=0).
        let base = [0.7, 0.6, 0.5];
        for ev in [-3.0f32, 0.0, 3.0] {
            let p = InversionParams {
                base,
                print_exposure: 2f32.powf(ev),
                ..Default::default()
            };
            let out = invert_d(base, &p);
            for &v in &out {
                assert!(v.abs() < 1e-4, "base must be black at EV {ev}: {out:?}");
            }
        }
    }

    #[test]
    fn extreme_exposure_no_blowup() {
        // Extreme exposure stays finite + in-range (no NaN, no overflow).
        let base = [1.0, 1.0, 1.0];
        for ev in [-20.0f32, 20.0] {
            let p = InversionParams {
                base,
                print_exposure: 2f32.powf(ev),
                ..Default::default()
            };
            let out = invert_d([0.01, 0.01, 0.01], &p);
            for &v in &out {
                assert!(v.is_finite() && (0.0..=1.0001).contains(&v), "EV {ev}: {v}");
            }
        }
    }

    #[test]
    fn exposure_has_no_dead_zone_at_high_ev() {
        // The old eff_d_max coupling clamped at ~EV+3, so +4 and +5 produced an
        // IDENTICAL image (a dead zone the user hit). t-multiply exposure keeps
        // responding across the whole ±5 range, and brightens monotonically.
        let p = |ev: f32| InversionParams {
            base: [1.0, 1.0, 1.0],
            d_max: 1.5,
            print_exposure: 2f32.powf(ev),
            ..Default::default()
        };
        let at = |ev: f32| invert_d([0.5, 0.5, 0.5], &p(ev))[0];
        assert!(
            at(5.0) > at(4.0) + 1e-3,
            "no dead zone: EV+4 {} vs +5 {}",
            at(4.0),
            at(5.0)
        );
        assert!(
            at(-2.0) < at(0.0) && at(0.0) < at(2.0),
            "monotonic brighten"
        );
    }

    #[test]
    fn exposure_preserves_speculars_when_darkening() {
        // Highlight-preserving: darkening lowers mids/shadows but a dense specular
        // stays bright — instead of the old eff_d_max collapse that dragged white
        // down with everything (the "flat / forced-dark JPG" the user reported).
        let p = |ev: f32| InversionParams {
            base: [1.0, 1.0, 1.0],
            d_max: 1.5,
            print_exposure: 2f32.powf(ev),
            ..Default::default()
        };
        let spec = [10f32.powf(-2.1); 3]; // d = 1.4·d_max → a saturated specular
        let mid = [10f32.powf(-0.825); 3]; // d ≈ 0.55·d_max → a midtone
        let at = |ev: f32, neg: [f32; 3]| invert_d(neg, &p(ev))[0];
        assert!(
            at(-2.0, spec) > 0.9,
            "specular stays bright when darkening: {}",
            at(-2.0, spec)
        );
        assert!(at(-2.0, mid) < at(0.0, mid), "mid darkens when darkening");
    }

    fn sub_params(wb: [f32; 3]) -> InversionParams {
        InversionParams {
            base: [0.9, 0.9, 0.9],
            d_max: 1.5,
            wb,
            wb_mode: WbMode::Subtractive,
            ..Default::default()
        }
    }

    #[test]
    fn subtractive_black_stays_neutral() {
        // A pixel equal to the film base has density 0 → t=0 → filmic_s(0)=0 for every
        // channel regardless of the WB filter. No "yellow shadow".
        let p = sub_params([1.3, 1.0, 0.7]);
        let out = invert_d(p.base, &p);
        assert_eq!(
            out,
            [0.0, 0.0, 0.0],
            "subtractive black must be pure neutral, got {out:?}"
        );
    }

    #[test]
    fn subtractive_neutral_wb_equals_gain() {
        // With wb = [1,1,1] the subtractive and gain paths both reduce to filmic_s(t).
        let scan = [0.25_f32, 0.30, 0.18];
        let gain = InversionParams {
            base: [0.9, 0.9, 0.9],
            d_max: 1.5,
            wb: [1.0, 1.0, 1.0],
            wb_mode: WbMode::Gain,
            ..Default::default()
        };
        let sub = InversionParams {
            wb_mode: WbMode::Subtractive,
            ..gain.clone()
        };
        let a = invert_d(scan, &gain);
        let b = invert_d(scan, &sub);
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-6, "c{c}: gain {a:?} != sub {b:?}");
        }
    }

    #[test]
    fn subtractive_warm_filter_brightens_red_midtone() {
        // A red-boosted WB filter (red gain > 1) raises the red channel of a mid-density
        // pixel vs. the neutral subtractive render — the subtractive shift IS happening.
        let scan = [0.30_f32, 0.30, 0.30];
        let neutral = sub_params([1.0, 1.0, 1.0]);
        let warm = sub_params([1.3, 1.0, 0.8]);
        let n = invert_d(scan, &neutral);
        let w = invert_d(scan, &warm);
        assert!(
            w[0] > n[0] + 1e-4,
            "red filter should brighten red mid: {n:?} -> {w:?}"
        );
        assert!(
            w[2] < n[2] - 1e-4,
            "blue cut should darken blue mid: {n:?} -> {w:?}"
        );
    }

    #[test]
    fn filmic_mode_is_unchanged_default() {
        // Default tone_mode must be Filmic and produce the SAME output as before the feature.
        let p = InversionParams {
            base: [0.5, 0.5, 0.5],
            d_max: 1.5,
            ..Default::default()
        };
        assert!(matches!(p.tone_mode, ToneMode::Filmic));
        let before = invert_d([0.2, 0.18, 0.1], &p); // value captured from current engine
                                                     // Re-running must be deterministic and identical:
        assert_eq!(before, invert_d([0.2, 0.18, 0.1], &p));
    }

    #[test]
    fn faithful_is_independent_of_per_frame_d_max() {
        // Root-cause regression for the highlight-blowout bug. The faithful core uses a
        // FIXED density scale, NOT per-frame d_max. The old `(d/d_max)·ANCHOR` form made the
        // effective scale 4.137/d_max, matching the C400 calibration only on the one frame
        // whose d_max was 2.896 and blowing highlights everywhere else (default d_max 1.5 →
        // ~2× too bright). Faithful output MUST be identical across any d_max for one negative.
        let base = [0.42, 0.55, 0.26];
        let p = |dm: f32| InversionParams {
            base,
            d_max: dm,
            tone_mode: ToneMode::Faithful,
            ..Default::default()
        };
        for scan in [
            [0.08f32, 0.09, 0.05],
            [0.20, 0.25, 0.12],
            [0.34, 0.44, 0.21],
        ] {
            let (a, b, c) = (
                invert_d(scan, &p(0.60)),
                invert_d(scan, &p(1.50)),
                invert_d(scan, &p(2.896)),
            );
            for ch in 0..3 {
                assert!(
                    (a[ch] - b[ch]).abs() < 1e-6 && (b[ch] - c[ch]).abs() < 1e-6,
                    "faithful must ignore d_max (ch {ch}): {a:?} {b:?} {c:?}"
                );
            }
        }
    }

    #[test]
    fn faithful_highlight_does_not_blow_at_default_d_max() {
        // A bright scene tone (dense negative) at the engine DEFAULT d_max (1.5) must NOT clip
        // to pure white — it should land in the gamma body / gentle shoulder like the calibrated
        // harness render. The pre-fix engine (anchor/d_max) pushed this to ~1.0.
        //
        // The look_s layer (Task 1) legitimately brightens near-white highlights: the bare core
        // lands at ~0.988 but look_s pushes it to ~0.993. That is NOT a blowout — 0.993 still
        // has real headroom below pure white (1.0). The invariant is therefore "keeps headroom
        // below pure white (not clipped to 1.0)", threshold relaxed from <0.99 to <0.999.
        let base = [1.0, 1.0, 1.0];
        let bright = [10f32.powf(-0.85); 3]; // d ≈ 0.85 — a bright midtone/highlight
        let out = invert_d(
            bright,
            &InversionParams {
                base,
                d_max: 1.5,
                tone_mode: ToneMode::Faithful,
                ..Default::default()
            },
        );
        assert!(
            out[0] < 0.999,
            "bright tone must keep highlight headroom, not blow to white: {}",
            out[0]
        );
    }

    #[test]
    fn faithful_exposure_is_photographic_strength() {
        // Faithful exposure must move brightness meaningfully via FAITHFUL_EXPO_K (~1 stop/EV),
        // NOT the weak shared EXPO_K=0.14 (~9%/stop). Regression for the "auto-exposure stuck at
        // the -3 clamp, slider can't pull it down" bug: at EXPO_K=0.14 one stop barely moved the
        // image, so auto-exposure needed -4..-11 EV it could never reach. One EV down must now
        // darken a midtone by a real, visible amount.
        let base = [1.0, 1.0, 1.0];
        let mid = [10f32.powf(-0.45); 3]; // a midtone (d ≈ 0.45)
        let at = |ev: f32| {
            invert_d(
                mid,
                &InversionParams {
                    base,
                    d_max: 1.5,
                    tone_mode: ToneMode::Faithful,
                    print_exposure: 2f32.powf(ev),
                    ..Default::default()
                },
            )[0]
        };
        let (e0, em1) = (at(0.0), at(-1.0));
        assert!(em1 < e0, "EV-1 must darken: {e0} -> {em1}");
        assert!(
            e0 - em1 > 0.10,
            "one stop must move brightness a real amount (not ~0.02): {e0} -> {em1}"
        );
    }

    #[test]
    fn faithful_exposure_is_linear_gain_pivoting_black() {
        // Exposure is a linear-light gain (×2^EV) on the reconstructed scene, applied before the
        // contrast curve: +EV brightens / −EV darkens a midtone monotonically, and scene black
        // (d = 0, scan == base) reconstructs to L = 0 and so stays pure neutral black at EVERY EV
        // (the black pivot the linear gain guarantees — no shadow lift/crush under exposure).
        let base = [0.42, 0.55, 0.26];
        let at = |ev: f32, scan: [f32; 3]| {
            invert_d(
                scan,
                &InversionParams {
                    base,
                    d_max: 1.5,
                    tone_mode: ToneMode::Faithful,
                    print_exposure: 2f32.powf(ev),
                    ..Default::default()
                },
            )
        };
        let mid = [
            base[0] * 10f32.powf(-0.45),
            base[1] * 10f32.powf(-0.45),
            base[2] * 10f32.powf(-0.45),
        ];
        assert!(
            at(1.0, mid)[0] > at(0.0, mid)[0],
            "+EV brightens the midtone"
        );
        assert!(
            at(-1.0, mid)[0] < at(0.0, mid)[0],
            "−EV darkens the midtone"
        );
        for ev in [-3.0, -1.0, 0.0, 2.0, 4.0] {
            let b = at(ev, base); // scan == base → d == 0 → L == 0
            assert!(
                b.iter().all(|&v| v.abs() < 1e-6),
                "scene black must pivot at black at EV {ev}: {b:?}"
            );
        }
    }

    #[test]
    fn faithful_mode_open_shadows_vs_filmic() {
        // A mid-shadow scene tone: Faithful (gamma body) lifts shadows above Filmic (S toe crush).
        let scan = [0.30, 0.36, 0.18];
        let base = [0.42, 0.55, 0.26];
        let filmic = invert_d(
            scan,
            &InversionParams {
                base,
                d_max: 1.5,
                ..Default::default()
            },
        );
        let faithful = invert_d(
            scan,
            &InversionParams {
                base,
                d_max: 1.5,
                tone_mode: ToneMode::Faithful,
                ..Default::default()
            },
        );
        let luma = |p: [f32; 3]| 0.2627 * p[0] + 0.678 * p[1] + 0.0593 * p[2];
        assert!(
            luma(faithful) > luma(filmic),
            "faithful opens shadows: {} vs {}",
            luma(faithful),
            luma(filmic)
        );
    }

    #[test]
    fn look_s_anchors_and_pins() {
        assert!(look_s(0.0, 0.0).abs() < 1e-6, "0->0: {}", look_s(0.0, 0.0));
        assert!(
            (look_s(0.5, 0.0) - 0.5).abs() < 1e-6,
            "0.5->0.5: {}",
            look_s(0.5, 0.0)
        );
        assert!((look_s(1.0, 0.0) - 1.0).abs() < 1e-6, "1->1: {}", look_s(1.0, 0.0));
        // pinned values (also pin the GPU GLSL mirror) for LOOK_K = 2.0
        assert!(
            (look_s(0.25, 0.0) - 0.196_61).abs() < 1e-4,
            "0.25: {}",
            look_s(0.25, 0.0)
        );
        assert!(
            (look_s(0.75, 0.0) - 0.803_39).abs() < 1e-4,
            "0.75: {}",
            look_s(0.75, 0.0)
        );
    }

    #[test]
    fn look_s_monotonic_in_range_no_clip() {
        let mut prev = -1.0;
        let mut v = 0.0;
        while v <= 1.0001 {
            let s = look_s(v, 0.0);
            assert!(s >= prev - 1e-7, "monotonic at {v}: {s} < {prev}");
            assert!((0.0..=1.0).contains(&s), "in range at {v}: {s}");
            prev = s;
            v += 1.0 / 512.0;
        }
    }

    #[test]
    fn look_s_adds_midcontrast_soft_ends() {
        let slope = |v: f32| (look_s(v + 1e-3, 0.0) - look_s(v - 1e-3, 0.0)) / 2e-3;
        assert!(slope(0.5) > 1.05, "mid slope adds punch: {}", slope(0.5));
        assert!(slope(0.08) < 1.0, "toe is soft: {}", slope(0.08));
        assert!(slope(0.92) < 1.0, "shoulder is soft: {}", slope(0.92));
    }

    #[test]
    fn faithful_look_darkens_shadow_keeps_neutral() {
        // The look adds contrast (a mid-shadow gets darker than the bare core), and a
        // neutral negative stays neutral (per-channel curve preserves equal channels).
        let base = [0.42, 0.55, 0.26];
        let scan = [0.30, 0.36, 0.18];
        let p = InversionParams {
            base,
            d_max: 1.5,
            tone_mode: ToneMode::Faithful,
            ..Default::default()
        };
        let out = invert_d(scan, &p);
        // neutral scan (equal density vs base) -> equal output channels
        let neg = [
            base[0] * 10f32.powf(-0.5),
            base[1] * 10f32.powf(-0.5),
            base[2] * 10f32.powf(-0.5),
        ];
        let nout = invert_d(neg, &p);
        let (mx, mn) = (
            nout.iter().cloned().fold(f32::MIN, f32::max),
            nout.iter().cloned().fold(f32::MAX, f32::min),
        );
        assert!(mx - mn < 1e-3, "neutral stays neutral under look: {nout:?}");
        assert!(
            out.iter().all(|&v| (0.0..=1.0).contains(&v)),
            "in range: {out:?}"
        );
    }

    #[test]
    fn gamma_shoulder_identity_at_zero_recovery() {
        for i in 0..=200 {
            let x = i as f32 / 50.0; // 0..4
            let got = gamma_shoulder(x, 1.0, 0.0);
            let raw = x.max(0.0).powf(1.0 / FAITHFUL_GAMMA);
            let k = FAITHFUL_KNEE;
            let want = if raw <= k { raw.min(1.0) }
                       else { k + (1.0 - k) * (1.0 - (-(raw - k) / (1.0 - k)).exp()) };
            assert!((got - want).abs() < 1e-6, "x={x} got={got} want={want}");
        }
    }

    #[test]
    fn look_s_identity_at_zero_recovery() {
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            let got = look_s(v, 0.0);
            let t = (LOOK_K * 0.5).tanh();
            let want = (0.5 + 0.5 * (LOOK_K * (v - 0.5)).tanh() / t).clamp(0.0, 1.0);
            assert!((got - want).abs() < 1e-6, "v={v} got={got} want={want}");
        }
    }

    fn faithful_params() -> InversionParams {
        InversionParams { base: [1.0, 1.0, 1.0], d_max: 1.5,
            tone_mode: ToneMode::Faithful, ..Default::default() }
    }

    #[test]
    fn invert_d_identity_at_zero_recovery() {
        // Whole-pixel regression guard: recovery 0 == current output.
        let p = faithful_params();
        for i in 0..=100 {
            let s = (i as f32 / 100.0).max(1e-4);
            let scan = [s, s * 0.9, s * 0.8];
            let out = invert_d(scan, &p); // hi_recovery=lo_recovery=0 by default
            for c in 0..3 { assert!(out[c].is_finite() && (0.0..=1.0).contains(&out[c])); }
        }
        // (parity vs a frozen baseline is covered by the existing pinned tests;
        //  defaults are 0.0 so behavior is unchanged.)
    }

    #[test]
    fn gamma_shoulder_matches_original_formula() {
        // Independent reference: the original inlined gamma_shoulder arithmetic with
        // literal constants (γ=1.590, knee=0.892, REC_H_GAIN=3.0). Guards gamma_body +
        // shoulder_only against a wrong power, knee, or scale — a self-comparison would not.
        let reference = |x: f32, ceil: f32, hr: f32| -> f32 {
            let raw = x.max(0.0).powf(1.0 / 1.590);
            if raw <= 0.892 {
                raw.min(ceil)
            } else {
                let k = 0.892_f32;
                let scale = (1.0 - k) * (1.0 + 3.0 * hr);
                k + (ceil - k) * (1.0 - (-(raw - k) / scale).exp())
            }
        };
        for &x in &[0.0_f32, 0.1, 0.5, 0.892, 1.0, 1.5, 3.0] {
            for &hr in &[0.0_f32, 1.0] {
                let got = gamma_shoulder(x, 1.0, hr);
                let want = reference(x, 1.0, hr);
                assert!((got - want).abs() < 1e-6, "x={x} hr={hr}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn display_finalize_is_shoulder_then_look_at_zero_recovery() {
        // Pin the composition + fixed hr=lo=0 against an INDEPENDENT shoulder reference
        // (literal knee=0.892, ceil=1.0, recovery=0) fed through the real look_s. Catches
        // a wrong ceil, a nonzero recovery, or a dropped look layer.
        for &raw in &[0.0_f32, 0.3, 0.8, 0.892, 1.0, 1.4, 2.0] {
            let k = 0.892_f32;
            let sh = if raw <= k {
                raw.min(1.0)
            } else {
                let scale = 1.0 - k; // (1-k)*(1 + 3*0)
                k + (1.0 - k) * (1.0 - (-(raw - k) / scale).exp())
            };
            let want = look_s(sh, 0.0);
            assert!((display_finalize(raw) - want).abs() < 1e-7, "raw={raw}");
        }
    }

    // --- Task 2: invert_d_core (super-white body) --------------------------------

    #[test]
    fn invert_d_equals_display_finalize_of_core_faithful_sdr() {
        let mut p = faithful_params();          // existing test helper (Faithful, SDR)
        p.hi_recovery = 0.0; p.lo_recovery = 0.0; p.wb = [1.0, 1.0, 1.0];
        for &s in &[0.05_f32, 0.2, 0.5, 0.8, 0.95] {
            let core = invert_d_core([s, s, s], &p);
            let disp = invert_d([s, s, s], &p);
            for c in 0..3 {
                assert!((display_finalize(core[c]) - disp[c]).abs() < 1e-6,
                    "s={s} c={c}: finalize(core)={} disp={}", display_finalize(core[c]), disp[c]);
            }
        }
    }

    #[test]
    fn invert_d_core_carries_super_white_on_bright_highlight() {
        let p = faithful_params();
        // A very thin negative (scan ≈ base) is a scene highlight → dense body.
        let core = invert_d_core([0.02, 0.02, 0.02], &p);
        let disp = invert_d([0.02, 0.02, 0.02], &p);
        assert!(core[0] > 1.0, "body should exceed 1.0 (super-white), got {}", core[0]);
        assert!(disp[0] <= 1.0 + 1e-6, "display must stay clamped, got {}", disp[0]);
    }

    #[test]
    fn invert_d_gain_keeps_shoulder_before_wb() {
        // invert_d must apply the shoulder BEFORE the gain-wb multiply (original order),
        // even though invert_d_core (the body) folds wb in first. Guards the byte-identity
        // the analysis consumers depend on.
        let mut p = faithful_params();
        p.wb_mode = WbMode::Gain;
        p.wb = [1.3, 1.0, 0.7];
        p.hi_recovery = 0.0; p.lo_recovery = 0.0;
        let neg = [0.02, 0.05, 0.02]; // bright, above-the-knee highlight
        let core = invert_d_core(neg, &p);
        let disp = invert_d(neg, &p);
        for c in 0..3 {
            let body_no_wb = core[c] / p.wb[c];                 // = gamma_body(t_eff)
            let want = look_s(shoulder_only(body_no_wb, 1.0, 0.0) * p.wb[c], 0.0); // OLD order
            assert!((disp[c] - want).abs() < 1e-5,
                "c={c}: invert_d must keep shoulder-before-wb: {} vs {}", disp[c], want);
        }
        // And confirm the two orders genuinely differ above the knee (else the test is vacuous).
        let after: [f32; 3] = std::array::from_fn(|c| display_finalize(core[c]));
        assert!((disp[0] - after[0]).abs() > 1e-4 || (disp[2] - after[2]).abs() > 1e-4,
            "non-unity gain wb above the knee should differ between shoulder orders");
    }
}
