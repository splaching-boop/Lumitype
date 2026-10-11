<script lang="ts">
  import { panelOwnsKeyboard } from "../lib/keymap";
  import { t } from "../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { edit } from "../lib/engine";
  import type { DocumentInfo } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import type { Camera } from "./camera";

  // Recadrage : un cadre à huit poignées sur l'image, tout ce qui est dehors est assombri. Le cadre
  // peut dépasser du document pour agrandir la zone de travail. Entrée ou double-clic valide,
  // Échap remet le cadre sur tout le document. Rien n'est détruit : les calques gardent ce qui sort du cadre.
  let { info, camera }: { info: DocumentInfo; camera: Camera } = $props();

  interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
  }
  const whole = (): Box => ({ x: 0, y: 0, w: info.width, h: info.height });
  let box = $state<Box>(untrack(whole));
  let root = $state<SVGSVGElement>();
  /** Poignée tirée : facteurs -1, 0, 1 sur chaque axe ; (0, 0) = tout le cadre. */
  let drag: { fx: number; fy: number; start: { x: number; y: number }; from: Box } | null = null;

  // Le document a changé de taille (recadrage appliqué, annulation) : le cadre repart de tout le document.
  $effect(() => {
    void info.width;
    void info.height;
    box = untrack(whole);
  });

  // Proportions imposées depuis la barre d'options : le cadre s'y plie tout de suite, autour de son centre.
  $effect(() => {
    const ratio = app.cropRatio;
    if (ratio === null) return;
    untrack(() => {
      const [cx, cy] = [box.x + box.w / 2, box.y + box.h / 2];
      const w = Math.min(box.w, box.h * ratio);
      box = { x: Math.round(cx - w / 2), y: Math.round(cy - w / ratio / 2), w: Math.round(w), h: Math.round(w / ratio) };
    });
  });

  const screen = $derived({ x: camera.x + box.x * camera.zoom, y: camera.y + box.y * camera.zoom, w: box.w * camera.zoom, h: box.h * camera.zoom });
  const HANDLES = [-1, 0, 1].flatMap((fy) => [-1, 0, 1].filter((fx) => fx !== 0 || fy !== 0).map((fx) => ({ fx, fy })));
  const CURSORS = ["nwse-resize", "ns-resize", "nesw-resize", "ew-resize", "", "ew-resize", "nesw-resize", "ns-resize", "nwse-resize"];

  function docPoint(event: PointerEvent): { x: number; y: number } {
    const rect = root?.getBoundingClientRect();
    return { x: (event.clientX - (rect?.left ?? 0) - camera.x) / camera.zoom, y: (event.clientY - (rect?.top ?? 0) - camera.y) / camera.zoom };
  }

  function begin(event: PointerEvent, fx: number, fy: number): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    root?.setPointerCapture(event.pointerId);
    drag = { fx, fy, start: docPoint(event), from: { ...box } };
  }

  function onPointerMove(event: PointerEvent): void {
    if (!drag) return;
    const p = docPoint(event);
    const [dx, dy] = [p.x - drag.start.x, p.y - drag.start.y];
    const { from, fx, fy } = drag;
    if (fx === 0 && fy === 0) {
      box = { ...from, x: Math.round(from.x + dx), y: Math.round(from.y + dy) };
      return;
    }
    let [x0, y0, x1, y1] = [from.x, from.y, from.x + from.w, from.y + from.h];
    if (fx < 0) x0 = Math.min(x1 - 1, x0 + dx);
    if (fx > 0) x1 = Math.max(x0 + 1, x1 + dx);
    if (fy < 0) y0 = Math.min(y1 - 1, y0 + dy);
    if (fy > 0) y1 = Math.max(y0 + 1, y1 + dy);
    // Proportions imposées (ou Maj : celles du cadre de départ) : l'axe le moins tiré suit l'autre.
    const ratio = app.cropRatio ?? (event.shiftKey ? from.w / from.h : null);
    if (ratio !== null) {
      let [w, h] = [x1 - x0, y1 - y0];
      if (fy === 0 || (fx !== 0 && w / ratio > h)) h = w / ratio;
      else w = h * ratio;
      // Le côté opposé à la poignée reste fixe ; pour une poignée de milieu, le cadre reste centré sur l'autre axe.
      if (fx < 0) x0 = x1 - w;
      else if (fx > 0) x1 = x0 + w;
      else [x0, x1] = [(x0 + x1 - w) / 2, (x0 + x1 + w) / 2];
      if (fy < 0) y0 = y1 - h;
      else if (fy > 0) y1 = y0 + h;
      else [y0, y1] = [(y0 + y1 - h) / 2, (y0 + y1 + h) / 2];
    }
    box = { x: Math.round(x0), y: Math.round(y0), w: Math.max(1, Math.round(x1 - x0)), h: Math.max(1, Math.round(y1 - y0)) };
  }

  function apply(): void {
    const same = box.x === 0 && box.y === 0 && box.w === info.width && box.h === info.height;
    if (!same) void edit({ op: "crop", x: box.x, y: box.y, width: box.w, height: box.h }).then((ok) => ok && app.fitToWindow?.());
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.target instanceof HTMLInputElement || panelOwnsKeyboard()) return; // le clavier est à la boîte ouverte
    if (event.key === "Enter") apply();
    else if (event.key === "Escape") box = whole();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<svelte:window onkeydowncapture={onKeyDown} />

