import { VERT, FRAG } from "./shaders";
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
    this.srcW = w; this.srcH = h;
    this.canvas.width = w; this.canvas.height = h;
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
    this.hasSource = true;
  }

  setUniforms(u: FinishUniforms) { this.uniforms = u; }

  draw() {
    const gl = this.gl, p = this.prog, u = this.uniforms;
    if (!gl || !p || !u || !this.hasSource) return;
    gl.useProgram(p);
    gl.bindVertexArray(this.vao);
    gl.viewport(0, 0, this.srcW, this.srcH);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.tex);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, this.lutTex);
    gl.uniform2f(this.loc.u_texel, 1 / this.srcW, 1 / this.srcH);
    for (const n of UNIFORM_NAMES) gl.uniform1f(this.loc[`u_${n}`], (u as unknown as Record<string, number>)[n]);
    const cg = this.cg;
    if (cg) {
      for (const [u, k] of CG_VEC3) gl.uniform3fv(this.loc[u], cg[k] as [number, number, number]);
      for (const [u, k] of CG_FLOAT) gl.uniform1f(this.loc[u], cg[k] as number);
    }
    gl.drawArrays(gl.TRIANGLES, 0, 3);
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
