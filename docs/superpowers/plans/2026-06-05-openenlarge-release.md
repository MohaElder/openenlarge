# OpenEnlarge Release Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take the project (local folder `filmrev`) to a public first release as **OpenEnlarge** — repo `github.com/mohaelder/openenlarge`, a solid README, signed cross-platform release builds via CI, and a GitHub Pages landing page at `mohaelder.github.io/openenlarge`.

**Architecture:** All work is file edits in the working tree (Tasks 1–6), each independently reviewable and committed. The only outward-facing step (Task 7 — create repo, add secrets, deploy, tag `v0.1.0`) is user-gated and run last. CI uses `tauri-apps/tauri-action` for release builds and the GitHub Pages Actions flow for the landing page.

**Tech Stack:** Rust workspace (`film-core`, `film-cli`), Tauri 2 + SvelteKit (`app/`), GitHub Actions, vanilla HTML/CSS/JS landing page.

**Design spec:** `docs/superpowers/specs/2026-06-05-openenlarge-release-design.md`

---

## File Structure

| File | Responsibility |
|---|---|
| `app/src-tauri/tauri.conf.json` | (modify) bundle identifier → `com.mohaelder.openenlarge` |
| `crates/film-core/src/export.rs` | (modify) rename test temp-file strings `filmrev_*` → `openenlarge_*` |
| `app/src-tauri/src/encode.rs` | (modify) rename test temp-file strings `filmrev_*` → `openenlarge_*` |
| `LICENSE` | (create) MIT license text |
| `README.md` | (create) root project README |
| `app/README.md` | (modify) replace Tauri-template content with pointer to root README |
| `docs/screenshots/README.md` | (create) manifest of screenshots the user must supply |
| `.github/workflows/ci.yml` | (create) PR/push checks: fmt, clippy, test, svelte-check, vitest |
| `.github/workflows/release.yml` | (create) tag-triggered signed cross-platform release |
| `.github/workflows/pages.yml` | (create) deploy `web/` to GitHub Pages |
| `web/index.html` | (create) landing page (glass design) |
| `web/releases.js` | (create) OS detection + latest-release download wiring |

---

## Task 1: Identity cleanup — bundle id, `filmrev` scrub, LICENSE

**Files:**
- Modify: `app/src-tauri/tauri.conf.json:5`
- Modify: `crates/film-core/src/export.rs:35,49`
- Modify: `app/src-tauri/src/encode.rs:151,159,168,188`
- Create: `LICENSE`

- [ ] **Step 1: Change the bundle identifier**

In `app/src-tauri/tauri.conf.json`, change line 5:

```json
  "identifier": "com.mohaelder.openenlarge",
```

(was `"com.mohaelder.app"`).

- [ ] **Step 2: Confirm the affected tests pass BEFORE the rename (baseline)**

Run: `cargo test -p film-core export:: && cargo test --manifest-path app/src-tauri/Cargo.toml encode::`
Expected: PASS (establishes green baseline before touching the strings).

Note: `app/src-tauri` is excluded from the workspace, so its tests need `--manifest-path app/src-tauri/Cargo.toml`.

- [ ] **Step 3: Rename `filmrev_` test temp-file strings in `export.rs`**

In `crates/film-core/src/export.rs`, replace the two occurrences:
- `dir.join("filmrev_roundtrip.tiff")` → `dir.join("openenlarge_roundtrip.tiff")`
- `dir.join("filmrev_rgba16.tiff")` → `dir.join("openenlarge_rgba16.tiff")`

- [ ] **Step 4: Rename `filmrev_` test temp-file strings in `encode.rs`**

In `app/src-tauri/src/encode.rs`, replace all four occurrences:
- `"filmrev_t1_png8.png"` → `"openenlarge_t1_png8.png"`
- `"filmrev_t1_png16.png"` → `"openenlarge_t1_png16.png"`
- `"filmrev_t1_tiff8.tiff"` → `"openenlarge_t1_tiff8.tiff"`
- `"filmrev_t1_cap.jpg"` → `"openenlarge_t1_cap.jpg"`

- [ ] **Step 5: Verify no user-facing `filmrev` strings remain in source**

Run: `grep -rni "filmrev" crates app/src app/src-tauri/src app/src-tauri/tauri.conf.json`
Expected: no matches.

- [ ] **Step 6: Re-run the tests to confirm the rename is safe**

Run: `cargo test -p film-core export:: && cargo test --manifest-path app/src-tauri/Cargo.toml encode::`
Expected: PASS (each test writes then reads back the same renamed path).

- [ ] **Step 7: Create `LICENSE` (MIT)**

Create `LICENSE` with exactly:

