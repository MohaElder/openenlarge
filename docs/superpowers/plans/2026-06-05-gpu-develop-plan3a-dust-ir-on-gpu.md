# GPU Develop — Plan 3a: Dust/IR on the GPU Path (Phase 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep dust-eraser and IR-removal images on the fast GPU path (instant exposure/temp/tint) instead of dropping to the CPU `render_view` fallback — by having Rust bake geometry + heal the working buffer **pre-inversion** and re-upload it, so the GPU inverts+finishes the healed buffer.

**Architecture:** When dust/IR is active, the frontend requests a *baked* working texture: Rust applies geometry (orient/rotate/crop) and runs the existing Telea inpaint (`dust::apply`/`apply_ir`) on the **raw negative** (pre-invert), returning half-float RGBA. The GPU then runs its normal invert→finish two-pass with **identity geometry** (geometry already baked in). Re-bake only happens on stroke-commit / IR-toggle / geometry change — never per slider. When dust/IR is off, the existing raw-buffer upload + GPU geometry path (Plan 2) is unchanged.

**Tech Stack:** Rust (Tauri 2, film-core `dust`/`convert`, `half`), WebGL2, TypeScript, Svelte.

**Spec:** `docs/superpowers/specs/2026-06-04-gpu-develop-pipeline-design.md` (Phase 5). Follows Plan 2 (`...plan2-float-upload-shader-inversion.md`), which is on `main` and E2E-verified.

---

## Context the implementer needs

