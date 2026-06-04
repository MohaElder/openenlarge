# M_post Matrix Fitting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fit the density-space unmixing matrix `M_post` from a physical forward model built on real digitized film dye spectral densities, so Mode B performs cross-channel dye unmixing and diverges from Mode C.

**Architecture:** A new `spectral` module holds the forward model (`D(λ)=D_min+Σc_jD_j(λ)`, `I_i=Σ L·S_i·10^(−D)`) and an analytic Gaussian sensor. `calibrate::fit_m_post` solves `c ≈ M_post·density` by closed-form linear least squares over a concentration-patch grid. The forward-model and fit math are TDD-tested against in-code synthetic spectral data (deterministic); real dye CSVs (Portra 400, Fuji C200) drive the shipped `--stock` presets. `M_pre` stays identity (out of scope).

**Tech Stack:** Rust, `nalgebra` (Matrix3 + DMatrix least squares), bundled CSV data.

**Reference spec:** `docs/superpowers/specs/2026-06-03-mpost-fitting-design.md`

**Environment:** Work from `/Users/mohaelder/Repos/filmrev`, branch `feat/inversion-poc`. `cargo` is NOT on PATH — prefix every cargo command with `source "$HOME/.cargo/env" && `.

---

## File Structure

```
crates/film-core/
├── data/
│   ├── DATA_SOURCES.md          # origins + licenses + attribution (Task 3)
│   ├── dye_portra400.csv        # wavelength,D_C,D_M,D_Y,D_min  @5nm (Task 3)
│   ├── dye_fujic200.csv         # (Task 3)
│   └── illuminant_d55.csv       # wavelength,power             (Task 3)
└── src/
    ├── spectral.rs              # NEW: grid, Stock, SpectralData, sensor, forward model, load_stock
    ├── calibrate.rs             # EXTEND: fit_m_post()
    ├── engine.rs                # EXTEND: params_for_stock()
    └── lib.rs                   # add `pub mod spectral;`
crates/film-cli/src/main.rs      # add --stock flag
```

---

## Task 1: `spectral` module — grid, sensor, forward model

**Files:**
- Create: `crates/film-core/src/spectral.rs`
- Modify: `crates/film-core/src/lib.rs`

- [ ] **Step 1: Register the module**

In `crates/film-core/src/lib.rs`, add after the other `pub mod` lines:

```rust
pub mod spectral;
```

- [ ] **Step 2: Write `spectral.rs` with the forward model and a synthetic test fixture**

Create `crates/film-core/src/spectral.rs`:

