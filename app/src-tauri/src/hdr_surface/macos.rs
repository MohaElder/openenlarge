//! macOS EDR (extended dynamic range) compositing spike.
//!
//! Goal of this spike: prove that an embedded `WKWebView` inside a Tauri window
//! still grants EDR to a *sibling* `CAMetalLayer` composited behind it. If the
//! synthetic gradient's bright end (linear ~4.0) glows brighter than the
//! webview's SDR white (1.0) on a real HDR display, EDR is granted and we can
//! build the real "live HDR" viewport on this foundation. If it does NOT, EDR
//! is not reachable through an embedded webview and we fall back to gain-maps.
//!
//! What it does, concretely:
//!   1. Reaches the native `NSWindow` + `WKWebView` via Tauri's
//!      `WebviewWindow::with_webview` (the closure runs on the main thread).
//!   2. Inserts a custom `NSView` (layer-hosted by a `CAMetalLayer`) as the
//!      *bottom-most* sibling of the webview inside the window's content view.
//!   3. Configures the layer for reference-EDR: `RGBA16Float`, an
//!      extended-linear Display-P3 colorspace, and
//!      `wantsExtendedDynamicRangeContent = true` (EDR metadata left nil =
//!      reference EDR).
//!   4. Makes the webview transparent (`drawsBackground = false`) so the layer
//!      shows through wherever the DOM is transparent.
//!   5. Overrides `hitTest:` to return nil so clicks/scroll fall through to the
//!      webview (the native view never steals input).
//!   6. Continuously renders a STATIC horizontal gradient ramp (0.0 -> 4.0
//!      linear) with a minimal Metal pipeline (fullscreen triangle, no vertex
//!      buffers), re-armed each tick on the main run loop.
//!
//! The gradient is redrawn continuously (~60Hz, via a `performSelector` re-arm
//! on the main run loop) so it stays visible for evaluation instead of only
//! flashing once before the DOM composites over it. It is still a STATIC test
//! pattern — no real image data is wired here; that is downstream work.

#![allow(unexpected_cfgs)]

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::rc::Allocated;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};

use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow, NSWindowOrderingMode};
use objc2_core_foundation::CGSize;
use objc2_core_graphics::{kCGColorSpaceExtendedLinearDisplayP3, CGColorSpace};
use objc2_foundation::{MainThreadMarker, NSNumber, NSPoint, NSRect, NSString};
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLLibrary, MTLLoadAction,
    MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLStoreAction,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};

/// Minimal Metal Shading Language for the static EDR test pattern.
///
/// The vertex stage emits a fullscreen triangle from `vertex_id` alone (no
/// vertex buffer). The fragment stage writes a neutral horizontal ramp whose
/// value runs from 0.0 on the left to ~4.0 on the right, in the layer's
/// extended-linear Display-P3 space — so the right edge is 4x SDR white and
/// should visibly glow on an EDR display.
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
    out.uv = uv;
    return out;
}

fragment float4 edr_fragment(VOut in [[stage_in]]) {
    float v = clamp(in.uv.x, 0.0, 1.0) * 4.0;       // 0 (black) .. 4.0 (super-white)
    return float4(v, v, v, 1.0);
}
"#;