```
MIT License

Copyright (c) 2026 mohaelder

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 8: Commit**

```bash
git add app/src-tauri/tauri.conf.json crates/film-core/src/export.rs app/src-tauri/src/encode.rs LICENSE
git commit -m "chore(release): set bundle id, scrub filmrev test strings, add LICENSE"
```

---

## Task 2: README and screenshot scaffold

**Files:**
- Create: `README.md`
- Modify: `app/README.md`
- Create: `docs/screenshots/README.md`

- [ ] **Step 1: Create `docs/screenshots/README.md` (the manifest)**

```markdown
# Screenshots

Drop the following PNGs here; the root README and the landing page (`web/`) reference them by these exact names.

| File | Used in | Suggested framing |
|---|---|---|
| `hero.png` | README hero, landing hero | Full app window, an image loaded in Develop, ~2400px wide |
| `before.png` | README before/after | Raw negative (orange cast), single frame |
| `after.png` | README before/after | The same frame inverted/developed |
| `library.png` | README features | Library/grid view with thumbnails |
| `develop.png` | README features | Develop view with curves/color panels open |

Keep them under ~500 KB each (PNG, optimized). Until supplied, the README and landing page show the alt text / a placeholder box.
```

- [ ] **Step 2: Create the root `README.md`**

```markdown
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

<!-- TODO: replace placeholder once docs/screenshots/hero.png exists -->
![OpenEnlarge](docs/screenshots/hero.png)

## What is OpenEnlarge?

OpenEnlarge is an open-source desktop darkroom for color film negatives. It inverts and develops scans of negatives into finished positives — the job a darkroom enlarger does for optical prints.

Most tools treat a negative scan as a generic image and fit per-channel tone curves to flip it. OpenEnlarge instead works in the **density domain**, using a Beer-Lambert model of how dye layers absorb light. Density is *linear* in dye concentration; transmittance is not — which is exactly why a naive invert-and-flip looks wrong. Working in density first, then applying creative finishing on top, yields cleaner, more faithful color.

> Engine **B** (density-domain matrix inversion) is the default. A naive per-channel engine (**C**) ships as a built-in comparison mode so you can see the difference for yourself.

## Negative → Positive

<!-- TODO: replace placeholders once docs/screenshots/{before,after}.png exist -->
| Negative (scan) | Developed (OpenEnlarge) |
|---|---|
| ![before](docs/screenshots/before.png) | ![after](docs/screenshots/after.png) |

## Features

- **Density-domain inversion** — physically-based Beer-Lambert engine, not a flipped curve
- **Decodes RAW & TIFF** — Fuji RAF, DNG, and 16-bit TIFF scans → linear RGB
- **Per-roll base calibration** — sample the orange film base once per roll and apply it
- **Full develop controls** — tonal curve, color grading, color wheels, exposure/black/gamma
- **Crop, rotate, straighten, flip** with a live viewport and histogram
- **Batch export** to 16-bit TIFF / PNG / JPEG
- **Headless CLI** (`film-cli`) for scripting and B-vs-C comparison
- **Cross-platform** — macOS, Windows, Linux, built on Tauri

## Architecture

| Component | Path | Responsibility |
|---|---|---|
| `film-core` | `crates/film-core` | Pure Rust engine — decode, inversion (B & C), calibration, export. No UI deps. |
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
# Invert a scan with the default density engine (mode B) → 16-bit TIFF
cargo run -p film-cli -- input.tiff -o output.tiff --mode b

# Emit B, C, and naive side by side for comparison
cargo run -p film-cli -- input.tiff -o out.tiff --compare

# Sample the film base from a rect (x,y,w,h) and pick a stock profile
cargo run -p film-cli -- input.tiff -o out.tiff --mode b --stock portra400 --base-rect 0,0,128,128
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
```

- [ ] **Step 3: Replace `app/README.md` with a pointer**

Overwrite `app/README.md` with:

```markdown
# OpenEnlarge — app shell

This directory is the Tauri 2 + SvelteKit desktop shell. For project overview, build, and CLI usage, see the [root README](../README.md).

```bash
npm install
npm run tauri dev     # run the desktop app
npm run tauri build   # build a release installer
```
```

- [ ] **Step 4: Verify links/paths referenced by the README exist**

Run: `ls app/src-tauri/icons/128x128@2x.png LICENSE docs/superpowers/specs/2026-06-03-film-inversion-poc-design.md`
Expected: all three paths listed (no "No such file"). Screenshot paths are intentionally absent (placeholders).

- [ ] **Step 5: Commit**

```bash
git add README.md app/README.md docs/screenshots/README.md
git commit -m "docs(release): root README, app README pointer, screenshot manifest"
```

---

## Task 3: CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Run the checks locally and fix any failures first**

Run:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd app && npm ci && npm run check && npm run test:unit)
```
Expected: all PASS. If `clippy -D warnings` reports pre-existing warnings, fix the trivial ones; for any warning in code unrelated to this release effort that is non-trivial, add a narrowly-scoped `#[allow(...)]` with a `// release: pre-existing, see #issue` comment. Do not proceed until the commands above pass, so CI will be green on first run.

