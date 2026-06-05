import { VERT, FRAG, INVERT_FRAG } from "./shaders";
import { type InversionUniforms } from "./invert";
import type { FinishUniforms } from "./uniforms";
import type { ColorGradeUniforms } from "../../develop/finish";
import { LUT_SIZE } from "../../develop/curve";

/** True if the environment can create a WebGL2 context. */
export function webgl2Available(): boolean {
  if (typeof document === "undefined") return false;
  try {
    const c = document.createElement("canvas");
    return !!c.getContext("webgl2");
  } catch {
    return false;
  }
}

/**
 * Spike (Plan 1): does THIS environment's WebGL2 support an RGBA16F render
 * target? Plans 2-3 (GPU inversion + offscreen export) depend on it. Creates a
 * tiny offscreen RGBA16F texture, attaches it to an FBO, and checks both the
 * float-color-buffer extension and framebuffer completeness. Returns a verdict
 * object so the result can be logged from the app.
 */
export function float16RenderTargetSupported():
  { ok: boolean; reason: string } {
  if (typeof document === "undefined") return { ok: false, reason: "no document" };
  let gl: WebGL2RenderingContext | null = null;
  try {
    gl = document.createElement("canvas").getContext("webgl2");
  } catch {
    return { ok: false, reason: "no webgl2 context" };
  }
  if (!gl) return { ok: false, reason: "no webgl2 context" };
  // Needed to RENDER to a float texture (not just sample one).
  if (!gl.getExtension("EXT_color_buffer_float")) {
    return { ok: false, reason: "EXT_color_buffer_float missing" };
  }
  const tex = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, 4, 4, 0, gl.RGBA, gl.HALF_FLOAT, null);
  const fbo = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const status = gl.checkFramebufferStatus(gl.FRAMEBUFFER);
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.deleteFramebuffer(fbo);
  gl.deleteTexture(tex);
  if (status !== gl.FRAMEBUFFER_COMPLETE) {
    return { ok: false, reason: `framebuffer incomplete: 0x${status.toString(16)}` };
  }
  return { ok: true, reason: "RGBA16F render target OK" };
}

const UNIFORM_NAMES = [
  "contrast", "highlights", "shadows", "whites", "blacks",
  "vibrance", "saturation", "texture",
] as const;

// Color-grading uniforms: [shader uniform name, ColorGradeUniforms key]. The
// shader names are cg-prefixed; the JS object keys are not — keep them paired.
const CG_VEC3: [string, keyof ColorGradeUniforms][] = [
  ["u_cg_sh_off", "sh_off"], ["u_cg_mid_off", "mid_off"],
  ["u_cg_hi_off", "hi_off"], ["u_cg_glob_off", "glob_off"],
];
const CG_FLOAT: [string, keyof ColorGradeUniforms][] = [
  ["u_cg_sh_lum", "sh_lum"], ["u_cg_mid_lum", "mid_lum"],
  ["u_cg_hi_lum", "hi_lum"], ["u_cg_glob_lum", "glob_lum"],
  ["u_cg_sh_edge", "sh_edge"], ["u_cg_hi_edge", "hi_edge"], ["u_cg_soft", "softness"],
];

/** Applies the finishing layer to a source preview texture via a fragment shader. */
export class FinishRenderer {
  readonly available: boolean;
  private gl: WebGL2RenderingContext | null = null;
  private prog: WebGLProgram | null = null;
  private tex: WebGLTexture | null = null;
  private lutTex: WebGLTexture | null = null;
  private vao: WebGLVertexArrayObject | null = null;
  private loc: Record<string, WebGLUniformLocation | null> = {};
  private uniforms: FinishUniforms | null = null;
  private cg: ColorGradeUniforms | null = null;
  private srcW = 0;
  private srcH = 0;
  private hasSource = false;
  private invProg: WebGLProgram | null = null;
  private srcTexF: WebGLTexture | null = null;   // RGBA16F raw negative
  private interTex: WebGLTexture | null = null;   // RGBA16F inverted intermediate
  private fbo: WebGLFramebuffer | null = null;
  private inv: InversionUniforms | null = null;
  private invLoc: Record<string, WebGLUniformLocation | null> = {};
  private geom = {
    crop_off: new Float32Array([0, 0]),
    crop_scale: new Float32Array([1, 1]),
    angle: 0,
    orient: new Float32Array([1, 0, 0, 1]),
    raw: false,
  };
  private useFloat = false; // true once setSourceFloat is used