```rust
//! Physical forward model: film dye concentrations → sensor RGB.
//!
//! D(λ) = D_min(λ) + Σ_{j=C,M,Y} c_j · D_j(λ)        (Beer-Lambert)
//! I_i  = Σ_λ L(λ) · S_i(λ) · 10^(−D(λ))             (sensor readout)

/// Which film stock's dye data to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stock {
    Portra400,
    FujiC200,
}

/// Spectral curves on a shared wavelength grid (nm). All Vecs are the same length.
#[derive(Debug, Clone)]
pub struct SpectralData {
    pub wavelengths: Vec<f32>,
    /// Per-unit-concentration spectral density of the C, M, Y dyes.
    pub dye: [Vec<f32>; 3],
    /// Minimum (base) spectral density.
    pub d_min: Vec<f32>,
    /// Light-source spectral power L(λ).
    pub illuminant: Vec<f32>,
    /// Sensor spectral sensitivity S_i(λ) for R, G, B.
    pub sensor: [Vec<f32>; 3],
}

/// The standard working grid: 380..=730 nm at 5 nm steps (71 samples).
pub fn grid_380_730_5() -> Vec<f32> {
    (0..=70).map(|k| 380.0 + 5.0 * k as f32).collect()
}

/// Representative analytic camera sensor: unit-peak Gaussians (R 600/σ30,
/// G 540/σ30, B 460/σ30). Open stand-in; replace with fitted per-camera SS later.
pub fn analytic_sensor(wavelengths: &[f32]) -> [Vec<f32>; 3] {
    let g = |center: f32, sigma: f32| -> Vec<f32> {
        wavelengths
            .iter()
            .map(|&w| (-0.5 * ((w - center) / sigma).powi(2)).exp())
            .collect()
    };
    [g(600.0, 30.0), g(540.0, 30.0), g(460.0, 30.0)]
}

impl SpectralData {
    /// Forward model: concentrations `c = [c_C, c_M, c_Y]` → sensor RGB readout.
    pub fn simulate(&self, c: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for k in 0..self.wavelengths.len() {
            let d = self.d_min[k] + c[0] * self.dye[0][k] + c[1] * self.dye[1][k] + c[2] * self.dye[2][k];
            let lt = self.illuminant[k] * 10f32.powf(-d);
            for i in 0..3 {
                out[i] += lt * self.sensor[i][k];
            }
        }
        out
    }

    /// Clear-film response (c = 0): the per-channel base for density normalization.
    pub fn base(&self) -> [f32; 3] {
        self.simulate([0.0, 0.0, 0.0])
    }
}

/// Deterministic synthetic spectral data with deliberately OVERLAPPING dyes
/// (primary band + a secondary lobe), so the recovered M_post is non-trivial.
/// Used only by tests — keeps the fit math independent of the bundled CSVs.
#[cfg(test)]
pub(crate) fn synthetic_overlapping() -> SpectralData {
    let w = grid_380_730_5();
    let g = |center: f32, sigma: f32, amp: f32| -> Vec<f32> {
        w.iter()
            .map(|&x| amp * (-0.5 * ((x - center) / sigma).powi(2)).exp())
            .collect()
    };
    let add = |a: Vec<f32>, b: Vec<f32>| -> Vec<f32> {
        a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
    };
    // Cyan absorbs red (primary 650) + unwanted green (secondary 560).
    let dye_c = add(g(650.0, 50.0, 1.0), g(560.0, 40.0, 0.2));
    // Magenta absorbs green (550) + unwanted blue (460).
    let dye_m = add(g(550.0, 50.0, 1.0), g(460.0, 40.0, 0.2));
    // Yellow absorbs blue (450) + unwanted green (560).
    let dye_y = add(g(450.0, 50.0, 1.0), g(560.0, 40.0, 0.2));
    let d_min = w.iter().map(|_| 0.1f32).collect();
    let illuminant = w.iter().map(|_| 1.0f32).collect(); // equal-energy for the synthetic test
    let sensor = analytic_sensor(&w);
    SpectralData { wavelengths: w, dye: [dye_c, dye_m, dye_y], d_min, illuminant, sensor }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_71_samples_from_380_to_730() {
        let w = grid_380_730_5();
        assert_eq!(w.len(), 71);
        assert_eq!(w[0], 380.0);
        assert_eq!(*w.last().unwrap(), 730.0);
    }

    #[test]
    fn sensor_peaks_are_unit_at_centers() {
        let w = grid_380_730_5();
        let s = analytic_sensor(&w);
        // index of 600nm = (600-380)/5 = 44; 540 -> 32; 460 -> 16
        assert!((s[0][44] - 1.0).abs() < 1e-6);
        assert!((s[1][32] - 1.0).abs() < 1e-6);
        assert!((s[2][16] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn simulate_zero_conc_equals_base_and_is_brightest() {
        let d = synthetic_overlapping();
        let base = d.base();
        // Any positive concentration adds density → strictly less transmission → dimmer.
        let dyed = d.simulate([1.0, 1.0, 1.0]);
        for i in 0..3 {
            assert!(base[i] > 0.0);
            assert!(dyed[i] < base[i], "channel {i}: dyed {} !< base {}", dyed[i], base[i]);
        }
    }

    #[test]
    fn cyan_dye_darkens_red_channel_most() {
        let d = synthetic_overlapping();
        let base = d.base();
        let cyan = d.simulate([1.5, 0.0, 0.0]); // only cyan dye
        // Cyan primarily absorbs red → R drops more than B.
        let red_drop = (base[0] - cyan[0]) / base[0];
        let blue_drop = (base[2] - cyan[2]) / base[2];
        assert!(red_drop > blue_drop, "red_drop {red_drop} !> blue_drop {blue_drop}");
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core spectral::`
Expected: 4 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/film-core/src/spectral.rs crates/film-core/src/lib.rs
git commit -m "feat(core): spectral forward model + analytic sensor + synthetic fixture"
```

---

## Task 2: `fit_m_post` — closed-form least squares

**Files:**
- Modify: `crates/film-core/src/calibrate.rs`

- [ ] **Step 1: Write `fit_m_post` above the existing `#[cfg(test)]` block**

