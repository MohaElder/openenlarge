import { invoke } from "@tauri-apps/api/core";
import type { DustStroke, IrRemoval } from "./develop/dust";
export type { DustStroke, IrRemoval };

export interface Metadata {
  camera?: string; lens?: string; iso?: string; shutter?: string;
  aperture?: string; width: number; height: number; file_size: number; date?: string;
  note?: string;
}
/** User-edited metadata overrides (one per image). Only changed, non-blank fields
 * are stored; an absent/empty field falls back to the source EXIF value. */
export interface MetaOverride {
  camera?: string; lens?: string; iso?: string; shutter?: string;
  aperture?: string; date?: string; note?: string;
}
/** The editable EXIF fields, in display order. Drives the metadata panel + reset. */
export const META_FIELDS = ["camera", "lens", "iso", "shutter", "aperture", "date", "note"] as const;
export type MetaField = (typeof META_FIELDS)[number];
export interface ImageEntry {
  id: string; path: string; file_name: string; thumbnail: string;
  metadata: Metadata; developed: boolean; has_ir: boolean; offline: boolean;
}
export type Quality = "performance" | "quality";
/** A tone-curve control point in [0,1]×[0,1] (input → output). */
export type CurvePoint = [number, number];
/** Identity curve: a straight 0→0, 1→1 line. */
export const IDENTITY_CURVE: CurvePoint[] = [[0, 0], [1, 1]];
/** One Point Color sample: a picked target color + its per-sample adjustments. */
export interface PointColorSample {
  hue: number;   // target hue, 0..360 (fixed at pick time)
  sat: number;   // target saturation, 0..1
  lum: number;   // target lightness, 0..1
  hue_shift: number;  // −100..100
  sat_shift: number;  // −100..100
  lum_shift: number;  // −100..100
  variance: number;   // −100..100 (widens sat/lum tolerance)
  range: number;      // 0..100 (hue-window half-width), default 50
}
/** Lightroom Color Mixer band order (fixed). */
export const CM_BANDS = ["red","orange","yellow","green","aqua","blue","purple","magenta"] as const;
export type CmBand = (typeof CM_BANDS)[number];
export interface InvertParams {
  mode: "b" | "c";
  stock: "none" | "portra400" | "fujic200";
  base_override: [number, number, number] | null;
  exposure: number; // EV stops (−5..5)
  black: number; gamma: number;
  auto_wb: boolean;
  temp: number; // Kelvin
  tint: number; // −150..150
  contrast: number; highlights: number; shadows: number;
  whites: number; blacks: number;
  texture: number; vibrance: number; saturation: number;

  // --- Tone Curve (−100..100 region sliders; point curves are 0..1 control points) ---
  tc_highlights: number; tc_lights: number; tc_darks: number; tc_shadows: number;
  tc_curve: CurvePoint[]; // master (RGB) point curve
  tc_red: CurvePoint[]; tc_green: CurvePoint[]; tc_blue: CurvePoint[];

  // --- Color Grading (hue 0..360, sat 0..100, lum −100..100 per region) ---
  cg_sh_hue: number; cg_sh_sat: number; cg_sh_lum: number;
  cg_mid_hue: number; cg_mid_sat: number; cg_mid_lum: number;
  cg_hi_hue: number; cg_hi_sat: number; cg_hi_lum: number;
  cg_glob_hue: number; cg_glob_sat: number; cg_glob_lum: number;
  cg_blending: number; // 0..100 (mask overlap width), default 50
  cg_balance: number;  // −100..100 (shadow↔highlight crossover), default 0

  // --- Color Mixer (HSL): 8 bands × hue/sat/lum, each −100..100, 0 = identity ---
  cm_red_hue: number; cm_red_sat: number; cm_red_lum: number;
  cm_orange_hue: number; cm_orange_sat: number; cm_orange_lum: number;
  cm_yellow_hue: number; cm_yellow_sat: number; cm_yellow_lum: number;
  cm_green_hue: number; cm_green_sat: number; cm_green_lum: number;
  cm_aqua_hue: number; cm_aqua_sat: number; cm_aqua_lum: number;
  cm_blue_hue: number; cm_blue_sat: number; cm_blue_lum: number;
  cm_purple_hue: number; cm_purple_sat: number; cm_purple_lum: number;
  cm_magenta_hue: number; cm_magenta_sat: number; cm_magenta_lum: number;
  // --- Point Color: up to 8 sampled swatches ---
  pc_samples: PointColorSample[];
}
export interface AsShotWb { temp: number; tint: number }
export interface ViewSpec {
  crop: [number, number, number, number];
  out_w: number;
  out_h: number;
  raw: boolean;
  finish?: boolean; // omit/true = backend applies finishing; false = GPU does it
  image_crop?: [number, number, number, number] | null; // normalized persistent crop
  rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number;
  dust?: DustStroke[];
  ir_removal?: IrRemoval;
}

