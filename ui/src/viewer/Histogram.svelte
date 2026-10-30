<script lang="ts">
  // Histogramme de l'image affichée : rouge, vert, bleu et luminance, calculés sur une réduction (256 px de côté).
  let { url }: { url: string | null } = $props();
  let paths = $state<{ r: string; g: string; b: string; l: string } | null>(null);
  const W = 264;
  const H = 72;

  function curve(bins: Uint32Array, peak: number): string {
    let d = `M0 ${H}`;
    for (let i = 0; i < 256; i++) d += ` L${((i / 255) * W).toFixed(1)} ${(H - (Math.sqrt((bins[i] ?? 0) / peak) * (H - 2))).toFixed(1)}`;
    return `${d} L${W} ${H} Z`;
  }

  $effect(() => {
    const source = url;
    paths = null;
    if (!source) return;
    const img = new Image();
    let cancelled = false;
    img.onload = () => {
      if (cancelled) return;
      const side = 256;
      const ratio = Math.min(1, side / Math.max(img.naturalWidth, img.naturalHeight));
      const w = Math.max(1, Math.round(img.naturalWidth * ratio));
      const h = Math.max(1, Math.round(img.naturalHeight * ratio));
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) return;
      ctx.drawImage(img, 0, 0, w, h);
      const data = ctx.getImageData(0, 0, w, h).data;
      const [r, g, b, l] = [new Uint32Array(256), new Uint32Array(256), new Uint32Array(256), new Uint32Array(256)];
      for (let i = 0; i + 3 < data.length; i += 4) {
        if (data[i + 3] === 0) continue;
        const [cr, cg, cb] = [data[i] ?? 0, data[i + 1] ?? 0, data[i + 2] ?? 0];
        r[cr]! += 1;
        g[cg]! += 1;
        b[cb]! += 1;
        l[Math.round(0.2126 * cr + 0.7152 * cg + 0.0722 * cb)]! += 1;
      }
      // Les deux extrêmes (noir et blanc purs) écraseraient le reste : le pic est pris entre les deux.
      let peak = 1;
      for (const bins of [r, g, b, l]) for (let i = 1; i < 255; i++) peak = Math.max(peak, bins[i] ?? 0);
      paths = { r: curve(r, peak), g: curve(g, peak), b: curve(b, peak), l: curve(l, peak) };
    };
    img.src = source;
    return () => {
      cancelled = true;
    };
  });
</script>

<svg class="histogram" viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
  {#if paths}
    <path d={paths.r} fill="rgba(236, 42, 60, 0.45)" />
    <path d={paths.g} fill="rgba(60, 200, 110, 0.35)" />
    <path d={paths.b} fill="rgba(60, 130, 240, 0.4)" />
    <path d={paths.l} fill="none" stroke="rgba(255, 255, 255, 0.55)" stroke-width="1" />
  {/if}
</svg>

<style>
  .histogram {
    width: 100%;
    height: 72px;
    background: var(--bg-field);
    mix-blend-mode: normal;
  }
  .histogram path {
    mix-blend-mode: screen;
  }
</style>
