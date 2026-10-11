<script lang="ts">
  import { hsvToRgb, type Hsv } from "../lib/color";

  // Roue de teinte + triangle saturation/luminosité. Comme dans Photoshop, le triangle est FIXE :
  // couleur pure à droite, blanc en haut à gauche, noir en bas à gauche. Seul son contenu change
  // avec la teinte ; c'est le repère sur l'anneau qui tourne.
  let { hsv, size = 176, onchange }: { hsv: Hsv; size?: number; onchange: (next: Hsv) => void } = $props();

  const RING = $derived(Math.round(size * 0.085));
  const OUTER = $derived(size / 2);
  const INNER = $derived(OUTER - RING);
  const TRIANGLE = $derived(INNER - 5);

  let canvas = $state<HTMLCanvasElement>();
  let dragging = $state<"ring" | "triangle" | null>(null);
  /** Sommets du triangle en coordonnées du canevas : couleur pure, blanc, noir. */
  function vertices(): [number, number][] {
    return [0, -120, 120].map((offset) => {
      const angle = (offset * Math.PI) / 180;
      return [OUTER + TRIANGLE * Math.cos(angle), OUTER + TRIANGLE * Math.sin(angle)] as [number, number];
    });
  }

  /** Poids barycentriques (couleur, blanc, noir) d'un point. */
  function weights(x: number, y: number, [a, b, c]: [number, number][]): [number, number, number] {
    if (!a || !b || !c) return [1, 0, 0];
    const det = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    const wa = ((b[1] - c[1]) * (x - c[0]) + (c[0] - b[0]) * (y - c[1])) / det;
    const wb = ((c[1] - a[1]) * (x - c[0]) + (a[0] - c[0]) * (y - c[1])) / det;
    return [wa, wb, 1 - wa - wb];
  }

  function draw(): void {
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    const dpr = window.devicePixelRatio;
    const pixels = Math.round(size * dpr);
    canvas.width = pixels;
    canvas.height = pixels;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    // Anneau : la teinte croît dans le sens trigonométrique, rouge à droite.
    const ring = ctx.createConicGradient(0, OUTER, OUTER);
    for (let i = 0; i <= 12; i++) ring.addColorStop(i / 12, `hsl(${360 - i * 30} 100% 50%)`);
    ctx.fillStyle = ring;
    ctx.beginPath();
    ctx.arc(OUTER, OUTER, OUTER, 0, Math.PI * 2);
    ctx.arc(OUTER, OUTER, INNER, 0, Math.PI * 2, true);
    ctx.fill();

    // Triangle : calculé pixel par pixel hors écran, puis découpé par un tracé pour des bords lisses.
    const corners = vertices();
    const pure = hsvToRgb({ h: hsv.h, s: 1, v: 1 });
    const image = new ImageData(pixels, pixels);
    for (let py = 0; py < pixels; py++) {
      for (let px = 0; px < pixels; px++) {
        const [wc, ww] = weights((px + 0.5) / dpr, (py + 0.5) / dpr, corners);
        const c = Math.max(0, wc);
        const w = Math.max(0, ww);
        const i = (py * pixels + px) * 4;
        image.data[i] = pure[0] * c + 255 * w;
        image.data[i + 1] = pure[1] * c + 255 * w;
        image.data[i + 2] = pure[2] * c + 255 * w;
        image.data[i + 3] = 255;
      }
    }
    const offscreen = new OffscreenCanvas(pixels, pixels);
    offscreen.getContext("2d")?.putImageData(image, 0, 0);
    ctx.save();
    ctx.beginPath();
    corners.forEach(([x, y], i) => (i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y)));
    ctx.closePath();
    ctx.clip();
    ctx.drawImage(offscreen, 0, 0, size, size);
    ctx.restore();

    // Repères : teinte sur l'anneau, couleur dans le triangle.
    const angle = (-hsv.h * Math.PI) / 180;
    const middle = (OUTER + INNER) / 2;
    const [wc, ww, wb] = [hsv.s * hsv.v, hsv.v * (1 - hsv.s), 1 - hsv.v];
    const [a, b, c] = corners;
    const marks: [number, number][] = [[OUTER + middle * Math.cos(angle), OUTER + middle * Math.sin(angle)]];
    if (a && b && c) marks.push([a[0] * wc + b[0] * ww + c[0] * wb, a[1] * wc + b[1] * ww + c[1] * wb]);
    for (const [x, y] of marks) {
      ctx.beginPath();
      ctx.arc(x, y, 5, 0, Math.PI * 2);
      ctx.lineWidth = 3;
      ctx.strokeStyle = "rgba(0, 0, 0, 0.55)";
      ctx.stroke();
      ctx.lineWidth = 1.5;
      ctx.strokeStyle = "#fff";
      ctx.stroke();
    }
  }

  $effect(() => {
    void hsv.h;
    void hsv.s;
    void hsv.v;
    void size;
    draw();
  });

  function pick(event: PointerEvent, start: boolean): void {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return;
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const radius = Math.hypot(x - OUTER, y - OUTER);
    if (start) dragging = radius >= INNER ? "ring" : "triangle";

    if (dragging === "ring") {
      const hue = ((-Math.atan2(y - OUTER, x - OUTER) * 180) / Math.PI + 360) % 360;
      onchange({ ...hsv, h: hue });
    } else if (dragging === "triangle") {
      // Hors du triangle : poids négatifs ramenés à zéro puis renormalisés.
      const raw = weights(x, y, vertices()).map((w) => Math.max(0, w));
      const total = raw.reduce((sum, w) => sum + w, 0) || 1;
      const wc = (raw[0] ?? 0) / total;
      const ww = (raw[1] ?? 0) / total;
      const v = wc + ww;
      onchange({ h: hsv.h, s: v > 0 ? wc / v : 0, v });
    }
  }

  function onPointerDown(event: PointerEvent): void {
    canvas?.setPointerCapture(event.pointerId);
    pick(event, true);
  }
</script>

<canvas
  bind:this={canvas}
  style:width="{size}px"
  style:height="{size}px"
  onpointerdown={onPointerDown}
  onpointermove={(event) => dragging && pick(event, false)}
  onpointerup={() => (dragging = null)}
  onpointercancel={() => (dragging = null)}
></canvas>

<style>
  canvas {
    align-self: center;
    flex-shrink: 0;
    touch-action: none;
    cursor: crosshair;
  }
</style>