- [ ] **Step 2: Create `.github/workflows/ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  frontend:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: app/package-lock.json
      - run: npm ci
        working-directory: app
      - run: npm run check
        working-directory: app
      - run: npm run test:unit
        working-directory: app
```

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: add fmt/clippy/test + svelte-check/vitest workflow"
```

---

## Task 4: Release workflow (signed, cross-platform)

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Create `.github/workflows/release.yml`**

```yaml
name: Release

on:
  push:
    tags: ['v*']
  workflow_dispatch:

permissions:
  contents: write

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-14          # Apple Silicon
            target: aarch64-apple-darwin
            args: '--target aarch64-apple-darwin'
          - platform: macos-13          # Intel
            target: x86_64-apple-darwin
            args: '--target x86_64-apple-darwin'
          - platform: ubuntu-22.04
            target: x86_64-unknown-linux-gnu
            args: ''
          - platform: windows-latest
            target: x86_64-pc-windows-msvc
            args: ''

    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4

      - name: Install Linux dependencies
        if: matrix.platform == 'ubuntu-22.04'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
            libappindicator3-dev patchelf

      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - uses: swatinem/rust-cache@v2
        with:
          workspaces: './app/src-tauri -> target'

      - uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: npm
          cache-dependency-path: app/package-lock.json

      - name: Install frontend dependencies
        run: npm ci
        working-directory: app

      - name: Import Windows signing certificate
        if: matrix.platform == 'windows-latest' && env.WINDOWS_CERTIFICATE != ''
        shell: pwsh
        env:
          WINDOWS_CERTIFICATE: ${{ secrets.WINDOWS_CERTIFICATE }}
          WINDOWS_CERTIFICATE_PASSWORD: ${{ secrets.WINDOWS_CERTIFICATE_PASSWORD }}
        run: |
          $bytes = [Convert]::FromBase64String($env:WINDOWS_CERTIFICATE)
          $pfx = "$env:RUNNER_TEMP\cert.pfx"
          [IO.File]::WriteAllBytes($pfx, $bytes)
          $pwd = ConvertTo-SecureString -String $env:WINDOWS_CERTIFICATE_PASSWORD -Force -AsPlainText
          $cert = Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\CurrentUser\My -Password $pwd
          "WIN_CERT_THUMBPRINT=$($cert.Thumbprint)" | Out-File -FilePath $env:GITHUB_ENV -Append

      - name: Compute Tauri build args
        shell: bash
        run: |
          ARGS="${{ matrix.args }}"
          if [ -n "$WIN_CERT_THUMBPRINT" ]; then
            ARGS="$ARGS --config {\"bundle\":{\"windows\":{\"certificateThumbprint\":\"$WIN_CERT_THUMBPRINT\",\"timestampUrl\":\"http://timestamp.digicert.com\",\"digestAlgorithm\":\"sha256\"}}}"
          fi
          echo "TAURI_ARGS=$ARGS" >> "$GITHUB_ENV"

      - uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # macOS signing + notarization (absent => unsigned build, no failure)
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_CERTIFICATE_PASSWORD: ${{ secrets.APPLE_CERTIFICATE_PASSWORD }}
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
        with:
          projectPath: app
          tagName: ${{ github.ref_name }}
          releaseName: 'OpenEnlarge ${{ github.ref_name }}'
          releaseBody: 'Download the installer for your OS below. See the README for install notes.'
          releaseDraft: true
          prerelease: false
          args: ${{ env.TAURI_ARGS }}
```

- [ ] **Step 2: Validate the workflow YAML parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('ok')"`
Expected: `ok`. (If PyYAML is unavailable, run `npx --yes yaml-lint .github/workflows/release.yml` instead.)

- [ ] **Step 3: Record the required secrets in the spec/README for Task 7**

Append a short "Release signing secrets" note to `docs/screenshots/README.md`? No — create `docs/RELEASING.md`:

