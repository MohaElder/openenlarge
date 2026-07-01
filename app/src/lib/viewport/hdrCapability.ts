// Frontend HDR capability detector: decides which HDR display mode is active
// (live extended-dynamic-range surface, a static gainmap fallback, or hidden
// entirely) for the current OS/display/GPU-surface combination.
//
// `detectHdrMode` is a PURE function of its `HdrEnv` input so it's
// deterministic and unit-testable without mocking browser/Tauri globals.
// `probeHdrEnv` is the only place that touches the environment.

export type HdrMode = "live-edr" | "gainmap-fallback" | "hidden";

export interface HdrEnv {
  os: "macos" | "windows" | "linux";
  displayHdr: boolean;
  surfaceSupported: boolean;
}

/** Pure rule table — no environment access here. */
export function detectHdrMode(env: HdrEnv): HdrMode {
  if (env.os === "linux") return "hidden";
  if (env.displayHdr && env.surfaceSupported) return "live-edr";
  return "gainmap-fallback";
}

// No @tauri-apps/plugin-os dependency in this repo; derive `os` from
// navigator the same way app/src/lib/keymap/hotkeys.ts's isMac() does.
function detectOs(): HdrEnv["os"] {
  if (typeof navigator === "undefined") return "linux";
  const p = (navigator.platform || navigator.userAgent || "").toLowerCase();
  if (p.includes("mac")) return "macos";
  if (p.includes("win")) return "windows";
  return "linux";
}

/** Probes the live browser/OS environment. Thin wiring only — keep untested or lightly tested. */
export async function probeHdrEnv(): Promise<HdrEnv> {
  const os = detectOs();
  const displayHdr =
    typeof window !== "undefined" && "matchMedia" in window
      ? window.matchMedia("(dynamic-range: high)").matches
      : false;
  // Windows/WebGPU surface not yet wired → route to gainmap-fallback;
  // revisit when the Windows path lands.
  const surfaceSupported = os === "macos" ? true : false;
  return { os, displayHdr, surfaceSupported };
}
