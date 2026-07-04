// app/src/lib/roll/exportSheet.ts
import { get } from "svelte/store";
import { save } from "@tauri-apps/plugin-dialog";
import { api, defaultParams, type ExportFormat } from "$lib/api";
import {
  editsById, cropById, rollFilmEdge, rollEdgeText, rollFilmFormat,
  sheetHeaderPhotographer, sheetHeaderCamera, sheetHeaderFilm, sheetHeaderDate,
} from "$lib/store";
import { developedFolderImages } from "$lib/export/eligible";
import { withEffectiveBase } from "$lib/develop/base";
import { imageDir } from "$lib/library/folderScope";
import { draftThumbView } from "./livePreview";
import { pickTileAspect, fitContain } from "./contactSheet";
import { perfLayout, type PerfLayout } from "./sprockets";
import { paperPx, paginateStrips, pagePath, PAGE_MARGIN_MM, type PaperSize } from "./printLayout";

// ─── Layout constants (match on-screen filmstrip) ────────────────────────────
// Exported so Roll.svelte derives its sprocket geometry from the SAME design
// space and the two renderers stay visually identical — issue #23 (upstream).
const STRIP_SIZE = 6;            // frames per strip row
export const FRAME_W = 260;      // frame width in pixels

// Filmstrip rebate/spacing (pixels, scaled to frame size). The sprocket band
// height is no longer a constant: it comes from perfLayout() per film format.
const FRAME_NUM_H = 26;
const BARCODE_INFO_H = 26;
const EDGE_REPEATS = 3; // edge marking copies distributed across the strip
export const FRAME_GAP = 7;      // gap between frames within a strip
export const FRAME_PAD = 6;      // left+right padding inside the black frames row
const STRIP_GAP = 16;     // vertical gap between strips
const OUTER_MARGIN = 24;  // canvas edge margin on all sides

// Proof-grid constants (film-edge OFF)
const PROOF_SHADOW_SIZE = 3;
const PROOF_PADDING = 3;
const PROOF_CAPTION_H = 8 + 12; // 8px gap + 12px text line