```markdown
# Releasing OpenEnlarge

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed.

## One-time GitHub secrets (Settings → Secrets and variables → Actions)

**macOS (sign + notarize):**
- `APPLE_CERTIFICATE` — base64 of your Developer ID Application `.p12`: `base64 -i cert.p12 | pbcopy`
- `APPLE_CERTIFICATE_PASSWORD` — the `.p12` export password
- `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID` — your Apple ID email
- `APPLE_PASSWORD` — an app-specific password (appleid.apple.com → Sign-In and Security)
- `APPLE_TEAM_ID` — your 10-char Apple Team ID

**Windows (sign):**
- `WINDOWS_CERTIFICATE` — base64 of your code-signing `.pfx`
- `WINDOWS_CERTIFICATE_PASSWORD` — the `.pfx` password

If a platform's secrets are absent, that platform still builds — unsigned.

## Cutting a release

```bash
# bump version in app/src-tauri/tauri.conf.json + app/package.json first if needed
git tag v0.1.0
git push origin v0.1.0
```

The workflow creates a **draft** release with all installers attached. Review it, then publish.
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release.yml docs/RELEASING.md
git commit -m "ci: signed cross-platform release workflow + releasing docs"
```

---

## Task 5: Landing page (`web/`)

**Files:**
- Create: `web/index.html`
- Create: `web/releases.js`

