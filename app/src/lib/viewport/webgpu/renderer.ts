// WebGPU twin of `app/src/lib/viewport/gl/renderer.ts`'s `FinishRenderer`:
// runs the fused invert+finish HDR/EDR shaders (`./shaders`) on a canvas
// configured for extended (HDR) tone mapping. Mirrors the WebGL renderer's
// public method names/params so `Viewport` can drive either renderer
// identically (a later task wires the choice in). Frontend-only; no GPU is
// available in this repo's test environment, so this class is NOT
// unit-tested here (see `uniforms.test.ts` for the one unit-testable piece,
// the byte packer) — it's exercised on-device in a later task.
import { INVERT_WGSL, FINISH_WGSL } from "./shaders";
import { packHdrUniforms, HDR_UNIFORMS_BYTES, type HdrUniformsInput } from "./uniforms";
import type { FinishUniforms } from "../gl/uniforms";
import type { InversionUniforms } from "../gl/invert";
import type { ClipUniforms } from "../gl/clip";
import type { ColorGradeUniforms, ColorMixUniforms, PerZoneWbUniforms } from "../../develop/finish";
import { LUT_SIZE } from "../../develop/curve";

const SURFACE_FORMAT: GPUTextureFormat = "rgba16float";

const IDENTITY_MAT3 = new Float32Array([1, 0, 0, 0, 1, 0, 0, 0, 1]);

const DEFAULT_FINISH: FinishUniforms = {
  contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
  texture: 0, vibrance: 0, saturation: 0, brightness: 0,
};
const DEFAULT_CG: ColorGradeUniforms = {
  sh_off: [0, 0, 0], sh_lum: 0,
  mid_off: [0, 0, 0], mid_lum: 0,
  hi_off: [0, 0, 0], hi_lum: 0,
  glob_off: [0, 0, 0], glob_lum: 0,
  sh_edge: 0.33, hi_edge: 0.66, softness: 0.1,
};
const DEFAULT_CM: ColorMixUniforms = {
  cm_hue: new Float32Array(8), cm_sat: new Float32Array(8), cm_lum: new Float32Array(8),
  pc_count: 0,
  pc_hue: new Float32Array(8), pc_sat: new Float32Array(8), pc_lum: new Float32Array(8),
  pc_hue_shift: new Float32Array(8), pc_sat_shift: new Float32Array(8), pc_lum_shift: new Float32Array(8),
  pc_variance: new Float32Array(8), pc_range: new Float32Array(8),
};
const DEFAULT_PZ: PerZoneWbUniforms = { enabled: 0, sh: [1, 1, 1], mid: [1, 1, 1], hi: [1, 1, 1] };
const DEFAULT_CLIP: ClipUniforms = { highOn: 0, lowOn: 0, strict: 0 };
const DEFAULT_INV: InversionUniforms = {
  base: new Float32Array([1, 1, 1]), wb: new Float32Array([1, 1, 1]),
  m_pre: IDENTITY_MAT3, m_post: IDENTITY_MAT3,
  exposure: 1, black: 0, gamma: 1, mode: 3, d_max: 1.5,
  print_exposure: 1, paper_black: 0, paper_grade: 0, soft_clip: 0.9,
  positive: false, wb_mode: 0, tone_mode: 1, hi_recovery: 0, lo_recovery: 0,
  cam_balance: new Float32Array([1, 1, 1]),
};

interface Geometry {
  crop_off: [number, number]; crop_scale: [number, number];
  angle: number; aspect: number; orient: [number, number, number, number];
  raw: boolean; outW: number; outH: number;
  view_off?: [number, number]; view_scale?: [number, number];
}
const DEFAULT_GEOM: Geometry = {
  crop_off: [0, 0], crop_scale: [1, 1], angle: 0, aspect: 1,
  orient: [1, 0, 0, 1], raw: false, outW: 1, outH: 1,
};

