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
export function detectOs(): HdrEnv["os"] {
  if (typeof navigator === "undefined") return "linux";
  const p = (navigator.platform || navigator.userAgent || "").toLowerCase();
  if (p.includes("mac")) return "macos";
  if (p.includes("win")) return "windows";
  return "linux";
}

/** True iff WebGPU is present AND an rgba16float canvas can be configured with
 *  toneMapping:'extended' (the EDR mechanism). Environment-touching; never throws. */
export async function probeWebGpuExtended(): Promise<boolean> {
  try {
    if (typeof navigator === "undefined" || !("gpu" in navigator) || !navigator.gpu) return false;
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) return false;
    const device = await adapter.requestDevice();
    const cv = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(2, 2) : null;
    const ctx = cv?.getContext("webgpu") as GPUCanvasContext | null;
    if (!ctx) { device.destroy?.(); return false; }
    ctx.configure({ device, format: "rgba16float", alphaMode: "opaque", toneMapping: { mode: "extended" } } as GPUCanvasConfiguration);
    ctx.unconfigure?.();
    device.destroy?.();
    return true;
  } catch {
    return false;
  }
}

/** Probes the live browser/OS environment. Thin wiring only — keep untested or lightly tested. */
export async function probeHdrEnv(): Promise<HdrEnv> {
  const os = detectOs();
  const displayHdr =
    typeof window !== "undefined" && "matchMedia" in window
      ? window.matchMedia("(dynamic-range: high)").matches
      : false;
  const surfaceSupported =
    os === "macos" ? true
    : os === "windows" ? await probeWebGpuExtended()
    : false;
  return { os, displayHdr, surfaceSupported };
}