- [ ] **Step 1: Create `web/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>OpenEnlarge — develop your film negatives with real physics</title>
<meta name="description" content="Open-source desktop darkroom for color film negatives. A physically-based, density-domain inverter — not a flipped tone curve.">
<link rel="icon" href="https://raw.githubusercontent.com/mohaelder/openenlarge/main/app/src-tauri/icons/32x32.png">
<style>
  :root{
    --bg-0:#0a0a0c; --bg-1:#141418;
    --glass-bg:rgba(28,28,34,.55); --glass-brd:rgba(255,255,255,.08); --glass-hi:rgba(255,255,255,.04);
    --text:#e8e8ea; --text-dim:#9a9aa2; --text-faint:#5f5f68;
    --accent:#f49d4e; --accent-dim:#df7136; --accent-grad:linear-gradient(90deg,#f49d4e,#df7136);
    --radius:14px;
  }
  *{box-sizing:border-box;}
  html,body{margin:0;scroll-behavior:smooth;}
  body{
    background:var(--bg-0); color:var(--text);
    font:15px/1.5 -apple-system,system-ui,sans-serif; -webkit-font-smoothing:antialiased;
    background-image:
      radial-gradient(80% 55% at 78% -5%, rgba(244,157,78,.16) 0%, transparent 55%),
      radial-gradient(60% 50% at 8% 8%, rgba(223,113,54,.10) 0%, transparent 50%);
    background-attachment:fixed;
  }
  a{color:inherit;}
  .wrap{max-width:1080px;margin:0 auto;padding:0 28px;}
  .glass{background:var(--glass-bg);border:1px solid var(--glass-brd);border-radius:var(--radius);
    backdrop-filter:blur(22px) saturate(140%);-webkit-backdrop-filter:blur(22px) saturate(140%);
    box-shadow:inset 0 1px 0 var(--glass-hi),0 8px 30px rgba(0,0,0,.35);}

  nav{position:sticky;top:14px;z-index:10;margin-top:14px;}
  .navbar{display:flex;align-items:center;gap:14px;padding:10px 16px;}
  .brand{display:flex;align-items:center;gap:9px;font-weight:600;letter-spacing:-.01em;text-decoration:none;}
  .brand img{width:26px;height:26px;border-radius:7px;}
  .navlinks{margin-left:auto;display:flex;gap:22px;color:var(--text-dim);font-size:13.5px;}
  .navlinks a{text-decoration:none;}
  .navlinks a:hover{color:var(--text);}
  .nav-cta{margin-left:6px;padding:7px 14px;border-radius:9px;background:var(--accent-grad);color:#1a1206;font-weight:700;font-size:13px;text-decoration:none;}

  .hero{display:grid;grid-template-columns:1.05fr .95fr;gap:30px;align-items:center;padding:74px 0 40px;}
  .eyebrow{display:inline-flex;align-items:center;gap:8px;font:600 11.5px/1 ui-monospace,monospace;letter-spacing:.16em;text-transform:uppercase;color:var(--accent);padding:6px 11px;border:1px solid var(--glass-brd);border-radius:999px;background:rgba(244,157,78,.06);}
  h1{font-size:50px;line-height:1.03;letter-spacing:-.025em;margin:20px 0 16px;font-weight:680;}
  h1 .grad{background:var(--accent-grad);-webkit-background-clip:text;background-clip:text;color:transparent;}
  .lede{color:var(--text-dim);font-size:17px;max-width:30em;margin:0 0 26px;}
  .cta-row{display:flex;gap:12px;align-items:center;flex-wrap:wrap;}
  .btn-primary{display:inline-flex;align-items:center;gap:9px;padding:12px 20px;border-radius:11px;background:var(--accent-grad);color:#1a1206;font-weight:700;font-size:14.5px;text-decoration:none;box-shadow:0 6px 24px rgba(244,157,78,.28);}
  .btn-ghost{display:inline-flex;align-items:center;gap:8px;padding:12px 18px;border-radius:11px;color:var(--text);font-weight:600;font-size:14px;text-decoration:none;}
  .meta{margin-top:14px;color:var(--text-faint);font-size:12.5px;}

  .card{position:relative;padding:20px;border-radius:18px;
    background:var(--glass-bg);border:1px solid var(--glass-brd);
    backdrop-filter:blur(22px) saturate(140%);-webkit-backdrop-filter:blur(22px) saturate(140%);
    box-shadow:inset 0 1px 0 var(--glass-hi),0 18px 60px rgba(0,0,0,.5);}
  .card .label{font:600 10.5px/1 ui-monospace,monospace;letter-spacing:.14em;text-transform:uppercase;color:var(--text-faint);display:flex;justify-content:space-between;margin-bottom:12px;}
  .graph{position:relative;height:200px;border-radius:12px;background:linear-gradient(180deg,#0d0d11,#08080a);border:1px solid rgba(255,255,255,.05);overflow:hidden;}
  .graph .grid{position:absolute;inset:0;background-image:linear-gradient(rgba(255,255,255,.05) 1px,transparent 1px),linear-gradient(90deg,rgba(255,255,255,.05) 1px,transparent 1px);background-size:25px 25px;}
  .eq{margin-top:14px;padding:11px 13px;border-radius:10px;background:rgba(255,255,255,.03);border:1px solid var(--glass-brd);font:13px/1 ui-monospace,monospace;color:var(--accent);display:flex;justify-content:space-between;align-items:center;}
  .eq span:last-child{color:var(--text-faint);font-size:11px;}

  section.block{padding:46px 0;}
  .kicker{font:600 11.5px/1 ui-monospace,monospace;letter-spacing:.16em;text-transform:uppercase;color:var(--accent);}
  h2{font-size:30px;letter-spacing:-.02em;margin:12px 0 26px;}
  .features{display:grid;grid-template-columns:repeat(3,1fr);gap:16px;}
  .feature{padding:20px;}
  .feature h3{margin:0 0 8px;font-size:16px;}
  .feature p{margin:0;color:var(--text-dim);font-size:13.5px;}
  .feature .ic{font-size:22px;margin-bottom:10px;}

  .steps{display:grid;grid-template-columns:repeat(3,1fr);gap:16px;}
  .step{padding:20px;}
  .step .n{font:700 13px/1 ui-monospace,monospace;color:var(--accent);}
  .step h3{margin:10px 0 8px;font-size:16px;}
  .step p{margin:0;color:var(--text-dim);font-size:13.5px;}

  .shots{display:grid;grid-template-columns:1fr 1fr;gap:16px;}
  .shot{border-radius:14px;overflow:hidden;border:1px solid var(--glass-brd);aspect-ratio:16/10;background:linear-gradient(180deg,#161620,#0c0c10);display:grid;place-items:center;color:var(--text-faint);font-size:13px;}
  .shot img{width:100%;height:100%;object-fit:cover;display:block;}

  .download{text-align:center;padding:60px 0;}
  .download .card-dl{display:inline-block;padding:34px 40px;}
  .os-row{display:flex;gap:10px;justify-content:center;flex-wrap:wrap;margin-top:18px;}
  .os-link{padding:9px 16px;border-radius:10px;border:1px solid var(--glass-brd);background:rgba(255,255,255,.03);text-decoration:none;font-size:13.5px;color:var(--text-dim);}
  .os-link:hover{color:var(--text);}

  footer{border-top:1px solid var(--glass-brd);margin-top:40px;padding:26px 0;color:var(--text-faint);font-size:13px;display:flex;justify-content:space-between;flex-wrap:wrap;gap:12px;}
  footer a{text-decoration:none;color:var(--text-dim);}

  @media(max-width:820px){
    .hero{grid-template-columns:1fr;}
    .features,.steps{grid-template-columns:1fr;}
    h1{font-size:38px;}
    .navlinks{display:none;}
  }
</style>
</head>
<body>

<div class="wrap">
  <nav><div class="navbar glass">
    <a class="brand" href="#top">
      <img src="https://raw.githubusercontent.com/mohaelder/openenlarge/main/app/src-tauri/icons/128x128.png" alt="">
      OpenEnlarge
    </a>
    <div class="navlinks">
      <a href="#features">Features</a><a href="#how">How it works</a><a href="#download">Download</a>
      <a href="https://github.com/mohaelder/openenlarge">GitHub</a>
    </div>
    <a class="nav-cta" id="nav-download" href="https://github.com/mohaelder/openenlarge/releases/latest">Download</a>
  </div></nav>

  <section class="hero" id="top">
    <div>
      <span class="eyebrow">◆ Open source · density-domain</span>
      <h1>The physics of film,<br><span class="grad">inverted properly.</span></h1>
      <p class="lede">OpenEnlarge develops your color negatives with a real Beer-Lambert density model — not a flipped tone curve. A darkroom enlarger, reimagined as a desktop app.</p>
      <div class="cta-row">
        <a class="btn-primary" id="hero-download" href="https://github.com/mohaelder/openenlarge/releases/latest">↓ Download</a>
        <a class="btn-ghost glass" href="https://github.com/mohaelder/openenlarge">★ Star on GitHub</a>
      </div>
      <div class="meta" id="release-meta">Free · MIT licensed · macOS, Windows &amp; Linux</div>
    </div>

    <div class="card">
      <div class="label"><span>Density inversion</span><span>film-core</span></div>
      <div class="graph">
        <div class="grid"></div>
        <svg width="100%" height="100%" viewBox="0 0 100 100" preserveAspectRatio="none">
          <path d="M3,94 C28,90 42,58 56,34 S86,10 97,5" fill="none" stroke="#f49d4e" stroke-width="2.2" vector-effect="non-scaling-stroke"/>
          <path d="M3,96 C30,93 45,64 58,40 S88,16 97,9" fill="none" stroke="#df7136" stroke-width="1.6" opacity=".8" vector-effect="non-scaling-stroke"/>
          <path d="M3,92 C26,87 40,52 54,30 S84,8 97,3" fill="none" stroke="#5fb0e8" stroke-width="1.4" opacity=".55" vector-effect="non-scaling-stroke"/>
        </svg>
      </div>
      <div class="eq"><span>Ĉ = M_post · log₁₀(M_pre · I₀ / I)</span><span>Beer-Lambert</span></div>
    </div>
  </section>

  <section class="block" id="features">
    <div class="kicker">Features</div>
    <h2>A real darkroom, on your desktop.</h2>
    <div class="features">
      <div class="feature glass"><div class="ic">🎞️</div><h3>Density-domain inversion</h3><p>Physically-based Beer-Lambert engine recovers dye concentrations with a cross-channel matrix — not a flipped curve.</p></div>
      <div class="feature glass"><div class="ic">📥</div><h3>RAW &amp; TIFF decode</h3><p>Fuji RAF, DNG and 16-bit TIFF scans decoded to linear RGB, infrared plane preserved.</p></div>
      <div class="feature glass"><div class="ic">🧪</div><h3>Per-roll base calibration</h3><p>Sample the orange film base once per roll and apply it across every frame.</p></div>
      <div class="feature glass"><div class="ic">🎚️</div><h3>Full develop controls</h3><p>Tonal curves, color grading, color wheels, exposure, black point and gamma — live.</p></div>
      <div class="feature glass"><div class="ic">✂️</div><h3>Crop, rotate &amp; export</h3><p>Straighten and crop with a live histogram, then batch export to 16-bit TIFF, PNG or JPEG.</p></div>
      <div class="feature glass"><div class="ic">⌨️</div><h3>Headless CLI</h3><p><code>film-cli</code> runs the same engine for scripting and B-vs-C comparison.</p></div>
    </div>
  </section>

  <section class="block" id="how">
    <div class="kicker">How it works</div>
    <h2>Density first, aesthetics second.</h2>
    <div class="steps">
      <div class="step glass"><div class="n">01</div><h3>Decode</h3><p>Your RAF/DNG/TIFF scan is decoded to linear RGB — the light the scanner actually measured through the film.</p></div>
      <div class="step glass"><div class="n">02</div><h3>Invert in density</h3><p>Take the log to enter the density domain, then unmix dye layers with a matrix. Density is linear in dye; transmittance isn't — so this is where naive flips go wrong.</p></div>
      <div class="step glass"><div class="n">03</div><h3>Develop</h3><p>Apply creative finishing — curves, color, exposure — on a faithful base. Export, or batch the whole roll.</p></div>
    </div>
  </section>

  <section class="block">
    <div class="kicker">Screenshots</div>
    <h2>See it in action.</h2>
    <div class="shots">
      <!-- TODO: replace placeholders once screenshots exist in web/img/ -->
      <div class="shot"><img src="img/library.png" alt="OpenEnlarge library view" onerror="this.replaceWith(document.createTextNode('library.png'))"></div>
      <div class="shot"><img src="img/develop.png" alt="OpenEnlarge develop view" onerror="this.replaceWith(document.createTextNode('develop.png'))"></div>
    </div>
  </section>

  <section class="download" id="download">
    <div class="card-dl glass card">
      <div class="kicker">Download</div>
      <h2 style="margin-bottom:6px">Get OpenEnlarge</h2>
      <p class="lede" style="margin:0 auto 4px;text-align:center">Free and open source. macOS, Windows &amp; Linux.</p>
      <div class="cta-row" style="justify-content:center;margin-top:18px">
        <a class="btn-primary" id="dl-download" href="https://github.com/mohaelder/openenlarge/releases/latest">↓ Download</a>
      </div>
      <div class="os-row" id="os-row">
        <a class="os-link" href="https://github.com/mohaelder/openenlarge/releases/latest">macOS</a>
        <a class="os-link" href="https://github.com/mohaelder/openenlarge/releases/latest">Windows</a>
        <a class="os-link" href="https://github.com/mohaelder/openenlarge/releases/latest">Linux</a>
      </div>
    </div>
  </section>

  <footer>
    <span>© 2026 OpenEnlarge · <a href="https://github.com/mohaelder/openenlarge/blob/main/LICENSE">MIT</a></span>
    <span><a href="https://github.com/mohaelder/openenlarge">GitHub</a> · <a href="https://github.com/mohaelder/openenlarge/releases/latest">Releases</a></span>
  </footer>
</div>

<script src="releases.js"></script>
</body>
</html>
```

