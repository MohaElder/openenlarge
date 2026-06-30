//! macOS EDR (extended dynamic range) compositing surface — production version.
//!
//! A native `CAMetalLayer` (RGBA16Float, extended-linear Display-P3, reference
//! EDR) hosted by an `NSView` that sits *behind* the WKWebView. The frontend
//! punches a transparent hole in the DOM over the image viewport; this layer
//! shows through it and can draw pixels brighter than SDR white (1.0) that a
//! real HDR display renders as true highlights.
//!
//! Lifecycle is on-demand, driven by three Tauri commands (in the parent
//! module): `hdr_surface_show` uploads an `rgba16f` buffer + positions the
//! surface, `hdr_surface_set_rect` repositions on pan/zoom/resize, and
//! `hdr_surface_hide` hides it (revealing the SDR canvas). The surface is
//! created lazily on the first `show`.
//!
//! ## Thread safety
//! `NSView`/`CAMetalLayer`/`MTLDevice`/… are main-thread-only and not
//! `Send`/`Sync`, but Tauri managed state must be `Send + Sync` and command
//! handlers can run off the main thread. The [`Surface`] handle therefore
//! carries `unsafe impl Send + Sync` whose invariant is "only ever dereferenced
//! on the main thread" — and every entry point here is reached via
//! `WebviewWindow::with_webview`, whose closure Tauri runs on the main thread.
//! Nothing in here touches a Cocoa/Metal object off the main thread.

#![allow(unexpected_cfgs)]

use std::sync::{Arc, Mutex};

use core::ffi::c_void;
use core::ptr::NonNull;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, MainThreadOnly};

use objc2_app_kit::{NSView, NSWindow, NSWindowOrderingMode};
use objc2_core_foundation::CGSize;
use objc2_core_graphics::{kCGColorSpaceExtendedLinearDisplayP3, CGColorSpace};
use objc2_foundation::{MainThreadMarker, NSNumber, NSPoint, NSRect, NSSize, NSString};
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLLibrary, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLSize, MTLStorageMode, MTLStoreAction,
    MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

use super::ViewportRect;

/// Metal Shading Language: a textured fullscreen triangle (no vertex buffers).
/// The fragment stage samples the uploaded image (linear extended-Display-P3,
/// already linearized) with linear filtering so it scales to fill the layer.
/// Alpha is forced to 1.0 so the surface stays opaque behind the punched hole.
const EDR_SHADER_SRC: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct VOut {
    float4 position [[position]];
    float2 uv;
};

vertex VOut edr_vertex(uint vid [[vertex_id]]) {
    float2 uv = float2((vid << 1) & 2, vid & 2);   // (0,0) (2,0) (0,2)
    VOut out;
    out.position = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;                                   // visible region uv in [0,1]
    return out;
}

fragment float4 edr_fragment(VOut in [[stage_in]], texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(filter::linear, address::clamp_to_edge);
    return float4(tex.sample(s, in.uv).rgb, 1.0);
}
"#;

define_class!(
    // SAFETY:
    // - `NSView` is the superclass; we add no instance variables and do not
    //   implement `Drop`, so its subclassing requirements are upheld.
    #[unsafe(super(NSView))]
    #[name = "OEHdrSurfaceView"]
    struct HdrSurfaceView;

    impl HdrSurfaceView {
        // Return nil from hit-testing so all clicks/scroll fall through to the
        // webview in front — the surface never steals input.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

impl HdrSurfaceView {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this: Allocated<Self> = Self::alloc(mtm);
        unsafe { msg_send![this, initWithFrame: frame] }
    }
}

/// Native surface handle stored in Tauri managed state.
///
/// SAFETY (the `unsafe impl`s below): every field is a main-thread-only Cocoa /
/// Metal object. It is sound to move this between threads ONLY because it is
/// never *dereferenced* off the main thread — all access goes through the
/// `*_on_main` functions, which run inside `WebviewWindow::with_webview`'s
/// main-thread closure.
pub struct Surface {
    view: Retained<HdrSurfaceView>,
    layer: Retained<CAMetalLayer>,
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    texture: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
}

// SAFETY: see the `Surface` doc comment — only ever dereferenced on the main thread.
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

/// Shared, lockable slot for the lazily-created surface.
pub type SurfaceSlot = Arc<Mutex<Option<Surface>>>;

// ---------------------------------------------------------------------------
// Command entry points (each runs inside a main-thread `with_webview` closure)
// ---------------------------------------------------------------------------

/// Lazily create the surface (if needed), upload the buffer, position + show.
pub fn show_on_main(
    webview: tauri::webview::PlatformWebview,
    slot: SurfaceSlot,
    rgba16f: Vec<u16>,
    width: u32,
    height: u32,
    rect: ViewportRect,
) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("[hdr] show closure not on main thread; ignoring");
        return;
    };
    let ns_window = webview.ns_window() as *mut NSWindow;
    let wk_webview = webview.inner() as *mut AnyObject;
    if ns_window.is_null() || wk_webview.is_null() {
        eprintln!("[hdr] null NSWindow/WKWebView handle; ignoring show");
        return;
    }
    // SAFETY: valid for the window's lifetime; only used on the main thread.
    let window: &NSWindow = unsafe { &*ns_window };
    let wk_webview: &AnyObject = unsafe { &*wk_webview };

    let mut guard = slot.lock().unwrap();
    if guard.is_none() {
        match create_surface(mtm, window, wk_webview) {
            Ok(s) => *guard = Some(s),
            Err(e) => {
                eprintln!("[hdr] create_surface failed: {e}");
                return;
            }
        }
    }
    let surface = guard.as_mut().unwrap();
    if let Err(e) = upload_texture(surface, &rgba16f, width, height) {
        eprintln!("[hdr] texture upload failed: {e}");
        return;
    }
    surface.view.setHidden(false);
    position(window, surface, rect);
    let _ = render(surface);
}