- After Plan 2, `gpuEligible = useGL && renderer && !raw && dust.length === 0 && !irRemoval.enabled` (`Viewport.svelte`). Dust/IR currently force the CPU fallback (`render()` / `cpuKey`). This plan **removes the `dust`/`irRemoval` exclusions** from `gpuEligible` and adds a "bake mode".
- `dust::apply(img, stamps)` and `dust::apply_ir(img, ir, sensitivity)` (`crates/film-core/src/dust.rs`) inpaint **in place** and are **domain-agnostic** (they just inpaint whatever pixel values are present). Today they run **post-invert**; this plan runs them **pre-invert** on the raw negative (the spec's intended re-domaining — visually equivalent for healing; preview and a future GPU export will then heal in the same domain).
- Dust strokes (`DustStroke { points: Vec<[f64;2]> /* normalized [0,1] */, r: f64 /* normalized to width */ }`) are normalized to the **post-geometry** (oriented + straightened + cropped) image — the same space `export_stamps(dust, w, h)` (`commands.rs:188`) maps into. So after baking geometry, `export_stamps` maps strokes onto the baked image directly.
- Geometry helpers (`convert.rs`): `orient(img, rot90, flip_h, flip_v)`, `rotate(img, deg)`, `crop(img, x, y, w, h)`, `crop_px(norm, w, h)` — all preserve the `ir` plane.
- `pack_rgba16f(&Image, cap) -> (u32,u32,Vec<u8>)` and `capped_dims(&Image, cap) -> (u32,u32)` and `MAX_GPU_EDGE` exist in `gpu_upload.rs` (from Plan 2).
- The renderer's `setGeometry({crop_off, crop_scale, angle, orient, raw, outW, outH})` and `setSourceFloat(Uint16Array, w, h)` exist (Plan 2). Identity geometry = `crop_off:[0,0], crop_scale:[1,1], angle:0, orient:[1,0,0,1]`.
- The `sourceUV` Y-flip fix (`uv.y = 1.0 - uv.y`) means the baked top-down texture displays right-side-up under identity geometry. Good — no change needed there.

## File Structure

```
app/src-tauri/src/
├── gpu_upload.rs   ADD: BakeSpec struct + bake_working(&Image, &BakeSpec) -> Image
└── commands.rs     ADD: working_baked_info, working_baked_pixels commands (take id + BakeSpec)
   lib.rs           register the 2 new commands

app/src/lib/
├── api.ts          ADD: workingBakedInfo(id, spec), workingBakedPixels(id, spec)
└── viewport/Viewport.svelte
                    gpuEligible drops dust/IR exclusion; uploadWorking branches on dust/IR
                    (bake mode → baked texture + identity geometry; else raw + GPU geometry);
                    upload key includes dust/geometry in bake mode
```

---

## Task 1: Rust — `BakeSpec` + `bake_working` (geometry + pre-invert heal)

**Files:**
- Modify: `app/src-tauri/src/gpu_upload.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `gpu_upload.rs`:

```rust
    use crate::commands::{DustStroke, IrRemoval};

    #[test]
    fn bake_working_applies_geometry_then_heals() {
        // 4x4 solid grey with one bright speck; a dust stroke over the speck should
        // inpaint it toward the surrounding grey (pre-invert raw domain).
        let mut pixels = vec![[0.5_f32, 0.5, 0.5]; 16];
        pixels[5] = [0.9, 0.9, 0.9]; // speck at (x=1,y=1)
        let img = Image { width: 4, height: 4, pixels, ir: None };
        let spec = BakeSpec {
            rot90: 0, flip_h: false, flip_v: false, angle: 0.0, image_crop: None,
            dust: vec![DustStroke { points: vec![[0.25, 0.25]], r: 0.5 }], // centered on the speck
            ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
        };
        let out = bake_working(&img, &spec);
        assert_eq!((out.width, out.height), (4, 4));
        // The speck should be healed toward grey, not still 0.9.
        assert!((out.pixels[5][0] - 0.5).abs() < 0.3, "speck healed: {}", out.pixels[5][0]);
    }

    #[test]
    fn bake_working_crop_changes_dims() {
        let img = Image { width: 10, height: 8, pixels: vec![[0.3, 0.3, 0.3]; 80], ir: None };
        let spec = BakeSpec {
            rot90: 0, flip_h: false, flip_v: false, angle: 0.0,
            image_crop: Some([0.0, 0.0, 0.5, 0.5]), // top-left quarter
            dust: vec![], ir_removal: IrRemoval { enabled: false, sensitivity: 0.0 },
        };
        let out = bake_working(&img, &spec);
        assert_eq!((out.width, out.height), (5, 4));
    }
```

- [ ] **Step 2: Run it to confirm failure**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests::bake_working`
Expected: FAIL to compile — `BakeSpec` and `bake_working` not found.

- [ ] **Step 3: Implement `BakeSpec` + `bake_working`**

In `gpu_upload.rs`, add (the geometry helpers and `crop_px` are in `convert.rs`/`commands.rs`; import as shown):

```rust
use crate::commands::{export_stamps, DustStroke, IrRemoval};
use crate::convert::{crop, orient, rotate};

/// Geometry + dust/IR for baking a heal-ready working buffer (raw negative).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BakeSpec {
    pub rot90: u8,
    pub flip_h: bool,
    pub flip_v: bool,
    pub angle: f32,
    pub image_crop: Option<[f64; 4]>,
    pub dust: Vec<DustStroke>,
    pub ir_removal: IrRemoval,
}

/// Apply geometry (orient → straighten → persistent crop) to the raw negative,
/// then heal dust strokes + IR defects IN THE RAW (pre-invert) DOMAIN. Returns the
/// baked raw-negative image; the GPU then inverts+finishes it with identity geometry.
pub fn bake_working(working: &Image, spec: &BakeSpec) -> Image {
    let oriented = orient(working, spec.rot90, spec.flip_h, spec.flip_v);
    let straightened = rotate(&oriented, spec.angle);
    let mut img = match spec.image_crop {
        Some(nc) => {
            let (x, y, w, h) = crate::commands::crop_px(nc, straightened.width, straightened.height);
            crop(&straightened, x, y, w, h)
        }
        None => straightened,
    };
    // Strokes are normalized to this (post-geometry) image — same space export_stamps maps into.
    let stamps = export_stamps(&spec.dust, img.width, img.height);
    crate::film_core_dust_apply(&mut img, &stamps);
    if spec.ir_removal.enabled {
        if let Some(ir) = img.ir.clone() {
            film_core::dust::apply_ir(&mut img, &ir, spec.ir_removal.sensitivity);
        }
    }
    img
}
```

Then make the needed items reachable: in `commands.rs` change `fn export_stamps` → `pub(crate) fn export_stamps`, `fn crop_px` → `pub(crate) fn crop_px`, and ensure `DustStroke`/`IrRemoval` are `pub` (they are, being command args). In `gpu_upload.rs` replace the `crate::film_core_dust_apply` placeholder with the real call — add a tiny local helper or call directly: use `film_core::dust::apply(&mut img, &stamps);` (delete the `film_core_dust_apply` line and the helper note — call `film_core::dust::apply` directly). Confirm `film_core::dust` is the public path (it is — `crates/film-core/src/dust.rs` is `pub mod dust`).

Final `bake_working` heal lines should read:

```rust
    let stamps = export_stamps(&spec.dust, img.width, img.height);
    film_core::dust::apply(&mut img, &stamps);
    if spec.ir_removal.enabled {
        if let Some(ir) = img.ir.clone() {
            film_core::dust::apply_ir(&mut img, &ir, spec.ir_removal.sensitivity);
        }
    }
    img
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --manifest-path app/src-tauri/Cargo.toml gpu_upload::tests::bake_working`
Expected: PASS (both). If the heal-tolerance assertion is too tight for Telea on a 4x4, loosen to `< 0.35` (still proves it moved off 0.9).

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri/src/gpu_upload.rs app/src-tauri/src/commands.rs
git commit -m "feat(gpu): bake_working — geometry + pre-invert dust/IR heal for GPU path"
```

---

## Task 2: Rust — `working_baked_info` + `working_baked_pixels` commands

**Files:**
- Modify: `app/src-tauri/src/commands.rs`, `app/src-tauri/src/lib.rs`

- [ ] **Step 1: Add the commands**

In `commands.rs` (mirror `working_info`/`working_pixels`, but bake first):

```rust
use crate::gpu_upload::{bake_working, BakeSpec};

/// Capped dims of the BAKED (geometry + heal) working texture.
#[tauri::command]
pub fn working_baked_info(id: String, spec: BakeSpec, session: State<Session>) -> Result<WorkingInfo, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let baked = bake_working(&dev.working, &spec);
    let (w, h) = crate::gpu_upload::capped_dims(&baked, crate::gpu_upload::MAX_GPU_EDGE);
    Ok(WorkingInfo { w, h })
}

