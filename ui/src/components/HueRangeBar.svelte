<script lang="ts" module>
  import { t } from "../lib/i18n.svelte";
  /** Transition minimale : en dessous, les triangles passeraient sous les barres claires. */
  export const MIN_FEATHER = 16;
  export const MAX_FEATHER = 170;
  /** Largeur pleine : assez pour distinguer les deux barres, jamais au point de faire le tour. */
  export const MIN_SPAN = 16;
  export const MAX_SPAN = 300;
  /** Marge laissée libre : les deux transitions ne se rejoignent pas dans le dos de la plage. */
  const FREE = 16;

  export interface HueRange {
    start: number;
    end: number;
    featherIn: number;
    featherOut: number;
  }

  /** Distance en avant sur le cercle, de `a` vers `b`, en degrés. */
  export const forward = (a: number, b: number): number => (((b - a) % 360) + 360) % 360;
  export const span = (range: HueRange): number => forward(range.start, range.end);

  /** Replie les transitions quand la plage s'élargit, pour qu'elles ne se rejoignent pas derrière. */
  export function fitFeathers(range: HueRange): HueRange {
    const room = Math.max(MIN_FEATHER, (360 - FREE - span(range)) / 2);
    return {
      ...range,
      featherIn: Math.max(MIN_FEATHER, Math.min(range.featherIn, room)),
      featherOut: Math.max(MIN_FEATHER, Math.min(range.featherOut, room)),
    };
  }
</script>

<script lang="ts">
  // Barre des couleurs visées par Teinte/Saturation, façon Photoshop : les deux barres claires
  // bornent la zone corrigée en entier, les deux triangles la largeur des transitions. Glisser entre
  // les barres déplace toute la plage, y compris en passant par 0°.
  let { range, onchange, onsettle }: { range: HueRange; onchange: (next: HueRange) => void; onsettle: () => void } = $props();

  type Grip = "start" | "end" | "featherIn" | "featherOut" | "move";

  const percent = (degrees: number) => ((((degrees % 360) + 360) % 360) / 360) * 100;
  const round = (degrees: number) => Math.round((((degrees % 360) + 360) % 360) * 10) / 10;

  /** Un segment coupé en deux quand il passe par 0° : la barre est un cercle déroulé. */
  function pieces(from: number, to: number): { left: number; width: number }[] {
    if (forward(from, to) <= 0.01) return [];
    const [a, b] = [percent(from), percent(to)];
    return a <= b ? [{ left: a, width: b - a }] : [{ left: a, width: 100 - a }, { left: 0, width: b }];
  }

  const fadeIn = $derived(pieces(range.start - range.featherIn, range.start));
  const core = $derived(pieces(range.start, range.end));
  const fadeOut = $derived(pieces(range.end, range.end + range.featherOut));
  const handles = $derived([
    { key: "featherIn" as Grip, at: range.start - range.featherIn, kind: "tri" },
    { key: "start" as Grip, at: range.start, kind: "grip" },
    { key: "end" as Grip, at: range.end, kind: "grip" },
    { key: "featherOut" as Grip, at: range.end + range.featherOut, kind: "tri" },
  ]);

  let bar = $state<HTMLElement>();

  function drag(event: PointerEvent, grip: Grip): void {
    if (!bar) return;
    event.preventDefault();
    event.stopPropagation();
    const box = bar.getBoundingClientRect();
    const degreesAt = (x: number) => ((((x - box.left) / box.width) * 360 % 360) + 360) % 360;
    const from = degreesAt(event.clientX);
    const snapshot = { ...range };
    bar.setPointerCapture(event.pointerId);

    const move = (e: PointerEvent) => {
      const at = degreesAt(e.clientX);
      if (grip === "start") {
        const width = forward(at, snapshot.end);
        if (width >= MIN_SPAN && width <= MAX_SPAN) onchange(fitFeathers({ ...snapshot, start: at }));
      } else if (grip === "end") {
        const width = forward(snapshot.start, at);
        if (width >= MIN_SPAN && width <= MAX_SPAN) onchange(fitFeathers({ ...snapshot, end: at }));
      } else if (grip === "featherIn") {
        onchange({ ...snapshot, featherIn: clampFeather(forward(at, snapshot.start), snapshot, snapshot.featherOut) });
      } else if (grip === "featherOut") {
        onchange({ ...snapshot, featherOut: clampFeather(forward(snapshot.end, at), snapshot, snapshot.featherIn) });
      } else {
        // Déplacement de toute la plage : le plus court chemin, pour ne pas sauter d'un tour.
        const ahead = forward(from, at);
        const shift = ahead > 180 ? ahead - 360 : ahead;
        onchange({ ...snapshot, start: (snapshot.start + shift + 360) % 360, end: (snapshot.end + shift + 360) % 360 });
      }
    };
    const stop = () => {
      bar?.removeEventListener("pointermove", move);
      bar?.removeEventListener("pointerup", stop);
      onsettle();
    };
    bar.addEventListener("pointermove", move);
    bar.addEventListener("pointerup", stop);
  }

  const clampFeather = (value: number, current: HueRange, other: number): number =>
    Math.min(MAX_FEATHER, Math.max(MIN_FEATHER, Math.min(value, 360 - FREE - span(current) - other)));
</script>

<div class="bar" bind:this={bar} onpointerdown={(event) => event.button === 0 && drag(event, "move")} role="presentation">
  {#each fadeIn as piece, i (i)}
    <span class="fade in" style:left="{piece.left}%" style:width="{piece.width}%"></span>
  {/each}
  {#each core as piece, i (i)}
    <span class="core" style:left="{piece.left}%" style:width="{piece.width}%"></span>
  {/each}
  {#each fadeOut as piece, i (i)}
    <span class="fade out" style:left="{piece.left}%" style:width="{piece.width}%"></span>
  {/each}
  {#each handles as handle (handle.key)}
    <button
      class={handle.kind}
      style:left="{percent(handle.at)}%"
      aria-label={handle.key === "start" ? t("Début de la plage") : handle.key === "end" ? t("Fin de la plage") : t("Largeur de la transition")}
      onpointerdown={(event) => event.button === 0 && drag(event, handle.key)}
    ></button>
  {/each}
</div>
<div class="degrees">
  <span>{round(range.start - range.featherIn)}° / {round(range.start)}°</span>
  <span>{round(range.end)}° / {round(range.end + range.featherOut)}°</span>
</div>

<style>
  .bar {
    position: relative;
    height: 22px;
    margin-top: 2px;
    cursor: grab;
    touch-action: none;
  }
  .core,
  .fade {
    position: absolute;
    top: 5px;
    height: 12px;
    border-radius: var(--radius-xs);
    pointer-events: none;
  }
  .core {
    background: #cfcfd4;
  }
  .fade.in {
    background: linear-gradient(90deg, rgba(207, 207, 212, 0), #cfcfd4);
  }
  .fade.out {
    background: linear-gradient(90deg, #cfcfd4, rgba(207, 207, 212, 0));
  }
  .grip,
  .tri {
    position: absolute;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: ew-resize;
  }
  .grip {
    top: 2px;
    width: 5px;
    height: 18px;
    margin-left: -2.5px;
    border-radius: var(--radius-xs);
    background: #f4f4f5;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.65);
  }
  .tri {
    top: 7px;
    width: 0;
    height: 0;
    margin-left: -5px;
    border-left: 5px solid transparent;
    border-right: 5px solid transparent;
    border-bottom: 9px solid #9a9aa2;
  }
  .degrees {
    display: flex;
    justify-content: space-between;
    margin-top: 1px;
    color: var(--text-3);
    font-size: 11px;
  }
</style>