At the top of `crates/film-core/src/calibrate.rs`, add the imports and function. The existing
`use crate::Image;` stays; add:

```rust
use crate::spectral::SpectralData;
use nalgebra::{DMatrix, Matrix3, Vector3};

/// Concentration levels used to build the fitting patch grid (6³ = 216 patches).
const FIT_LEVELS: [f32; 6] = [0.0, 0.4, 0.8, 1.2, 1.6, 2.0];

/// Per-channel density of a simulated patch relative to the clear-film base.
fn patch_density(data: &SpectralData, base: [f32; 3], c: [f32; 3]) -> Vector3<f32> {
    let i = data.simulate(c);
    Vector3::new(
        -(i[0] / base[0].max(1e-8)).max(1e-8).log10(),
        -(i[1] / base[1].max(1e-8)).max(1e-8).log10(),
        -(i[2] / base[2].max(1e-8)).max(1e-8).log10(),
    )
}

/// Fit the 3×3 density-space unmixing matrix `M_post` so that
/// `c ≈ M_post · density` over a grid of known concentration patches.
///
/// Stacking patches as rows: C(n×3) ≈ D(n×3) · M_postᵀ, solved by normal
/// equations `M_postᵀ = (DᵀD)⁻¹ DᵀC`. Linear, closed-form, deterministic.
pub fn fit_m_post(data: &SpectralData) -> Matrix3<f32> {
    let base = data.base();
    let mut rows: Vec<([f32; 3], Vector3<f32>)> = Vec::new();
    for &cc in &FIT_LEVELS {
        for &mm in &FIT_LEVELS {
            for &yy in &FIT_LEVELS {
                let c = [cc, mm, yy];
                rows.push((c, patch_density(data, base, c)));
            }
        }
    }
    let n = rows.len();
    let dmat = DMatrix::from_fn(n, 3, |r, col| rows[r].1[col]);
    let cmat = DMatrix::from_fn(n, 3, |r, col| rows[r].0[col]);
    let dtd = dmat.transpose() * &dmat; // 3×3
    let dtc = dmat.transpose() * &cmat; // 3×3
    let inv = dtd.try_inverse().expect("DᵀD must be invertible for a non-degenerate patch set");
    let mpost_t = inv * dtc; // = M_postᵀ
    let m = mpost_t.transpose();
    Matrix3::new(
        m[(0, 0)], m[(0, 1)], m[(0, 2)],
        m[(1, 0)], m[(1, 1)], m[(1, 2)],
        m[(2, 0)], m[(2, 1)], m[(2, 2)],
    )
}
```

- [ ] **Step 2: Add tests inside the existing `calibrate` `tests` module**

Add these tests (they use the synthetic fixture from Task 1):

```rust
    #[test]
    fn fit_m_post_beats_identity_on_held_out_patches() {
        use crate::spectral::synthetic_overlapping;
        let data = synthetic_overlapping();
        let m = fit_m_post(&data);
        let base = data.base();
        // Held-out grid: disjoint from FIT_LEVELS {0,0.4,...,2.0}.
        let held = [0.2f32, 0.6, 1.0, 1.4, 1.8];
        let (mut sse_fit, mut sse_id, mut count) = (0.0f32, 0.0f32, 0u32);
        for &cc in &held {
            for &mm in &held {
                for &yy in &held {
                    let c = [cc, mm, yy];
                    let i = data.simulate(c);
                    let dens = nalgebra::Vector3::new(
                        -(i[0] / base[0]).max(1e-8).log10(),
                        -(i[1] / base[1]).max(1e-8).log10(),
                        -(i[2] / base[2]).max(1e-8).log10(),
                    );
                    let rec_fit = m * dens;
                    for k in 0..3 {
                        let e_fit = rec_fit[k] - c[k];
                        sse_fit += e_fit * e_fit;
                        let e_id = dens[k] - c[k]; // identity M_post = mode C
                        sse_id += e_id * e_id;
                        count += 1;
                    }
                }
            }
        }
        let rms_fit = (sse_fit / count as f32).sqrt();
        let rms_id = (sse_id / count as f32).sqrt();
        // The fitted unmixing must beat identity by a clear margin on unseen data.
        assert!(rms_fit < rms_id * 0.8, "fit RMS ΔC {rms_fit} not < 0.8 × identity {rms_id}");
    }

    #[test]
    fn fit_m_post_has_significant_off_diagonals() {
        use crate::spectral::synthetic_overlapping;
        let m = fit_m_post(&synthetic_overlapping());
        let off = [m[(0, 1)], m[(0, 2)], m[(1, 0)], m[(1, 2)], m[(2, 0)], m[(2, 1)]];
        let max_off = off.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(max_off > 0.1, "expected real crosstalk correction; max off-diagonal = {max_off}");
    }
```

