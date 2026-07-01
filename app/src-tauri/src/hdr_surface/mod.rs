//! Native HDR/EDR compositing surface.
//!
//! A platform-native extended-dynamic-range (EDR) layer composited *behind* the
//! Tauri webview so that pixels brighter than SDR white (1.0) can actually glow
//! on an HDR display, instead of being clamped by the WKWebView's SDR canvas.
//! The frontend punches a transparent hole in the DOM over the image viewport;
//! this surface shows through it.
//!
//! Driven on-demand by three Tauri commands (`hdr_surface_show` / `_set_rect` /
//! `_hide`). Only macOS is implemented for now (CAMetalLayer + reference-EDR);
//! other platforms get the commands as no-ops (gain-map fallback lives
//! elsewhere). The surface handle lives in managed [`HdrSurfaceState`].

#[cfg(target_os = "macos")]
pub mod macos;
pub(crate) mod msl;
pub(crate) mod uniforms;

/// Raw HDR pixel buffer for the native EDR compositing surface: row-major RGBA
/// half-float (linear extended sRGB, BT.709 primaries), 4 channels per
/// pixel. Highlights brighter than SDR white (1.0) are preserved (not
/// clamped) so a real EDR display can render them brighter than the
/// webview's SDR canvas.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HdrBuffer {
    pub width: u32,
    pub height: u32,
    pub rgba16f: Vec<u16>,
}

/// Placement of the surface, supplied by the frontend: the SDR canvas's
/// bounding box in CSS pixels (top-left origin, DOM coordinates) plus the
/// device-pixel-ratio for backing resolution.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct ViewportRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dpr: f64,
}

/// Tauri managed state holding the lazily-created native surface handle.
///
/// On macOS the surface lives behind an `Arc<Mutex<Option<..>>>` whose contents
/// are only ever dereferenced on the main thread (see `macos::Surface`). On
/// other platforms this is an empty marker and the commands are no-ops.
#[derive(Default)]
pub struct HdrSurfaceState {
    #[cfg(target_os = "macos")]
    surface: macos::SurfaceSlot,
}

/// Upload an `rgba16f` buffer to the native EDR surface and show it at `rect`,
/// on the main thread. Shared by the `hdr_surface_show` command and the fused
/// `commands::hdr_surface_render_show` (which renders the buffer in Rust), so
/// the main-thread show path lives in exactly one place. The buffer is LINEAR
/// extended sRGB (BT.709 primaries) half-float, uploaded verbatim (no
/// re-linearization); the surface is created lazily on first call.
#[cfg(target_os = "macos")]
pub(crate) fn show_buffer(
    window: &tauri::WebviewWindow,
    state: &HdrSurfaceState,
    rgba16f: Vec<u16>,
    width: u32,
    height: u32,
    rect: ViewportRect,
) -> Result<(), String> {
    let slot = state.surface.clone();
    // The closure runs on the main thread; all native work happens there.
    window
        .with_webview(move |webview| {
            macos::show_on_main(webview, slot, rgba16f, width, height, rect);
        })
        .map_err(|e| format!("with_webview failed: {e}"))
}

/// Upload the raw linear negative + invert uniforms to the native EDR surface
/// and render the per-frame invert path (invert → intermediate → TEMP
/// passthrough), on the main thread. Shared entry so `commands::
/// hdr_surface_set_source` stays thin. `source_bytes` is packed RGBA16Float
/// (8 bytes/px) at `width`×`height`; `uniforms` already has base/d_max/
/// cam_balance/aspect patched from the session.
#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn set_source(
    window: &tauri::WebviewWindow,
    state: &HdrSurfaceState,
    source_bytes: Vec<u8>,
    width: u32,
    height: u32,
    uniforms: uniforms::HdrUniforms,
    lut_bytes: Vec<u8>,
    rect: ViewportRect,
) -> Result<(), String> {
    let slot = state.surface.clone();
    window
        .with_webview(move |webview| {
            macos::set_source_on_main(
                webview, slot, source_bytes, width, height, uniforms, lut_bytes, rect,
            );
        })
        .map_err(|e| format!("with_webview failed: {e}"))
}

/// Per-frame update of the native EDR surface's uniforms + tone LUT, re-rendering
/// the EXISTING source through invert→finish on the main thread — WITHOUT
/// re-uploading the raw-negative texture. Shared entry so `commands::
/// hdr_surface_set_uniforms` stays thin. No-op if the surface isn't created yet.
#[cfg(target_os = "macos")]
pub(crate) fn set_uniforms(
    window: &tauri::WebviewWindow,
    state: &HdrSurfaceState,
    uniforms: uniforms::HdrUniforms,
    lut_bytes: Vec<u8>,
    rect: ViewportRect,
) -> Result<(), String> {
    let slot = state.surface.clone();
    window
        .with_webview(move |webview| {
            macos::set_uniforms_on_main(webview, slot, uniforms, lut_bytes, rect);
        })
        .map_err(|e| format!("with_webview failed: {e}"))
}

/// Upload an `rgba16f` buffer to the native EDR surface and show it at `rect`.
/// The buffer is LINEAR extended sRGB (BT.709 primaries) half-float (from
/// `encode_hdr_raw`); it is uploaded verbatim (no re-linearization). Creates
/// the surface lazily.
#[tauri::command]
pub fn hdr_surface_show(
    app: tauri::AppHandle,
    state: tauri::State<'_, HdrSurfaceState>,
    rgba16f: Vec<u16>,
    width: u32,
    height: u32,
    rect: ViewportRect,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "no main window".to_string())?;
        show_buffer(&window, &state, rgba16f, width, height, rect)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state, rgba16f, width, height, rect);
        Ok(())
    }
}

/// Reposition / resize the native EDR surface (pan, zoom, window resize).
#[tauri::command]
pub fn hdr_surface_set_rect(
    app: tauri::AppHandle,
    state: tauri::State<'_, HdrSurfaceState>,
    rect: ViewportRect,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let slot = state.surface.clone();
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "no main window".to_string())?;
        window
            .with_webview(move |webview| {
                macos::set_rect_on_main(webview, slot, rect);
            })
            .map_err(|e| format!("with_webview failed: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state, rect);
        Ok(())
    }
}

/// Hide the native EDR surface, revealing the SDR webview canvas.
#[tauri::command]
pub fn hdr_surface_hide(
    app: tauri::AppHandle,
    state: tauri::State<'_, HdrSurfaceState>,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        let slot = state.surface.clone();
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "no main window".to_string())?;
        window
            .with_webview(move |webview| {
                macos::hide_on_main(webview, slot);
            })
            .map_err(|e| format!("with_webview failed: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state);
        Ok(())
    }
}
