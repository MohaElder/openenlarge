# Mainstream Film Profiles Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add 11 mainstream color-negative film profiles (Portra 160/800, Ektar 100, Gold 200, UltraMax 400, Pro 400H, Superia X-tra 400, Vision3 50D/200T/250D/500T) sourced from spektrafilm.

**Architecture:** A committed, reproducible converter downloads each spektrafilm profile JSON and writes a `dye_<key>.csv` in the repo's exact format (verified byte-identical to the existing two). Then the `Stock` enum + `load_stock`, the `stock_from` wire mapping, the TS `stock` union, both UI `<select>`s, and the `basic.stock.*` i18n strings each gain 11 entries.

**Tech Stack:** Python (converter), Rust (`film-core` + Tauri), Svelte + TypeScript, Vitest, `cargo test`.

**Spec:** `docs/superpowers/specs/2026-06-05-mainstream-film-profiles-design.md`

---

## File Structure

- Create `scripts/build_film_data.py` — downloads spektrafilm profiles, writes all `dye_*.csv`.
- Create `crates/film-core/data/dye_<key>.csv` × 11 — generated bundled data.
- Modify `crates/film-core/data/DATA_SOURCES.md` — list the 11 new stocks.
- Modify `crates/film-core/src/spectral.rs` — 11 `Stock` variants, `load_stock` arms, a `Stock::ALL` const, extended tests.
- Modify `app/src-tauri/src/commands.rs` — 11 `stock_from` arms.
- Modify `app/src/lib/api.ts` — 11 `stock` union literals.
- Modify `app/src/lib/develop/Basic.svelte` and `app/src/lib/overlay/ConfirmDevelop.svelte` — 11 `<option>`s each.
- Modify `i18n-strings.csv` (+ regenerated `app/src/lib/i18n/dict.ts`) — 11 `basic.stock.*` rows.

**Canonical key list (used verbatim everywhere):** `portra160`, `portra800`, `ektar100`, `gold200`, `ultramax400`, `fujipro400h`, `fujixtra400`, `vision350d`, `vision3200t`, `vision3250d`, `vision3500t`.

---

## Task 1: Reproducible converter + generated CSVs + DATA_SOURCES

**Files:**
- Create: `scripts/build_film_data.py`, `crates/film-core/data/dye_<key>.csv` × 11
- Modify: `crates/film-core/data/DATA_SOURCES.md`

- [ ] **Step 1: Create `scripts/build_film_data.py`** with this exact content:

