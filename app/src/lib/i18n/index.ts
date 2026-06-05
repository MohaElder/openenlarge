import { derived, get, writable } from "svelte/store";
import { dict } from "./dict";

export type Locale = "en" | "zh";

/** Selectable UI languages, in display order. */
export const LOCALES: { id: Locale; label: string }[] = [
  { id: "en", label: "English" },
  { id: "zh", label: "中文" },
];

/** Active UI language. Persisted via the catalog (see catalog.ts). */
export const locale = writable<Locale>("en");

function lookup(l: Locale, key: string): string {
  return dict[l]?.[key] ?? dict.en[key] ?? key;
}

/** Substitute {placeholder} tokens. Unknown tokens are left intact. */
function fill(s: string, params?: Record<string, string | number>): string {
  if (!params) return s;
  let out = s;
  for (const k in params) out = out.split(`{${k}}`).join(String(params[k]));
  return out;
}

/**
 * Reactive translator. In markup: `{$t('some.key')}` or `{$t('some.key', { name })}`.
 * Falls back to the English string, then the key itself, when a translation is missing.
 */
export const t = derived(
  locale,
  ($l) => (key: string, params?: Record<string, string | number>): string =>
    fill(lookup($l, key), params),
);

/** Non-reactive lookup for plain .ts modules (does not track locale changes). */
export function translate(key: string, params?: Record<string, string | number>): string {
  return fill(lookup(get(locale), key), params);
}
