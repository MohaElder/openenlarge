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