/** A neutral 256×1 RGBA8 ramp LUT (identity tone curve) — mirrors gl/renderer.ts's twin. */
function identityLut(): Uint8Array {
  const out = new Uint8Array(LUT_SIZE * 4);
  for (let i = 0; i < LUT_SIZE; i++) {
    const v = Math.round((i / (LUT_SIZE - 1)) * 255);
    out[i * 4] = v; out[i * 4 + 1] = v; out[i * 4 + 2] = v; out[i * 4 + 3] = 255;
  }
  return out;
}

/** Runs `INVERT_WGSL` + `FINISH_WGSL` on an in-webview WebGPU canvas configured
 *  for extended (HDR/EDR) tone mapping. Mirrors `FinishRenderer` (gl/renderer.ts)'s
 *  public API so a later task can drive either renderer through the same surface. */
export class WebGPUFinishRenderer {
  private srcTex: GPUTexture | null = null;
  private srcW = 0;
  private srcH = 0;
  private interTex: GPUTexture | null = null;
  private hasSource = false;

  private geom: Geometry = DEFAULT_GEOM;
  private finish: FinishUniforms = DEFAULT_FINISH;
  private cg: ColorGradeUniforms = DEFAULT_CG;
  private cm: ColorMixUniforms = DEFAULT_CM;
  private pz: PerZoneWbUniforms = DEFAULT_PZ;
  private clip: ClipUniforms = DEFAULT_CLIP;
  private inv: InversionUniforms = DEFAULT_INV;
  private finishMode = 0;

  private constructor(
    private canvas: HTMLCanvasElement,
    private device: GPUDevice,
    private ctx: GPUCanvasContext,
    private sampler: GPUSampler,
    private uniformBuffer: GPUBuffer,
    private invertPipeline: GPURenderPipeline,
    private finishPipeline: GPURenderPipeline,
    private lutTex: GPUTexture,
  ) {}

  /** Never throws — returns null on any WebGPU init failure (no adapter, no
   *  device, no webgpu canvas context, pipeline-compile failure, etc.), so
   *  the caller can fall back to the WebGL renderer. */
  static async create(canvas: HTMLCanvasElement): Promise<WebGPUFinishRenderer | null> {
    try {
      if (typeof navigator === "undefined" || !("gpu" in navigator) || !navigator.gpu) return null;
      const adapter = await navigator.gpu.requestAdapter();
      if (!adapter) return null;
      const device = await adapter.requestDevice();
      const ctx = canvas.getContext("webgpu") as GPUCanvasContext | null;
      if (!ctx) return null;
      ctx.configure({
        device,
        format: SURFACE_FORMAT,
        alphaMode: "opaque",
        toneMapping: { mode: "extended" },
      });

      const invertModule = device.createShaderModule({ code: INVERT_WGSL });
      const finishModule = device.createShaderModule({ code: FINISH_WGSL });
      const invertPipeline = device.createRenderPipeline({
        layout: "auto",
        vertex: { module: invertModule, entryPoint: "vs_main" },
        fragment: { module: invertModule, entryPoint: "fs_invert", targets: [{ format: SURFACE_FORMAT }] },
        primitive: { topology: "triangle-list" },
      });
      const finishPipeline = device.createRenderPipeline({
        layout: "auto",
        vertex: { module: finishModule, entryPoint: "vs_main" },
        fragment: { module: finishModule, entryPoint: "fs_finish", targets: [{ format: SURFACE_FORMAT }] },
        primitive: { topology: "triangle-list" },
      });

      const sampler = device.createSampler({
        magFilter: "linear", minFilter: "linear",
        addressModeU: "clamp-to-edge", addressModeV: "clamp-to-edge",
      });

      const uniformBuffer = device.createBuffer({
        size: HDR_UNIFORMS_BYTES,
        usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
      });

      const lutTex = device.createTexture({
        size: { width: LUT_SIZE, height: 1 },
        format: "rgba8unorm",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });

      const renderer = new WebGPUFinishRenderer(
        canvas, device, ctx, sampler, uniformBuffer, invertPipeline, finishPipeline, lutTex,
      );
      renderer.setLut(identityLut());
      return renderer;
    } catch (err) {
      console.error("WebGPUFinishRenderer.create failed:", err);
      return null;
    }
  }