/** Geometry + dust/IR for baking a heal-ready working buffer (raw negative). */
export interface BakeSpec {
  rot90: number; flip_h: boolean; flip_v: boolean; angle: number;
  image_crop: [number, number, number, number] | null;
  dust: DustStroke[];
  ir_removal: IrRemoval;
}

/** Persistent per-image edits that shape a thumbnail (no zoom/view crop). */
export interface ThumbView {
  image_crop?: [number, number, number, number] | null;
  rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number;
  dust?: DustStroke[];
  ir_removal?: IrRemoval;
}

export interface ExportFormat {
  kind: "jpeg" | "tiff" | "png";
  bitDepth?: 8 | 16;        // tiff/png only
  quality?: number;         // jpeg only, 1–100
  maxBytes?: number | null; // jpeg only
}

/** One image as returned by load_catalog. `developed`/`has_ir` reflect whether a
 * decoded-image cache exists on disk (pixels load lazily on first view). */
export interface CatalogImage {
  id: string; path: string; file_name: string; thumbnail: string;
  metadata: Metadata; offline: boolean; developed: boolean; has_ir: boolean;
}
/** One image's stored edits as returned by load_catalog (JSON already parsed). */
export interface CatalogEdits {
  image_id: string;
  params: InvertParams | null;
  crop: import("./crop/types").CropRect | null;
  dust: import("./develop/dust").DustEdits | null;
  meta: MetaOverride | null;
}
/** The whole catalog returned at launch. */
export interface CatalogSnapshot {
  images: CatalogImage[];
  edits: CatalogEdits[];
  prefs: Record<string, string>;
  app_state: Record<string, string>;
}

/** Convert app-internal {x,y} points to the [x,y] tuple format the Rust side expects. */
const wireDust = (dust?: DustStroke[]) =>
  (dust ?? []).map((s) => ({ points: s.points.map((p) => [p.x, p.y]), r: s.r }));

