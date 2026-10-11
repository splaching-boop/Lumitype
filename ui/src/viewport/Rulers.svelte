<script lang="ts">
  import type { Camera } from "./camera";

  let {
    camera,
    pointer,
    onguidestart,
  }: {
    camera: Camera | null;
    pointer: { x: number; y: number } | null;
    /** Appui sur une règle : on en « tire » un repère, comme dans Photoshop. */
    onguidestart: (orientation: "horizontal" | "vertical", event: PointerEvent) => void;
  } = $props();

  let top = $state<HTMLCanvasElement>();
  let left = $state<HTMLCanvasElement>();
  let size = $state({ width: 0, height: 0 });

  const STEPS = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000];

  /** Couleurs et police lues une fois (lire les styles calculés à chaque image forçait une mise en page). */
  let palette: { tick: string; label: string; marker: string; font: string } | null = null;
  function readPalette(canvas: HTMLCanvasElement): void {
    const styles = getComputedStyle(canvas);
    palette = {
      tick: styles.getPropertyValue("--text-disabled"),
      label: styles.getPropertyValue("--text-muted"),
      marker: styles.getPropertyValue("--text"),
      font: `9px ${styles.getPropertyValue("--font")}`,
    };
  }

  /**
   * Graduations de la règle. Le canevas n'est réalloué qu'au changement de taille (le réallouer à chaque image
   * faisait recréer son calque par la webview, mille fois par seconde avec une souris rapide) ; le repère du
   * pointeur est un élément à part, déplacé sans redessiner les graduations.
   */
  function draw(canvas: HTMLCanvasElement, horizontal: boolean, cam: Camera): void {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = window.devicePixelRatio;
    const length = horizontal ? size.width : size.height;
    const thickness = horizontal ? canvas.clientHeight : canvas.clientWidth;
    const width = Math.round((horizontal ? length : thickness) * dpr);
    const height = Math.round((horizontal ? thickness : length) * dpr);
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
      palette = null;
    } else {
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.clearRect(0, 0, width, height);
    }
    if (!palette) readPalette(canvas);
    const colors = palette!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.strokeStyle = colors.tick;
    ctx.fillStyle = colors.label;
    ctx.font = colors.font;
    ctx.lineWidth = 1;

    // Comme Photoshop : valeurs absolues de part et d'autre du zéro.
    // Graduation principale : au moins 64 px d'écran entre deux libellés.
    const major = STEPS.find((step) => step * cam.zoom >= 64) ?? 10000;
    const minor = major / (major === 2 || major === 20 || major === 200 || major === 2000 ? 4 : 5);
    const origin = horizontal ? cam.x : cam.y;
    const first = Math.floor(-origin / cam.zoom / minor) * minor;

    ctx.beginPath();
    for (let value = first; origin + value * cam.zoom <= length; value += minor) {
      const pos = Math.round(origin + value * cam.zoom) + 0.5;
      const isMajor = Math.abs(value / major - Math.round(value / major)) < 1e-6;
      const tick = isMajor ? thickness : 5;
      if (horizontal) {
        ctx.moveTo(pos, thickness);
        ctx.lineTo(pos, thickness - tick);
        if (isMajor) ctx.fillText(String(Math.abs(Math.round(value))), pos + 3, 9);
      } else {
        ctx.moveTo(thickness, pos);
        ctx.lineTo(thickness - tick, pos);
        if (isMajor) {
          ctx.save();
          ctx.translate(9, pos + 3);
          ctx.rotate(-Math.PI / 2);
          ctx.fillText(String(Math.abs(Math.round(value))), 0, 0);
          ctx.restore();
        }
      }
    }
    ctx.stroke();
  }

  // Graduations : redessinées quand la vue ou la taille change — pas quand le pointeur bouge.
  $effect(() => {
    if (!top || !left || !camera || size.width === 0) return;
    draw(top, true, camera);
    draw(left, false, camera);
  });

  /** Position du repère du pointeur sur chaque règle, en pixels CSS (`null` : hors de la zone). */
  const marker = $derived(camera && pointer ? { x: Math.round(camera.x + pointer.x * camera.zoom), y: Math.round(camera.y + pointer.y * camera.zoom) } : null);
</script>

<div class="corner"></div>
<!-- La règle du haut donne des repères horizontaux, celle de gauche des repères verticaux. -->
<div class="top" role="presentation" bind:clientWidth={size.width} onpointerdown={(event) => onguidestart("horizontal", event)}>
  <canvas bind:this={top}></canvas>
  {#if marker}<i class="marker h" style:transform="translateX({marker.x}px)"></i>{/if}
</div>
<div class="left" role="presentation" bind:clientHeight={size.height} onpointerdown={(event) => onguidestart("vertical", event)}>
  <canvas bind:this={left}></canvas>
  {#if marker}<i class="marker v" style:transform="translateY({marker.y}px)"></i>{/if}
</div>

<style>
  .corner,
  .top,
  .left {
    background: var(--bg-panel);
  }
  .corner {
    grid-area: corner;
    border-right: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .top {
    cursor: row-resize;
    grid-area: top;
    border-bottom: 1px solid var(--border);
    overflow: hidden;
  }
  .left {
    cursor: col-resize;
    grid-area: left;
    border-right: 1px solid var(--border);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .top,
  .left {
    position: relative;
  }
  /* Repère du pointeur : un trait déplacé par transformation, sans redessiner la règle. */
  .marker {
    position: absolute;
    left: 0;
    top: 0;
    background: var(--text);
    pointer-events: none;
    will-change: transform;
  }
  .marker.h {
    width: 1px;
    height: 100%;
  }
  .marker.v {
    height: 1px;
    width: 100%;
  }
</style>
