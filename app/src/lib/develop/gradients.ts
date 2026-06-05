// CSS linear-gradient track backgrounds for sliders.
export const TEMP_GRADIENT =
  "linear-gradient(90deg, #4a90ff 0%, #cfd8e6 50%, #ffd24a 100%)";
export const TINT_GRADIENT =
  "linear-gradient(90deg, #4ad24a 0%, #cfcfcf 50%, #ff4af0 100%)";
export const SAT_GRADIENT =
  "linear-gradient(90deg, #808080 0%, #ff0000 17%, #ffff00 33%, " +
  "#00ff00 50%, #00ffff 67%, #0000ff 83%, #ff00ff 100%)";

/** Lightroom-style signed integer (e.g. +24, −13, 0). */
export function signed(v: number): string {
  const r = Math.round(v);
  return r > 0 ? `+${r}` : `${r}`;
}

/** EV display with two decimals and sign (e.g. +1.30, 0.00). */
export function ev(v: number): string {
  return (v > 0 ? "+" : "") + v.toFixed(2);
}

/** Kelvin display (rounded to nearest 10). */
export function kelvin(v: number): string {
  return `${Math.round(v / 10) * 10}`;
}

/** Per-band hue-slider tracks (Lightroom-style: band hue shifting to its neighbors).
 *  Keyed by CM band name. */
export const CM_HUE_GRADIENTS: Record<string, string> = {
  red:     "linear-gradient(90deg,#ff00d4 0%,#ff0000 50%,#ff7a00 100%)",
  orange:  "linear-gradient(90deg,#ff0000 0%,#ff7a00 50%,#ffe000 100%)",
  yellow:  "linear-gradient(90deg,#ff7a00 0%,#ffe000 50%,#9dff00 100%)",
  green:   "linear-gradient(90deg,#ffe000 0%,#1fdf3f 50%,#00d9c0 100%)",
  aqua:    "linear-gradient(90deg,#1fdf3f 0%,#00d9c0 50%,#2a7bff 100%)",
  blue:    "linear-gradient(90deg,#00d9c0 0%,#2a7bff 50%,#7a3cff 100%)",
  purple:  "linear-gradient(90deg,#2a7bff 0%,#7a3cff 50%,#ff00d4 100%)",
  magenta: "linear-gradient(90deg,#7a3cff 0%,#ff00d4 50%,#ff0000 100%)",
};
/** Per-band saturation track: gray → the band's pure color. */
export const CM_SAT_GRADIENTS: Record<string, string> = {
  red:     "linear-gradient(90deg,#808080 0%,#ff2b2b 100%)",
  orange:  "linear-gradient(90deg,#808080 0%,#ff8c1a 100%)",
  yellow:  "linear-gradient(90deg,#808080 0%,#ffe000 100%)",
  green:   "linear-gradient(90deg,#808080 0%,#1fdf3f 100%)",
  aqua:    "linear-gradient(90deg,#808080 0%,#00d9c0 100%)",
  blue:    "linear-gradient(90deg,#808080 0%,#2a7bff 100%)",
  purple:  "linear-gradient(90deg,#808080 0%,#7a3cff 100%)",
  magenta: "linear-gradient(90deg,#808080 0%,#ff00d4 100%)",
};
/** Luminance track: dark → light. */
export const CM_LUM_GRADIENT = "linear-gradient(90deg,#1a1a1a 0%,#808080 50%,#f0f0f0 100%)";
