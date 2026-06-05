<div align="center">

<img src="app/src-tauri/icons/128x128@2x.png" width="96" alt="OpenEnlarge icon" />

# OpenEnlarge

**Develop your film negatives with real physics — not a flipped tone curve.**

[![License: MIT](https://img.shields.io/badge/License-MIT-f49d4e.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/mohaelder/openenlarge?color=f49d4e)](https://github.com/mohaelder/openenlarge/releases/latest)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-555)
[![CI](https://github.com/mohaelder/openenlarge/actions/workflows/ci.yml/badge.svg)](https://github.com/mohaelder/openenlarge/actions/workflows/ci.yml)

[Download](https://github.com/mohaelder/openenlarge/releases/latest) · [Website](https://mohaelder.github.io/openenlarge) · [How it works](#how-it-works)

</div>

![OpenEnlarge](docs/screenshots/hero.png)

## What is OpenEnlarge?

OpenEnlarge is an open-source desktop darkroom for color film negatives. It inverts and develops scans of negatives into finished positives — the job a darkroom enlarger does for optical prints.

Most tools treat a negative scan as a generic image and fit per-channel tone curves to flip it. OpenEnlarge instead works in the **density domain**, using a Beer-Lambert model of how dye layers absorb light. Density is *linear* in dye concentration; transmittance is not — which is exactly why a naive invert-and-flip looks wrong. Working in density first, then applying creative finishing on top, yields cleaner, more faithful color.

> Every image is developed with a density-domain matrix inversion grounded in the Beer-Lambert model — a physically-based engine, not a per-channel tone curve.

## Negative → Positive

| Negative (scan) | Developed (OpenEnlarge) |
|---|---|
| ![before](docs/screenshots/before.jpg) | ![after](docs/screenshots/after.jpg) |

## Features

- **Density-domain inversion** — physically-based Beer-Lambert engine, not a flipped curve
- **Decodes RAW & TIFF** — Fuji RAF, DNG, and 16-bit TIFF scans → linear RGB
- **Per-roll base calibration** — sample the orange film base once per roll and apply it
- **Full develop controls** — tonal curve, color grading, color wheels, exposure/black/gamma
- **Crop, rotate, straighten, flip** with a live viewport and histogram
- **Batch export** to 16-bit TIFF / PNG / JPEG
- **Headless CLI** (`film-cli`) for scripting and batch inversion
- **Cross-platform** — macOS, Windows, Linux, built on Tauri

## Architecture

| Component | Path | Responsibility |
|---|---|---|
| `film-core` | `crates/film-core` | Pure Rust engine — decode, density-domain inversion, calibration, export. No UI deps. |
| `film-cli` | `crates/film-cli` | Headless CLI over `film-core` for batch/scripted inversion. |
| App shell | `app/` | Tauri 2 + SvelteKit UI wrapping `film-core`. |

## Download

Grab the latest installer for your OS from the [**Releases page**](https://github.com/mohaelder/openenlarge/releases/latest):

- **macOS** — `.dmg` (Apple Silicon or Intel)
- **Windows** — `.msi` or `.exe`
- **Linux** — `.AppImage` or `.deb`

## Build from source

**Prerequisites:** [Rust](https://rustup.rs) (stable), [Node.js](https://nodejs.org) ≥ 18, and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS (on Linux: `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, `libappindicator3-dev`, `patchelf`).

```bash
# Run the desktop app in dev mode
cd app
npm install
npm run tauri dev

# Build a release installer for your OS
npm run tauri build
```

## CLI usage

The engine also runs headless. From the repo root:

```bash
# Invert a scan with the density-domain engine → 16-bit TIFF
cargo run -p film-cli -- input.tiff -o output.tiff

# Sample the film base from a rect (x,y,w,h) and pick a stock profile
cargo run -p film-cli -- input.tiff -o out.tiff --stock portra400 --base-rect 0,0,128,128
```

Run `cargo run -p film-cli -- --help` for all options.

## How it works

A developed color negative is three stacked dye layers (Cyan, Magenta, Yellow) over an orange base. A scan is the forward model:

```
I_i = ∫ L(λ) · S_i(λ) · 10^(−D(λ)) dλ          (spectral integration)
D(λ) = D_min(λ) + Σ_j C_j · D_j(λ)              (Beer-Lambert: density linear in dye conc.)
```

OpenEnlarge's default engine inverts this in the density domain:

```
Ĉ = M_post · log₁₀(M_pre · I₀ / I)
```

It recovers dye concentrations with a cross-channel matrix instead of flipping each channel independently — the difference that makes color come out right. The deep version lives in [`docs/superpowers/specs/2026-06-03-film-inversion-poc-design.md`](docs/superpowers/specs/2026-06-03-film-inversion-poc-design.md).

## Contributing

Issues and pull requests are welcome. Before opening a PR, run the same checks CI does:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
cd app && npm run check && npm run test:unit
```

## License

[MIT](LICENSE) © 2026 mohaelder