/// Render context stored on the view so the repeating `redrawEDR` tick can
/// re-encode the gradient without re-resolving the device/pipeline each frame.
struct EdrIvars {
    layer: Retained<CAMetalLayer>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

define_class!(
    // SAFETY:
    // - `NSView` is the superclass; it (and the thread-kind MainThreadOnly it
    //   carries) impose no extra subclassing requirements we violate here.
    // - This class does not implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "OEEdrPassthroughView"]
    #[ivars = EdrIvars]
    struct EdrPassthroughView;

    impl EdrPassthroughView {
        // Return nil from hit-testing so every click / scroll over this view
        // falls through to whatever sibling sits in front of it (the webview).
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }

        // Repeating redraw tick. Re-encodes the same static 0..4 gradient into
        // the next drawable, then re-arms itself (~60Hz) on the main run loop so
        // the layer stays continuously presented behind the webview — otherwise
        // a single one-shot draw only flashes before the DOM composites over it.
        #[unsafe(method(redrawEDR))]
        fn redraw_edr(&self) {
            let iv = self.ivars();
            // A nil drawable (e.g. layer momentarily not displayable) just skips
            // this tick; we never panic and always re-arm below.
            let _ = render_gradient(&iv.layer, &iv.pipeline, &iv.queue);
            // Re-arm on the main run loop. performSelector:withObject:afterDelay:
            // retains `self` until it fires, keeping the view alive across ticks.
            unsafe {
                let _: () = msg_send![
                    self,
                    performSelector: sel!(redrawEDR),
                    withObject: std::ptr::null::<AnyObject>(),
                    afterDelay: 1.0 / 60.0,
                ];
            }
        }
    }
);

impl EdrPassthroughView {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        layer: Retained<CAMetalLayer>,
        pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
        queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    ) -> Retained<Self> {
        let this: Allocated<Self> = Self::alloc(mtm);
        let this = this.set_ivars(EdrIvars {
            layer,
            pipeline,
            queue,
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Kick off the continuous redraw loop on the next main-run-loop turn.
    fn start_render_loop(&self) {
        unsafe {
            let _: () = msg_send![
                self,
                performSelector: sel!(redrawEDR),
                withObject: std::ptr::null::<AnyObject>(),
                afterDelay: 0.0,
            ];
        }
    }
}

/// Attach the EDR spike layer behind `window`'s webview.
///
/// Returns `Ok(())` once the native work has been *scheduled* on the main
/// thread (Tauri runs the `with_webview` closure there). Internal native
/// failures (e.g. no Metal device, shader compile error) are logged to stderr;
/// they do not abort app startup — this is an exploratory spike.
pub fn attach_edr_spike(window: &tauri::WebviewWindow) -> Result<(), String> {
    window
        .with_webview(|webview| {
            // The closure is documented to run on the main thread.
            let Some(mtm) = MainThreadMarker::new() else {
                eprintln!("[hdr] with_webview closure not on main thread; skipping EDR spike");
                return;
            };

            // SAFETY: On macOS, `ns_window()` returns the `NSWindow*` that hosts
            // the WKWebView, and `inner()` returns the `WKWebView*`. Both are
            // valid for the lifetime of the window.
            let ns_window = webview.ns_window() as *mut NSWindow;
            let wk_webview = webview.inner() as *mut AnyObject;
            if ns_window.is_null() || wk_webview.is_null() {
                eprintln!("[hdr] null NSWindow/WKWebView handle; skipping EDR spike");
                return;
            }
            let window: &NSWindow = unsafe { &*ns_window };
            let wk_webview: &AnyObject = unsafe { &*wk_webview };

            if let Err(e) = build_and_attach(mtm, window, wk_webview) {
                eprintln!("[hdr] EDR spike attach failed: {e}");
            } else {
                eprintln!("[hdr] EDR spike attached (static 0..4 linear gradient behind webview)");
            }
        })
        .map_err(|e| format!("with_webview failed: {e}"))
}

fn build_and_attach(
    mtm: MainThreadMarker,
    window: &NSWindow,
    wk_webview: &AnyObject,
) -> Result<(), String> {
    let content_view = window
        .contentView()
        .ok_or_else(|| "NSWindow has no contentView".to_string())?;
    let bounds: NSRect = content_view.frame();
    let scale = window.backingScaleFactor();

    // --- Metal device + pipeline -----------------------------------------
    let device: Retained<ProtocolObject<dyn MTLDevice>> =
        MTLCreateSystemDefaultDevice().ok_or_else(|| "no system default Metal device".to_string())?;

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

    // --- CAMetalLayer configured for reference EDR -----------------------
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::RGBA16Float);
    layer.setFramebufferOnly(true);
    layer.setWantsExtendedDynamicRangeContent(true);
    // edrMetadata intentionally left nil == reference EDR (HDR10/HLG metadata
    // is a later tuning knob, out of scope for this spike).

    // Extended-linear Display-P3: values > 1.0 carry through as true EDR.
    // SAFETY: `kCGColorSpaceExtendedLinearDisplayP3` is a framework constant.
    let cs_name = unsafe { kCGColorSpaceExtendedLinearDisplayP3 };
    if let Some(cs) = CGColorSpace::with_name(Some(cs_name)) {
        layer.setColorspace(Some(&cs));
    } else {
        eprintln!("[hdr] could not create extended-linear Display-P3 colorspace");
    }

    let drawable_w = (bounds.size.width * scale).max(1.0);
    let drawable_h = (bounds.size.height * scale).max(1.0);
    layer.setDrawableSize(CGSize::new(drawable_w, drawable_h));
    layer.setFrame(bounds);
    layer.setContentsScale(scale);

    // --- Layer-hosting NSView, inserted behind the webview ---------------
    // The view owns the render context (layer/pipeline/queue) so its repeating
    // redraw tick can re-encode the gradient every frame.
    let view = EdrPassthroughView::new(mtm, bounds, layer.clone(), pipeline, queue);
    // Order matters: set the backing layer *before* enabling wantsLayer so the
    // view becomes "layer-hosting" with our CAMetalLayer as its backing store.
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );

    // Insert as the bottom-most sibling so the webview composites in front.
    content_view.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
    // Belt-and-suspenders: also push the layer back in z.
    layer.setZPosition(-1.0);

    // --- Make the webview transparent so the layer shows through ----------
    // WKWebView has no public `setDrawsBackground:`; the supported way (also
    // used by wry) is KVC on the private `drawsBackground` property.
    unsafe {
        let no = NSNumber::numberWithBool(false);
        let key = NSString::from_str("drawsBackground");
        let _: () = msg_send![wk_webview, setValue: &*no, forKey: &*key];
    }

    // --- Start the continuous redraw loop --------------------------------
    // A repeating ~60Hz tick (driven by performSelector re-arm on the main run
    // loop) keeps the gradient presented; a single one-shot draw would only
    // flash before the DOM composites over it. The view is retained by the
    // AppKit hierarchy AND by the pending performSelector, and the layer is its
    // backing store — so dropping the local handles here is safe.
    view.start_render_loop();

    let _ = view;
    let _ = layer;
    Ok(())
}