```python
#!/usr/bin/env python3
"""Download spektrafilm film profiles and write per-dye density CSVs.

Source: https://github.com/andreavolpato/spektrafilm (CC BY-SA 4.0).
Each profile's `data.channel_density` (C/M/Y) + `data.base_density` (D_min) are
resampled onto the repo grid (380-730 nm @ 5 nm); None samples outside a curve's
measured span are edge-held via np.interp. This script is the single source of
truth for crates/film-core/data/dye_*.csv and regenerates the existing two
byte-for-byte. Run from anywhere: `python3 scripts/build_film_data.py`.
"""
import json
import os
import urllib.request

import numpy as np

BASE = "https://raw.githubusercontent.com/andreavolpato/spektrafilm/main/src/spektrafilm/data/profiles"
DATA_DIR = os.path.join(os.path.dirname(__file__), "..", "crates", "film-core", "data")
GRID = list(range(380, 731, 5))  # 380..730 @ 5 nm (71 points)

# repo stock key -> spektrafilm profile name
FILMS = {
    "portra400": "kodak_portra_400",
    "fujic200": "fujifilm_c200",
    "portra160": "kodak_portra_160",
    "portra800": "kodak_portra_800",
    "ektar100": "kodak_ektar_100",
    "gold200": "kodak_gold_200",
    "ultramax400": "kodak_ultramax_400",
    "fujipro400h": "fujifilm_pro_400h",
    "fujixtra400": "fujifilm_xtra_400",
    "vision350d": "kodak_vision3_50d",
    "vision3200t": "kodak_vision3_200t",
    "vision3250d": "kodak_vision3_250d",
    "vision3500t": "kodak_vision3_500t",
}


def _fill(series, wl):
    """np.interp over valid (non-None) samples: linear interior, edge-hold outside."""
    idx = [i for i, v in enumerate(series) if v is not None]
    xs = [wl[i] for i in idx]
    ys = [float(series[i]) for i in idx]
    return np.interp(GRID, xs, ys)


def convert(profile_json):
    d = profile_json["data"]
    wl, cd, bd = d["wavelengths"], d["channel_density"], d["base_density"]
    dc = _fill([c[0] if c else None for c in cd], wl)
    dm = _fill([c[1] if c else None for c in cd], wl)
    dy = _fill([c[2] if c else None for c in cd], wl)
    dmin = _fill(bd, wl)
    lines = ["wavelength,D_C,D_M,D_Y,D_min"]
    for k, w in enumerate(GRID):
        c = lambda v: max(0.0, float(v))
        lines.append(f"{w},{c(dc[k]):.4f},{c(dm[k]):.4f},{c(dy[k]):.4f},{c(dmin[k]):.4f}")
    return "\n".join(lines) + "\n"


def main():
    for key, profile in FILMS.items():
        with urllib.request.urlopen(f"{BASE}/{profile}.json", timeout=30) as r:
            data = json.load(r)
        csv = convert(data)
        out = os.path.join(DATA_DIR, f"dye_{key}.csv")
        with open(out, "w") as f:
            f.write(csv)
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Run the converter.**

Run: `cd /Users/mohaelder/Repos/filmrev && python3 scripts/build_film_data.py`
Expected: prints `wrote .../dye_<key>.csv` for all 13 keys. (Requires network + `numpy` — `python3 -c "import numpy"` should succeed; if missing, `pip3 install numpy`.)

- [ ] **Step 3: Verify the existing two are unchanged (converter regression guard) and 11 new files exist.**

Run: `git status --porcelain crates/film-core/data/`
Expected: exactly 11 new `dye_*.csv` files listed as untracked (`??`), and **NO** `M` (modified) line for `dye_portra400.csv` / `dye_fujic200.csv` — proving the converter reproduces them byte-for-byte.

Run: `for f in portra160 portra800 ektar100 gold200 ultramax400 fujipro400h fujixtra400 vision350d vision3200t vision3250d vision3500t; do n=$(wc -l < crates/film-core/data/dye_$f.csv); c=$(head -2 crates/film-core/data/dye_$f.csv | tail -1 | awk -F, '{print NF}'); echo "$f rows=$n cols=$c"; done`
Expected: every file `rows=72 cols=5` (header + 71 data rows, 5 columns).

- [ ] **Step 4: Update `crates/film-core/data/DATA_SOURCES.md`.** Replace the heading line `## \`dye_portra400.csv\`, \`dye_fujic200.csv\` — film dye spectral densities + base` with `## \`dye_*.csv\` — film dye spectral densities + base`, and under its `- **Source:**` bullet's `Profiles:` sub-bullet, replace the two-profile mention with the full list. Specifically change the `Profiles:` sub-bullet to:

