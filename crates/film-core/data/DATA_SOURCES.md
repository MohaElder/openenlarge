# Bundled Spectral Data — Sources & Licenses

All curves resampled to the 380–730 nm @ 5 nm grid (71 points) by linear interpolation;
densities clamped to ≥ 0. Conversion script: `/tmp/convert.py` (recorded in commit history).

## `dye_portra400.csv`, `dye_fujic200.csv` — film dye spectral densities + base

- **Source:** `spektrafilm` by Andrea Volpato — https://github.com/andreavolpato/spektrafilm
  - Profiles: `src/spektrafilm/data/profiles/kodak_portra_400.json`,
    `.../fujifilm_c200.json` (`data.channel_density` → C/M/Y per-dye spectral density;
    `data.base_density` → `D_min`, the orange mask).
  - These are community-digitized/processed from manufacturer datasheets; they include
    realistic dye spectral overlap / secondary absorptions (the crosstalk M_post corrects).
  - Verified peaks: C ≈ 695–700 nm (absorbs red), M ≈ 540–545 nm (green), Y ≈ 450 nm (blue);
    base density decreases with wavelength (orange mask), as expected.
- **License:** **CC BY-SA 4.0** (Copyright © 2026 Andrea Volpato). Attribution required;
  **share-alike** applies to derivatives of the profile data.
  - ⚠️ **Productization note:** CC BY-SA share-alike is a constraint for a closed commercial
    ship. Used here for research/validation. Before commercial release, either (a) comply with
    BY-SA, (b) obtain a license, or (c) swap for MIT data (e.g. derive per-dye curves from
    `spectral_film_lut`'s neutral `d_ref_sd` + `d_min_sd`, MIT) or measured data.

## `illuminant_d55.csv` — scan light source L(λ)

- **Source / method:** **Approximate D55 stand-in** — a 5500 K Planckian (blackbody) curve,
  normalized to 100 at 560 nm. NOT the true CIE D55 daylight SPD.
- **Rationale:** the inversion normalizes every channel by the clear-film base, so a smooth
  illuminant choice has limited impact on the fitted `M_post`. The real scan light (V600 lamp /
  camera-scan LED backlight) differs from any standard illuminant anyway.
- **Future work:** replace with the true CIE D55 SPD (colour-science `SDS_ILLUMINANTS['D55']`,
  BSD-3) or, better, the measured backlight SPD for a given capture rig.

## Sensor S(λ)

Not bundled as data — generated analytically in `spectral::analytic_sensor` (unit-peak
Gaussians R600/G540/B460, σ30). Open stand-in; replace with fitted per-camera spectral
sensitivity later.