- [ ] **Step 3: Run the tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core calibrate::`
Expected: existing calibrate tests PLUS the 2 new ones PASS. If `fit_m_post_beats_identity...`
fails because `rms_fit` is not < 0.8×identity, STOP and report BLOCKED with both RMS values —
do NOT loosen the 0.8 factor; a failure means the model/fit needs revisiting (spec Assumption 2).

- [ ] **Step 4: Commit**

```bash
git add crates/film-core/src/calibrate.rs
git commit -m "feat(core): fit_m_post density-unmixing via closed-form least squares"
```

---

## Task 3: Acquire & convert real dye + illuminant data

**Files:**
- Create: `crates/film-core/data/dye_portra400.csv`
- Create: `crates/film-core/data/dye_fujic200.csv`
- Create: `crates/film-core/data/illuminant_d55.csv`
- Create: `crates/film-core/data/DATA_SOURCES.md`

This is an integration/judgment task (web access + format discovery). The math tasks do NOT
depend on it, so it can be iterated without blocking the rest.

- [ ] **Step 1: Fetch and inspect the source dye data**

Source repo: `JanLohse/spectral_film_lut` (MIT). Fetch the two stock modules (try `main` then
`master` branch). Example raw URLs to retrieve and read:
- `https://raw.githubusercontent.com/JanLohse/spectral_film_lut/main/src/spectral_film_lut/negative_film/kodak_portra_400.py`
- `https://raw.githubusercontent.com/JanLohse/spectral_film_lut/main/src/spectral_film_lut/negative_film/fuji_c200.py`

Read each file. Identify the three **dye spectral density** curves (cyan/magenta/yellow image
dyes) as `{wavelength_nm: density}` data, and the base/`d_min` if present (else use 0.0). Note:
these modules also contain `log_sensitivity` (spectral *sensitivity*, NOT what we want) — be
sure to extract the **dye density** curves, not sensitivity. If the exact field names are
unclear, read the repo's `density_curves.py` / a base class to see how dye density is stored.

- [ ] **Step 2: Resample onto the 380–730 @ 5 nm grid and write CSVs**

For each stock, linearly interpolate the digitized points onto wavelengths
`380,385,…,730` (71 rows). Clamp negative densities to 0. Write CSV with a header and one row
per wavelength:

```
wavelength,D_C,D_M,D_Y,D_min
380,0.123,0.045,0.567,0.10
385,...
```

If a stock has no explicit base curve, set `D_min` to a small constant (0.05) for every row and
note it in DATA_SOURCES.md. Sanity: densities should be roughly in [0, 3]; the three dye peaks
should fall in plausible bands (C peak ~620–680, M ~520–560, Y ~430–470 nm).

- [ ] **Step 3: Obtain D55 illuminant**

Write `illuminant_d55.csv` (`wavelength,power`, 71 rows on the same grid) from the CIE D55
relative SPD (e.g. `colour-science`'s `SDS_ILLUMINANTS['D55']` values, or the canonical CIE
D-series table, resampled to 5 nm). If D55 values cannot be reliably obtained, use an
equal-energy illuminant (power = 100.0 for all rows) and document the substitution in
DATA_SOURCES.md (the base-normalization makes the inversion robust to a smooth illuminant).

- [ ] **Step 4: Write DATA_SOURCES.md**

Record, for each file: source project/URL, license (spectral_film_lut = MIT; D55 = colour-science BSD-3 or CIE), the exact film module used, the digitization/interpolation method, and any substitutions (e.g. D_min constant, equal-energy fallback). Include the MIT attribution for spectral_film_lut.

- [ ] **Step 5: Verify the CSVs**

Run: `source "$HOME/.cargo/env" && head -3 crates/film-core/data/dye_portra400.csv && wc -l crates/film-core/data/*.csv`
Expected: header + 71 data rows (72 lines) per file; plausible density values.

- [ ] **Step 6: Commit**

```bash
git add crates/film-core/data
git commit -m "data: bundle Portra400 + FujiC200 dye densities + D55 (spectral_film_lut MIT)"
```

