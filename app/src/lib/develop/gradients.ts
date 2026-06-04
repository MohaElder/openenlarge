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
