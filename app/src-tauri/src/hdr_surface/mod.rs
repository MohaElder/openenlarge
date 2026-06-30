//! Native HDR/EDR compositing surface.
//!
//! This module hosts the platform-specific "live HDR" spike: a native
//! extended-dynamic-range (EDR) layer composited *behind* the Tauri webview so
//! that pixels brighter than SDR white (1.0) can actually glow on an HDR
//! display, instead of being clamped by the WKWebView's SDR canvas.
//!
//! Only macOS is implemented for now (CAMetalLayer + reference-EDR). Other
//! platforms get a gain-map fallback elsewhere.

#[cfg(target_os = "macos")]
pub mod macos;