  constructor(private canvas: HTMLCanvasElement) {
    const gl = canvas.getContext("webgl2", { preserveDrawingBuffer: true, premultipliedAlpha: false });
    if (!gl) { this.available = false; return; }
    this.gl = gl;
    const prog = this.build(gl);
    if (!prog) { this.available = false; return; }
    this.prog = prog;
    this.vao = gl.createVertexArray(); // empty VAO required to draw in WebGL2
    this.tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    // Tone-curve LUT texture (256x1 RGBA8, linear for smooth interpolation).
    this.lutTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.lutTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    this.setLut(identityLut());
    gl.useProgram(prog);
    this.loc.u_src = gl.getUniformLocation(prog, "u_src");
    this.loc.u_lut = gl.getUniformLocation(prog, "u_lut");
    this.loc.u_texel = gl.getUniformLocation(prog, "u_texel");
    for (const n of UNIFORM_NAMES) this.loc[`u_${n}`] = gl.getUniformLocation(prog, `u_${n}`);
    for (const [u] of CG_VEC3) this.loc[u] = gl.getUniformLocation(prog, u);
    for (const [u] of CG_FLOAT) this.loc[u] = gl.getUniformLocation(prog, u);
    gl.uniform1i(this.loc.u_src, 0);
    gl.uniform1i(this.loc.u_lut, 1);

    // Invert program (pass 1). Requires float color-buffer for the FBO.
    if (!gl.getExtension("EXT_color_buffer_float")) { this.available = false; return; }
    const ivs = this.compile(gl, gl.VERTEX_SHADER, VERT);
    const ifs = this.compile(gl, gl.FRAGMENT_SHADER, INVERT_FRAG);
    if (!ivs || !ifs) { this.available = false; return; }
    const ip = gl.createProgram()!;
    gl.attachShader(ip, ivs); gl.attachShader(ip, ifs); gl.linkProgram(ip);
    if (!gl.getProgramParameter(ip, gl.LINK_STATUS)) {
      console.error("invert link:", gl.getProgramInfoLog(ip)); this.available = false; return;
    }
    this.invProg = ip;
    for (const n of [
      "u_src","u_base","u_wb","u_m_pre","u_m_post","u_exposure","u_black","u_gamma",
      "u_mode","u_raw","u_crop_off","u_crop_scale","u_angle","u_orient",
    ]) this.invLoc[n] = gl.getUniformLocation(ip, n);
    gl.useProgram(ip); gl.uniform1i(this.invLoc.u_src, 0);

    // RGBA16F raw-negative source texture.
    this.srcTexF = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);

    // RGBA16F intermediate (inverted) + FBO.
    this.interTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, this.interTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    this.fbo = gl.createFramebuffer();

