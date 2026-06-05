import { api, type InvertParams } from "./api";
import type { CropRect } from "./crop/types";
import type { DustEdits } from "./develop/dust";

/** A debounced function with a `flush()` that fires any pending call now. */
export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  flush(): void;
}

/** Trailing-edge debounce: coalesce rapid calls; last args win. `flush()` fires now. */
export function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number,
): Debounced<A> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;
  const wrapped = ((...args: A) => {
    pending = args;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      const p = pending; pending = null;
      if (p) fn(...p);
    }, ms);
  }) as Debounced<A>;
  wrapped.flush = () => {
    if (timer) { clearTimeout(timer); timer = null; }
    const p = pending; pending = null;
    if (p) fn(...p);
  };
  return wrapped;
}