- [ ] **Step 2: Create `web/releases.js`**

```javascript
// Detect the visitor's OS and point the download buttons at the matching
// installer from the latest GitHub release. Falls back to /releases/latest.
(function () {
  var REPO = "mohaelder/openenlarge";
  var LATEST = "https://github.com/" + REPO + "/releases/latest";

  function detectOS() {
    var ua = (navigator.userAgent || "") + " " + (navigator.platform || "");
    if (/Mac|iPhone|iPad/i.test(ua)) return "macos";
    if (/Win/i.test(ua)) return "windows";
    if (/Linux|X11/i.test(ua)) return "linux";
    return null;
  }

  function label(os) {
    return os === "macos" ? "Download for macOS"
      : os === "windows" ? "Download for Windows"
      : os === "linux" ? "Download for Linux"
      : "Download";
  }

  // Pick the best asset for an OS from a release's asset list.
  function pickAsset(assets, os) {
    var isArm = /arm|aarch64/i.test(navigator.userAgent + navigator.platform);
    var rank = {
      macos: function (n) {
        if (!/\.dmg$/i.test(n)) return -1;
        var arm = /aarch64|arm64/i.test(n);
        return isArm === arm ? 2 : 1;
      },
      windows: function (n) { return /\.msi$/i.test(n) ? 2 : /\.exe$/i.test(n) ? 1 : -1; },
      linux: function (n) { return /\.AppImage$/i.test(n) ? 2 : /\.deb$/i.test(n) ? 1 : -1; }
    }[os];
    if (!rank) return null;
    var best = null, bestScore = 0;
    assets.forEach(function (a) {
      var s = rank(a.name);
      if (s > bestScore) { bestScore = s; best = a; }
    });
    return best;
  }

  var os = detectOS();
  var heroBtn = document.getElementById("hero-download");
  var dlBtn = document.getElementById("dl-download");
  var navBtn = document.getElementById("nav-download");
  var meta = document.getElementById("release-meta");

  if (os && heroBtn) heroBtn.textContent = "↓ " + label(os);
  if (os && dlBtn) dlBtn.textContent = "↓ " + label(os);

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", {
    headers: { Accept: "application/vnd.github+json" }
  })
    .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
    .then(function (rel) {
      var assets = rel.assets || [];
      var asset = os ? pickAsset(assets, os) : null;
      var url = asset ? asset.browser_download_url : LATEST;
      [heroBtn, dlBtn, navBtn].forEach(function (b) { if (b) b.href = url; });

      if (meta && rel.tag_name) {
        meta.textContent = "Free · MIT licensed · macOS, Windows & Linux · " + rel.tag_name;
      }

      // Wire the per-OS quick links to their best asset, if present.
      var row = document.getElementById("os-row");
      if (row) {
        ["macos", "windows", "linux"].forEach(function (o, i) {
          var a = pickAsset(assets, o);
          if (a && row.children[i]) row.children[i].href = a.browser_download_url;
        });
      }
    })
    .catch(function () { /* no release yet / offline: links already point at /releases/latest */ });
})();
```

