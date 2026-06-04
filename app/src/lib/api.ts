import { invoke } from "@tauri-apps/api/core";

export interface Metadata {
  camera?: string; lens?: string; iso?: string; shutter?: string;
  aperture?: string; width: number; height: number; file_size: number; date?: string;
}
export interface ImageEntry {
  id: string; file_name: string; thumbnail: string; metadata: Metadata;
}
export interface InvertParams {
  mode: "b" | "c";
  stock: "none" | "portra400" | "fujic200";
  base_rect: [number, number, number, number] | null;
  exposure: number; black: number; gamma: number;
}

export const api = {
  importImage: (path: string) => invoke<ImageEntry>("import_image", { path }),
  rawPreview: (id: string) => invoke<string>("raw_preview", { id }),
  invertedPreview: (id: string, params: InvertParams) =>
    invoke<string>("inverted_preview", { id, params }),
  exportImage: (id: string, params: InvertParams, outPath: string) =>
    invoke<void>("export_image", { id, params, outPath }),
};

export const defaultParams = (): InvertParams => ({
  mode: "b", stock: "none", base_rect: null, exposure: 1, black: 0, gamma: 0.4545,
});
