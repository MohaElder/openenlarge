pub mod autocrop;
pub mod bench;
pub mod calibrate;
pub mod chart;
pub mod classify;
pub mod color;
pub mod colorchecker;
pub mod curve;
pub mod decode;
pub mod dust;
pub mod engine;
pub mod export;
pub mod finish;
pub mod image;
pub mod tone;
pub mod wb;

pub use engine::ToneMode;
pub use engine::WbMode;
pub use image::Image;

/// Render-engine version: bump whenever the inversion/finish math that bakes a
/// cached display thumbnail changes (e.g. the filmic display curve). The catalog
/// stamps the version it last rendered thumbnails with; on load, a mismatch
/// triggers a background regeneration so the contact-sheet / grid / filmstrip
/// thumbnails stop showing the old look. See `commands.rs::load_catalog`.
///
/// History: 1 = filmic display S-curve + auto-WB-seeded develop-time thumbnails
/// (2026-06-20), replacing the pre-filmic paper encode + neutral-WB thumbnails.
/// 2 = Faithful core + clean-punchy look becomes the sole develop path (Filmic
/// retired); all prior thumbnails are stale and regenerate on app entry (2026-06-21).
/// 3 = Faithful exposure made photographic (FAITHFUL_EXPO_K=1.0) so auto-exposure can
/// actually land; stale exposures are reset by the catalog migration and the grid
/// re-solves auto-exposure on entry (2026-06-21).
/// 4 = Faithful exposure changed to a LINEAR-LIGHT gain (×2^EV on the reconstructed scene
/// `L = 10^d − 1`, applied before the contrast curve — "expose the log-inverted negative like a
/// TIFF") in place of the old density-multiply that scaled contrast. The gamma_shoulder + look_s
/// core is unchanged and EV 0 is identical; only edits with exposure ≠ 0 re-render (2026-06-22).
/// 5 = Headroom tone recovery: the Faithful shoulder + look layer move from invert_d
/// to the end of finish::tone_curve, so the tone tools recover clipped highlight/shadow
/// detail on the super-white body. EV-0/slider-0 is identical except gain-mode highlights
/// above the knee (WB now precedes the rolloff); those thumbnails regenerate on entry (2026-06-25).
/// 6 = Camera color matrix is now always applied during RAW decode (the optional toggle was
/// removed). Existing libraries were mostly decoded without it, so every thumbnail regenerates
/// to pick up the matrix-corrected decode — paired with an .oecache version bump that forces a
/// re-decode rather than rehydrating the old matrix-less sidecar (2026-06-30).
/// 7 = 0 EV baseline anchored to base+0.6D → mid-gray (FAITHFUL_BASELINE_EV, upstream #41).
/// Stored exposures are shifted by the catalog v6 migration so edited frames re-render
/// visually identical; thumbnails regenerate anyway so frames baked WITHOUT stored edits
/// (auto-seed at exposure 0) pick up the anchored default look (2026-07-05).
pub const ENGINE_VERSION: u32 = 7;
