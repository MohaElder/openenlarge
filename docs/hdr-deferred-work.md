# HDR — deferred work / known limitations

The macOS Live HDR effort (sub-projects A–C + HDR thumbnails) is complete: a native
CAMetalLayer EDR surface renders the full invert+finish pipeline per-frame, the finish
preserves highlight chroma into headroom, and both the gain-map export and the on-screen
thumbnails (export window, develop strip, library grid) carry HDR. The items below are
intentionally deferred.

## 1. Clip-warning + dust markers are hidden over the native EDR surface (DEFERRED)

**Symptom:** When an HDR frame is settled, the clip-overexposure warning overlay and the
dust-spot markers are not visible. They are drawn *into the WebGL canvas* (via `clip.ts`
and the dust overlay renderer), and the native Metal EDR layer composites *on top of* that
canvas when a frame is at rest — so the overlays are occluded. Crop handles are unaffected
(they are DOM elements above the surface).

**Scope:** macOS only (the native EDR surface path). Windows/Linux use the gain-map `<img>`
fallback where the WebGL overlays remain visible.

**Fix options (both moderate–hard, native-surface compositing work):**
- Port the clip-threshold test and dust-marker drawing into the Metal layer (MSL) so the
  overlays are drawn on the same surface. One surface, but duplicates the overlay logic in
  the shader.
- Float a transparent DOM overlay above the EDR layer and draw the markers there, kept
  aligned with the viewport geometry on zoom/pan (same class of alignment work as the EDR
  surface positioning). Lowest-risk since the markers are cheap 2D shapes.

**Why deferred:** it is native compositing work with real alignment/complexity cost, and the
markers are edit-time aids the user can still see in the live (pre-settle) SDR canvas.

## 2. Texture (USM) in the HDR rendition (PARTIALLY WORKED AROUND)

`finish_image_hdr` skips the spatial unsharp-mask (texture) pass, and the live MSL shader has
no spatial pass at all — so texture cannot reach the HDR rendition. Applying it to the SDR
base alone would bake inverse-texture edge halos into the gain map.

**Current handling:** the Texture slider is hidden while HDR is on (`Basic.svelte`) and
`finish_from` forces `texture = 0` whenever `hdr` is set (`commands.rs`), so no halos and the
control matches reality.

**If revisited:** the CPU path (export + thumbnails + settled preview) is the tractable half —
apply the USM to the HDR rendition too, handling super-white (>1.0) overshoot so highlights
don't ring. Bringing texture into the *live* EDR preview is a bigger job (a spatial pass in
Metal, which the per-pixel `finish_frag` shader does not currently have).

## 3. HDR contact-sheet / film-strip export file (DEFERRED)

The exported *composited* contact-sheet/film-strip JPEG stays SDR. Making the stitched file
HDR requires compositing the HDR + SDR renditions of every tile and gain-map-encoding the
composite. The on-screen strip/grid tiles are already HDR (they are individual gain-map
`<img>` thumbnails); only the stitched export file is SDR.