If the dye data cannot be cleanly extracted after genuine effort, report DONE_WITH_CONCERNS
describing exactly what the source format looked like and where extraction broke down, and
commit whatever partial CSVs you produced — the loader (Task 4) and integration (Task 5) can
still be built and the data refined later.

---

## Task 4: `load_stock` — CSV loader → SpectralData

**Files:**
- Modify: `crates/film-core/src/spectral.rs`

- [ ] **Step 1: Add the loader above the `#[cfg(test)]` block**

The CSVs are bundled into the binary with `include_str!` so no runtime file paths are needed.

```rust
/// Parse a 5-column dye CSV (`wavelength,D_C,D_M,D_Y,D_min`, header + rows) into
/// (wavelengths, [D_C,D_M,D_Y], D_min).
fn parse_dye_csv(text: &str) -> (Vec<f32>, [Vec<f32>; 3], Vec<f32>) {
    let mut w = Vec::new();
    let (mut dc, mut dm, mut dy, mut dmin) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for line in text.lines().skip(1).filter(|l| !l.trim().is_empty()) {
        let f: Vec<f32> = line.split(',').map(|s| s.trim().parse().unwrap()).collect();
        w.push(f[0]);
        dc.push(f[1]);
        dm.push(f[2]);
        dy.push(f[3]);
        dmin.push(f[4]);
    }
    (w, [dc, dm, dy], dmin)
}

/// Parse a 2-column illuminant CSV (`wavelength,power`, header + rows) into power values.
fn parse_illuminant_csv(text: &str) -> Vec<f32> {
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.split(',').nth(1).unwrap().trim().parse().unwrap())
        .collect()
}

/// Load bundled spectral data for a stock: real dye densities + D55 illuminant +
/// the analytic Gaussian sensor, all on the shared grid.
pub fn load_stock(stock: Stock) -> SpectralData {
    let dye_csv = match stock {
        Stock::Portra400 => include_str!("../data/dye_portra400.csv"),
        Stock::FujiC200 => include_str!("../data/dye_fujic200.csv"),
    };
    let illum_csv = include_str!("../data/illuminant_d55.csv");
    let (wavelengths, dye, d_min) = parse_dye_csv(dye_csv);
    let illuminant = parse_illuminant_csv(illum_csv);
    let sensor = analytic_sensor(&wavelengths);
    SpectralData { wavelengths, dye, d_min, illuminant, sensor }
}
```

- [ ] **Step 2: Add loader + real-data integration tests**

Add inside the `spectral` `tests` module:

```rust
    #[test]
    fn load_stock_returns_consistent_grid() {
        for stock in [Stock::Portra400, Stock::FujiC200] {
            let d = load_stock(stock);
            let n = d.wavelengths.len();
            assert_eq!(n, 71, "{stock:?} grid len");
            assert_eq!(d.dye[0].len(), n);
            assert_eq!(d.dye[1].len(), n);
            assert_eq!(d.dye[2].len(), n);
            assert_eq!(d.d_min.len(), n);
            assert_eq!(d.illuminant.len(), n);
            // wavelengths strictly increasing
            assert!(d.wavelengths.windows(2).all(|w| w[1] > w[0]));
        }
    }

    #[test]
    fn real_stock_yields_nontrivial_unmixing() {
        // On real dye data, the fitted matrix must beat identity on held-out patches.
        use crate::calibrate::fit_m_post;
        let data = load_stock(Stock::Portra400);
        let m = fit_m_post(&data);
        let base = data.base();
        let held = [0.2f32, 0.6, 1.0, 1.4, 1.8];
        let (mut sse_fit, mut sse_id, mut count) = (0.0f32, 0.0f32, 0u32);
        for &cc in &held {
            for &mm in &held {
                for &yy in &held {
                    let c = [cc, mm, yy];
                    let i = data.simulate(c);
                    let dens = nalgebra::Vector3::new(
                        -(i[0] / base[0]).max(1e-8).log10(),
                        -(i[1] / base[1]).max(1e-8).log10(),
                        -(i[2] / base[2]).max(1e-8).log10(),
                    );
                    let rec = m * dens;
                    for k in 0..3 {
                        sse_fit += (rec[k] - c[k]).powi(2);
                        sse_id += (dens[k] - c[k]).powi(2);
                        count += 1;
                    }
                }
            }
        }
        let rms_fit = (sse_fit / count as f32).sqrt();
        let rms_id = (sse_id / count as f32).sqrt();
        assert!(rms_fit <= rms_id, "real-data fit RMS {rms_fit} should not exceed identity {rms_id}");
    }
```

