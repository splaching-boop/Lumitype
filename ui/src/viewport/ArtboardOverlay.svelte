<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { artboardsOf, addArtboard, duplicateArtboard, moveArtboard, setArtboard, activeArtboard } from "../lib/artboards.svelte";
  import type { Artboard, DocumentInfo, LayerInfo } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import { type Camera, clientToDoc } from "./camera";

  // Plans de travail sur la zone de travail : le nom de chaque plan au-dessus de lui (un clic le sélectionne), son
  // cadre, et l'espace entre les plans assombri comme une table de montage. Avec l'outil Plan de travail : glisser
  // dans le vide crée un plan, glisser un plan le déplace avec son contenu, les poignées le redimensionnent, et les
  // boutons « + » ajoutent un plan du même format à droite ou en dessous (Alt : avec son contenu).
  let { info, camera, interactive }: { info: DocumentInfo; camera: Camera; interactive: boolean } = $props();

  type Board = LayerInfo & { artboard: Artboard };
  const boards = $derived(artboardsOf(info.layers));
  const active = $derived.by(() => {
    void app.activeLayer;
    return activeArtboard();
  });
  let root = $state<SVGSVGElement>();
  /** Geste en cours : déplacement, poignée, ou création d'un plan. */
  let drag = $state<
    | { kind: "move"; board: Board; start: { x: number; y: number }; gesture: string; dx: number; dy: number }
    | { kind: "resize"; board: Board; fx: number; fy: number; start: { x: number; y: number }; gesture: string; box: Artboard }
    | { kind: "create"; start: { x: number; y: number }; end: { x: number; y: number } }
    | null
  >(null);

  const toScreen = (b: { x: number; y: number; width: number; height: number }) => ({ x: camera.x + b.x * camera.zoom, y: camera.y + b.y * camera.zoom, w: b.width * camera.zoom, h: b.height * camera.zoom });
  /** Cadre affiché d'un plan, geste en cours compris (le moteur ne le confirme qu'au fil des étapes). */
  function shown(board: Board): Artboard {
    if (drag?.kind === "move" && drag.board.id === board.id) return { ...drag.board.artboard, x: drag.board.artboard.x + Math.round(drag.dx), y: drag.board.artboard.y + Math.round(drag.dy) };
    if (drag?.kind === "resize" && drag.board.id === board.id) return drag.box;
    return board.artboard;
  }

  function docPoint(event: PointerEvent): { x: number; y: number } {
    const mapped = clientToDoc(event.clientX, event.clientY);
    if (mapped) return mapped;
    const rect = root?.getBoundingClientRect();
    return { x: (event.clientX - (rect?.left ?? 0) - camera.x) / camera.zoom, y: (event.clientY - (rect?.top ?? 0) - camera.y) / camera.zoom };
  }

  function select(board: Board): void {
    if (app.active) app.active.selection = [board.id];
  }

  function beginMove(event: PointerEvent, board: Board): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    select(board);
    if (!interactive) return;
    root?.setPointerCapture(event.pointerId);
    drag = { kind: "move", board, start: docPoint(event), gesture: `plan-${board.id}-${Date.now()}`, dx: 0, dy: 0 };
  }

  function beginResize(event: PointerEvent, board: Board, fx: number, fy: number): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    root?.setPointerCapture(event.pointerId);
    drag = { kind: "resize", board, fx, fy, start: docPoint(event), gesture: `plan-taille-${board.id}-${Date.now()}`, box: { ...board.artboard } };
  }

  function beginCreate(event: PointerEvent): void {
    if (event.button !== 0 || !interactive) return;
    root?.setPointerCapture(event.pointerId);
    const p = docPoint(event);
    drag = { kind: "create", start: p, end: p };
  }

  function onMove(event: PointerEvent): void {
    if (!drag) return;
    const p = docPoint(event);
    if (drag.kind === "move") {
      let [dx, dy] = [p.x - drag.start.x, p.y - drag.start.y];
      if (event.shiftKey) [dx, dy] = Math.abs(dx) > Math.abs(dy) ? [dx, 0] : [0, dy];
      drag.dx = dx;
      drag.dy = dy;
      void moveArtboard(drag.board.id, drag.board.artboard, dx, dy, drag.gesture, false);
    } else if (drag.kind === "resize") {
      const from = drag.board.artboard;
      const [dx, dy] = [p.x - drag.start.x, p.y - drag.start.y];
      let [x0, y0, x1, y1] = [from.x, from.y, from.x + from.width, from.y + from.height];
      if (drag.fx < 0) x0 = Math.min(x1 - 1, x0 + dx);
      if (drag.fx > 0) x1 = Math.max(x0 + 1, x1 + dx);
      if (drag.fy < 0) y0 = Math.min(y1 - 1, y0 + dy);
      if (drag.fy > 0) y1 = Math.max(y0 + 1, y1 + dy);
      drag.box = { ...from, x: Math.round(x0), y: Math.round(y0), width: Math.max(1, Math.round(x1 - x0)), height: Math.max(1, Math.round(y1 - y0)), preset: null };
      void setArtboard(drag.board.id, drag.box, drag.gesture, false);
    } else drag.end = p;
  }

  async function onUp(): Promise<void> {
    const ended = drag;
    drag = null;
    if (!ended) return;
    if (ended.kind === "move") {
      if (ended.dx !== 0 || ended.dy !== 0) await moveArtboard(ended.board.id, ended.board.artboard, ended.dx, ended.dy, ended.gesture, true);
    } else if (ended.kind === "resize") {
      await setArtboard(ended.board.id, ended.box, ended.gesture, true);
    } else {
      const [x0, y0] = [Math.min(ended.start.x, ended.end.x), Math.min(ended.start.y, ended.end.y)];
      const [w, h] = [Math.abs(ended.end.x - ended.start.x), Math.abs(ended.end.y - ended.start.y)];
      // Un simple clic dans le vide ne crée rien (moins de 8 pixels à l'écran).
      if (w * camera.zoom < 8 || h * camera.zoom < 8) return;
      await addArtboard(t("Plan de travail {n}", { n: boards.length + 1 }), { x: Math.round(x0), y: Math.round(y0), width: Math.round(w), height: Math.round(h), background: [255, 255, 255], clip: true, preset: null });
    }
  }

  const HANDLES = [-1, 0, 1].flatMap((fy) => [-1, 0, 1].filter((fx) => fx !== 0 || fy !== 0).map((fx) => ({ fx, fy })));
  const CURSORS = ["nwse-resize", "ns-resize", "nesw-resize", "ew-resize", "", "ew-resize", "nesw-resize", "ns-resize", "nwse-resize"];
  const docScreen = $derived(toScreen({ x: 0, y: 0, width: info.width, height: info.height }));
  /**
   * Table de montage : le document moins les plans. Le cadre est découpé en cellules par les bords des plans ; les
   * cellules qu'aucun plan ne couvre sont dessinées (des plans qui se chevauchent restent bien évidés).
   */
  const pasteboard = $derived.by(() => {
    // Bords au pixel entier : les plans rétrécis (plafond à gauche et en haut, plancher à droite et en bas) et le
    // document élargi. La table recouvre ainsi le bord de chaque plan d'une fraction de pixel au lieu de s'y arrêter
    // juste : arrêtée au plus proche, elle laissait selon le zoom et la position de la vue un filet de canevas
    // découvert le long des plans (le canevas est en pixels d'écran, la table en pixels CSS : 1,25 pour 1 à 125 %).
    const inner = (r: { x: number; y: number; w: number; h: number }) => {
      const [x0, y0] = [Math.ceil(r.x), Math.ceil(r.y)];
      return { x: x0, y: y0, w: Math.max(0, Math.floor(r.x + r.w) - x0), h: Math.max(0, Math.floor(r.y + r.h) - y0) };
    };
    const rects = boards.map((b) => inner(toScreen(shown(b))));
    const d0 = docScreen;
    const d = { x: Math.floor(d0.x), y: Math.floor(d0.y), w: Math.ceil(d0.x + d0.w) - Math.floor(d0.x), h: Math.ceil(d0.y + d0.h) - Math.floor(d0.y) };
    const xs = [...new Set([d.x, d.x + d.w, ...rects.flatMap((r) => [r.x, r.x + r.w])])].filter((v) => v >= d.x && v <= d.x + d.w).sort((a, b) => a - b);
    const ys = [...new Set([d.y, d.y + d.h, ...rects.flatMap((r) => [r.y, r.y + r.h])])].filter((v) => v >= d.y && v <= d.y + d.h).sort((a, b) => a - b);
    let path = "";
    for (let j = 0; j + 1 < ys.length; j++) {
      for (let i = 0; i + 1 < xs.length; i++) {
        const [x0, x1, y0, y1] = [xs[i]!, xs[i + 1]!, ys[j]!, ys[j + 1]!];
        const [cx, cy] = [(x0 + x1) / 2, (y0 + y1) / 2];
        if (!rects.some((r) => cx > r.x && cx < r.x + r.w && cy > r.y && cy < r.y + r.h)) path += `M${x0} ${y0}H${x1}V${y1}H${x0}Z`;
      }
    }
    return path;
  });
  const sizeText = (b: Artboard) => `${b.width} × ${b.height}`;