fn render_gradient(
    layer: &CAMetalLayer,
    pipeline: &ProtocolObject<dyn objc2_metal::MTLRenderPipelineState>,
    queue: &ProtocolObject<dyn MTLCommandQueue>,
) -> Result<(), String> {
    let drawable = layer
        .nextDrawable()
        .ok_or_else(|| "CAMetalLayer vended no drawable".to_string())?;
    let texture = drawable.texture();

    let pass = MTLRenderPassDescriptor::new();
    // SAFETY: single color attachment at index 0.
    let attach = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
    attach.setTexture(Some(&texture));
    attach.setLoadAction(MTLLoadAction::Clear);
    attach.setStoreAction(MTLStoreAction::Store);
    attach.setClearColor(MTLClearColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    });

    let cmd = queue
        .commandBuffer()
        .ok_or_else(|| "no command buffer".to_string())?;
    let encoder = cmd
        .renderCommandEncoderWithDescriptor(&pass)
        .ok_or_else(|| "no render command encoder".to_string())?;
    encoder.setRenderPipelineState(pipeline);
    // SAFETY: 3 vertices generated procedurally from vertex_id; no buffers bound.
    unsafe { encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3) };
    encoder.endEncoding();

    // Present the drawable (upcast CAMetalDrawable -> MTLDrawable).
    let mtl_drawable = ProtocolObject::<dyn MTLDrawable>::from_ref(&*drawable);
    cmd.presentDrawable(mtl_drawable);
    cmd.commit();
    Ok(())
}
