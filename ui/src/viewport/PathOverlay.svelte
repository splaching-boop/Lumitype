<script lang="ts">
  import { panelOwnsKeyboard } from "../lib/keymap";
  import { t } from "../lib/i18n.svelte";
  import cursorAdd from "../assets/cursors/pen-add.png";
  import cursorConvert from "../assets/cursors/pen-convert.png";
  import cursorDelete from "../assets/cursors/pen-delete.png";
  import cursorPen from "../assets/cursors/pen.png";
  import { untrack } from "svelte";
  import { isActionEnabled, runAction } from "../lib/actions";
  import ContextMenu, { type MenuEntry } from "../components/ContextMenu.svelte";
  import { edit } from "../lib/engine";
  import type { Anchor, SubPath } from "../lib/ipc";
  import { type Vec, corner, distance, insertAnchor, isCorner, nearestOnPath, toSvg, toggleSmooth } from "../lib/path";
  import { snapPoint } from "../lib/snapping.svelte";
  import { app } from "../state/app.svelte";
  import type { Camera } from "./camera";

  // Plume et sélection directe. Le tracé est édité ici, localement : rien ne part vers le moteur
  // pendant un glisser. À chaque manipulation terminée, le tracé complet lui est envoyé, ce qui
  // fait un état d'historique par geste (poser un point, déplacer une poignée…).
  let { paths, camera, tool }: { paths: SubPath[]; camera: Camera; tool: "pen" | "pathSelect" } = $props();

  const REACH = 7; // zone de saisie d'un point ou d'une poignée, en pixels d'écran

  /** Sous-tracé ouvert que la plume prolonge ; point sélectionné par la sélection directe. */
  let activeSub = $state<number | null>(null);
  let selected = $state<{ sub: number; index: number } | null>(null);
  /** Copie de travail ; resynchronisée quand le document change hors d'un geste (annulation, ouverture…). */
  let draft = $state<SubPath[]>([]);
  /**
   * Envois au moteur encore sans réponse. Tant qu'il en reste, le document reçu est celui d'un envoi
   * ANTÉRIEUR : le recopier dans la copie de travail effacerait ce qui a été posé depuis (un point
   * « décrochait » quand on cliquait vite). Un booléen ne suffisait pas : remis à faux dès l'envoi
   * suivant, il laissait passer la réponse en retard.
   */
  let inFlight = 0;

  type Drag =
    // `fresh` : le point vient d'être posé (sinon, c'est le dernier point repris pour régler sa courbe) ;
    // `moved` : les poignées ont bougé, il y a quelque chose à valider.
    | { kind: "newHandle"; sub: number; index: number; fresh: boolean; moved?: boolean }
    // Fermeture du tracé : tirer depuis le premier point arrondit le dernier segment.
    | { kind: "closeHandle"; sub: number }
    | { kind: "anchor"; sub: number; index: number; last: Vec }
    | { kind: "handle"; sub: number; index: number; side: "handleIn" | "handleOut" };
  let drag: Drag | null = null;

  /** Reprend le tracé du document dans la copie de travail, hors geste et sans envoi en attente. */
  function syncFromDocument(fromDocument: SubPath[]): void {
    if (drag || inFlight > 0) return;
    draft = structuredClone($state.snapshot(fromDocument));
    // Le document vient de changer sans nous (Ctrl+Z, Ctrl+Maj+Z…) : le sous-tracé qu'on prolongeait
    // peut avoir disparu ou changé de place. On se raccroche au dernier sous-tracé encore ouvert, pour
    // que le prochain clic reparte du dernier point posé ; sinon le clic visait un sous-tracé fantôme et ne faisait rien.
    const current = activeSub !== null ? draft[activeSub] : undefined;
    if (activeSub !== null && (!current || current.closed)) {
      const last = draft.length - 1;
      activeSub = last >= 0 && draft[last] && !draft[last].closed ? last : null;
    }
    if (selected && !draft[selected.sub]?.anchors[selected.index]) selected = null;
  }
  $effect(() => {
    // Seul un changement du tracé DANS LE DOCUMENT doit relancer cette resynchronisation. Tout le reste
    // est lu sans abonnement : sinon chaque retouche de la copie de travail (donc chaque mouvement de
    // souris pendant qu'on tire une poignée) la relançait, avec une copie complète du tracé à la clé —
    // c'était la cause des gels de la plume.
    const fromDocument = paths;
    untrack(() => syncFromDocument(fromDocument));
  });
  let root = $state<SVGSVGElement>();
  let menu = $state<{ x: number; y: number } | null>(null);

  const entry = (label: string, action: Parameters<typeof runAction>[0], shortcut?: string): MenuEntry => ({
    label,
    shortcut,
    disabled: !isActionEnabled(action),
    run: () => {
      activeSub = null;
      runAction(action);
    },
  });
  const menuEntries = (): MenuEntry[] => [
    entry(t("Définir une sélection"), "path.toSelection", t("Ctrl+Entrée")),
    "separator",
    entry(t("Fond du tracé"), "path.fill"),
    entry(t("Contour du tracé"), "path.stroke"),
    "separator",
    entry(t("Effacer le tracé"), "path.clear"),
  ];

  const toScreen = (p: Vec): Vec => [camera.x + p[0] * camera.zoom, camera.y + p[1] * camera.zoom];
  function docPoint(event: PointerEvent): Vec {
    const rect = root?.getBoundingClientRect();
    return [(event.clientX - (rect?.left ?? 0) - camera.x) / camera.zoom, (event.clientY - (rect?.top ?? 0) - camera.y) / camera.zoom];
  }
  const near = (a: Vec, b: Vec) => distance(a, b) * camera.zoom <= REACH;

  /** Un point posé s'aimante aux repères, aux bords et au centre du document. Ctrl suspend. */
  function snapped(p: Vec, event: PointerEvent): Vec {
    const { x, y } = snapPoint({ x: p[0], y: p[1] }, camera.zoom, event.ctrlKey);
    return [x, y];
  }

  function commit(label: string): void {
    // Un sous-tracé vidé de ses points disparaît.
    draft = draft.filter((sub) => sub.anchors.length > 0);
    inFlight++;
    void edit({ op: "setPath", paths: $state.snapshot(draft), label }).finally(() => {
      inFlight--;
      // Dernière réponse reçue : le document fait foi (un envoi refusé, par exemple, n'y a rien changé).
      if (inFlight === 0) syncFromDocument(paths);
    });
  }

  function findAnchor(p: Vec): { sub: number; index: number } | null {
    for (const [sub, path] of draft.entries()) {
      const index = path.anchors.findIndex((a) => near(a.point, p));
      if (index >= 0) return { sub, index };
    }
    return null;
  }

  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || event.defaultPrevented) return;
    const p = docPoint(event);
    root?.setPointerCapture(event.pointerId);
    if (tool === "pen") penDown(event, p);
    else selectDown(p);
  }

  function penDown(event: PointerEvent, p: Vec): void {
    const hit = findAnchor(p);
    const open = activeSub !== null ? draft[activeSub] : undefined;

    if (hit && event.altKey && open && hit.sub === activeSub && hit.index === open.anchors.length - 1) {
      // Comme Photoshop : Alt + clic sur le DERNIER point posé annule la moitié sortante de sa
      // courbe. Le segment déjà tracé garde sa forme, le suivant repart en ligne droite.
      const last = open.anchors[hit.index];
      if (last) last.handleOut = last.point;
      return commit("Retirer la poignée sortante");
    }
    if (hit && event.altKey) {
      // Alt + clic ailleurs : angle <-> point lisse.
      const path = draft[hit.sub];
      if (path) draft[hit.sub] = toggleSmooth(path, hit.index);
      return commit("Convertir le point");
    }
    if (hit && open && hit.sub === activeSub && hit.index === 0 && open.anchors.length >= 2) {
      open.closed = true;
      drag = { kind: "closeHandle", sub: hit.sub };
      activeSub = null;
      return; // validé au relâchement, après un éventuel glisser
    }
    if (hit) {
      const path = draft[hit.sub];
      const isEnd = path && !path.closed && (hit.index === 0 || hit.index === path.anchors.length - 1);
      if (path && isEnd && activeSub === null) {
        // Reprendre un sous-tracé ouvert par l'une de ses extrémités.
        activeSub = hit.sub;
        if (hit.index === 0) {
          // Par son premier point : le tracé est retourné, pour que le dernier point soit celui qu'on prolonge.
          // Le moteur doit le savoir, sinon sa copie et la nôtre divergent dès la resynchronisation suivante.
          path.anchors.reverse().forEach((a) => ([a.handleIn, a.handleOut] = [a.handleOut, a.handleIn]));
          commit("Reprendre le tracé");
        }
        return;
      }
      if (open && hit.sub === activeSub && hit.index === open.anchors.length - 1) {
        // Clic sur le DERNIER point posé : on ne le supprime pas, on règle sa courbe (glisser tire ses poignées,
        // comme en le posant). Un simple clic le laisse tel quel.
        drag = { kind: "newHandle", sub: hit.sub, index: hit.index, fresh: false };
        return;
      }
      path?.anchors.splice(hit.index, 1);
      if (activeSub === hit.sub && (path?.anchors.length ?? 0) === 0) activeSub = null;
      return commit("Supprimer le point");
    }
    if (activeSub === null) {
      // Clic sur le tracé lui-même : on y ajoute un point, sans changer la forme.
      for (const [sub, path] of draft.entries()) {
        const nearest = nearestOnPath(path, p);
        if (nearest && nearest.distance * camera.zoom <= REACH) {
          draft[sub] = insertAnchor(path, nearest.segment, nearest.t);
          return commit("Ajouter un point");
        }
      }
    }
    // Nouveau point : à la suite du sous-tracé en cours, sinon début d'un nouveau sous-tracé.
    if (activeSub === null) {
      draft.push({ anchors: [], closed: false });
      activeSub = draft.length - 1;
    }
    const target = draft[activeSub];
    if (!target) return;
    target.anchors.push(corner(snapped(p, event)));
    drag = { kind: "newHandle", sub: activeSub, index: target.anchors.length - 1, fresh: true };
  }

  function selectDown(p: Vec): void {
    // Poignées du point sélectionné d'abord : elles peuvent recouvrir un point voisin. Sauf si la poignée
    // est collée à son propre point (courbe à peine amorcée) : c'est alors le point qu'on veut saisir.
    const current = selected ? draft[selected.sub]?.anchors[selected.index] : undefined;
    if (selected && current && !isCorner(current)) {
      for (const side of ["handleIn", "handleOut"] as const) {
        const handleOnPoint = distance(current.point, current[side]) * camera.zoom < REACH;
        if (handleOnPoint && near(current.point, p)) break; // le point l'emporte : saisi plus bas
        if (near(current[side], p)) {
          drag = { kind: "handle", ...selected, side };
          return;
        }
      }
    }
    selected = findAnchor(p);
    if (selected) drag = { kind: "anchor", ...selected, last: p };
  }

  // ---- Curseurs de la plume, comme Photoshop : ils annoncent ce qu'un clic va faire à cet endroit.
  // (point chaud : la pointe de la plume ; pour « convertir », le point d'ancrage dessiné.)
  const PEN_CURSORS = {
    // Adresses entre guillemets, comme pour les icônes (voir Icon.svelte).
    pen: `url(${JSON.stringify(cursorPen)}) 7 26, crosshair`,
    add: `url(${JSON.stringify(cursorAdd)}) 3 23, crosshair`,
    delete: `url(${JSON.stringify(cursorDelete)}) 3 23, crosshair`,
    convert: `url(${JSON.stringify(cursorConvert)}) 16 7, crosshair`,
  };
  let penCursor = $state<keyof typeof PEN_CURSORS>("pen");

  /** Ce que ferait un clic de la plume en `p` (mêmes règles que penDown, sans rien modifier). */
  function penIntent(p: Vec, alt: boolean): keyof typeof PEN_CURSORS {
    const hit = findAnchor(p);
    const open = activeSub !== null ? draft[activeSub] : undefined;
    if (hit && alt) return "convert";
    if (hit && open && hit.sub === activeSub && hit.index === 0 && open.anchors.length >= 2) return "pen"; // fermer le tracé
    if (hit && open && hit.sub === activeSub && hit.index === open.anchors.length - 1) return "pen"; // régler la courbe du dernier point
    if (hit) {
      const path = draft[hit.sub];
      const isEnd = path && !path.closed && (hit.index === 0 || hit.index === path.anchors.length - 1);
      return isEnd && activeSub === null ? "pen" : "delete";
    }
    if (activeSub === null && draft.some((path) => (nearestOnPath(path, p)?.distance ?? Infinity) * camera.zoom <= REACH)) return "add";
    return "pen";
  }

  function onPointerMove(event: PointerEvent): void {
    if (!drag) {
      if (tool === "pen") penCursor = penIntent(docPoint(event), event.altKey);
      return;
    }
    const p = docPoint(event);
    if (drag.kind === "closeHandle") {
      // Comme en posant un point : on tire dans le sens de la courbe, la poignée d'ARRIVÉE du premier
      // point part à l'opposé. Sa poignée de départ n'est pas touchée : le premier segment garde sa forme.
      const first = draft[drag.sub]?.anchors[0];
      if (first && distance(first.point, p) * camera.zoom > 3) first.handleIn = [2 * first.point[0] - p[0], 2 * first.point[1] - p[1]];
      return;
    }
    const anchor: Anchor | undefined = draft[drag.sub]?.anchors[drag.index];
    if (!anchor) return;
    const mirror = (h: Vec): Vec => [2 * anchor.point[0] - h[0], 2 * anchor.point[1] - h[1]];

    if (drag.kind === "newHandle") {
      // Glisser en posant un point : courbe, avec deux poignées symétriques. Même seuil qu'à la fermeture :
      // le tremblement d'un simple clic ne doit pas créer de poignées (le point resterait un faux angle).
      if (distance(anchor.point, p) * camera.zoom <= 3) return;
      anchor.handleOut = p;
      anchor.handleIn = mirror(p);
      drag.moved = true;
    } else if (drag.kind === "handle") {
      anchor[drag.side] = p;
      // Alt casse la symétrie (point d'inflexion) ; sinon la poignée opposée reste alignée.
      if (!event.altKey) anchor[drag.side === "handleIn" ? "handleOut" : "handleIn"] = mirror(p);
    } else {
      const [dx, dy] = [p[0] - drag.last[0], p[1] - drag.last[1]];
      for (const key of ["point", "handleIn", "handleOut"] as const) anchor[key] = [anchor[key][0] + dx, anchor[key][1] + dy];
      drag.last = p;
    }
  }

  function onPointerUp(): void {
    // Le geste est fini : plus de ligne d'alignement à l'écran.
    app.smartGuides = { x: null, y: null };
    if (!drag) return;
    // Dernier point repris sans avoir bougé : rien n'a changé, pas d'état d'historique.
    const unchanged = drag.kind === "newHandle" && !drag.fresh && !drag.moved;
    const label = drag.kind === "newHandle" ? (drag.fresh ? "Plume" : "Régler la courbe") : drag.kind === "closeHandle" ? "Fermer le tracé" : drag.kind === "handle" ? "Modifier une poignée" : "Déplacer le point";
    drag = null;
    if (!unchanged) commit(label);
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.ctrlKey || panelOwnsKeyboard()) return;
    if ((event.key === "Enter" || event.key === "Escape") && activeSub !== null) {
      activeSub = null; // termine le sous-tracé en le laissant ouvert
    } else if ((event.key === "Delete" || event.key === "Backspace") && selected) {
      draft[selected.sub]?.anchors.splice(selected.index, 1);
      selected = null;
      commit("Supprimer le point");
    } else {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  // Changer d'outil termine le sous-tracé en cours.
  $effect(() => {
    void tool;
    activeSub = null;
  });
</script>

<svelte:window onkeydowncapture={onKeyDown} />

<svg
  bind:this={root}
  class="overlay"
  style:cursor={tool === "pen" ? PEN_CURSORS[penCursor] : undefined}
  role="application"
  aria-label={t("Tracé de la plume")}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
  oncontextmenu={(event) => {
    event.preventDefault();
    menu = { x: event.clientX, y: event.clientY };
  }}
>
  {#each draft as sub, s (s)}
    <path d={toSvg(sub, toScreen)} class="under" />
    <path d={toSvg(sub, toScreen)} class="line" />
    {#each sub.anchors as anchor, i (i)}
      {@const point = toScreen(anchor.point)}
      {@const shown = selected?.sub === s && selected.index === i}
      {#if !isCorner(anchor) && (shown || (tool === "pen" && activeSub === s && i === sub.anchors.length - 1))}
        {#each [anchor.handleIn, anchor.handleOut] as handle, h (h)}
          {@const end = toScreen(handle)}
          <line x1={point[0]} y1={point[1]} x2={end[0]} y2={end[1]} class="arm" />
          <circle cx={end[0]} cy={end[1]} r="3.5" class="knob" />
        {/each}
      {/if}
      {#if tool === "pen" && activeSub === s && i === 0}
        <!-- Anneau autour du premier point : c'est lui qu'on vise pour fermer le tracé. -->
        <circle cx={point[0]} cy={point[1]} r="6" class="closer" />
      {/if}
      <rect x={point[0] - 3.5} y={point[1] - 3.5} width="7" height="7" class="anchor" class:selected={shown} />
    {/each}
  {/each}
</svg>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} entries={menuEntries()} onclose={() => (menu = null)} />
{/if}

<style>
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
    touch-action: none;
  }
  path {
    fill: none;
  }
  .under {
    stroke: rgba(0, 0, 0, 0.5);
    stroke-width: 3;
  }
  .line,
  .arm {
    stroke: var(--accent);
    stroke-width: 1;
  }
  .knob {
    fill: var(--accent);
    stroke: #fff;
    stroke-width: 1;
  }
  .anchor {
    fill: #fff;
    stroke: var(--accent);
    stroke-width: 1;
    /* Halo sombre : un point blanc cerné de bleu disparaîtrait sur une image claire. */
    filter: drop-shadow(0 0 1px rgba(0, 0, 0, 0.9));
  }
  .anchor.selected {
    fill: var(--accent);
  }
  /* Premier point du sous-tracé en cours : cliquer dessus ferme le tracé. L'anneau le désigne
     sans toucher au point lui-même, qui reste dessiné comme les autres. */
  .closer {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
    filter: drop-shadow(0 0 1px rgba(0, 0, 0, 0.9));
  }
</style>