export const api = {
  importImage: (path: string) => invoke<ImageEntry>("import_image", { path }),
  renderView: (id: string, params: InvertParams, view: ViewSpec) =>
    invoke<string>("render_view", { id, params, view: { ...view, dust: wireDust(view.dust) } }),
  exportImage: (
    id: string, params: InvertParams, outPath: string,
    imageCrop: [number, number, number, number] | null = null,
    geom: { rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number } = {},
    dust: DustStroke[] = [],
    irRemoval: IrRemoval = { enabled: false, sensitivity: 50 },
    format: ExportFormat = { kind: "tiff", bitDepth: 16 },
    metaOverride: MetaOverride | null = null,
  ) =>
    invoke<void>("export_image", {
      id, params, outPath, imageCrop,
      rot90: geom.rot90 ?? 0, flipH: geom.flip_h ?? false,
      flipV: geom.flip_v ?? false, angle: geom.angle ?? 0,
      dust: wireDust(dust), irRemoval, format, metaOverride,
    }),
  developImage: (id: string) => invoke<ImageEntry>("develop_image", { id }),
  setQuality: (quality: Quality) => invoke<void>("set_quality", { quality }),
  /** Forget an image; when deleteFile is true also move the file to the OS trash. */
  deleteImage: (id: string, deleteFile: boolean) => invoke<void>("delete_image", { id, deleteFile }),
  thumbnail: (id: string, params: InvertParams, view: ThumbView = {}) =>
    invoke<string>("thumbnail", { id, params, view: { ...view, dust: wireDust(view.dust) } }),
  asShotWb: (id: string, params: InvertParams) =>
    invoke<AsShotWb>("as_shot_wb", { id, params }),
  loadCatalog: () => invoke<CatalogSnapshot>("load_catalog"),
  saveEdits: (id: string, paramsJson: string) =>
    invoke<void>("save_edits", { id, paramsJson }),
  saveCrop: (id: string, cropJson: string) =>
    invoke<void>("save_crop", { id, cropJson }),
  saveDust: (id: string, dustJson: string) =>
    invoke<void>("save_dust", { id, dustJson }),
  saveMeta: (id: string, metaJson: string) =>
    invoke<void>("save_meta", { id, metaJson }),
  savePref: (key: string, value: string) =>
    invoke<void>("save_pref", { key, value }),
  saveAppState: (key: string, value: string) =>
    invoke<void>("save_app_state", { key, value }),
  workingInfo: (id: string) =>
    invoke<{ w: number; h: number }>("working_info", { id }),

  // Tauri returns the command's `Response` bytes as an ArrayBuffer.
  workingPixels: (id: string) =>
    invoke<ArrayBuffer>("working_pixels", { id }),

  workingBakedInfo: (id: string, spec: BakeSpec) =>
    invoke<{ w: number; h: number }>("working_baked_info", { id, spec: { ...spec, dust: wireDust(spec.dust) } }),

  workingBakedPixels: (id: string, spec: BakeSpec) =>
    invoke<ArrayBuffer>("working_baked_pixels", { id, spec: { ...spec, dust: wireDust(spec.dust) } }),

  resolvedInversion: (id: string, params: InvertParams) =>
    invoke<import("./viewport/gl/invert").ResolvedInversion>("resolved_inversion", { id, params }),

  sampleBaseAt: (id: string, rect: [number, number, number, number]) =>
    invoke<[number, number, number]>("sample_base_at", { id, rect }),

  // ---- GPU export (offscreen invert+finish through the preview shader) ----
  /** Decode+bake full-res, stash the half-float bytes, return dims + inversion uniforms. */
  exportBegin: (id: string, params: InvertParams, spec: BakeSpec) =>
    invoke<{ w: number; h: number; uniforms: import("./viewport/gl/invert").ResolvedInversion }>(
      "export_begin", { id, params, spec: { ...spec, dust: wireDust(spec.dust) } }),
  /** Fetch the stashed full-res half-float RGBA bytes for GPU upload. */
  exportPixels: () => invoke<ArrayBuffer>("export_pixels"),
  /** Encode the GPU readback into the chosen format (+ EXIF). `data` is a byte array. */
  exportFinish: (
    id: string, outPath: string, readback: { w: number; h: number; bit16: boolean },
    data: number[], format: ExportFormat, metaOverride: MetaOverride | null,
  ) =>
    invoke<void>("export_finish", { id, outPath, readback, data, format, metaOverride }),
};

export const defaultParams = (): InvertParams => ({
  mode: "b", stock: "none", base_override: null,
  exposure: 0, black: 0, gamma: 0.4545,
  auto_wb: true, temp: 5500, tint: 0,
  contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  texture: 0, vibrance: 0, saturation: 0,

  tc_highlights: 0, tc_lights: 0, tc_darks: 0, tc_shadows: 0,
  tc_curve: IDENTITY_CURVE.map((p) => [...p] as CurvePoint),
  tc_red: IDENTITY_CURVE.map((p) => [...p] as CurvePoint),
  tc_green: IDENTITY_CURVE.map((p) => [...p] as CurvePoint),
  tc_blue: IDENTITY_CURVE.map((p) => [...p] as CurvePoint),

  cg_sh_hue: 0, cg_sh_sat: 0, cg_sh_lum: 0,
  cg_mid_hue: 0, cg_mid_sat: 0, cg_mid_lum: 0,
  cg_hi_hue: 0, cg_hi_sat: 0, cg_hi_lum: 0,
  cg_glob_hue: 0, cg_glob_sat: 0, cg_glob_lum: 0,
  cg_blending: 50, cg_balance: 0,

  cm_red_hue: 0, cm_red_sat: 0, cm_red_lum: 0,
  cm_orange_hue: 0, cm_orange_sat: 0, cm_orange_lum: 0,
  cm_yellow_hue: 0, cm_yellow_sat: 0, cm_yellow_lum: 0,
  cm_green_hue: 0, cm_green_sat: 0, cm_green_lum: 0,
  cm_aqua_hue: 0, cm_aqua_sat: 0, cm_aqua_lum: 0,
  cm_blue_hue: 0, cm_blue_sat: 0, cm_blue_lum: 0,
  cm_purple_hue: 0, cm_purple_sat: 0, cm_purple_lum: 0,
  cm_magenta_hue: 0, cm_magenta_sat: 0, cm_magenta_lum: 0,
  pc_samples: [],
});
