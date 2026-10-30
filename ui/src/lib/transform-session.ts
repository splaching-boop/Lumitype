// Cycle de vie d'une transformation (Ctrl+T, ou poignées du cadre de l'outil Déplacement).
import { app, type Point, type TransformSession } from "../state/app.svelte";
import { dropLatest, edit, editLatest, setBeforeEdit, waitForFrame } from "./engine";
import { centroid, homography, isUsableQuad } from "./geometry";
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

/**
 * Aperçu d'une transformation par la carte graphique, fourni par la zone de travail (Viewport.svelte, renderer.ts) :
 * pendant le glisser d'une poignée, les textures des calques sont placées par l'homographie du cadre, à la fréquence de
 * l'écran, sans rien demander au moteur (qui rééchantillonnait tout le calque et son masque, puis renvoyait toute la
 * texture, à chaque mouvement de la souris). Au relâchement, UNE étape du moteur en qualité d'aperçu part en arrière-
 * plan (miniatures, pipette, clics voient alors l'état transformé) ; à la validation, le rendu final de qualité.
 */
export interface TransformPreviewControls {
  /**
   * Montre `ids` du document `docId` transformés par `matrix` (document → document, par lignes) ; `inside` : centre du
   * cadre. L'aperçu ne s'applique qu'à ce document, et y reste si l'on change d'onglet. Faux : impossible ici.
   */
  show(gesture: string, docId: number, ids: readonly number[], matrix: readonly number[], inside: Point): boolean;
  /** Le moteur vient d'appliquer cette étape (`null` : état d'avant le geste) : ses textures vont arriver. */
  step(gesture: string, matrix: readonly number[] | null): void;
  end(gesture: string): void;
}
let previewControls: TransformPreviewControls | null = null;
export function provideTransformPreview(controls: TransformPreviewControls | null): void {
  previewControls = controls;
}
/** Gestes montrés par la carte graphique, et le document de chacun. */
const gpuShown = new Map<string, number>();
/**
 * Gestes passés par le moteur à chaque mouvement (la carte graphique ne pouvait pas les montrer) : ils y restent
 * jusqu'au bout, les étapes du moteur n'y sont pas suivies par l'aperçu de la carte.
 */
const engineFallback = new Set<string>();
/** Gestes dont le moteur a reçu au moins une étape : les abandonner demande de la lui retirer. */
const engineTouched = new Set<string>();
/** Étape du relâchement en cours (une à la fois), et le cadre du dernier relâchement, à envoyer ensuite. */
const releaseSteps = new Map<string, { busy: boolean; next: Point[] | null }>();

/**
 * Aperçu par la carte graphique, si possible : composition GPU prête (budget de mémoire vidéo respecté), aperçus par la
 * carte permis (préférences), et pas de sélection active (le moteur ne transformerait que les pixels sélectionnés).
 * Texte, formes et objets dynamiques : leur texture est étirée le temps du geste, le rendu net vient au relâchement.
 */
function previewOnGpu(session: TransformSession, matrix: readonly number[]): boolean {
  const doc = app.active;
  if (!previewControls || engineFallback.has(session.gesture) || !doc || doc.info.hasSelection || app.performance.gpuPreviews === false) return false;
  const shownIn = gpuShown.get(session.gesture);
  if (shownIn !== undefined && shownIn !== doc.info.id) return false;
  if (previewControls.show(session.gesture, doc.info.id, session.ids, matrix, centroid(session.quad))) {
    gpuShown.set(session.gesture, doc.info.id);
    return true;
  }
  return false;
}

/** Fin de l'aperçu par la carte graphique de ce geste. */
function endGpuPreview(gesture: string): void {
  if (gpuShown.delete(gesture)) previewControls?.end(gesture);
}

/** Opération du moteur pour le cadre `quad` : forme redessinée (cadre resté droit), sinon transformation. */
function operationFor(session: TransformSession, quad: Point[], matrix: number[], finalize: boolean): EditOp {
  // Forme seule, cadre resté droit (ni rotation, ni inclinaison, ni retournement) : on la redessine à sa nouvelle
  // taille plutôt que d'étirer ses pixels — ses coins arrondis et son contour gardent leurs dimensions.
  const placed = shapePlacement(session, quad);
  if (placed) return { op: "placeShape", ...placed, gesture: session.gesture };
  return { op: "transform", ids: session.ids, matrix, finalize, gesture: session.gesture };
}

/**
 * Étape du moteur montrée par la carte graphique (relâchement, validation, abandon) : envoyée en arrière-plan ; dès sa
 * réponse, l'aperçu sait que les textures qui arrivent portent cet état (`matrix`, `null` : celui d'avant le geste) et
 * les replace en conséquence — aucun saut, même si une nouvelle poignée a été saisie entre-temps. Rend la fin de
 * l'affichage de ses textures.
 */
function engineStep(gesture: string, op: EditOp, matrix: readonly number[] | null, docId: number): Promise<void> {
  engineTouched.add(gesture);
  return edit(op, docId)
    .then((applied) => {
      if (applied) previewControls?.step(gesture, matrix);
      return waitForFrame();
    })
    .catch(() => undefined);
}

