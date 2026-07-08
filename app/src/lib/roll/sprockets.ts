// app/src/lib/roll/sprockets.ts
//
// Standards-correct film perforation geometry, shared by the on-screen filmstrip
// (Roll.svelte CSS background) and the contact-sheet export (exportSheet.ts canvas)
// so the two renderers can never drift apart — issue #23 (upstream).
//
// Both formats use the KS-1870 (Kodak Standard) perforation *shape*; the two
// modes differ only in how millimetres are mapped to sheet pixels:
//   "135" — one 135 frame period (36mm frame + 2mm gap = 38mm) spans exactly
//           8 perforations at 4.75mm pitch, so pitch = framePeriodPx / 8 and the
//           holes land 8-per-frame like real 35mm stock.
//   "120" — real 120 film has no perforations at all; per issue #23 we style the
//           rebate after 70mm cine stock instead, scaling the same KS hole from
//           the frame SHORT edge (nominal 56mm for medium format) and running the
//           holes at constant pitch along the whole strip (not frame-aligned).

export type FilmFormat = "135" | "120";

// ── KS-1870 perforation, millimetres ─────────────────────────────────────────
export const KS_PITCH_MM = 4.75;   // hole-centre to hole-centre along the film
export const KS_PERF_W_MM = 2.794; // hole width (along the film)
export const KS_PERF_H_MM = 1.981; // hole height (across the film)
export const KS_RADIUS_MM = 0.51;  // corner radius
export const PERFS_PER_FRAME_135 = 8; // 38mm frame period / 4.75mm pitch
export const SHORT_EDGE_120_MM = 56;  // nominal 120 frame short edge (6×6 = 56mm)

// Breathing room above+below the hole inside the rebate band (px each side),
// so the holes read as punched into the rebate rather than touching its edges.
const BAND_PAD_PX = 2;

export interface PerfLayout {
  bandH: number;  // sprocket band height (px): perf height + breathing room
  pitch: number;  // hole-centre spacing (px)
  perfW: number;  // hole width along the film (px)
  perfH: number;  // hole height (px)
  radius: number; // hole corner radius (px)
  // Left edge of the first one-hole tile, relative to the strip's left edge
  // (the hole sits centred inside its pitch-wide tile). Doubles as the CSS
  // background-position-x for the repeat-x tile.
  offset: number;
}

/** Compute the perforation layout for one strip, in sheet pixels.
 *  `frameW`/`frameGap`/`framePad` are the sheet design constants (exportSheet.ts);
 *  `frameH` is the tile height — only "120" uses it (short-edge scaling). */
export function perfLayout(
  mode: FilmFormat,
  frameW: number,
  frameH: number,
  frameGap: number,
  framePad = 6,
): PerfLayout {
  // px-per-mm scale: "135" pins 8 perfs under each frame period; "120" derives
  // the scale from the frame short edge so the holes keep true KS proportions
  // relative to a 56mm-tall frame (visibly smaller than 135 — correct).
  let pitch: number;
  let offset: number;
  if (mode === "135") {
    pitch = (frameW + frameGap) / PERFS_PER_FRAME_135;
    // First hole centred at frameLeft + pitch/2 ⇒ the 8 holes of each period sit
    // wholly under their frame (frameLeft = framePad for the first frame; the
    // period equals 8 pitches exactly, so the phase holds for every frame).
    offset = framePad;
  } else {
    const scale = frameH / SHORT_EDGE_120_MM;
    pitch = KS_PITCH_MM * scale;
    offset = 0; // constant pitch along the whole strip, not frame-aligned
  }
  const mmToPx = pitch / KS_PITCH_MM;
  const perfW = KS_PERF_W_MM * mmToPx;
  const perfH = KS_PERF_H_MM * mmToPx;
  const radius = KS_RADIUS_MM * mmToPx;
  const bandH = Math.round(perfH + 2 * BAND_PAD_PX);
  return { bandH, pitch, perfW, perfH, radius, offset };
}

/** One-hole SVG tile (pitch × bandH, hole centred) as a data URI for the CSS
 *  `background-image` of `.sprocket-holes` — repeat-x turns it into the full
 *  perforation row. Fill defaults to the historic backlit-hole tint (holes read
 *  slightly brighter than the #131210 rebate). Uses fill-opacity instead of
 *  rgba() and percent-encodes everything so the URI is safe unquoted in CSS. */
export function perfTileDataUri(
  layout: PerfLayout,
  fill = "#d8cfb8",
  fillOpacity = 0.16,
): string {
  const { pitch, bandH, perfW, perfH, radius } = layout;
  const x = (pitch - perfW) / 2;
  const y = (bandH - perfH) / 2;
  const num = (v: number) => String(Math.round(v * 1000) / 1000); // trim float noise
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${num(pitch)}" height="${bandH}" ` +
    `viewBox="0 0 ${num(pitch)} ${bandH}">` +
    `<rect x="${num(x)}" y="${num(y)}" width="${num(perfW)}" height="${num(perfH)}" ` +
    `rx="${num(radius)}" fill="${fill}" fill-opacity="${fillOpacity}"/></svg>`;
  // encodeURIComponent leaves ( ) ' unescaped, which can break CSS url(...);
  // encode them too so the caller can embed the URI without quoting concerns.
  const enc = encodeURIComponent(svg)
    .replace(/\(/g, "%28").replace(/\)/g, "%29").replace(/'/g, "%27");
  return `data:image/svg+xml,${enc}`;
}

function hashString(str: string): number {
  let hash = 0;
  for (let i = 0; i < str.length; i++) {
    hash = (hash << 5) - hash + str.charCodeAt(i);
    hash |= 0;
  }
  return (Math.abs(hash) % 1000) / 1000;
}

export interface StripOffsets {
  topDx: number;
  topDy: number;
  botDx: number;
  botDy: number;
}

export function getStripOffsets(stripImageIds: string[]): StripOffsets {
  const seed = stripImageIds.join("-");
  const h1 = hashString(seed + "-h1");
  const v1 = hashString(seed + "-v1");
  const h2 = hashString(seed + "-h2");
  const v2 = hashString(seed + "-v2");
  
  return {
    topDx: Math.round(h1 * 60 - 30), // -30px to +30px
    topDy: Math.round(v1 * 8 - 4),   // -4px to +4px
    botDx: Math.round(h2 * 60 - 30), // -30px to +30px
    botDy: Math.round(v2 * 8 - 4),   // -4px to +4px
  };
}