  /** Upload a 256×1 RGBA8 tone LUT. */
  setLut(bytes: Uint8Array): void {
    this.device.queue.writeTexture(
      { texture: this.lutTex },
      bytes,
      { bytesPerRow: LUT_SIZE * 4, rowsPerImage: 1 },
      { width: LUT_SIZE, height: 1 },
    );
  }

  setUniforms(u: FinishUniforms): void { this.finish = u; }
  setColorGrade(cg: ColorGradeUniforms): void { this.cg = cg; }
  setColorMix(cm: ColorMixUniforms): void { this.cm = cm; }
  setPerZoneWb(pz: PerZoneWbUniforms): void { this.pz = pz; }
  setClip(c: ClipUniforms): void { this.clip = c; }
  setInversion(u: InversionUniforms): void { this.inv = u; }

  /** Geometry from the host (identity-safe). out{W,H} = post-geometry canvas size. */
  setGeometry(g: Geometry): void {
    this.geom = g;
    this.canvas.width = g.outW;
    this.canvas.height = g.outH;
    this.allocIntermediate(g.outW, g.outH);
  }

  /**
   * Upload the raw linear negative as an `rgba16float` texture (once per
   * image). Returns false on a zero/invalid size or a GPU allocation failure
   * — the caller should fall back rather than render a stale/black frame.
   */
  setSourceFloat(pixels: Uint16Array, w: number, h: number): boolean {
    if (w <= 0 || h <= 0) return false;
    try {
      this.srcTex?.destroy();
      this.srcTex = this.device.createTexture({
        size: { width: w, height: h },
        format: "rgba16float",
        usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
      });
      // rgba16float: 4 channels * 2 bytes/channel = 8 bytes/pixel.
      this.device.queue.writeTexture(
        { texture: this.srcTex },
        pixels,
        { bytesPerRow: w * 8, rowsPerImage: h },
        { width: w, height: h },
      );
    } catch (err) {
      console.error("WebGPUFinishRenderer.setSourceFloat failed:", err);
      return false;
    }
    this.srcW = w; this.srcH = h;
    this.hasSource = true;
    return true;
  }

  private allocIntermediate(w: number, h: number): void {
    this.interTex?.destroy();
    this.interTex = this.device.createTexture({
      size: { width: Math.max(1, w), height: Math.max(1, h) },
      format: "rgba16float",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.RENDER_ATTACHMENT,
    });
  }