```markdown
  - Profiles (spektrafilm `src/spektrafilm/data/profiles/*.json`,
    `data.channel_density` → C/M/Y per-dye spectral density; `data.base_density` → `D_min`):
    `kodak_portra_400`, `fujifilm_c200`, `kodak_portra_160`, `kodak_portra_800`,
    `kodak_ektar_100`, `kodak_gold_200`, `kodak_ultramax_400`, `fujifilm_pro_400h`,
    `fujifilm_xtra_400`, `kodak_vision3_50d`, `kodak_vision3_200t`,
    `kodak_vision3_250d`, `kodak_vision3_500t`. All color negatives (C-41/ECN-2).
    Reversal/slide stocks (e.g. Kodachrome) are excluded — they are positives and
    do not fit the negative-inversion model.
```

Also update line 4's `Conversion script:` reference from `\`/tmp/convert.py\` (recorded in commit history)` to `\`scripts/build_film_data.py\` (reproducible; regenerates every \`dye_*.csv\`)`.

- [ ] **Step 5: Commit.**

```bash
git add scripts/build_film_data.py crates/film-core/data/dye_*.csv crates/film-core/data/DATA_SOURCES.md
git commit -m "data(film): 11 mainstream negative profiles + reproducible converter"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 2: `Stock` enum + `load_stock` + tests (film-core)

**Files:**
- Modify: `crates/film-core/src/spectral.rs`
- Test: inline `#[cfg(test)]` in `spectral.rs`

- [ ] **Step 1: Extend the `Stock` enum.** Replace the enum (currently `Portra400`, `FujiC200`) body so it reads:

```rust
pub enum Stock {
    Portra400,
    FujiC200,
    Portra160,
    Portra800,
    Ektar100,
    Gold200,
    Ultramax400,
    FujiPro400H,
    FujiXtra400,
    Vision350D,
    Vision3200T,
    Vision3250D,
    Vision3500T,
}
```

- [ ] **Step 2: Add a `Stock::ALL` const** right after the enum (used by tests and any caller wanting to enumerate stocks):

```rust
impl Stock {
    /// Every bundled stock, for enumeration in tests/UI-parity checks.
    pub const ALL: [Stock; 13] = [
        Stock::Portra400, Stock::FujiC200, Stock::Portra160, Stock::Portra800,
        Stock::Ektar100, Stock::Gold200, Stock::Ultramax400, Stock::FujiPro400H,
        Stock::FujiXtra400, Stock::Vision350D, Stock::Vision3200T, Stock::Vision3250D,
        Stock::Vision3500T,
    ];
}
```

- [ ] **Step 3: Extend `load_stock`'s match** so every variant maps to its CSV. Replace the `let dye_csv = match stock { ... };` block with:

```rust
    let dye_csv = match stock {
        Stock::Portra400 => include_str!("../data/dye_portra400.csv"),
        Stock::FujiC200 => include_str!("../data/dye_fujic200.csv"),
        Stock::Portra160 => include_str!("../data/dye_portra160.csv"),
        Stock::Portra800 => include_str!("../data/dye_portra800.csv"),
        Stock::Ektar100 => include_str!("../data/dye_ektar100.csv"),
        Stock::Gold200 => include_str!("../data/dye_gold200.csv"),
        Stock::Ultramax400 => include_str!("../data/dye_ultramax400.csv"),
        Stock::FujiPro400H => include_str!("../data/dye_fujipro400h.csv"),
        Stock::FujiXtra400 => include_str!("../data/dye_fujixtra400.csv"),
        Stock::Vision350D => include_str!("../data/dye_vision350d.csv"),
        Stock::Vision3200T => include_str!("../data/dye_vision3200t.csv"),
        Stock::Vision3250D => include_str!("../data/dye_vision3250d.csv"),
        Stock::Vision3500T => include_str!("../data/dye_vision3500t.csv"),
    };
```

- [ ] **Step 4: Update the existing `load_stock_returns_consistent_grid` test** to cover ALL stocks. Change its loop header from `for stock in [Stock::Portra400, Stock::FujiC200] {` to:

```rust
        for stock in Stock::ALL {
```

- [ ] **Step 5: Add a test that every stock fits a finite, non-identity `M_post`.** Add this test in `spectral.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn every_stock_loads_and_fits_finite_nonidentity_mpost() {
        use crate::calibrate::fit_m_post;
        let id = nalgebra::Matrix3::identity();
        for stock in Stock::ALL {
            let data = load_stock(stock);
            let m = fit_m_post(&data);
            assert!(m.iter().all(|v| v.is_finite()), "{stock:?} M_post not finite");
            assert!((m - id).norm() > 1e-3, "{stock:?} M_post unexpectedly identity");
        }
    }
```

- [ ] **Step 6: Run the film-core tests.**

Run: `cd /Users/mohaelder/Repos/filmrev && cargo test -p film-core`
Expected: PASS (all existing + the extended grid test now covering 13 stocks + the new fit test). If a `dye_*.csv` is missing, `include_str!` fails at compile — confirms Task 1 produced all 11 files.

- [ ] **Step 7: Commit.**

```bash
git add crates/film-core/src/spectral.rs
git commit -m "feat(film): Stock variants + load_stock for 11 new profiles"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 3: `stock_from` wire mapping (Tauri backend)

**Files:**
- Modify: `app/src-tauri/src/commands.rs`

- [ ] **Step 1: Extend `stock_from`.** Replace the function body's match so it reads:

```rust
fn stock_from(s: &str) -> Option<Stock> {
    match s {
        "portra400" => Some(Stock::Portra400),
        "fujic200" => Some(Stock::FujiC200),
        "portra160" => Some(Stock::Portra160),
        "portra800" => Some(Stock::Portra800),
        "ektar100" => Some(Stock::Ektar100),
        "gold200" => Some(Stock::Gold200),
        "ultramax400" => Some(Stock::Ultramax400),
        "fujipro400h" => Some(Stock::FujiPro400H),
        "fujixtra400" => Some(Stock::FujiXtra400),
        "vision350d" => Some(Stock::Vision350D),
        "vision3200t" => Some(Stock::Vision3200T),
        "vision3250d" => Some(Stock::Vision3250D),
        "vision3500t" => Some(Stock::Vision3500T),
        _ => None,
    }
}
```

- [ ] **Step 2: Build the app crate.**

Run: `cd /Users/mohaelder/Repos/filmrev/app/src-tauri && cargo build`
Expected: builds clean (the new `Stock` variants resolve from `film_core::spectral::Stock`, already imported).

- [ ] **Step 3: Run app tests.**

Run: `cd /Users/mohaelder/Repos/filmrev/app/src-tauri && cargo test`
Expected: all pass.

- [ ] **Step 4: Commit.**

```bash
git add app/src-tauri/src/commands.rs
git commit -m "feat(film): map new stock keys to Stock in stock_from"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## Task 4: Frontend — type union, dropdowns, i18n

**Files:**
- Modify: `app/src/lib/api.ts`, `app/src/lib/develop/Basic.svelte`, `app/src/lib/overlay/ConfirmDevelop.svelte`, `i18n-strings.csv` (+ regenerated `app/src/lib/i18n/dict.ts`)

- [ ] **Step 1: Extend the `stock` union in `app/src/lib/api.ts`.** Replace the line `  stock: "none" | "portra400" | "fujic200";` with:

```typescript
  stock: "none" | "portra400" | "fujic200" | "portra160" | "portra800" | "ektar100"
    | "gold200" | "ultramax400" | "fujipro400h" | "fujixtra400"
    | "vision350d" | "vision3200t" | "vision3250d" | "vision3500t";
```

- [ ] **Step 2: Add i18n rows to `i18n-strings.csv`.** After the existing `basic.stock.fujic200` row, add these 11 rows (columns `key,en,zh,file,note`):

```csv
basic.stock.portra160,"Kodak Portra 160","柯达 Portra 160","src/lib/develop/Basic.svelte","option"
basic.stock.portra800,"Kodak Portra 800","柯达 Portra 800","src/lib/develop/Basic.svelte","option"
basic.stock.ektar100,"Kodak Ektar 100","柯达 Ektar 100","src/lib/develop/Basic.svelte","option"
basic.stock.gold200,"Kodak Gold 200","柯达 Gold 200","src/lib/develop/Basic.svelte","option"
basic.stock.ultramax400,"Kodak UltraMax 400","柯达 UltraMax 400","src/lib/develop/Basic.svelte","option"
basic.stock.fujipro400h,"Fujifilm Pro 400H","富士 Pro 400H","src/lib/develop/Basic.svelte","option"
basic.stock.fujixtra400,"Fujifilm Superia X-tra 400","富士 Superia X-tra 400","src/lib/develop/Basic.svelte","option"
basic.stock.vision350d,"Kodak Vision3 50D","柯达 Vision3 50D","src/lib/develop/Basic.svelte","option"
basic.stock.vision3200t,"Kodak Vision3 200T","柯达 Vision3 200T","src/lib/develop/Basic.svelte","option"
basic.stock.vision3250d,"Kodak Vision3 250D","柯达 Vision3 250D","src/lib/develop/Basic.svelte","option"
basic.stock.vision3500t,"Kodak Vision3 500T","柯达 Vision3 500T","src/lib/develop/Basic.svelte","option"
```

- [ ] **Step 3: Regenerate dict.ts.**

Run: `cd /Users/mohaelder/Repos/filmrev && python3 scripts/gen-i18n.py`
Verify: `grep -c "basic.stock.vision3500t" app/src/lib/i18n/dict.ts` prints `2` (en + zh).

- [ ] **Step 4: Add the `<option>`s to the develop dropdown.** In `app/src/lib/develop/Basic.svelte`, after the `<option value="fujic200">{$t('basic.stock.fujic200')}</option>` line, insert:

```svelte
        <option value="portra160">{$t('basic.stock.portra160')}</option>
        <option value="portra800">{$t('basic.stock.portra800')}</option>
        <option value="ektar100">{$t('basic.stock.ektar100')}</option>
        <option value="gold200">{$t('basic.stock.gold200')}</option>
        <option value="ultramax400">{$t('basic.stock.ultramax400')}</option>
        <option value="fujipro400h">{$t('basic.stock.fujipro400h')}</option>
        <option value="fujixtra400">{$t('basic.stock.fujixtra400')}</option>
        <option value="vision350d">{$t('basic.stock.vision350d')}</option>
        <option value="vision3200t">{$t('basic.stock.vision3200t')}</option>
        <option value="vision3250d">{$t('basic.stock.vision3250d')}</option>
        <option value="vision3500t">{$t('basic.stock.vision3500t')}</option>
```

- [ ] **Step 5: Add the same `<option>`s to the Confirm Develop dropdown.** In `app/src/lib/overlay/ConfirmDevelop.svelte`, after the `<option value="fujic200">{$t('basic.stock.fujic200')}</option>` line, insert the identical 11 `<option>` lines from Step 4.

- [ ] **Step 6: Typecheck + unit tests.**

Run: `cd app && npm run check && npm run test:unit`
Expected: 0 errors (every `<option value>` is a member of the `stock` union, so a typo would fail typecheck); all unit tests pass.

- [ ] **Step 7: Commit.**

```bash
git add app/src/lib/api.ts app/src/lib/develop/Basic.svelte app/src/lib/overlay/ConfirmDevelop.svelte i18n-strings.csv app/src/lib/i18n/dict.ts
git commit -m "feat(film): expose 11 new stocks in the develop + confirm-develop pickers"
```
End the commit body with:
`Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

- [ ] **Step 8: Manual verification** (restart `cd app && npm run tauri dev`):
  - Develop panel → Film Profile dropdown lists all 13 stocks; pick Ektar 100, Gold 200, Vision3 500T → each inverts plausibly and roughly neutral (balance_neutral keeps them anchored).
  - Develop all → Confirm dialog stock picker lists the same 13; pick one → applies to the roll.

---

## Self-Review Notes (verified during planning)

- **Spec coverage:** converter + 11 CSVs + regeneration guard (Task 1); DATA_SOURCES update (Task 1 Step 4); `Stock` enum + `load_stock` + tests (Task 2); `stock_from` (Task 3); TS union + both dropdowns + i18n (Task 4). Kodachrome/reversal excluded (not in FILMS map). Each profile uses the existing stock-agnostic `params_for_stock`/`balance_neutral` — no change needed there.
- **Key consistency:** the 11 keys are identical across the converter `FILMS` map, the CSV filenames, `load_stock`'s `include_str!` paths, `stock_from` arms, the TS union, the `<option value>`s, and the `basic.stock.<key>` i18n keys. Enum variants (`Portra160`…`Vision3500T`) map 1:1 to those keys.
- **Type consistency:** `Stock::ALL` has 13 entries matching the 13 variants; `load_stock` match is exhaustive (compiler-enforced); the TS union members exactly match the `<option value>`s (typecheck-enforced).
- **Edge cases:** unknown stock string → `stock_from` `None` → no-profile fallback (unchanged); missing CSV → compile error (caught in Task 2 Step 6); converter None samples → `np.interp` edge-hold (validated).
- **Dependency order:** Task 2 (`include_str!`) requires Task 1's CSVs to exist; Task 3 requires Task 2's enum variants; Task 4 is independent of 2/3 but pairs with them for end-to-end. Execute in order 1→2→3→4.
