// Plans de travail (crates/engine/src/document.rs, `Artboard`) : un groupe qui porte un format. Le moteur tient la
// découpe et le fond ; ici, le plan actif, les opérations et la compensation de la vue quand le document s'agrandit
// vers la gauche ou le haut (son origine se déplace : la vue est recalée pour que rien ne saute à l'écran).
import { edit, editLatest, whenIdle } from "./engine";
import type { Artboard, Guide, LayerInfo } from "./ipc";
import { app } from "../state/app.svelte";

export const artboardsOf = (layers: readonly LayerInfo[]): (LayerInfo & { artboard: Artboard })[] => layers.filter((l): l is LayerInfo & { artboard: Artboard } => !!l.artboard).reverse();

/** Cadre la vue sur un rectangle du document (marge de 32 px à l'écran). */
export function fitRect(rect: { x: number; y: number; width: number; height: number }): void {
  const { width: vw, height: vh } = app.viewport;
  if (vw <= 0 || vh <= 0 || rect.width <= 0 || rect.height <= 0) return;
  const zoom = Math.min((vw - 64) / rect.width, (vh - 64) / rect.height, 32);
  app.setCamera({ zoom, x: Math.round(vw / 2 - (rect.x + rect.width / 2) * zoom), y: Math.round(vh / 2 - (rect.y + rect.height / 2) * zoom), rotation: 0 });
}

/** Ne travailler que dans un plan (`null` : tous) : la liste des calques et la vue se limitent à lui. */
export function focusBoard(id: number | null): void {
  const doc = app.active;
  if (!doc) return;
  doc.boardFocus = id;
  const board = id === null ? null : doc.info.layers.find((l) => l.id === id)?.artboard;
  // Sélection hors du plan choisi : elle passerait inaperçue (et son cadre déborderait) ; le plan est sélectionné.
  const inside = (layerId: number): boolean => {
    for (let at = doc.info.layers.find((l) => l.id === layerId); at; at = at.parent === null ? undefined : doc.info.layers.find((l) => l.id === at!.parent)) {
      if (at.id === id) return true;
    }
    return false;
  };
  if (id !== null && board && !doc.selection.every(inside)) doc.selection = [id];
  // Le moteur refuse ensuite toute modification hors de ce plan (calques des autres plans, sélection).
  void edit({ op: "setWorkArea", board: board ? id : null });
  if (board) fitRect(board);
  else app.fitToWindow?.();
}

/** Plan qui contient le calque actif (ou le calque actif lui-même). */
export function activeArtboard(): (LayerInfo & { artboard: Artboard }) | null {
  const doc = app.active;
  let layer = app.activeLayer;
  while (layer) {
    if (layer.artboard) return layer as LayerInfo & { artboard: Artboard };
    const parent: number | null = layer.parent;
    layer = parent === null ? null : (doc?.info.layers.find((l) => l.id === parent) ?? null);
  }
  return null;
}

/**
 * Cadre qui reçoit les repères : dans un document à plans de travail, le plan actif (isolé, sinon celui du calque
 * sélectionné) et rien d'autre (`null` : aucun plan actif, les repères sont indisponibles) ; sans plan, le document
 * entier (`board` absent). Un repère n'existe que dans son plan.
 */
export function guideTarget(): { board: number | null; x: number; y: number; width: number; height: number } | null {
  const doc = app.active;
  if (!doc) return null;
  if (!doc.info.layers.some((l) => l.artboard)) return { board: null, x: 0, y: 0, width: doc.info.width, height: doc.info.height };
  const focus = doc.boardFocus !== null ? (doc.info.layers.find((l) => l.id === doc.boardFocus) ?? null) : null;
  const board = focus?.artboard ? (focus as LayerInfo & { artboard: Artboard }) : activeArtboard();
  return board ? { board: board.id, x: board.artboard.x, y: board.artboard.y, width: board.artboard.width, height: board.artboard.height } : null;
}

/** Repères du cadre `target` (ceux du plan, ou tous sans plan). */
export function guidesOf(guides: readonly Guide[], board: number | null): Guide[] {
  return guides.filter((g) => (g.board ?? null) === board);
}

/** Après une opération qui ajuste le document : l'origine a pu bouger de `shift` (pixels) ; la vue suit. */
function compensate(shift: { x: number; y: number }): void {
  const doc = app.active;
  if (!doc?.camera) return;
  if (shift.x !== 0 || shift.y !== 0) {
    doc.camera = { ...doc.camera, x: doc.camera.x - shift.x * doc.camera.zoom, y: doc.camera.y - shift.y * doc.camera.zoom };
    doc.autoFit = false;
  }
  // Le document a pu changer de taille : sa grille de tuiles est refaite ; la vue est redemandée le temps que les
  // nouvelles tuiles arrivent.
  for (const delay of [60, 250, 700]) setTimeout(() => doc.camera && (doc.camera = { ...doc.camera }), delay);
}

/** Décalage de l'origine : où est passé un plan témoin (`id`, attendu en `expected`). */
function shiftOf(id: number, expected: { x: number; y: number }): { x: number; y: number } {
  const board = app.active?.info.layers.find((l) => l.id === id)?.artboard;
  return board ? { x: board.x - expected.x, y: board.y - expected.y } : { x: 0, y: 0 };
}

export async function addArtboard(name: string, board: Omit<Artboard, "backgroundLayer">): Promise<void> {
  const existing = artboardsOf(app.active?.info.layers ?? []);
  const witness = existing[0];
  await edit({ op: "addArtboard", name, board: { ...board, backgroundLayer: null } });
  // Premier plan : le document prend son cadre, son origine passe en (x, y).
  compensate(witness ? shiftOf(witness.id, witness.artboard) : { x: -board.x, y: -board.y });
  const known = new Set(existing.map((l) => l.id));
  const created = artboardsOf(app.active?.info.layers ?? []).find((l) => !known.has(l.id));
  if (created && app.active) app.active.selection = [created.id];
}

/** Pas d'un déplacement (total depuis le début du geste) ; `done` : fin du geste, le document s'ajuste. */
export async function moveArtboard(id: number, start: Artboard, dx: number, dy: number, gesture: string, done: boolean): Promise<void> {
  const op = { op: "moveArtboard" as const, id, dx: Math.round(dx), dy: Math.round(dy), fit: done, gesture };
  // Toujours par la même file : la dernière étape (qui ajuste le document) passe après celles du geste.
  editLatest(gesture, op);
  if (!done) return;
  await whenIdle(gesture);
  compensate(shiftOf(id, { x: start.x + Math.round(dx), y: start.y + Math.round(dy) }));
}

export async function setArtboard(id: number, board: Artboard, gesture: string | null, done: boolean): Promise<void> {
  const op = { op: "setArtboard" as const, id, board, fit: done, gesture };
  if (gesture) {
    editLatest(gesture, op);
    if (!done) return;
    await whenIdle(gesture);
  } else await edit(op);
  compensate(shiftOf(id, board));
}

export async function duplicateArtboard(layer: LayerInfo & { artboard: Artboard }, direction: "right" | "down", content: boolean): Promise<void> {
  const b = layer.artboard;
  const gap = Math.round(Math.max(40, Math.min(b.width, b.height) * 0.08));
  const [dx, dy] = direction === "right" ? [b.width + gap, 0] : [0, b.height + gap];
  await edit({ op: "duplicateArtboard", id: layer.id, dx, dy, content });
  compensate(shiftOf(layer.id, b));
}