// ─── Helper: rounded-rect path with a fallback for engines without roundRect ─
function pathRoundRect(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number, r: number,
) {
  if (typeof ctx.roundRect === "function") {
    ctx.roundRect(x, y, w, h, r);
    return;
  }
  // arcTo fallback — WebView2 ships roundRect, but keep exports working anywhere
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

// ─── Helper: draw one sprocket-hole band (KS-1870 rounded rects) ─────────────
// Mirrors the CSS repeat-x SVG tile in Roll.svelte (issue #23, upstream): holes
// slightly brighter than the #131210 rebate — backlit — clipped at strip edges
// exactly like the CSS background is.
function drawPerfBand(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number,
  perf: PerfLayout,
) {
  ctx.save();
  ctx.beginPath();
  ctx.rect(x, y, w, perf.bandH);
  ctx.clip();
  ctx.fillStyle = "rgba(216,207,184,0.16)";
  const holeY = y + (perf.bandH - perf.perfH) / 2;
  // First hole centred pitch/2 into its tile; tiles start at x + offset.
  for (let cx = x + perf.offset + perf.pitch / 2; cx - perf.perfW / 2 < x + w; cx += perf.pitch) {
    ctx.beginPath();
    pathRoundRect(ctx, cx - perf.perfW / 2, holeY, perf.perfW, perf.perfH, perf.radius);
    ctx.fill();
  }
  ctx.restore();
}

// ─── Helper: draw barcode (approximate the CSS gradient) ─────────────────────
function drawBarcode(
  ctx: CanvasRenderingContext2D,
  x: number, y: number, w: number, h: number,
) {
  // Replicate: repeating-linear-gradient(90deg,#c9c3b0 0 1px,transparent 1px 3px,#c9c3b0 3px 4px,transparent 4px 6px,#c9c3b0 6px 8px,transparent 8px 11px,#c9c3b0 11px 12px,transparent 12px 15px,#c9c3b0 15px 17px,transparent 17px 19px)
  // Pattern: [bar at 0-1], [gap 1-3], [bar 3-4], [gap 4-6], [bar 6-8], [gap 8-11], [bar 11-12], [gap 12-15], [bar 15-17], [gap 17-19], repeat every 19px
  const pattern: Array<[number, number]> = [[0,1],[3,4],[6,8],[11,12],[15,17]]; // [start, end] within 19px period
  const period = 19;
  ctx.fillStyle = "#c9c3b0";
  let dx = x;
  while (dx < x + w) {
    for (const [s, e] of pattern) {
      const bx = dx + s;
      const bw = e - s;
      if (bx < x + w && bx + bw > x) {
        const cx = Math.max(bx, x);
        const cw = Math.min(bx + bw, x + w) - cx;
        ctx.fillRect(cx, y, cw, h);
      }
    }
    dx += period;
  }
}

/** Resolution + output-format options chosen in the export dialog. `scale`
 *  uniformly enlarges the whole sheet (canvas + fonts + strokes); `thumbEdge` is
 *  the long-edge cap requested for each frame render so the tiles stay sharp at
 *  the larger size; `format` is the on-disk encoding. `paper` (issue #24,
 *  upstream) selects the canvas shape: "strip" hugs the content (historic
 *  behaviour); "a4"/"letter" produce print-ready pages — on paper, `scale`
 *  doubles as print quality (1 = 150dpi, 2 = 300dpi, 4 = 600dpi). */
export interface ExportSheetOpts {
  scale: number;       // 1 = standard (260px/frame), 2 = high, 4 = print
  thumbEdge: number;   // per-frame render long-edge cap (px)
  format: ExportFormat;
  paper?: PaperSize;   // default "strip"
}

const DEFAULT_OPTS: ExportSheetOpts = {
  scale: 1, thumbEdge: 320, format: { kind: "png", bitDepth: 8 },
};

/** Render each developed frame at its own stored edits + crop, composite them
 *  into a contact-sheet canvas matching the on-screen film-strip design, and
 *  save the result to a file chosen by the user via the OS save dialog. */
export async function exportContactSheet(opts: ExportSheetOpts = DEFAULT_OPTS): Promise<void> {
  const frames = get(developedFolderImages);
  if (frames.length === 0) return;

  const edits = get(editsById);
  const crops = get(cropById);
  const filmEdge = get(rollFilmEdge);
  const edgeText = get(rollEdgeText);

  // ── Render every frame tile via the backend (same as on-screen) ──────────
  const images = await Promise.all(
    frames.map(async (frame) => {
      const params = withEffectiveBase(
        edits[frame.id] ?? defaultParams(),
        imageDir(frame),
      );
      const crop = crops[frame.id] ?? null;
      const view = { ...draftThumbView(crop), edge: opts.thumbEdge };
      const dataUrl = await api.thumbnail(frame.id, params, view);

      return new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image();
        img.onload = () => resolve(img);
        img.onerror = reject;
        img.src = dataUrl;
      });
    }),
  );

  // Ensure custom fonts are loaded before drawing text
  if (typeof document !== "undefined" && document.fonts?.ready) {
    await document.fonts.ready;
  }

  // ── Chunk frames into strips of STRIP_SIZE ────────────────────────────────
  const strips: { imgs: HTMLImageElement[]; nums: string[]; padCount: number }[] = [];
  for (let i = 0; i < images.length; i += STRIP_SIZE) {
    const slice = images.slice(i, i + STRIP_SIZE);
    const nums = slice.map((_, j) => String(i + j + 1).padStart(2, "0"));
    strips.push({ imgs: slice, nums, padCount: STRIP_SIZE - slice.length });
  }

  // ── Tile aspect from the roll's actual frame shapes (matches on-screen) ───
  // Landscape frames fill their tile edge-to-edge; every tile is FRAME_H tall.
  const tileAspect = pickTileAspect(
    images.map((im) => (im.naturalWidth > 0 && im.naturalHeight > 0 ? im.naturalWidth / im.naturalHeight : 0)),
  );
  const FRAME_H = Math.round(FRAME_W / tileAspect);

  // ── Perforation geometry (issue #23, upstream) ────────────────────────────
  // Shared with Roll.svelte's CSS tile; "120" scales from the frame short edge
  // so it must wait for FRAME_H. The rebate heights grow with the band.
  const perf = perfLayout(get(rollFilmFormat), FRAME_W, FRAME_H, FRAME_GAP, FRAME_PAD);
  const SPROCKET_H = perf.bandH;
  const REBATE_TOP_H = SPROCKET_H + FRAME_NUM_H;
  const REBATE_BOT_H = BARCODE_INFO_H + SPROCKET_H;

  // ── Compute sheet geometry (design space, 1×) ─────────────────────────────
  // Strip width: 6 frames + gaps + padding on both sides
  const stripContentW = STRIP_SIZE * FRAME_W + (STRIP_SIZE - 1) * FRAME_GAP + 2 * FRAME_PAD;
  // One strip block's height in design px (both modes are constant-height rows).
  const perStripH = filmEdge
    ? REBATE_TOP_H + FRAME_H + REBATE_BOT_H
    : PROOF_PADDING * 2 + FRAME_H + PROOF_CAPTION_H;

  // ── Draw ONE strip block at (leftX, topY) in design coordinates ───────────
  // Shared by the roll-strip canvas and the paginated paper pages (issue #24,
  // upstream), so paper layouts can never drift from the classic export.
  function drawStrip(
    ctx: CanvasRenderingContext2D,
    strip: { imgs: HTMLImageElement[]; nums: string[]; padCount: number },
    leftX: number,
    topY: number,
  ): void {
    const rowH = FRAME_H; // fixed landscape tile height for every strip
    let cursorY = topY;

    if (filmEdge) {
      // ── FILMSTRIP mode ──────────────────────────────────────────────────
      const stripW = stripContentW;

      // TOP REBATE (background #131210)
      ctx.fillStyle = "#131210";
      ctx.fillRect(leftX, cursorY, stripW, REBATE_TOP_H);

      // Sprocket holes — top band
      drawPerfBand(ctx, leftX, cursorY, stripW, perf);

      // Frame numbers
      ctx.fillStyle = "#a39a82";
      ctx.font = "600 18px 'Spline Sans Mono', ui-monospace, monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const numY = cursorY + SPROCKET_H + FRAME_NUM_H / 2;
      for (let fi = 0; fi < STRIP_SIZE; fi++) {
        const frameLeft = leftX + FRAME_PAD + fi * (FRAME_W + FRAME_GAP);
        const frameCenterX = frameLeft + FRAME_W / 2;
        if (fi < strip.nums.length) {
          ctx.fillText(strip.nums[fi], frameCenterX, numY);
        }
      }

      cursorY += REBATE_TOP_H;

      // FRAMES ROW (black background) — height = rowH for this strip
      ctx.fillStyle = "#000";
      ctx.fillRect(leftX, cursorY, stripW, rowH);

      // Draw each frame fit (contained) inside its fixed landscape tile, flush left
      // (no leading gap) + vertically centered. Slack letterboxes against the black row.
      for (let fi = 0; fi < strip.imgs.length; fi++) {
        const img = strip.imgs[fi];
        const frameLeft = leftX + FRAME_PAD + fi * (FRAME_W + FRAME_GAP);
        const { dx, dy, dw, dh } = fitContain(img.naturalWidth, img.naturalHeight, FRAME_W, rowH, "left");
        ctx.drawImage(img, frameLeft + dx, cursorY + dy, dw, dh);
      }

      cursorY += rowH;

      // BOTTOM REBATE (background #131210)
      ctx.fillStyle = "#131210";
      ctx.fillRect(leftX, cursorY, stripW, REBATE_BOT_H);

      // Info row: barcode + edge text + arrow
      const infoY = cursorY;
      const infoMidY = infoY + BARCODE_INFO_H / 2;

      // Barcode (34×11px)
      const barcodeW = 34;
      const barcodeX = leftX + 12;
      const barcodeY = infoY + (BARCODE_INFO_H - 11) / 2;
      drawBarcode(ctx, barcodeX, barcodeY, barcodeW, 11);

      // Arrow "→" on the right
      ctx.fillStyle = "#7a7464";
      ctx.font = "600 16px 'Spline Sans Mono', ui-monospace, monospace";
      ctx.textAlign = "right";
      ctx.textBaseline = "middle";
      const arrowX = leftX + stripW - 12;
      ctx.fillText("→", arrowX, infoMidY);

      // Edge text — repeated and evenly distributed between the barcode and arrow
      ctx.fillStyle = "#968f7c";
      ctx.font = "600 15px 'Spline Sans Mono', ui-monospace, monospace";
      ctx.textAlign = "center";
      ctx.letterSpacing = "0.24em";
      const trackLeft = barcodeX + barcodeW + 16;
      const trackRight = arrowX - 24;
      const trackW = Math.max(0, trackRight - trackLeft);
      for (let r = 0; r < EDGE_REPEATS; r++) {
        const cx = trackLeft + (trackW * (r + 0.5)) / EDGE_REPEATS;
        ctx.fillText(edgeText, cx, infoMidY);
      }
      ctx.letterSpacing = "0px";

      // Sprocket holes — bottom band
      drawPerfBand(ctx, leftX, cursorY + BARCODE_INFO_H, stripW, perf);

    } else {
      // ── PROOF GRID mode ─────────────────────────────────────────────────
      // Each cell: proof-frame (shadow + #d8d3c4 bg + 3px padding + image at true aspect) + caption below
      const proofCellW = FRAME_W;
      const proofFrameH = PROOF_PADDING * 2 + rowH;

      for (let fi = 0; fi < STRIP_SIZE; fi++) {
        const cellLeft = leftX + fi * (proofCellW + FRAME_GAP);

        if (fi < strip.imgs.length) {
          const img = strip.imgs[fi];

          // Shadow (dark rect behind)
          ctx.fillStyle = "rgba(0,0,0,0.5)";
          ctx.fillRect(cellLeft + 2, cursorY + 2, proofCellW, proofFrameH);

          // Warm-white background
          ctx.fillStyle = "#d8d3c4";
          ctx.fillRect(cellLeft, cursorY, proofCellW, proofFrameH);

          // Image fit (contained) inside the padded tile, flush left + vertically
          // centered. Slack letterboxes against the warm-white background.
          const innerW = proofCellW - PROOF_PADDING * 2;
          const innerH = proofFrameH - PROOF_PADDING * 2;
          const { dx, dy, dw, dh } = fitContain(img.naturalWidth, img.naturalHeight, innerW, innerH, "left");
          ctx.drawImage(
            img,
            cellLeft + PROOF_PADDING + dx,
            cursorY + PROOF_PADDING + dy,
            dw,
            dh,
          );

          // Caption below frame
          ctx.fillStyle = "#6f6a5e";
          ctx.font = "600 12px 'Spline Sans Mono', ui-monospace, monospace";
          ctx.textAlign = "center";
          ctx.textBaseline = "top";
          ctx.fillText(strip.nums[fi], cellLeft + proofCellW / 2, cursorY + proofFrameH + 8);
        }
        // Pad cells: leave empty (background shows through)
      }
    }
  }

  const scale = Math.max(1, opts.scale || 1);
  const paper: PaperSize = opts.paper ?? "strip";
  const pageCanvases: HTMLCanvasElement[] = [];

  if (paper === "strip") {
    // ── ROLL STRIP (historic behaviour): one canvas hugging the content ─────
    // The layout is computed in base (1×) coordinates; `scale` enlarges the
    // backing store and a one-shot ctx.scale() draws everything — frames,
    // fonts, strokes — proportionally larger for a higher-resolution sheet.
    const totalStripsH = strips.length * perStripH + Math.max(0, strips.length - 1) * STRIP_GAP;
    const canvasW = 2 * OUTER_MARGIN + stripContentW;
    const canvasH = 2 * OUTER_MARGIN + totalStripsH;

    const canvas = document.createElement("canvas");
    canvas.width = Math.round(canvasW * scale);
    canvas.height = Math.round(canvasH * scale);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Could not get 2D canvas context");
    ctx.scale(scale, scale);

    // Background
    ctx.fillStyle = "#0b0b0c";
    ctx.fillRect(0, 0, canvasW, canvasH);

    for (let si = 0; si < strips.length; si++) {
      drawStrip(ctx, strips[si], OUTER_MARGIN, OUTER_MARGIN + si * (perStripH + STRIP_GAP));
    }
    pageCanvases.push(canvas);

  } else {
    // ── A4 / US LETTER pages (issue #24, upstream) ──────────────────────────
    // Fixed paper-shaped canvases at print density; strips scale to the
    // printable width and paginate top-to-bottom. Header (user prefs) on page 1
    // only; "OpenEnlarge" footer + page number on every page.
    const { w: pageW, h: pageH, pxPerMm } = paperPx(paper, scale);
    const margin = PAGE_MARGIN_MM * pxPerMm;
    const printableW = pageW - 2 * margin;
    const k = printableW / stripContentW; // design px → page px for strip blocks
    const stripHpx = perStripH * k;
    const gapPx = STRIP_GAP * k;

    // Header content — free-text prefs; empty fields drop out of the block.
    const mono = "'Spline Sans Mono', ui-monospace, monospace";
    const photographer = get(sheetHeaderPhotographer).trim();
    const detail = [get(sheetHeaderCamera), get(sheetHeaderFilm), get(sheetHeaderDate)]
      .map((s) => s.trim()).filter(Boolean).join(" · ");
    // TODO(issue #24): optional PNG logo/signature next to the header. Skipped
    // for now — the webview can't read an arbitrary local file without a new
    // Rust command (no fs plugin, no asset protocol; reference_thumb re-encodes
    // to a tiny lossy JPEG). Revisit when a byte-accurate file-read API lands.
    const titleFont = 4.2 * pxPerMm;   // ≈12pt at any print density
    const detailFont = 3.0 * pxPerMm;
    const footerFont = 2.8 * pxPerMm;
    const titleLineH = titleFont * 1.5;
    const detailLineH = detailFont * 1.8;
    let headerH = (photographer ? titleLineH : 0) + (detail ? detailLineH : 0);
    if (headerH > 0) headerH += 4 * pxPerMm; // rule + air below the block
    const footerH = 6 * pxPerMm;

    // Strips-per-page: page 1 loses the header's height, later pages don't.
    const pageCounts = paginateStrips(
      strips.length, stripHpx, gapPx,
      pageH - 2 * margin - footerH - headerH,
      pageH - 2 * margin - footerH,
    );

    let stripIdx = 0;
    for (let pi = 0; pi < pageCounts.length; pi++) {
      const canvas = document.createElement("canvas");
      canvas.width = pageW;
      canvas.height = pageH;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("Could not get 2D canvas context");

      // Background — same darkroom paper as the strip export
      ctx.fillStyle = "#0b0b0c";
      ctx.fillRect(0, 0, pageW, pageH);

      let yPx = margin;

      // Header block — page 1 only
      if (pi === 0 && headerH > 0) {
        ctx.textAlign = "left";
        ctx.textBaseline = "top";
        let hy = yPx;
        if (photographer) {
          ctx.fillStyle = "#d8cfb8";
          ctx.font = `600 ${titleFont}px ${mono}`;
          ctx.letterSpacing = "0.06em";
          ctx.fillText(photographer, margin, hy);
          hy += titleLineH;
        }
        if (detail) {
          ctx.fillStyle = "#968f7c";
          ctx.font = `600 ${detailFont}px ${mono}`;
          ctx.letterSpacing = "0.12em";
          ctx.fillText(detail, margin, hy);
          hy += detailLineH;
        }
        ctx.letterSpacing = "0px";
        // Hairline rule under the block, spanning the printable width
        ctx.strokeStyle = "rgba(216,207,184,0.25)";
        ctx.lineWidth = Math.max(1, 0.15 * pxPerMm);
        ctx.beginPath();
        ctx.moveTo(margin, hy + 1 * pxPerMm);
        ctx.lineTo(pageW - margin, hy + 1 * pxPerMm);
        ctx.stroke();
        yPx += headerH;
      }

      // Strips — drawn in design coordinates through a uniform scale transform
      for (let n = 0; n < pageCounts[pi]; n++, stripIdx++) {
        ctx.setTransform(k, 0, 0, k, margin, yPx);
        drawStrip(ctx, strips[stripIdx], 0, 0);
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        yPx += stripHpx + gapPx;
      }

      // Footer — every page: dim wordmark, plus page number when paginated
      ctx.fillStyle = "#7a7464";
      ctx.font = `600 ${footerFont}px ${mono}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.letterSpacing = "0.24em";
      const footer = pageCounts.length > 1
        ? `OpenEnlarge · ${pi + 1}/${pageCounts.length}`
        : "OpenEnlarge";
      ctx.fillText(footer, pageW / 2, pageH - margin - footerH / 2);
      ctx.letterSpacing = "0px";

      pageCanvases.push(canvas);
    }
  }

  // ── Encode each page as a lossless PNG intermediate ───────────────────────
  // Always hand the backend lossless pixels; it re-encodes to the chosen format
  // (JPEG quality / PNG) — encoding JPEG here first would double-compress.
  const base64s = pageCanvases.map((c) => {
    const dataUrl = c.toDataURL("image/png");
    const comma = dataUrl.indexOf(",");
    return comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
  });

  // ── OS save dialog — ONE dialog for the base name; extra pages save as
  // -p1/-p2/… siblings next to it (issue #24, upstream) ──────────────────────
  const isJpeg = opts.format.kind === "jpeg";
  const ext = isJpeg ? "jpg" : "png";
  const path = await save({
    defaultPath: `contact-sheet.${ext}`,
    filters: [{ name: isJpeg ? "JPEG" : "PNG", extensions: [ext] }],
  });
  if (!path) return; // user cancelled

  // Write via the same Rust command used by AiEnhancePanel: it decodes the PNG
  // and re-encodes to opts.format (JPEG quality or PNG).
  for (let i = 0; i < base64s.length; i++) {
    await api.saveEnhanced(pagePath(path, i + 1, base64s.length), base64s[i], opts.format);
  }
}