/// Half-float RGBA bytes of the BAKED working buffer (geometry applied, dust/IR
/// healed pre-invert), for a one-shot RGBA16F upload. GPU then inverts with
/// IDENTITY geometry.
#[tauri::command]
pub fn working_baked_pixels(id: String, spec: BakeSpec, session: State<Session>) -> Result<tauri::ipc::Response, String> {
    ensure_resident(&session, &id)?;
    let images = session.images.lock().unwrap();
    let img = images.get(&id).ok_or("unknown image id")?;
    let dev = img.developed.as_ref().ok_or("not developed")?;
    let baked = bake_working(&dev.working, &spec);
    let (_, _, bytes) = crate::gpu_upload::pack_rgba16f(&baked, crate::gpu_upload::MAX_GPU_EDGE);
    Ok(tauri::ipc::Response::new(bytes))
}
```

- [ ] **Step 2: Register in `lib.rs`**

Add `working_baked_info, working_baked_pixels` to `tauri::generate_handler![...]` (same `commands::` prefix as siblings).

- [ ] **Step 3: Build**

Run: `cargo build --manifest-path app/src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add app/src-tauri/src/commands.rs app/src-tauri/src/lib.rs
git commit -m "feat(gpu): working_baked_info/working_baked_pixels commands"
```

---

## Task 3: TS — api bindings

**Files:**
- Modify: `app/src/lib/api.ts`

- [ ] **Step 1: Add a `BakeSpec` type + the two bindings**

In `api.ts` (place the type near `ViewSpec`; reuse `DustStroke`/`IrRemoval` types already defined there; note `wireDust` is used for `dust` in `renderView` — apply the same here):

```ts
export interface BakeSpec {
  rot90: number; flip_h: boolean; flip_v: boolean; angle: number;
  image_crop: [number, number, number, number] | null;
  dust: DustStroke[];
  ir_removal: IrRemoval;
}
```

Add to the api object:

```ts
  workingBakedInfo: (id: string, spec: BakeSpec) =>
    invoke<{ w: number; h: number }>("working_baked_info", { id, spec: { ...spec, dust: wireDust(spec.dust) } }),

  workingBakedPixels: (id: string, spec: BakeSpec) =>
    invoke<ArrayBuffer>("working_baked_pixels", { id, spec: { ...spec, dust: wireDust(spec.dust) } }),