NOTE: `real_stock_yields_nontrivial_unmixing` uses a lenient bound (`<=`) because real dye data
quality varies; the synthetic test (Task 2) is the strict proof of the fit math. If this test
fails (fit WORSE than identity on real data), report it — it signals a data-conversion problem
in Task 3, not a fit-code bug.

- [ ] **Step 3: Run the tests**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core spectral::`
Expected: all spectral tests pass including the 2 new ones.

- [ ] **Step 4: Commit**

```bash
git add crates/film-core/src/spectral.rs
git commit -m "feat(core): load_stock (bundled CSV → SpectralData) + real-data fit test"
```

---

## Task 5: Engine + CLI integration (`--stock`)

**Files:**
- Modify: `crates/film-core/src/engine.rs`
- Modify: `crates/film-cli/src/main.rs`

- [ ] **Step 1: Add `params_for_stock` to engine.rs**

Above the `#[cfg(test)]` block in `crates/film-core/src/engine.rs`, add:

```rust
/// Build inversion params whose `m_post` is fitted from the given film stock's
/// physical model (`m_pre` stays identity). Used by Mode B for cross-channel
/// dye unmixing.
pub fn params_for_stock(
    stock: crate::spectral::Stock,
    base: [f32; 3],
    exposure: f32,
    black: f32,
    gamma: f32,
) -> InversionParams {
    let data = crate::spectral::load_stock(stock);
    let m_post = crate::calibrate::fit_m_post(&data);
    InversionParams {
        base,
        m_pre: Matrix3::identity(),
        m_post,
        exposure,
        black,
        gamma,
    }
}
```

(`Matrix3` is already imported in engine.rs.)

- [ ] **Step 2: Add a test confirming stock params change Mode B output**

Inside the engine `tests` module:

```rust
    #[test]
    fn stock_params_make_b_differ_from_identity() {
        use crate::spectral::Stock;
        let base = [0.5, 0.4, 0.3];
        let plain = InversionParams { base, gamma: 1.0, ..Default::default() };
        let stock = params_for_stock(Stock::Portra400, base, 1.0, 0.0, 1.0);
        let probe = [0.3, 0.22, 0.15];
        let a = invert_b(probe, &plain);
        let b = invert_b(probe, &stock);
        let diff: f32 = (0..3).map(|c| (a[c] - b[c]).abs()).sum();
        assert!(diff > 1e-3, "stock M_post should change B output; diff={diff}");
    }
```

- [ ] **Step 3: Run the test**

Run: `source "$HOME/.cargo/env" && cargo test -p film-core engine::stock_params`
Expected: PASS.

- [ ] **Step 4: Add the `--stock` flag to the CLI**

In `crates/film-cli/src/main.rs`:

(a) Add the import near the others:
```rust
use film_core::spectral::Stock;
```

(b) Add the enum after the `CliMode` enum:
```rust
#[derive(Copy, Clone, Debug, ValueEnum)]
enum CliStock {
    None,
    Portra400,
    FujiC200,
}

impl CliStock {
    fn to_stock(self) -> Option<Stock> {
        match self {
            CliStock::None => None,
            CliStock::Portra400 => Some(Stock::Portra400),
            CliStock::FujiC200 => Some(Stock::FujiC200),
        }
    }
}
```

(c) Add the field to the `Cli` struct (after `gamma`):
```rust
    /// Film stock for Mode B density unmixing (fits M_post). `none` = identity.
    #[arg(long, value_enum, default_value = "none")]
    stock: CliStock,
```

(d) Replace the params construction. Find:
```rust
    let params = InversionParams {
        base,
        exposure: cli.exposure,
        black: cli.black,
        gamma: cli.gamma,
        ..Default::default()
    };
```
and replace with a plain params (for C/naive) plus a Mode-B params that honors `--stock`:
```rust
    let params = InversionParams {
        base,
        exposure: cli.exposure,
        black: cli.black,
        gamma: cli.gamma,
        ..Default::default()
    };
    let b_params = match cli.stock.to_stock() {
        Some(s) => {
            eprintln!("using fitted M_post for stock {:?}", cli.stock);
            film_core::engine::params_for_stock(s, base, cli.exposure, cli.black, cli.gamma)
        }
        None => params.clone(),
    };
```