/// Reposition / resize the surface (pan, zoom, window resize). No-op if not yet shown.
pub fn set_rect_on_main(webview: tauri::webview::PlatformWebview, slot: SurfaceSlot, rect: ViewportRect) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let ns_window = webview.ns_window() as *mut NSWindow;
    if ns_window.is_null() {
        return;
    }
    // SAFETY: valid for the window's lifetime; only used on the main thread.
    let window: &NSWindow = unsafe { &*ns_window };
    let guard = slot.lock().unwrap();
    if let Some(surface) = guard.as_ref() {
        position(window, surface, rect);
        let _ = render(surface);
    }
}

/// Hide the surface, revealing the SDR webview canvas. No-op if not yet shown.
pub fn hide_on_main(_webview: tauri::webview::PlatformWebview, slot: SurfaceSlot) {
    if MainThreadMarker::new().is_none() {
        return;
    }
    let guard = slot.lock().unwrap();
    if let Some(surface) = guard.as_ref() {
        surface.view.setHidden(true);
    }
}

// ---------------------------------------------------------------------------
// Native helpers (main thread only)
// ---------------------------------------------------------------------------

fn create_surface(
    mtm: MainThreadMarker,
    window: &NSWindow,
    wk_webview: &AnyObject,
) -> Result<Surface, String> {
    let content_view = window
        .contentView()
        .ok_or_else(|| "NSWindow has no contentView".to_string())?;

    let device: Retained<ProtocolObject<dyn MTLDevice>> =
        MTLCreateSystemDefaultDevice().ok_or_else(|| "no system default Metal device".to_string())?;

    // Textured render pipeline.
    let source = NSString::from_str(EDR_SHADER_SRC);
    let library = device
        .newLibraryWithSource_options_error(&source, None)
        .map_err(|e| format!("shader compile failed: {e:?}"))?;
    let vertex_fn = library
        .newFunctionWithName(&NSString::from_str("edr_vertex"))
        .ok_or_else(|| "missing edr_vertex".to_string())?;
    let fragment_fn = library
        .newFunctionWithName(&NSString::from_str("edr_fragment"))
        .ok_or_else(|| "missing edr_fragment".to_string())?;

    let pipeline_desc = MTLRenderPipelineDescriptor::new();
    pipeline_desc.setVertexFunction(Some(&vertex_fn));
    pipeline_desc.setFragmentFunction(Some(&fragment_fn));
    // SAFETY: index 0 is the single color attachment we render to.
    let color_attach = unsafe { pipeline_desc.colorAttachments().objectAtIndexedSubscript(0) };
    color_attach.setPixelFormat(MTLPixelFormat::RGBA16Float);

    let pipeline = device
        .newRenderPipelineStateWithDescriptor_error(&pipeline_desc)
        .map_err(|e| format!("pipeline creation failed: {e:?}"))?;
    let queue = device
        .newCommandQueue()
        .ok_or_else(|| "could not create command queue".to_string())?;

    // CAMetalLayer configured for reference EDR.
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::RGBA16Float);
    layer.setFramebufferOnly(true);
    layer.setWantsExtendedDynamicRangeContent(true);
    // edrMetadata left nil == reference EDR.
    // SAFETY: `kCGColorSpaceExtendedLinearDisplayP3` is a framework constant.
    let cs_name = unsafe { kCGColorSpaceExtendedLinearDisplayP3 };
    if let Some(cs) = CGColorSpace::with_name(Some(cs_name)) {
        layer.setColorspace(Some(&cs));
    } else {
        eprintln!("[hdr] could not create extended-linear Display-P3 colorspace");
    }

    // Layer-hosting view, positioned later by `show`/`set_rect`.
    let zero = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
    let view = HdrSurfaceView::new(mtm, zero);
    // Order matters: set the backing layer before enabling wantsLayer so the
    // view becomes layer-hosting with our CAMetalLayer as its backing store.
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    view.setHidden(true);

    // Insert directly BELOW the webview so the DOM composites in front and the
    // surface shows only through the transparent viewport hole.
    // SAFETY: WKWebView inherits NSView.
    let webview_view: &NSView = unsafe { &*(wk_webview as *const AnyObject as *const NSView) };
    content_view.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(webview_view));
    layer.setZPosition(-1.0);

    // Make the webview transparent so the layer is visible through the hole.
    // WKWebView has no public `setDrawsBackground:`; KVC on the private
    // `drawsBackground` property is the supported path (also used by wry).
    unsafe {
        let no = NSNumber::numberWithBool(false);
        let key = NSString::from_str("drawsBackground");
        let _: () = msg_send![wk_webview, setValue: &*no, forKey: &*key];
    }

    Ok(Surface {
        view,
        layer,
        device,
        queue,
        pipeline,
        texture: None,
    })
}