- [ ] **Step 3: Verify the page renders locally**

Run: `cd web && python3 -m http.server 8765 >/dev/null 2>&1 & sleep 1 && curl -sSI http://localhost:8765/index.html | head -1 && curl -sS http://localhost:8765/releases.js | head -1 && kill %1`
Expected: first line `HTTP/1.0 200 OK`, second line shows the JS comment. (Optional: open `http://localhost:8765` in a browser to eyeball the glass design before committing.)

- [ ] **Step 4: Commit**

```bash
git add web/index.html web/releases.js
git commit -m "feat(web): GitHub Pages landing page with OS-aware download links"
```

---

## Task 6: Pages deploy workflow

**Files:**
- Create: `.github/workflows/pages.yml`

- [ ] **Step 1: Create `.github/workflows/pages.yml`**

```yaml
name: Deploy landing page

on:
  push:
    branches: [main]
    paths: ['web/**', '.github/workflows/pages.yml']
  workflow_dispatch:

permissions:
  contents: read
  pages: write
  id-token: write

concurrency:
  group: pages
  cancel-in-progress: true

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with:
          path: web
      - id: deployment
        uses: actions/deploy-pages@v4
```

- [ ] **Step 2: Validate the workflow YAML parses**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/pages.yml')); print('ok')"`
Expected: `ok`.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pages.yml
git commit -m "ci: deploy web/ landing page to GitHub Pages"
```

---

## Task 7: Go live (USER-GATED — do not run without explicit confirmation)

This task creates the public repo, pushes code, configures signing, deploys the site, and cuts the first release. Every step here is outward-facing. **Stop and get the user's explicit go-ahead before running anything in this task**, and confirm whether the repo should be public.

**Files:** none (operational).

- [ ] **Step 1: Create the GitHub repo and push**

```bash
gh repo create mohaelder/openenlarge --public --source=. --remote=origin --description "Develop your film negatives with real physics — a density-domain inverter." --push
```
Expected: repo created, `main` pushed. If `origin` already exists, instead: `git remote add origin https://github.com/mohaelder/openenlarge.git && git push -u origin main`.

