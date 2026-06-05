import { invoke } from "@tauri-apps/api/core";

export interface Metadata {
  camera?: string; lens?: string; iso?: string; shutter?: string;
  aperture?: string; width: number; height: number; file_size: number; date?: string;
}
export interface ImageEntry {
  id: string; path: string; file_name: string; thumbnail: string; metadata: Metadata; developed: boolean;
}
export type Quality = "performance" | "quality";
export interface InvertParams {
  mode: "b" | "c";
  stock: "none" | "portra400" | "fujic200";
  base_rect: [number, number, number, number] | null;
  exposure: number; // EV stops (−5..5)
  black: number; gamma: number;
  auto_wb: boolean;
  temp: number; // Kelvin
  tint: number; // −150..150
  contrast: number; highlights: number; shadows: number;
  whites: number; blacks: number;
  texture: number; vibrance: number; saturation: number;
}
export interface AsShotWb { temp: number; tint: number }
export interface DustStroke { points: { x: number; y: number }[]; r: number }
export interface ViewSpec {
  crop: [number, number, number, number];
  out_w: number;
  out_h: number;
  raw: boolean;
  finish?: boolean; // omit/true = backend applies finishing; false = GPU does it
  image_crop?: [number, number, number, number] | null; // normalized persistent crop
  rot90?: number; flip_h?: boolean; flip_v?: boolean; angle?: number;
  dust?: DustStroke[];
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
  ) =>
    invoke<void>("export_image", {
      id, params, outPath, imageCrop,
      rot90: geom.rot90 ?? 0, flipH: geom.flip_h ?? false,
      flipV: geom.flip_v ?? false, angle: geom.angle ?? 0, dust: wireDust(dust),
    }),
  developImage: (id: string) => invoke<ImageEntry>("develop_image", { id }),
  setQuality: (quality: Quality) => invoke<void>("set_quality", { quality }),
  thumbnail: (id: string, params: InvertParams) => invoke<string>("thumbnail", { id, params }),
  asShotWb: (id: string) => invoke<AsShotWb>("as_shot_wb", { id }),
};

export const defaultParams = (): InvertParams => ({
  mode: "b", stock: "none", base_rect: null,
  exposure: 0, black: 0, gamma: 0.4545,
  auto_wb: true, temp: 5500, tint: 0,
  contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  texture: 0, vibrance: 0, saturation: 0,
});
