// Cycle de vie d'une transformation (Ctrl+T, ou poignées du cadre de l'outil Déplacement).
import { app, type Point, type TransformSession } from "../state/app.svelte";
import { dropLatest, edit, editLatest, setBeforeEdit } from "./engine";
import { homography, isUsableQuad } from "./geometry";
import type { Bounds, DocumentInfo, EditOp } from "./ipc";

/**
 * Vrai si l'un de ces calques ne peut pas bouger : cadenas complet ou verrou de position, sur lui-même, sur un dossier
 * qui le contient ou sur un calque qu'il contient (un dossier emporte tout son contenu). Un tel calque ne se transforme
 * pas et ne se déplace pas : sans ce contrôle, le cadre et les poignées s'affichaient, le geste se faisait… et rien ne
 * bougeait, le moteur refusant à la fin.
 */
export function positionLocked(info: DocumentInfo, ids: readonly number[]): boolean {
  const byId = new Map(info.layers.map((layer) => [layer.id, layer]));
  const fixed = (id: number | null) => {
    const layer = id === null ? undefined : byId.get(id);
    return layer !== undefined && (layer.locks.all || layer.locks.position);
  };
  const involved = new Set(ids);
  for (let grew = true; grew; ) {
    grew = false;
    for (const layer of info.layers) {
      if (layer.parent !== null && involved.has(layer.parent) && !involved.has(layer.id)) {
        involved.add(layer.id);
        grew = true;
      }
    }
  }
  for (const id of involved) {
    // Remontée des dossiers parents, avec garde contre un cycle.
    const seen = new Set<number>();
    for (let current: number | null = id; current !== null && !seen.has(current); current = byId.get(current)?.parent ?? null) {
      seen.add(current);
      if (fixed(current)) return true;
    }
  }
  return false;
}

/**
 * Ouvre un cadre autour de `bounds`. `auto` : cadre de l'outil Déplacement, non modal ; il ne
 * devient une transformation en cours qu'à la première poignée tirée, et se valide tout seul
 * dès qu'on fait autre chose. Sinon (Ctrl+T) : modal jusqu'à validation ou abandon.
 */
export function openTransform(ids: number[], [x0, y0, x1, y1]: Bounds, auto: boolean): void {
  // Calque verrouillé : pas de cadre du tout, ni celui de l'outil Déplacement, ni Ctrl+T.
  if (app.active && positionLocked(app.active.info, ids)) {
    app.transform = null;
    return;
  }
  const corners: Point[] = [
    { x: x0, y: y0 },
    { x: x1, y: y0 },
    { x: x1, y: y1 },
    { x: x0, y: y1 },
  ];
  app.transform = { ids: [...ids], gesture: `transformation-${Date.now()}`, source: corners, quad: corners.map((p) => ({ ...p })), steps: [], previewed: false, auto };
}

/** Vrai quand une transformation accapare l'interface : Ctrl+T, ou cadre automatique dont une poignée a été tirée. */
export const transformIsPending = (): boolean => app.transform !== null && (!app.transform.auto || app.transform.previewed);
/** Vrai pour Ctrl+T seulement : les autres outils et raccourcis sont alors suspendus. */
export const transformIsModal = (): boolean => app.transform !== null && !app.transform.auto;

/** Envoie le cadre courant au moteur : aperçu rapide, ou rendu final de qualité. */
export function sendTransform(finalize: boolean): void {
  const session = app.transform;
  if (!session) return;
  const matrix = homography(session.source, session.quad);
  if (!matrix || !isUsableQuad(session.quad)) return;
  session.previewed = true;
  // Forme seule, cadre resté droit (ni rotation, ni inclinaison, ni retournement) : on la redessine à sa nouvelle
  // taille plutôt que d'étirer ses pixels — ses coins arrondis et son contour gardent leurs dimensions.
  const placed = shapePlacement(session);
  if (placed) {
    const op = { op: "placeShape", ...placed, gesture: session.gesture } as const;
    if (finalize) {
      dropLatest(session.gesture);
      void edit(op);
    } else editLatest(session.gesture, op);
    return;
  }
  const op = { op: "transform", ids: session.ids, matrix, finalize, gesture: session.gesture } as const;
  if (finalize) {
    // Un aperçu encore en attente partirait APRÈS le rendu final et le remplacerait par une version rapide.
    dropLatest(session.gesture);
    void edit(op);
  } else {
    editLatest(session.gesture, op);
  }
}

/** Cadre d'une forme resté rectangle droit : position et taille de la forme redessinée, sinon `null`. */
export function shapePlacement(session: TransformSession): { id: number; x: number; y: number; width: number; height: number } | null {
  if (session.ids.length !== 1) return null;
  const layer = app.active?.info.layers.find((l) => l.id === session.ids[0]);
  if (!layer?.shape) return null;
  const [a, b, c, d] = session.quad;
  if (!a || !b || !c || !d) return null;
  const eps = 0.01;
  const upright = Math.abs(a.y - b.y) < eps && Math.abs(c.y - d.y) < eps && Math.abs(a.x - d.x) < eps && Math.abs(b.x - c.x) < eps;
  if (!upright || b.x - a.x < 1 || d.y - a.y < 1) return null;
  return { id: layer.id, x: Math.round(a.x), y: Math.round(a.y), width: Math.round(b.x - a.x), height: Math.round(d.y - a.y) };
}

/** Poignée du cadre tirée en ce moment (voir TransformOverlay) : le cadre ne doit pas être remplacé sous la souris. */
let handleDragged = false;
export function setTransformDragging(dragging: boolean): void {
  handleDragged = dragging;
}
export const transformDragging = (): boolean => handleDragged && app.transform !== null;

/** Vrai si `ids` et la sélection `selection` désignent exactement les mêmes calques (dans n'importe quel ordre). */
export function sameLayers(ids: readonly number[], selection: readonly number[]): boolean {
  const wanted = new Set(selection);
  return ids.length === wanted.size && ids.every((id) => wanted.has(id));
}

/**
 * Opérations qui ne touchent ni aux calques ni à l'historique de l'image : elles ne valident pas une transformation
 * en attente (déplier un dossier du panneau, poser un repère).
 */
const PASSIVE_OPS = new Set<EditOp["op"]>(["setExpanded", "setGuides", "setGuidesLocked", "addGuide", "moveGuide", "removeGuide", "clearGuides"]);

// Règle centrale : toute opération du moteur qui n'appartient pas au geste de transformation en cours valide d'abord
// celui-ci (dupliquer, supprimer, grouper, fusionner, réglage, propriétés, coller…). Sans elle, Ctrl+J pendant une
// rotation au cadre de l'outil Déplacement dupliquait le calque d'avant la rotation. La validation entre dans la file
// du document avant l'opération demandée : celle-ci voit donc le calque transformé.
setBeforeEdit((op) => {
  const session = app.transform;
  if (!session || !transformIsPending()) return;
  if (op && "gesture" in op && op.gesture === session.gesture) return; // aperçu, rendu final ou abandon du cadre lui-même
  if (op && PASSIVE_OPS.has(op.op)) return;
  commitTransform();
});

/** Valide. Rien n'a bougé : aucun état d'historique n'est créé. */
export function commitTransform(): void {
  if (app.transform?.previewed) sendTransform(true);
  app.transform = null;
}

/** Abandonne : l'aperçu est retiré de l'historique, comme s'il n'avait jamais existé. */
export function cancelTransform(): void {
  const session = app.transform;
  if (!session) return;
  app.transform = null;
  if (session.previewed) {
    dropLatest(session.gesture);
    void edit({ op: "cancelGesture", gesture: session.gesture });
  }
}