(e) In the `--compare` branch, use `b_params` for Mode B only. Replace the compare loop body:
```rust
        for (mode, suffix) in [(Mode::B, "b"), (Mode::C, "c"), (Mode::Naive, "naive")] {
            let out = invert_image(&img, &params, mode);
            let path = dir.join(format!("{stem}_{suffix}.tiff"));
            write_tiff16(&out, &path).context("writing compare output")?;
            eprintln!("wrote {path:?}");
        }
```
with:
```rust
        for (mode, suffix) in [(Mode::B, "b"), (Mode::C, "c"), (Mode::Naive, "naive")] {
            let p = if mode == Mode::B { &b_params } else { &params };
            let out = invert_image(&img, p, mode);
            let path = dir.join(format!("{stem}_{suffix}.tiff"));
            write_tiff16(&out, &path).context("writing compare output")?;
            eprintln!("wrote {path:?}");
        }
```

(f) In the single-mode (non-compare) path, use `b_params` when the mode is B. Replace:
```rust
    let out = invert_image(&img, &params, cli.mode.into());
```
with:
```rust
    let mode: Mode = cli.mode.into();
    let chosen = if mode == Mode::B { &b_params } else { &params };
    let out = invert_image(&img, chosen, mode);
```

(`InversionParams` derives `Clone`, and `Mode` derives `PartialEq` — both already in place.)

- [ ] **Step 5: Build, clippy, smoke test**

Run: `source "$HOME/.cargo/env" && cargo build && cargo clippy --all-targets 2>&1 | grep -E "warning|error" | head`
Expected: compiles; no new warnings.

Run: `source "$HOME/.cargo/env" && cargo test -p film-core`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/film-core/src/engine.rs crates/film-cli/src/main.rs
git commit -m "feat: --stock flag fits M_post per film stock for Mode B unmixing"
```

---

## Task 6: Real-file validation + findings

**Files:**
- Modify: `docs/superpowers/poc-findings.md`

- [ ] **Step 1: Run the color V600 frame with and without a stock**

```bash
cd /Users/mohaelder/Repos/filmrev && source "$HOME/.cargo/env"
cp "samples/Image 4.dng" /tmp/v600.tiff
./target/release/film-cli /tmp/v600.tiff -o /tmp/v600s.tiff --compare --stock portra400
```
(Build release first if needed: `cargo build --release`.) Expected: prints "using fitted M_post
for stock Portra400" and writes `/tmp/v600s_b.tiff` (stock B), `_c.tiff`, `_naive.tiff`.

- [ ] **Step 2: Compare B-with-stock vs B-without-stock**

```bash
./target/release/film-cli /tmp/v600.tiff -o /tmp/v600n.tiff --mode b              # identity
./target/release/film-cli /tmp/v600.tiff -o /tmp/v600p.tiff --mode b --stock portra400
cmp -s /tmp/v600n.tiff /tmp/v600p.tiff && echo "IDENTICAL (unexpected)" || echo "DIFFER (expected: stock changes B)"
```
Convert to PNG for eyeballing: `cd /tmp && for f in v600n v600p; do sips -s format png -Z 900 "$f.tiff" --out "$f.png" >/dev/null 2>&1; done`

- [ ] **Step 3: Record results in `docs/superpowers/poc-findings.md`**

Add a dated "M_post fitting — results" section: the fitted off-diagonal magnitudes (from test
output / a quick print), held-out RMS ΔC (synthetic + real), whether stock-B visibly differs
from identity-B on the V600 frame and in which direction (more neutral? more/less saturated?),
and the honest verdict on whether the cross-channel unmixing is a real improvement or marginal.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs: M_post fitting real-file validation results"
```

---

## Definition of Done

- [ ] `cargo test -p film-core` green, including: synthetic fit beats identity by ≥20% on
      held-out patches; fitted `M_post` has significant off-diagonals; real-stock load + fit
      tests; stock changes Mode B output.
- [ ] `cargo clippy --all-targets` clean.
- [ ] `data/` holds Portra400 + FujiC200 dye CSVs + D55 + `DATA_SOURCES.md` with licenses.
- [ ] `film-cli --stock portra400` works on the real V600 frame; B-with-stock ≠ B-identity.
- [ ] Findings recorded: is the unmixing a real, visible improvement? (Honest verdict, per
      spec Assumption 2.)
