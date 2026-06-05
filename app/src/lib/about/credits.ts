/** Third-party open-source dependencies that ship in OpenEnlarge and warrant
 * attribution. Grouped by where they live. Keep in sync with package.json and the
 * Cargo manifests when dependencies change. */
export interface Credit {
  name: string;
  license: string;
  url: string;
}

export const GITHUB_URL = "https://github.com/openenlarge/openenlarge";

export const credits: { group: string; items: Credit[] }[] = [
  {
    group: "about.credits.group.applicationUi",
    items: [
      { name: "Svelte", license: "MIT", url: "https://svelte.dev" },
      { name: "SvelteKit", license: "MIT", url: "https://kit.svelte.dev" },
      { name: "Vite", license: "MIT", url: "https://vitejs.dev" },
      { name: "TypeScript", license: "Apache-2.0", url: "https://www.typescriptlang.org" },
      { name: "Tauri", license: "MIT / Apache-2.0", url: "https://tauri.app" },
    ],
  },
  {
    group: "about.credits.group.imagePipeline",
    items: [
      { name: "image", license: "MIT", url: "https://github.com/image-rs/image" },
      { name: "rawler", license: "LGPL-2.1", url: "https://github.com/dnglab/dnglab" },
      { name: "tiff", license: "MIT", url: "https://github.com/image-rs/image-tiff" },
      { name: "nalgebra", license: "Apache-2.0", url: "https://nalgebra.org" },
      { name: "ndarray", license: "MIT / Apache-2.0", url: "https://github.com/rust-ndarray/ndarray" },
      { name: "inpaint", license: "MIT", url: "https://crates.io/crates/inpaint" },
      { name: "rayon", license: "MIT / Apache-2.0", url: "https://github.com/rayon-rs/rayon" },
      { name: "little_exif", license: "MIT", url: "https://crates.io/crates/little_exif" },
    ],
  },
  {
    group: "about.credits.group.storageSystem",
    items: [
      { name: "SQLite (via rusqlite)", license: "Public Domain / MIT", url: "https://www.sqlite.org" },
      { name: "Zstandard (via zstd)", license: "BSD-3-Clause", url: "https://github.com/facebook/zstd" },
      { name: "serde", license: "MIT / Apache-2.0", url: "https://serde.rs" },
      { name: "base64", license: "MIT / Apache-2.0", url: "https://github.com/marshallpierce/rust-base64" },
      { name: "uuid", license: "MIT / Apache-2.0", url: "https://github.com/uuid-rs/uuid" },
      { name: "trash", license: "MIT", url: "https://github.com/Byron/trash-rs" },
      { name: "thiserror", license: "MIT / Apache-2.0", url: "https://github.com/dtolnay/thiserror" },
    ],
  },
];