  /** Flattens the stashed setX() inputs into the packer's byte-layout shape. */
  private buildUniformsInput(): HdrUniformsInput {
    const fu = this.finish, cg = this.cg, cm = this.cm, pz = this.pz, clip = this.clip, inv = this.inv;
    const outW = Math.max(1, this.geom.outW), outH = Math.max(1, this.geom.outH);
    return {
      contrast: fu.contrast, highlights: fu.highlights, shadows: fu.shadows,
      whites: fu.whites, blacks: fu.blacks, texture: fu.texture,
      vibrance: fu.vibrance, saturation: fu.saturation, brightness: fu.brightness,

      texel: [1 / outW, 1 / outH],

      cg_sh_off: cg.sh_off, cg_mid_off: cg.mid_off, cg_hi_off: cg.hi_off, cg_glob_off: cg.glob_off,
      cg_sh_lum: cg.sh_lum, cg_mid_lum: cg.mid_lum, cg_hi_lum: cg.hi_lum, cg_glob_lum: cg.glob_lum,
      cg_sh_edge: cg.sh_edge, cg_hi_edge: cg.hi_edge, cg_soft: cg.softness,

      cm_hue: cm.cm_hue, cm_sat: cm.cm_sat, cm_lum: cm.cm_lum,

      pc_count: cm.pc_count,
      pc_hue: cm.pc_hue, pc_sat: cm.pc_sat, pc_lum: cm.pc_lum,
      pc_hue_shift: cm.pc_hue_shift, pc_sat_shift: cm.pc_sat_shift, pc_lum_shift: cm.pc_lum_shift,
      pc_variance: cm.pc_variance, pc_range: cm.pc_range,

      pz_enabled: pz.enabled, pz_sh: pz.sh, pz_mid: pz.mid, pz_hi: pz.hi,

      clip_high_on: clip.highOn, clip_low_on: clip.lowOn, clip_strict: clip.strict,
      soft_clip: inv.soft_clip,

      finish_mode: this.finishMode,
      // Positive passthrough is already display-referred → skip body finalize.
      // Mirrors gl/renderer.ts's drawFinishPass u_finalize_body wiring.
      finalize_body: inv.positive ? 0 : 1,

      base: [inv.base[0], inv.base[1], inv.base[2]],
      wb: [inv.wb[0], inv.wb[1], inv.wb[2]],
      m_pre: inv.m_pre, m_post: inv.m_post,
      exposure: inv.exposure, black: inv.black, gamma: inv.gamma, d_max: inv.d_max,
      print_exposure: inv.print_exposure, paper_black: inv.paper_black, paper_grade: inv.paper_grade,
      mode: inv.mode, raw: this.geom.raw ? 1 : 0, positive: inv.positive ? 1 : 0,
      wb_mode: inv.wb_mode, tone_mode: inv.tone_mode,
      hi_recovery: inv.hi_recovery, lo_recovery: inv.lo_recovery,
      cam_balance: [inv.cam_balance[0], inv.cam_balance[1], inv.cam_balance[2]],

      crop_off: this.geom.crop_off, crop_scale: this.geom.crop_scale,
      angle: this.geom.angle, aspect: this.geom.aspect,
      orient: this.geom.orient,
      view_off: this.geom.view_off ?? [0, 0],
      view_scale: this.geom.view_scale ?? [1, 1],
    };
  }

  /** Encodes invert (src -> intermediate) then finish (intermediate+LUT -> canvas)
   *  and submits. No-ops until a source texture has been uploaded. */
  render(): void {
    if (!this.hasSource || !this.srcTex || !this.interTex) return;

    const packed = packHdrUniforms(this.buildUniformsInput());
    this.device.queue.writeBuffer(this.uniformBuffer, 0, packed);

    const invertBindGroup = this.device.createBindGroup({
      layout: this.invertPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.uniformBuffer } },
        { binding: 1, resource: this.sampler },
        { binding: 2, resource: this.srcTex.createView() },
      ],
    });
    const finishBindGroup = this.device.createBindGroup({
      layout: this.finishPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.uniformBuffer } },
        { binding: 1, resource: this.sampler },
        { binding: 2, resource: this.interTex.createView() },
        { binding: 3, resource: this.lutTex.createView() },
      ],
    });

    const encoder = this.device.createCommandEncoder();

    const invertPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: this.interTex.createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        loadOp: "clear", storeOp: "store",
      }],
    });
    invertPass.setPipeline(this.invertPipeline);
    invertPass.setBindGroup(0, invertBindGroup);
    invertPass.draw(3);
    invertPass.end();

    const finishPass = encoder.beginRenderPass({
      colorAttachments: [{
        view: this.ctx.getCurrentTexture().createView(),
        clearValue: { r: 0, g: 0, b: 0, a: 1 },
        loadOp: "clear", storeOp: "store",
      }],
    });
    finishPass.setPipeline(this.finishPipeline);
    finishPass.setBindGroup(0, finishBindGroup);
    finishPass.draw(3);
    finishPass.end();

    this.device.queue.submit([encoder.finish()]);
  }

  /** Releases every GPU resource owned by this renderer. */
  dispose(): void {
    this.srcTex?.destroy();
    this.interTex?.destroy();
    this.lutTex.destroy();
    this.uniformBuffer.destroy();
    this.device.destroy();
    this.srcTex = null;
    this.interTex = null;
    this.hasSource = false;
  }
}