```

(If `wireDust` expects a specific shape, match how `renderView` calls it.)

- [ ] **Step 2: Build**

Run: `cd app && npm run build`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add app/src/lib/api.ts
git commit -m "feat(gpu): api bindings for working_baked_info/pixels"
```

---

## Task 4: Svelte — bake-mode wiring in the viewport

**Files:**
- Modify: `app/src/lib/viewport/Viewport.svelte`

Verification is **manual E2E**; build after each edit. Read the CURRENT file first (concurrent agents edit it).

- [ ] **Step 1: Widen `gpuEligible` and add a bake-mode flag**

Change:

```ts
$: gpuEligible = !!(useGL && renderer && !raw);
$: bakeMode = dust.length > 0 || irRemoval.enabled;
```

(Removes the `dust.length === 0 && !irRemoval.enabled` exclusion. `raw` still uses the CPU path.)

- [ ] **Step 2: Branch `uploadWorking` on bake mode**

Replace `uploadWorking` so it picks the baked texture when dust/IR is active. The upload key must re-fire when strokes or geometry change in bake mode (not just on `id`):

```ts
let uploadKey = "";

function currentUploadKey(): string {
  if (bakeMode) {
    return `bake|${id}|${dustRev}|${irRemoval.enabled}|${irRemoval.sensitivity}|${imageCrop ? imageCrop.join(',') : 'full'}|${rot90}|${flipH}|${flipV}|${angle}`;
  }
  return `raw|${id}`;
}

async function uploadWorking() {
  if (!gpuEligible || !id || !renderer) return;
  const key = currentUploadKey();
  if (uploadKey === key) return;
  const k = key;
  if (bakeMode) {
    const spec = {
      rot90, flip_h: flipH, flip_v: flipV, angle,
      image_crop: imageCrop, dust, ir_removal: irRemoval,
    };
    const info = await api.workingBakedInfo(id, spec);
    const buf = await api.workingBakedPixels(id, spec);
    if (currentUploadKey() !== k) return; // stale (params changed mid-fetch)
    renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h);
    texW = info.w; texH = info.h;
  } else {
    const info = await api.workingInfo(id);
    const buf = await api.workingPixels(id);
    if (currentUploadKey() !== k) return;
    renderer.setSourceFloat(new Uint16Array(buf), info.w, info.h);
    texW = info.w; texH = info.h;
  }
  uploadKey = k;
  await refreshInversion();
  applyGeometryAndDraw();
}
```

- [ ] **Step 3: Identity geometry in bake mode**

In `applyGeometryAndDraw`, when `bakeMode` the geometry is already baked into the texture → use identity and the baked dims:

```ts
function applyGeometryAndDraw() {
  if (!gpuEligible || !renderer) return;
  if (bakeMode) {
    renderer.setGeometry({
      crop_off: [0, 0], crop_scale: [1, 1], angle: 0,
      orient: [1, 0, 0, 1], raw, outW: texW, outH: texH,
    });
    drawGL();
    return;
  }
  // ...existing raw-mode geometry body (orient/crop/straighten) unchanged...
}
```

- [ ] **Step 4: Re-trigger upload on stroke/geometry change in bake mode**

The existing `$: if (gpuEligible) { id; uploadWorking(); }` only keys on `id`. Replace its trigger so it also re-fires when the bake inputs change:

```ts
$: if (gpuEligible) { dustRev; irRemoval.enabled; irRemoval.sensitivity; imageCrop; rot90; flipH; flipV; angle; uploadWorking(); }
$: if (!gpuEligible) uploadKey = "";
```

And in the `geomKey` reactive (raw-mode GPU geometry), guard it to raw mode so it doesn't double-draw in bake mode:

```ts
$: if (gpuEligible && !bakeMode) { geomKey; applyGeometryAndDraw(); }
```

(`invKey` → `refreshInversion` stays as-is; inversion is always GPU.)

- [ ] **Step 5: Update `cpuKey` so the CPU path only handles the true fallbacks**

`gpuEligible` is now true for dust/IR, so `cpuKey` (CPU fallback) should be empty whenever `gpuEligible` — it already is (`gpuEligible ? '' : ...`). Confirm `render()` still early-returns on `gpuEligible` (from Plan 2's fix). No change needed beyond verifying. The CPU path now only runs for `raw` or no-WebGL2.

- [ ] **Step 6: Build + MANUAL E2E**

Run: `cd app && npm run build` → clean.
Then `cd app && npm run tauri dev`. On a DNG:
1. Enable the **dust eraser**, paint over a speck → it heals, and the preview stays the GPU path (drag **exposure** afterward — still instant, **no `render_view` call** in devtools).
2. Enable **IR removal** → defects heal; exposure still instant.
3. With dust on, **rotate-90 / crop** → re-bakes (brief), result correct (strokes land on the right spots).
4. Turn dust **off** → returns to the raw-buffer GPU path; image still correct.
5. Confirm healed result looks equivalent to the old CPU-fallback heal (it now heals pre-invert; should look the same).

- [ ] **Step 7: Commit**

```bash
git add app/src/lib/viewport/Viewport.svelte
git commit -m "feat(gpu): dust/IR stay on GPU path via baked working texture"
```

---

## Task 5: Parity verification + record

**Files:** `docs/superpowers/poc-findings.md`

- [ ] **Step 1: Walk the dust/IR parity checks** (Task 4 Step 6 list) in the running app; note PASS/FAIL. Especially confirm the pre-invert heal looks equivalent to the prior post-invert CPU heal.

- [ ] **Step 2: Append a dated `Plan 3a results` section** to `poc-findings.md` recording: dust/IR now on the GPU path via baked pre-invert heal; exposure/etc instant with dust on; heal-domain change (post→pre invert) verified visually equivalent; any caveats.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/poc-findings.md
git commit -m "docs(gpu): record Plan 3a dust/IR-on-GPU results"
```

---

## Self-Review

- **Spec coverage:** Phase 5 (dust/IR re-domaining onto the GPU path) → Task 1 (`bake_working`: geometry + pre-invert heal), Tasks 2–3 (commands + api), Task 4 (bake-mode wiring), Task 5 (verify). The CPU `dust`/`apply_ir` are reused (no inpaint port), exactly as the spec requires. Re-bake only on commit/geometry change (not per slider).
- **Placeholder scan:** No TBD/TODO. The one placeholder (`crate::film_core_dust_apply`) is explicitly replaced in Task 1 Step 3 with the real `film_core::dust::apply` call + the final code block. Manual-E2E steps are inherent to GPU work and list expected results.
- **Type consistency:** `BakeSpec` fields identical across Rust (`gpu_upload.rs`), api (`api.ts`), and the command signatures; `WorkingInfo`/`{w,h}` reused from Plan 2; `setSourceFloat`/`setGeometry`/`refreshInversion`/`applyGeometryAndDraw` names match Plan 2. `export_stamps`/`crop_px` widened to `pub(crate)` and used consistently.
- **Risk noted:** Telea heal moves from post-invert to pre-invert (raw negative, log domain) — visually equivalent for fill/clone, verified in Task 4 Step 6.5 / Task 5. Re-bake on geometry change in bake mode is an extra upload, but dust+geometry edits are infrequent.
```