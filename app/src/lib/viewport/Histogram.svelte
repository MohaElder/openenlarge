<script lang="ts">
  import { previewSrc } from "../store";
  import { binPixels, channelPath } from "./histogram";

  const W = 256, H = 76;
  let rPath = "", gPath = "", bPath = "";
  let timer: ReturnType<typeof setTimeout> | null = null;
  const cv = typeof document !== "undefined" ? document.createElement("canvas") : null;

  function compute(src: string) {
    if (!src || !cv) { rPath = gPath = bPath = ""; return; }
    const img = new Image();
    img.onload = () => {
      const w = 256, h = Math.max(1, Math.round((img.height / img.width) * 256));
      cv.width = w; cv.height = h;
      const ctx = cv.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(img, 0, 0, w, h);
      const { data } = ctx.getImageData(0, 0, w, h);
      const bins = binPixels(data);
      rPath = channelPath(bins.r, W, H);
      gPath = channelPath(bins.g, W, H);
      bPath = channelPath(bins.b, W, H);
    };
    img.src = src;
  }
  $: { const s = $previewSrc; if (timer) clearTimeout(timer); timer = setTimeout(() => compute(s), 120); }
</script>

<div class="hist">
  <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none">
    <polyline points={rPath} class="r" />
    <polyline points={gPath} class="g" />
    <polyline points={bPath} class="b" />
  </svg>
</div>

<style>
  .hist { height: 76px; border-radius: 8px; background: rgba(0,0,0,0.35);
    padding: 4px; margin-bottom: 10px; }
  svg { width: 100%; height: 100%; display: block; }
  polyline { fill: none; stroke-width: 1; mix-blend-mode: screen; }
  .r { stroke: #ff5a5a; } .g { stroke: #5aff7a; } .b { stroke: #5a9cff; }
</style>
