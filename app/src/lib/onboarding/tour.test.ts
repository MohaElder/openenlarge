import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  TOUR_STEPS, tourStep, onboardingDone,
  startTour, advanceTour, dismissTour, initTourAutoStart, stepDef, isLastStep,
} from "./tour";
import { module as moduleStore } from "../store";

beforeEach(() => {
  tourStep.set(null);
  onboardingDone.set(false);
  moduleStore.set("library");
});

describe("step machine", () => {
  it("startTour activates the first step and jumps to its module", () => {
    startTour();
    expect(get(tourStep)).toBe(TOUR_STEPS[0].id);
    expect(get(moduleStore)).toBe(TOUR_STEPS[0].module);
  });

  it("advanceTour walks every step in order, then completes and marks the pref", () => {
    startTour();
    for (const s of TOUR_STEPS) {
      expect(get(tourStep)).toBe(s.id);
      advanceTour(s.id);
    }
    expect(get(tourStep)).toBeNull();
    expect(get(onboardingDone)).toBe(true);
  });

  it("advanceTour ignores a stale step id (double-fire guard)", () => {
    startTour();
    advanceTour(TOUR_STEPS[1].id); // not the current step
    expect(get(tourStep)).toBe(TOUR_STEPS[0].id);
    expect(get(onboardingDone)).toBe(false);
  });

  it("advanceTour is a no-op while the tour is inactive", () => {
    advanceTour(TOUR_STEPS[0].id);
    expect(get(tourStep)).toBeNull();
    expect(get(onboardingDone)).toBe(false);
  });

  it("dismissTour stops the tour and marks it done", () => {
    startTour();
    dismissTour();
    expect(get(tourStep)).toBeNull();
    expect(get(onboardingDone)).toBe(true);
  });

  it("stepDef/isLastStep resolve ids against the step list", () => {
    expect(stepDef("crop-tool").id).toBe("crop-tool");
    expect(isLastStep(TOUR_STEPS[TOUR_STEPS.length - 1].id)).toBe(true);
    expect(isLastStep(TOUR_STEPS[0].id)).toBe(false);
  });
});

describe("initTourAutoStart (pref gating)", () => {
  it("starts on the first Roll entry when the pref is unset", () => {
    const stop = initTourAutoStart();
    expect(get(tourStep)).toBeNull(); // still in library
    moduleStore.set("roll");
    expect(get(tourStep)).toBe(TOUR_STEPS[0].id);
    stop();
  });

  it("starts immediately when the session restored straight into Roll", () => {
    moduleStore.set("roll"); // hydration set the persisted module before arming
    const stop = initTourAutoStart();
    expect(get(tourStep)).toBe(TOUR_STEPS[0].id);
    stop();
  });

  it("never starts once onboarding_done is set", () => {
    onboardingDone.set(true);
    const stop = initTourAutoStart();
    moduleStore.set("roll");
    expect(get(tourStep)).toBeNull();
    stop();
  });

  it("does not start on library or develop entry", () => {
    const stop = initTourAutoStart();
    moduleStore.set("develop");
    expect(get(tourStep)).toBeNull();
    stop();
  });

  it("fires at most once per session (a dismissed tour stays dismissed)", () => {
    const stop = initTourAutoStart();
    moduleStore.set("roll");
    dismissTour();
    moduleStore.set("library");
    moduleStore.set("roll");
    expect(get(tourStep)).toBeNull();
    stop();
  });
});
