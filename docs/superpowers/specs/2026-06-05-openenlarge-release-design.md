# OpenEnlarge — Release Readiness Design

**Date:** 2026-06-05
**Status:** Approved (design phase)
**Goal:** Take the project (local folder `filmrev`) to a public first release as **OpenEnlarge**:
a GitHub repo, a solid README, signed cross-platform release builds via CI, and a hosted
GitHub Pages landing page.

---

## 0. Context

OpenEnlarge is a physically-based film-negative inversion / digital darkroom desktop app.
Stack: Rust workspace (`crates/film-core` engine + `crates/film-cli` headless CLI) wrapped by
a Tauri 2 + SvelteKit shell in `app/`. The product is **already named "OpenEnlarge"** in
`app/src-tauri/tauri.conf.json` (productName + window title). Engine strategy (from the POC
spec): density-domain Beer-Lambert matrix inversion (engine **B**) with a naive per-channel
baseline (engine **C**) shipped as a comparison mode.

**Decided parameters (from brainstorming):**
- Repo home: `github.com/mohaelder/openenlarge`; Pages at `mohaelder.github.io/openenlarge`.
- Release targets: macOS (Apple Silicon + Intel), Windows, Linux.
- Landing page: rich single-page, self-contained, no build step.
- Screenshots: **user-provided**, dropped into `docs/screenshots/`. Use placeholder slots +
  TODO markers until provided.
- Signing: **macOS** Developer ID sign + notarize, **Windows** code-sign. Linux unsigned.
- Bundle identifier: change `com.mohaelder.app` → **`com.mohaelder.openenlarge`**.
- Include a lightweight **CI workflow** (test/clippy/fmt + svelte-check).
- Local folder stays `filmrev`; only the GitHub repo is named `openenlarge`.
- First release tag: **`v0.1.0`**.

**Art direction (approved via visual companion):** Apple-style glassmorphism matching the
app's `theme.css` — ground `#0a0a0c`, glass panels `rgba(28,28,34,.55)` with
`backdrop-filter: blur(22px) saturate(140%)` + inset highlight, amber accent gradient
`#f49d4e → #df7136`, Apple system font, ambient amber radial glow. Hero keeps a "technical"
density-inversion curve panel + the Beer-Lambert equation chip
`Ĉ = M_post · log₁₀(M_pre · I₀/I)`. Approved mockup saved at
`.superpowers/brainstorm/.../content/landing-glass.html`.

---

## 1. Work Item: Repo rename & `filmrev` scrub

### 1.1 GitHub repo
- Create `mohaelder/openenlarge` (public). Add as `origin`, push `main`. Existing feature
  branches may be pushed as-is (not required for release).
- **Confirm before pushing:** creating a public repo and pushing is outward-facing — get
  explicit go-ahead at execution time (the repo may already exist or be intended private).

### 1.2 Scrub `filmrev` strings
The name "filmrev" only survives in **test temp-file names** (not user-facing):
- `crates/film-core/src/export.rs:35,49` — `filmrev_roundtrip.tiff`, `filmrev_rgba16.tiff`
- `app/src-tauri/src/encode.rs:151,159,168,188` — `filmrev_t1_*`
Rename these to `openenlarge_*` for tidiness. These are inside `#[cfg(test)]` paths; renaming
must keep tests passing (they create + read back the same path, so a literal swap is safe).

### 1.3 Bundle identifier
In `app/src-tauri/tauri.conf.json`: `identifier: "com.mohaelder.app"` →
`"com.mohaelder.openenlarge"`. No other code references the identifier. This changes app-data
location on next run (acceptable pre-release).

### 1.4 Crate names — unchanged
`film-core` / `film-cli` stay (internal, film-themed, not "filmrev", and renaming would churn
`Cargo.toml`, imports, and CLI invocation for no user-facing gain).

---

## 2. Work Item: `README.md` (new, repo root)

A single `README.md` at the repo root (the current `app/README.md` is the stock Tauri
template — replace it with a one-line pointer to the root README, or delete it).

