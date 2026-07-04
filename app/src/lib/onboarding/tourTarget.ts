import type { Action } from "svelte/action";
import { get } from "svelte/store";
import { translate } from "../i18n";
import {
  tourStep, stepDef, isLastStep, advanceTour, dismissTour,
  TOUR_STEPS, type TourStepId,
} from "./tour";

/**
 * Tour highlight — issue #21 (upstream). `use:tourTarget={"step-id"}` marks an
 * element as a tour step's target: while that step is active the element gets
 * the pulsing accent outline (`.tour-glow`, styles/theme.css — a box-shadow
 * ring, so layout never shifts) and a small floating card (title + body +
 * Next/Skip) anchored below or above it.
 *
 * The card is plain imperative DOM appended to <body> (no component, no deps):
 * it must escape any overflow/stacking context of the target's panel, and its
 * lifetime is exactly "this step is active", which the store subscription
 * already owns. Texts are resolved at show time via translate() — a locale
 * switch mid-step is not re-rendered, which is fine for a transient card.
 *
 * Clicking the highlighted control itself also advances the tour: doing what
 * the card asks IS the natural "next" (advanceTour's stale-step guard makes
 * this safe when the tour is inactive or on another step).
 */

const GAP = 10; // px between the target's rect and the card
const MARGIN = 8; // px the card keeps clear of the viewport edges
// The module switch plays a ~220ms fly-in (+page.svelte); re-anchor once it
// settles so a card shown mid-transition doesn't keep the 10px offset.
const SETTLE_MS = 260;

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

export const tourTarget: Action<HTMLElement, TourStepId> = (node, step) => {
  let current = step;
  let tip: HTMLDivElement | null = null;
  let settle: ReturnType<typeof setTimeout> | null = null;

  function position(): void {
    if (!tip) return;
    const r = node.getBoundingClientRect();
    // Prefer below the target; flip above when there's no room. Clamp
    // horizontally so the card never leaves the viewport.
    const below = r.bottom + GAP + tip.offsetHeight <= window.innerHeight - MARGIN;
    tip.style.top = `${below ? r.bottom + GAP : Math.max(MARGIN, r.top - GAP - tip.offsetHeight)}px`;
    tip.style.left = `${clamp(
      r.left + r.width / 2 - tip.offsetWidth / 2,
      MARGIN,
      window.innerWidth - tip.offsetWidth - MARGIN,
    )}px`;
  }

  function el(cls: string, text: string): HTMLDivElement {
    const d = document.createElement("div");
    d.className = cls;
    d.textContent = text;
    return d;
  }

  function show(): void {
    if (tip) return;
    node.classList.add("tour-glow");
    const def = stepDef(current);
    tip = document.createElement("div");
    tip.className = "tour-tip";
    tip.setAttribute("role", "dialog");
    tip.setAttribute("aria-label", translate(def.titleKey));
    tip.append(el("tour-tip-title", translate(def.titleKey)), el("tour-tip-body", translate(def.bodyKey)));

    const actions = document.createElement("div");
    actions.className = "tour-tip-actions";
    const progress = el("tour-tip-progress",
      `${TOUR_STEPS.indexOf(def) + 1} / ${TOUR_STEPS.length}`);
    const skip = document.createElement("button");
    skip.className = "tour-tip-skip";
    skip.textContent = translate("tour.skip");
    skip.addEventListener("click", () => dismissTour());
    const next = document.createElement("button");
    next.className = "tour-tip-next";
    next.textContent = translate(isLastStep(current) ? "tour.done" : "tour.next");
    const from = current; // freeze: advance THIS step even if params changed since
    next.addEventListener("click", () => advanceTour(from));
    actions.append(progress, skip, next);
    tip.append(actions);

    document.body.appendChild(tip);
    position();
    window.addEventListener("resize", position);
    settle = setTimeout(position, SETTLE_MS);
  }

  function hide(): void {
    node.classList.remove("tour-glow");
    if (settle !== null) { clearTimeout(settle); settle = null; }
    if (!tip) return;
    window.removeEventListener("resize", position);
    tip.remove();
    tip = null;
  }

  // Following the instruction (clicking the highlighted control) advances too.
  const onNodeClick = (): void => advanceTour(current);
  node.addEventListener("click", onNodeClick);

  const unsub = tourStep.subscribe((s) => (s === current ? show() : hide()));

  return {
    update(next: TourStepId) {
      if (next === current) return;
      hide();
      current = next;
      if (get(tourStep) === current) show();
    },
    destroy() {
      unsub();
      hide();
      node.removeEventListener("click", onNodeClick);
    },
  };
};