</script>

<svg
  bind:this={root}
  class="overlay"
  class:interactive
  role="application"
  aria-label={t("Plans de travail")}
  onpointerdown={beginCreate}
  onpointermove={onMove}
  onpointerup={() => void onUp()}
  onpointercancel={() => (drag = null)}
>
  <path class="pasteboard" shape-rendering="crispEdges" d={pasteboard} />
  {#each boards as board (board.id)}
    {@const b = shown(board)}
    {@const s = toScreen(b)}
    {@const isActive = active?.id === board.id}
    {#if interactive}
      <rect class="body" x={s.x} y={s.y} width={s.w} height={s.h} role="presentation" onpointerdown={(event) => beginMove(event, board)} />
    {/if}
    <!-- Seul le plan actif a un cadre (comme Photoshop et Figma : un plan inactif n'est marqué que par son nom). Il est
         calé sur la grille des pixels : à un zoom fractionnaire, un trait posé entre deux pixels s'étale en liseré flou. -->
    {#if isActive}
      <rect class="frame" shape-rendering="crispEdges" x={Math.round(s.x)} y={Math.round(s.y)} width={Math.max(1, Math.round(s.x + s.w) - Math.round(s.x))} height={Math.max(1, Math.round(s.y + s.h) - Math.round(s.y))} />
    {/if}
    <!-- Vue miroir : le nom est retourné autour de son ancre (et ancré à la fin) pour rester lisible au même endroit. -->
    <text class="name" class:active={isActive} x={s.x} y={s.y - 7} text-anchor={app.viewMirrored ? "end" : undefined} transform={app.viewMirrored ? `translate(${2 * s.x} 0) scale(-1 1)` : undefined} role="button" tabindex="-1" onpointerdown={(event) => beginMove(event, board)}>{board.name}<tspan class="dim"> · {sizeText(b)}</tspan></text>
    {#if interactive && isActive && !drag}
      {#each HANDLES as handle (`${handle.fx}/${handle.fy}`)}
        <rect
          class="handle"
          role="presentation"
          x={s.x + (s.w * (handle.fx + 1)) / 2 - 4}
          y={s.y + (s.h * (handle.fy + 1)) / 2 - 4}
          width="8"
          height="8"
          style:cursor={CURSORS[(handle.fy + 1) * 3 + handle.fx + 1]}
          onpointerdown={(event) => beginResize(event, board, handle.fx, handle.fy)}
        />
      {/each}
      <g class="plus" role="button" tabindex="-1" aria-label={t("Plan du même format à droite (Alt : avec son contenu)")} onpointerdown={(event) => (event.stopPropagation(), void duplicateArtboard(board, "right", event.altKey))}>
        <circle cx={s.x + s.w + 22} cy={s.y + s.h / 2} r="10" />
        <path d="M{s.x + s.w + 17} {s.y + s.h / 2}h10M{s.x + s.w + 22} {s.y + s.h / 2 - 5}v10" />
      </g>
      <g class="plus" role="button" tabindex="-1" aria-label={t("Plan du même format en dessous (Alt : avec son contenu)")} onpointerdown={(event) => (event.stopPropagation(), void duplicateArtboard(board, "down", event.altKey))}>
        <circle cx={s.x + s.w / 2} cy={s.y + s.h + 22} r="10" />
        <path d="M{s.x + s.w / 2 - 5} {s.y + s.h + 22}h10M{s.x + s.w / 2} {s.y + s.h + 17}v10" />
      </g>
    {/if}
  {/each}
  {#if drag?.kind === "create"}
    {@const s = toScreen({ x: Math.min(drag.start.x, drag.end.x), y: Math.min(drag.start.y, drag.end.y), width: Math.abs(drag.end.x - drag.start.x), height: Math.abs(drag.end.y - drag.start.y) })}
    <rect class="draft" x={s.x} y={s.y} width={s.w} height={s.h} />
    <text class="name active" x={s.x} y={s.y - 7}>{Math.round(Math.abs(drag.end.x - drag.start.x))} × {Math.round(Math.abs(drag.end.y - drag.start.y))}</text>
  {/if}
</svg>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    touch-action: none;
    overflow: visible;
  }
  .overlay.interactive {
    pointer-events: all;
    cursor: crosshair;
  }
  .pasteboard {
    fill: var(--bg-canvas, #232327);
    pointer-events: none;
  }
  .body {
    fill: transparent;
    cursor: move;
  }
  .frame {
    fill: none;
    stroke: var(--accent, #e5343e);
    stroke-width: 2;
    pointer-events: none;
  }
  .name {
    fill: var(--text-muted);
    font-size: 11px;
    cursor: pointer;
    pointer-events: all;
    user-select: none;
  }
  .name.active {
    fill: var(--text-strong);
  }
  .name .dim {
    fill: var(--text-muted);
    opacity: 0.8;
  }
  .handle {
    fill: #fff;
    stroke: rgba(0, 0, 0, 0.6);
    stroke-width: 1;
  }
  .plus {
    cursor: pointer;
  }
  .plus circle {
    fill: var(--bg-panel);
    stroke: var(--border-strong);
  }
  .plus path {
    stroke: var(--text);
    stroke-width: 1.5;
  }
  .draft {
    fill: rgba(255, 255, 255, 0.08);
    stroke: var(--accent, #e5343e);
    stroke-dasharray: 4 3;
  }
</style>