/**
 * Relâchement d'une poignée (ou rotation droite, miroir, Ctrl+Z du cadre) : le moteur reçoit une étape en qualité
 * d'aperçu, toujours calculée depuis l'état d'avant le geste (aucune dégradation cumulée). Une à la fois : un
 * relâchement pendant qu'une étape est en cours attend la fin de celle-ci, seul le dernier cadre part.
 */
export function settleTransform(): void {
  const session = app.transform;
  if (!session || !gpuShown.has(session.gesture) || !isUsableQuad(session.quad)) return;
  const state = releaseSteps.get(session.gesture) ?? { busy: false, next: null };
  releaseSteps.set(session.gesture, state);
  state.next = session.quad.map((p) => ({ ...p }));
  if (!state.busy) pumpSteps(session);
}

function pumpSteps(session: TransformSession): void {
  const gesture = session.gesture;
  const state = releaseSteps.get(gesture);
  const docId = gpuShown.get(gesture);
  // Validé ou abandonné entre-temps : le rendu final (ou l'abandon) a déjà pris le relais.
  if (!state || !state.next || docId === undefined || app.transform !== session) {
    releaseSteps.delete(gesture);
    return;
  }
  const quad = state.next;
  state.next = null;
  const matrix = homography(session.source, quad);
  if (!matrix) return;
  state.busy = true;
  void engineStep(gesture, operationFor(session, quad, matrix, false), matrix, docId).finally(() => {
    state.busy = false;
    pumpSteps(session);
  });
}

/**
 * Envoie le cadre courant : aperçu (par la carte graphique si possible, sinon le moteur en qualité rapide), ou rendu
 * final de qualité par le moteur.
 */
export function sendTransform(finalize: boolean): void {
  const session = app.transform;
  if (!session) return;
  const matrix = homography(session.source, session.quad);
  if (!matrix || !isUsableQuad(session.quad)) return;
  session.previewed = true;
  const gesture = session.gesture;
  if (!finalize && previewOnGpu(session, matrix)) return;
  const op = operationFor(session, session.quad, matrix, finalize);
  if (!finalize) {
    // Aperçu par le moteur à chaque pas (la carte graphique ne peut pas, ou plus) : le sien remplace celui de la carte.
    endGpuPreview(gesture);
    engineFallback.add(gesture);
    engineTouched.add(gesture);
    editLatest(gesture, op);
    return;
  }
  // Rendu final. Un aperçu encore en attente partirait APRÈS lui et le remplacerait par une version rapide ; une étape
  // de relâchement pas encore partie n'a plus lieu d'être.
  dropLatest(gesture);
  releaseSteps.delete(gesture);
  engineFallback.delete(gesture);
  engineTouched.delete(gesture);
  const shownIn = gpuShown.get(gesture);
  if (shownIn === undefined) {
    void edit(op);
    return;
  }
  // L'aperçu de la carte reste affiché jusqu'à l'arrivée du rendu final, dans le document du geste (onglet changé ?).
  void engineStep(gesture, op, matrix, shownIn).finally(() => {
    engineTouched.delete(gesture);
    endGpuPreview(gesture);
  });
}

/** Cadre d'une forme resté rectangle droit : position et taille de la forme redessinée, sinon `null`. */
export function shapePlacement(session: TransformSession, quad: readonly Point[] = session.quad): { id: number; x: number; y: number; width: number; height: number } | null {
  if (session.ids.length !== 1) return null;
  const layer = app.active?.info.layers.find((l) => l.id === session.ids[0]);
  if (!layer?.shape) return null;
  const [a, b, c, d] = quad;
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

/**
 * Abandonne : l'aperçu est retiré de l'historique, comme s'il n'avait jamais existé. Montré par la carte graphique
 * seulement, il n'y a rien à retirer : les calques reprennent simplement leur place.
 */
export function cancelTransform(): void {
  const session = app.transform;
  if (!session) return;
  const gesture = session.gesture;
  app.transform = null;
  releaseSteps.delete(gesture);
  engineFallback.delete(gesture);
  const touched = session.previewed && engineTouched.delete(gesture);
  const shownIn = gpuShown.get(gesture);
  if (!touched) {
    endGpuPreview(gesture);
    return;
  }
  dropLatest(gesture);
  const op: EditOp = { op: "cancelGesture", gesture };
  // Étapes déjà appliquées par le moteur et montrées par la carte : les calques reprennent tout de suite leur place
  // d'avant le geste (aperçu ramené à l'identité), et les textures du moteur, une fois l'étape retirée, prennent le relais.
  if (shownIn !== undefined && previewControls?.show(gesture, shownIn, session.ids, [1, 0, 0, 0, 1, 0, 0, 0, 1], centroid(session.source))) {
    void engineStep(gesture, op, null, shownIn).finally(() => {
      engineTouched.delete(gesture);
      endGpuPreview(gesture);
    });
    return;
  }
  endGpuPreview(gesture);
  void edit(op, shownIn ?? app.activeId);
}