/// Upload the `rgba16f` buffer into a fresh RGBA16Float texture on the surface.
/// The buffer is LINEAR extended-Display-P3 half-float — uploaded verbatim, no
/// re-linearization.
fn upload_texture(
    surface: &mut Surface,
    rgba16f: &[u16],
    width: u32,
    height: u32,
) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("zero texture dimensions".to_string());
    }
    let expected = (width as usize) * (height as usize) * 4;
    if rgba16f.len() < expected {
        return Err(format!(
            "rgba16f buffer too small: {} < {} ({}x{}x4)",
            rgba16f.len(),
            expected,
            width,
            height
        ));
    }

    // SAFETY: standard 2D texture descriptor; args are plain values.
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::RGBA16Float,
            width as usize,
            height as usize,
            false,
        )
    };
    desc.setUsage(MTLTextureUsage::ShaderRead);
    desc.setStorageMode(MTLStorageMode::Shared);

    let texture = surface
        .device
        .newTextureWithDescriptor(&desc)
        .ok_or_else(|| "texture allocation failed".to_string())?;

    let region = MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width: width as usize,
            height: height as usize,
            depth: 1,
        },
    };
    let bytes_per_row = (width as usize) * 4 * 2; // 4 channels * 2 bytes (f16)
    // SAFETY: `rgba16f` is at least `expected` u16s long (checked above), so the
    // pointer + region + bytesPerRow describe an in-bounds copy. The texture is
    // shared storage, so the GPU sees the data after this synchronous copy.
    unsafe {
        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
            region,
            0,
            NonNull::new(rgba16f.as_ptr() as *mut c_void).unwrap(),
            bytes_per_row,
        );
    }

    surface.texture = Some(texture);
    Ok(())
}

/// Position the view at `rect`, flipping Y from DOM (top-left) to AppKit
/// (bottom-left) coordinates, and size the layer's drawable for the dpr.
fn position(window: &NSWindow, surface: &Surface, rect: ViewportRect) {
    let content_h = window
        .contentView()
        .map(|v| v.frame().size.height)
        .unwrap_or(0.0);
    // DOM rect is top-left origin in CSS points; AppKit frames are bottom-left.
    let y_flipped = content_h - rect.y - rect.h;
    let frame = NSRect::new(NSPoint::new(rect.x, y_flipped), NSSize::new(rect.w, rect.h));
    surface.view.setFrame(frame);

    let dpr = rect.dpr.max(1.0);
    surface.layer.setContentsScale(dpr);
    // Layer frame is in the view's own coordinate space (origin at 0,0).
    surface
        .layer
        .setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(rect.w, rect.h)));
    let dw = (rect.w * dpr).max(1.0);
    let dh = (rect.h * dpr).max(1.0);
    surface.layer.setDrawableSize(CGSize::new(dw, dh));
}

/// Draw the uploaded texture into the layer's next drawable (scaled to fill).
/// A nil drawable or missing texture is a no-op (no panic).
fn render(surface: &Surface) -> Result<(), String> {
    let Some(texture) = surface.texture.as_ref() else {
        return Ok(());
    };
    let Some(drawable) = surface.layer.nextDrawable() else {
        return Ok(());
    };
    let out = drawable.texture();

    let pass = MTLRenderPassDescriptor::new();
    // SAFETY: single color attachment at index 0.
    let attach = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
    attach.setTexture(Some(&out));
    attach.setLoadAction(MTLLoadAction::Clear);
    attach.setStoreAction(MTLStoreAction::Store);
    attach.setClearColor(MTLClearColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    });

    let cmd = surface
        .queue
        .commandBuffer()
        .ok_or_else(|| "no command buffer".to_string())?;
    let encoder = cmd
        .renderCommandEncoderWithDescriptor(&pass)
        .ok_or_else(|| "no render command encoder".to_string())?;
    encoder.setRenderPipelineState(&surface.pipeline);
    // SAFETY: the shader declares texture(0); we bind the uploaded image there.
    unsafe { encoder.setFragmentTexture_atIndex(Some(texture), 0) };
    // SAFETY: 3 vertices generated procedurally from vertex_id; no buffers bound.
    unsafe { encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3) };
    encoder.endEncoding();

    // Present (upcast CAMetalDrawable -> MTLDrawable).
    let mtl_drawable = ProtocolObject::<dyn MTLDrawable>::from_ref(&*drawable);
    cmd.presentDrawable(mtl_drawable);
    cmd.commit();
    Ok(())
}