**Sections (modeled on openpano-swift, in the maintainer's voice):**
1. **Title + tagline** — e.g. "OpenEnlarge — develop your film negatives with real physics, not a flipped curve."
2. **Badges** — MIT license · latest release · platforms (macOS/Win/Linux) · CI status.
3. **Hero screenshot** — `docs/screenshots/hero.png` (placeholder until provided).
4. **Intro** — what it is + the density-domain pitch (Beer-Lambert engine B vs naive baseline C). 2–3 short paragraphs, links to the POC design for the deep version.
5. **Before → After** — a negative/positive sample pair (`docs/screenshots/before.png`, `after.png`).
6. **Features** — bulleted (decode RAF/DNG/TIFF, density-domain inversion, per-image develop, curves/color grading, crop/rotate, base calibration per roll, batch export, CLI).
7. **Architecture** — table: `film-core` (pure engine), `film-cli` (headless), `app/` (Tauri+Svelte shell), engine B (density matrix) / C (baseline).
8. **Download / Install** — links to GitHub Releases, one line per OS.
9. **Build from source** — prerequisites (Rust stable, Node ≥18, Tauri system deps per OS), then: `cd app && npm install && npm run tauri dev` (dev) / `npm run tauri build` (release). CLI: `cargo run -p film-cli -- invert in.dng -o out.tiff --mode b`.
10. **CLI usage** — short example block.
11. **How it works** — condensed physics (forward model + the inversion equation), link to spec.
12. **Contributing** — brief: issues/PRs welcome, run `ci` checks locally.
13. **License** — MIT.

**Screenshot handling:** reference `docs/screenshots/*.png`; create the folder with a
`README.md` listing the expected files (`hero.png`, `before.png`, `after.png`, plus 2–3 UI
shots) so the user knows exactly what to drop in. README image tags include `alt` text and
an HTML comment `<!-- TODO: replace placeholder -->` near each.

---

## 3. Work Item: Release CI — `.github/workflows/release.yml`

**Trigger:** push of tag matching `v*`; plus `workflow_dispatch` (manual) for dry runs.

**Build matrix** (uses `tauri-apps/tauri-action`, `projectPath: app`):
| Runner | Target | Artifacts |
|---|---|---|
| `macos-14` | aarch64-apple-darwin | `.dmg`, `.app.tar.gz` |
| `macos-13` | x86_64-apple-darwin | `.dmg`, `.app.tar.gz` |
| `windows-latest` | x86_64-pc-windows-msvc | `.msi`, NSIS `.exe` |
| `ubuntu-22.04` | x86_64-unknown-linux-gnu | `.AppImage`, `.deb` |

**Steps per job:** checkout → setup Rust (stable, target) → setup Node + `npm ci` in `app` →
Linux system deps (`libwebkit2gtk-4.1`, `libgtk-3`, `librsvg2`, `libayatana-appindicator3`)
→ `tauri-action` build + upload. `tauri-action` creates/updates a single GitHub Release
(draft first, `tagName: ${{ github.ref_name }}`, `releaseName: OpenEnlarge ${tag}`) and
attaches all matrix artifacts.

**macOS signing + notarization** (env on macOS jobs, from repo secrets):
`APPLE_CERTIFICATE` (base64 .p12), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_PASSWORD` (app-specific password), `APPLE_TEAM_ID`. `tauri-action` /
Tauri's bundler consume these to sign + notarize + staple.

**Windows signing** (Windows job, from repo secrets): import PFX from
`WINDOWS_CERTIFICATE` (base64) + `WINDOWS_CERTIFICATE_PASSWORD`; Tauri bundle config
references the cert (thumbprint or `signCommand`). Configured via env so no secret is
committed.

**Linux:** unsigned.

**Secrets documentation:** the spec/README enumerate every secret and how to produce it
(export Developer ID .p12 → base64; create app-specific password; export Windows PFX). The
workflow degrades gracefully: if Apple/Windows secrets are absent, the corresponding job
still builds **unsigned** rather than failing (guarded by `if` on secret presence) — so the
pipeline is testable before secrets land.

**Permissions:** job needs `contents: write` to publish the release.

---

## 4. Work Item: Landing page — GitHub Pages

### 4.1 Files
- `web/index.html` — self-contained, the approved glass design. Inline `<style>` using the
  app's exact tokens. Sections: glass nav · hero (headline + CTAs + density-curve glass card +
  equation chip) · feature grid (glass tiles) · "how it works" (forward model → inversion,
  3-step) · screenshots strip (`../docs/screenshots/` copied in, or `web/img/`) · download
  section · footer (MIT, GitHub, made by mohaelder).
- `web/releases.js` — on load, `fetch` `https://api.github.com/repos/mohaelder/openenlarge/releases/latest`,
  detect OS from `navigator.userAgent`/`platform`, set the primary download button to the
  matching asset's `browser_download_url`. Fallbacks: secondary "all downloads" link to
  `/releases/latest`; if the API call fails or no release exists yet, buttons point at
  `/releases/latest` directly. No API token (unauthenticated, public repo, low traffic).
- `web/img/` — landing-specific images (app icon, screenshots). Reuse `app/src-tauri/icons/icon.png`
  for the favicon/logo.

### 4.2 Deployment — `.github/workflows/pages.yml`
- Trigger: push to `main` affecting `web/**` (+ `workflow_dispatch`).
- Modern Pages Actions flow: `actions/configure-pages` → `actions/upload-pages-artifact`
  (`path: web`) → `actions/deploy-pages`. Permissions `pages: write`, `id-token: write`.
- Keeps the site source in `web/` — **separate from `docs/`** (which holds specs/plans and
  must not be served).
- One-time: enable Pages "GitHub Actions" source in repo settings (manual, documented).

### 4.3 Download-button behavior
Primary CTA label adapts to detected OS ("Download for macOS / Windows / Linux"). Asset
matching heuristic: macOS → `.dmg` (prefer aarch64 on Apple Silicon UA when distinguishable,
else first `.dmg`); Windows → `.msi` or `.exe`; Linux → `.AppImage`.

---

## 5. Work Item: Supporting files

- **`LICENSE`** at repo root — MIT, copyright holder "mohaelder" (matches `package.json`
  `"license": "MIT"`). Year 2026.
- **`.github/workflows/ci.yml`** — trigger on push + PR to `main`:
  - Rust job (ubuntu): `cargo fmt --all --check`, `cargo clippy --workspace -- -D warnings`,
    `cargo test --workspace` (excludes the Tauri app crate, per the `Cargo.toml` workspace
    `exclude`). Add Linux Tauri deps only if needed by `film-core`/`film-cli` (they have no UI
    deps, so likely not).
  - Frontend job (ubuntu): `cd app && npm ci && npm run check` (svelte-check) and
    `npm run test:unit` (vitest).
  - Cache cargo + npm for speed.
- **`docs/screenshots/README.md`** — lists the expected screenshot files + recommended
  dimensions/framing, so the user can supply matching assets.

---

## 6. Data flow / sequencing

```
[1] rename identifier + scrub filmrev + LICENSE        (local edits, tests stay green)
        │
[2] README.md + docs/screenshots/ scaffold             (depends on architecture facts)
        │
[3] ci.yml                                              (validates [1] didn't break build)
        │
[4] release.yml (+ secrets doc)                         (independent of [5])
        │
[5] web/ landing page + pages.yml                       (independent of [4])
        │
[6] create GitHub repo, push, add secrets, enable Pages, tag v0.1.0   (execution-time, user-gated)
```

Items [1]–[5] are file edits in the working tree, fully reviewable before anything outward-
facing happens. Item [6] (create repo, push, publish release, deploy site) is the only
outward-facing step and requires explicit user confirmation + the user adding secrets.

---

## 7. Out of scope (YAGNI)

- Renaming the local folder or the `film-*` crates.
- Auto-updater / Tauri updater signing (separate from release code-signing; future).
- Homebrew cask / winget / Linux repo packaging (can follow once releases exist).
- Issue/PR templates, code of conduct, multi-language landing page.
- Apple/Windows certificate *acquisition* — user already has certs; we only wire them in.

---

## 8. Risks & mitigations

- **Secrets not yet in CI** → workflows guard signing on secret presence and build unsigned
  otherwise, so the pipeline is testable before secrets land.
- **No screenshots yet** → placeholders + a manifest so the page/README render and the user
  knows exactly what to drop in; nothing blocks on art.
- **Pages serving `docs/`** → avoided by serving `web/` via the Actions artifact flow, never
  branch-`/docs`.
- **macOS universal vs per-arch** → ship per-arch `.dmg`s (simpler, smaller) rather than a
  universal binary; the landing JS picks the right one.
- **Bundle-id change** → done once, pre-release, before any installed user base exists.
