import { writable, get } from "svelte/store";
import { module as moduleStore } from "../store";

/**
 * First-run guided tour — issue #21 (upstream).
 *
 * A tiny linear step machine: `tourStep` holds the active step id (null =
 * inactive). Each step points at one workflow control via the `tourTarget`
 * action (lib/onboarding/tourTarget.ts), which pulses the control and shows a
 * floating explanation card. The steps teach the app's core loop in order:
 * sample the film base once per roll, crop the whole roll, then finish single
 * frames in the Frame (develop) tab.
 */

export type TourStepId = "base-tool" | "crop-tool" | "tune-tab";

export interface TourStep {
  id: TourStepId;
  /** Module whose UI hosts the step's target element (startTour jumps there). */
  module: "library" | "roll" | "develop";
  /** i18n keys for the tooltip card (see i18n-strings.csv `tour.*`). */
  titleKey: string;
  bodyKey: string;
}

/** Steps in tour order. The tune-tab step lives in the top bar (visible from
 * every module) but belongs to the roll flow, so its module stays "roll". */
export const TOUR_STEPS: TourStep[] = [
  { id: "base-tool", module: "roll", titleKey: "tour.base.title", bodyKey: "tour.base.body" },
  { id: "crop-tool", module: "roll", titleKey: "tour.crop.title", bodyKey: "tour.crop.body" },
  { id: "tune-tab", module: "roll", titleKey: "tour.tune.title", bodyKey: "tour.tune.body" },
];

/** Active step id, or null when no tour is running. */
export const tourStep = writable<TourStepId | null>(null);

/** True once the user has finished OR skipped the tour. Persisted via prefs as
 * `onboarding_done` ("true"/"false"); hydrated + write-through in catalog.ts.
 * Absent pref = never seen, so the tour auto-starts once (see initTourAutoStart). */
export const onboardingDone = writable<boolean>(false);

/** Definition lookup — the ids are a closed union, so this always finds one. */
export function stepDef(id: TourStepId): TourStep {
  return TOUR_STEPS.find((s) => s.id === id)!;
}

/** Whether `id` is the final step (its Next button reads "Done"). */
export function isLastStep(id: TourStepId): boolean {
  return TOUR_STEPS[TOUR_STEPS.length - 1].id === id;
}

/** Start (or replay) the tour from the first step. Jumps to the step's module
 * so a replay from Settings works from anywhere — the highlighted controls
 * only exist in the Roll tab. */
export function startTour(): void {
  moduleStore.set(TOUR_STEPS[0].module);
  tourStep.set(TOUR_STEPS[0].id);
}

/** Advance past `from`. Guarded on the CURRENT step so a stale caller (e.g. a
 * click on the highlighted control racing the tooltip's Next button) can never
 * double-advance or resurrect a dismissed tour. Advancing past the last step
 * completes the tour. */
export function advanceTour(from: TourStepId): void {
  if (get(tourStep) !== from) return;
  const next = TOUR_STEPS[TOUR_STEPS.indexOf(stepDef(from)) + 1];
  if (next) tourStep.set(next.id);
  else finishTour();
}

/** "Skip tour": stop now and never auto-start again (marks the pref). */
export function dismissTour(): void {
  finishTour();
}

function finishTour(): void {
  tourStep.set(null);
  onboardingDone.set(true); // persisted as onboarding_done via catalog.ts
}

/**
 * Auto-start trigger — ONE simple, robust condition: the first time the user
 * lands in the Roll tab with `onboarding_done` unset, start the tour. That is
 * exactly "the user is on their first roll": the normal path (first import →
 * Develop-all confirm → developAll switches to roll) triggers it, and so does
 * a direct Roll visit or a session restored into Roll — all places where the
 * highlighted Base/Crop tools are actually on screen. Call AFTER hydrate() so
 * the persisted pref (and any restored module) is respected; existing users
 * upgrading see the tour once, which is intended for a new feature.
 *
 * Returns an unsubscribe for component teardown.
 */
export function initTourAutoStart(): () => void {
  let started = false;
  return moduleStore.subscribe((m) => {
    if (started || m !== "roll" || get(onboardingDone)) return;
    started = true; // fire at most once per app session even if dismissed mid-way
    startTour();
  });
}