    this.available = true;
  }

  /** Upload a 256×1 RGBA8 tone LUT. */
  setLut(bytes: Uint8Array) {
    const gl = this.gl; if (!gl || !this.lutTex) return;
    gl.bindTexture(gl.TEXTURE_2D, this.lutTex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, LUT_SIZE, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
  }

  setColorGrade(cg: ColorGradeUniforms) { this.cg = cg; }

  private build(gl: WebGL2RenderingContext): WebGLProgram | null {
    const vs = this.compile(gl, gl.VERTEX_SHADER, VERT);
    const fs = this.compile(gl, gl.FRAGMENT_SHADER, FRAG);
    if (!vs || !fs) return null;
    const p = gl.createProgram()!;
    gl.attachShader(p, vs); gl.attachShader(p, fs); gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
      console.error("link:", gl.getProgramInfoLog(p)); return null;
    }
    return p;
  }
  private compile(gl: WebGL2RenderingContext, type: number, src: string): WebGLShader | null {
    const s = gl.createShader(type)!;
    gl.shaderSource(s, src); gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
      console.error("shader:", gl.getShaderInfoLog(s)); return null;
    }
    return s;
  }

  /** Upload a decoded preview image as the source texture; sizes the canvas. */
  setSource(img: TexImageSource, w: number, h: number) {
    const gl = this.gl; if (!gl || !this.tex) return;
    this.useFloat = false; // 8-bit CPU-fallback source → draw() uses the single finishing pass
    this.srcW = w; this.srcH = h;
    this.canvas.width = w; this.canvas.height = h;
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
    this.hasSource = true;
  }

  setUniforms(u: FinishUniforms) { this.uniforms = u; }

  /** Upload the raw linear negative as an RGBA16F texture (once per image). */
  setSourceFloat(pixels: Uint16Array, w: number, h: number) {
    const gl = this.gl; if (!gl || !this.srcTexF || !this.interTex) return;
    this.srcW = w; this.srcH = h; this.useFloat = true;
    gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false); // geometry handled in-shader
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, pixels);
    // (Re)allocate the intermediate to the OUTPUT size; default = source size.
    this.allocInter(w, h);
    this.hasSource = true;
  }

  /** Size the intermediate FBO texture (output dims = post-geometry canvas). */
  private allocInter(w: number, h: number) {
    const gl = this.gl; if (!gl || !this.interTex) return;
    gl.bindTexture(gl.TEXTURE_2D, this.interTex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  }

  setInversion(u: InversionUniforms) { this.inv = u; }

  /** Geometry from the host (identity-safe). out{W,H} = post-geometry canvas size. */
  setGeometry(g: {
    crop_off: [number, number]; crop_scale: [number, number];
    angle: number; orient: [number, number, number, number];
    raw: boolean; outW: number; outH: number;
  }) {
    this.geom.crop_off = new Float32Array(g.crop_off);
    this.geom.crop_scale = new Float32Array(g.crop_scale);
    this.geom.angle = g.angle;
    this.geom.orient = new Float32Array(g.orient);
    this.geom.raw = g.raw;
    this.canvas.width = g.outW; this.canvas.height = g.outH;
    this.allocInter(g.outW, g.outH);
    this.srcW = g.outW; this.srcH = g.outH; // finishing pass uses these for u_texel/viewport
  }

  /** Max texture dimension this GL context supports (0 if no context). */
  maxTextureSize(): number {
    return this.gl ? this.gl.getParameter(this.gl.MAX_TEXTURE_SIZE) : 0;
  }

  /** PASS 1: INVERT raw negative → intermediate FBO (output-sized). */
  private drawInvertPass() {
    const gl = this.gl; if (!gl) return;
    gl.useProgram(this.invProg);
    gl.bindVertexArray(this.vao);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.fbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.interTex, 0);
    gl.viewport(0, 0, this.srcW, this.srcH);
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.srcTexF);
    const L = this.invLoc, u = this.inv!;
    gl.uniform3fv(L.u_base, u.base); gl.uniform3fv(L.u_wb, u.wb);
    gl.uniformMatrix3fv(L.u_m_pre, false, u.m_pre);
    gl.uniformMatrix3fv(L.u_m_post, false, u.m_post);
    gl.uniform1f(L.u_exposure, u.exposure); gl.uniform1f(L.u_black, u.black);
    gl.uniform1f(L.u_gamma, u.gamma); gl.uniform1i(L.u_mode, u.mode);
    gl.uniform1i(L.u_raw, this.geom.raw ? 1 : 0);
    gl.uniform2fv(L.u_crop_off, this.geom.crop_off);
    gl.uniform2fv(L.u_crop_scale, this.geom.crop_scale);
    gl.uniform1f(L.u_angle, this.geom.angle);
    gl.uniformMatrix2fv(L.u_orient, false, this.geom.orient);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  }

  /**
   * PASS 2: FINISH (existing program) reads the intermediate, draws into
   * WHATEVER framebuffer is currently bound (canvas for live, export FBO for
   * export) at viewport (vw, vh).
   */
  private drawFinishPass(vw: number, vh: number) {
    const gl = this.gl; if (!gl) return;
    const p = this.prog, fu = this.uniforms;
    if (!p || !fu) return;
    gl.useProgram(p);
    gl.bindVertexArray(this.vao);
    gl.viewport(0, 0, vw, vh);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.useFloat ? this.interTex : this.tex);
    gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, this.lutTex);
    gl.uniform2f(this.loc.u_texel, 1 / vw, 1 / vh);
    for (const n of UNIFORM_NAMES) gl.uniform1f(this.loc[`u_${n}`], (fu as unknown as Record<string, number>)[n]);
    const cg = this.cg;
    if (cg) {
      for (const [uu, k] of CG_VEC3) gl.uniform3fv(this.loc[uu], cg[k] as [number, number, number]);
      for (const [uu, k] of CG_FLOAT) gl.uniform1f(this.loc[uu], cg[k] as number);
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  draw() {
    const gl = this.gl; if (!gl || !this.hasSource) return;
    if (this.useFloat && this.invProg && this.inv) this.drawInvertPass();
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.drawFinishPass(this.srcW, this.srcH);
  }

  /**
   * Render invert+finish for EXPORT into an offscreen FBO at (w,h) and read back.
   * bit16=false → returns RGBA8 (Uint8Array); bit16=true → RGBA f32 (Float32Array).
   * Geometry is identity (Rust already baked orient/crop/heal into `src`).
   * Returns null if WebGL is unavailable or (w,h) exceeds MAX_TEXTURE_SIZE.
   */
  renderExport(
    src: Uint16Array, w: number, h: number,
    inv: InversionUniforms,
    fu: FinishUniforms, lut: Uint8Array, cg: ColorGradeUniforms,
    bit16: boolean,
  ): { data: Uint8Array | Float32Array; w: number; h: number } | null {
    const gl = this.gl; if (!gl || !this.invProg || !this.prog) return null;
    const max = gl.getParameter(gl.MAX_TEXTURE_SIZE);
    if (w > max || h > max) return null;

    // Upload the full-res source, set inversion + IDENTITY geometry + finishing.
    this.setSourceFloat(src, w, h);
    this.setInversion(inv);
    this.setGeometry({ crop_off: [0, 0], crop_scale: [1, 1], angle: 0, orient: [1, 0, 0, 1], raw: false, outW: w, outH: h });
    this.setUniforms(fu); this.setLut(lut); this.setColorGrade(cg);

    // Offscreen output texture + FBO (RGBA8 for 8-bit, RGBA16F for 16-bit).
    const outInternal = bit16 ? gl.RGBA16F : gl.RGBA8;
    const outType = bit16 ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE;
    const outTex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, outTex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texImage2D(gl.TEXTURE_2D, 0, outInternal, w, h, 0, gl.RGBA, outType, null);
    const outFbo = gl.createFramebuffer();

    // PASS 1: invert → interTex.
    this.drawInvertPass();
    // PASS 2: finish interTex → outTex.
    gl.bindFramebuffer(gl.FRAMEBUFFER, outFbo);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, outTex, 0);
    this.drawFinishPass(w, h);

    // Read back.
    let data: Uint8Array | Float32Array;
    if (bit16) { data = new Float32Array(w * h * 4); gl.readPixels(0, 0, w, h, gl.RGBA, gl.FLOAT, data); }
    else { data = new Uint8Array(w * h * 4); gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, data); }

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(outFbo); gl.deleteTexture(outTex);
    return { data, w, h };
  }
}

/** A neutral 256×1 RGBA8 ramp LUT (identity tone curve). */
function identityLut(): Uint8Array {
  const out = new Uint8Array(LUT_SIZE * 4);
  for (let i = 0; i < LUT_SIZE; i++) {
    const v = Math.round((i / (LUT_SIZE - 1)) * 255);
    out[i * 4] = v; out[i * 4 + 1] = v; out[i * 4 + 2] = v; out[i * 4 + 3] = 255;
  }
  return out;
}