- [ ] **Step 2: Add signing secrets**

Following `docs/RELEASING.md`, add the macOS and Windows secrets via `gh secret set` or the GitHub UI. Example:
```bash
base64 -i DeveloperIDApplication.p12 | gh secret set APPLE_CERTIFICATE
gh secret set APPLE_CERTIFICATE_PASSWORD
# ...repeat for APPLE_SIGNING_IDENTITY, APPLE_ID, APPLE_PASSWORD, APPLE_TEAM_ID,
#    WINDOWS_CERTIFICATE, WINDOWS_CERTIFICATE_PASSWORD
```

- [ ] **Step 3: Enable GitHub Pages (Actions source)**

In repo Settings → Pages → Build and deployment → Source: **GitHub Actions**. Then trigger the page deploy:
```bash
gh workflow run pages.yml
```
Expected: after the run, the site is live at `https://mohaelder.github.io/openenlarge`.

- [ ] **Step 4: Cut the first release**

```bash
git tag v0.1.0
git push origin v0.1.0
```
Expected: `release.yml` runs the 4-platform matrix and creates a **draft** GitHub Release with signed installers attached. Review the draft, verify each installer downloads and the macOS build opens without a Gatekeeper warning (notarization), then publish the release.

- [ ] **Step 5: Verify the landing page download links resolve**

After publishing the release, load `https://mohaelder.github.io/openenlarge` and confirm the download button points at a real installer asset for your OS (releases.js reads the now-published release).

---

## Self-Review Notes

- **Spec coverage:** §1 repo/scrub/bundle-id → Task 1 + Task 7.1. §2 README → Task 2. §3 release CI + secrets → Task 4 (+ `docs/RELEASING.md`). §4 landing page + pages workflow → Tasks 5–6. §5 LICENSE/CI/screenshot scaffold → Task 1.7, Task 3, Task 2.1. §6 sequencing → task order; outward-facing step isolated in Task 7. All covered.
- **CLI syntax** matches the real `film-cli` (positional input, `-o`, `--mode`, `--compare`, `--stock`, `--base-rect`) — no nonexistent `invert` subcommand.
- **Identifiers consistent:** `com.mohaelder.openenlarge`, repo `mohaelder/openenlarge`, Pages URL `mohaelder.github.io/openenlarge`, secret names identical across `release.yml`, `docs/RELEASING.md`, and Task 7.
- **Graceful degradation:** missing Apple/Windows secrets → unsigned build, not a failure (verified by the `if: env.WINDOWS_CERTIFICATE != ''` guard and Tauri's skip-when-absent behavior).
