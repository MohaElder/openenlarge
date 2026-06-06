import { compareVersions } from "./version";

export const DAY_MS = 86_400_000;

/** True when at least `intervalMs` has elapsed since the last check attempt. */
export function shouldAutoCheck(nowMs: number, lastCheckMs: number, intervalMs = DAY_MS): boolean {
  return nowMs - lastCheckMs >= intervalMs;
}

/** True when `latest` is strictly newer than `current` and not the skipped version. */
export function shouldPrompt(latest: string, current: string, skipped: string): boolean {
  return compareVersions(latest, current) > 0 && latest !== skipped;
}