<svg
  bind:this={root}
  class="overlay"
  role="application"
  aria-label={t("Cadre de recadrage : Entrée pour valider, Échap pour réinitialiser")}
  onpointermove={onPointerMove}
  onpointerup={() => (drag = null)}
  onpointercancel={() => (drag = null)}
  ondblclick={apply}
>
  <!-- Tout ce qui sera retiré est assombri : un rectangle plein, évidé à l'emplacement du cadre. -->
  <path class="shade" fill-rule="evenodd" d="M-10000 -10000H20000V20000H-10000ZM{screen.x} {screen.y}h{screen.w}v{screen.h}h{-screen.w}Z" />
  <rect class="move" x={screen.x} y={screen.y} width={screen.w} height={screen.h} role="presentation" onpointerdown={(event) => begin(event, 0, 0)} />
  <!-- Règle des tiers. -->
  {#each [1, 2] as third (third)}
    <line class="third" x1={screen.x + (screen.w * third) / 3} y1={screen.y} x2={screen.x + (screen.w * third) / 3} y2={screen.y + screen.h} />
    <line class="third" x1={screen.x} y1={screen.y + (screen.h * third) / 3} x2={screen.x + screen.w} y2={screen.y + (screen.h * third) / 3} />
  {/each}
  <rect class="frame" x={screen.x} y={screen.y} width={screen.w} height={screen.h} />
  {#each HANDLES as handle (`${handle.fx}/${handle.fy}`)}
    <rect
      class="handle"
      role="presentation"
      x={screen.x + (screen.w * (handle.fx + 1)) / 2 - 5}
      y={screen.y + (screen.h * (handle.fy + 1)) / 2 - 5}
      width="10"
      height="10"
      style:cursor={CURSORS[(handle.fy + 1) * 3 + handle.fx + 1]}
      onpointerdown={(event) => begin(event, handle.fx, handle.fy)}
    />
  {/each}
  <text class="size" x={screen.x + screen.w / 2} y={screen.y - 10}>{t("{w} × {h} px", { w: box.w, h: box.h })}</text>
</svg>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
  }
  .shade {
    fill: rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  .move {
    fill: transparent;
    cursor: move;
  }
  .frame {
    fill: none;
    stroke: #fff;
    stroke-width: 1;
    pointer-events: none;
  }
  .third {
    stroke: rgba(255, 255, 255, 0.35);
    stroke-width: 1;
    pointer-events: none;
  }
  .handle {
    fill: #fff;
    stroke: rgba(0, 0, 0, 0.6);
    stroke-width: 1;
  }
  .size {
    fill: #fff;
    font-size: 12px;
    text-anchor: middle;
    paint-order: stroke;
    stroke: rgba(0, 0, 0, 0.7);
    stroke-width: 3px;
    pointer-events: none;
  }
</style>
