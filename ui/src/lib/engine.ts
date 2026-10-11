// Modifications du document : file ordonnée, rafraîchissement de l'affichage, miniatures.
import { t } from "./i18n.svelte";
import { app } from "../state/app.svelte";
import { type DirtyInfo, type DocumentInfo, type EditOp, api } from "./ipc";

// ---------------------------------------------------------------- file ordonnée

const queues = new Map<number, Promise<unknown>>();

/**
 * Exécute les modifications d'un document une par une, dans l'ordre d'appel.
 * C'est ce qui garantit que les points d'un trait arrivent au moteur dans l'ordre,
 * et qu'une annulation ne double jamais l'opération qu'elle annule.
 */
export function enqueue<T>(docId: number, task: () => Promise<T>): Promise<T> {
  const previous = queues.get(docId) ?? Promise.resolve();
  const next = previous.then(task, task);
  queues.set(
    docId,
    next.catch(() => undefined),
  );
  return next;
}

// ------------------------------------------------------------------- affichage

/** `immediate` : trait de pinceau en cours, les tuiles s'affichent une à une dès leur arrivée. */
type DirtyListener = (docId: number, dirty: DirtyInfo, revision: number, immediate: boolean) => void;
const listeners = new Set<DirtyListener>();

/** Le viewport s'abonne ici pour savoir quelles tuiles recharger. */
export function onDirty(listener: DirtyListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function notifyDirty(docId: number, dirty: DirtyInfo, revision: number, immediate = false): void {
  if (!dirty.all && dirty.tiles.length === 0 && !dirty.selection) return;
  for (const listener of listeners) listener(docId, dirty, revision, immediate);
}

// ------------------------------------------------------------------ miniatures

const thumbnailKey = (docId: number, layer: number, mask: boolean) => `${docId}:${layer}:${mask ? "m" : "p"}`;

export function thumbnailOf(docId: number, layer: number, mask: boolean): ImageData | undefined {
  return app.thumbnails.get(thumbnailKey(docId, layer, mask));
}

async function loadThumbnail(docId: number, layer: number, mask: boolean): Promise<void> {
  const buffer = await api.thumbnail(docId, layer, mask);
  const key = thumbnailKey(docId, layer, mask);
  if (buffer.byteLength < 8) {
    app.thumbnails.delete(key);
    return;
  }
  const view = new DataView(buffer);
  const [width, height] = [view.getUint32(0, true), view.getUint32(4, true)];
  app.thumbnails.set(key, new ImageData(new Uint8ClampedArray(buffer, 8), width, height));
}

/** Recharge les miniatures des calques modifiés et charge celles qui manquent. */
export function refreshThumbnails(info: DocumentInfo, changed: readonly number[]): void {
  const stale = new Set(changed);
  for (const layer of info.layers) {
    const wanted: boolean[] = [];
    if (!layer.isGroup) wanted.push(false);
    if (layer.hasMask) wanted.push(true);
    for (const mask of wanted) {
      if (stale.has(layer.id) || !app.thumbnails.has(thumbnailKey(info.id, layer.id, mask))) {
        loadThumbnail(info.id, layer.id, mask).catch((error: unknown) => console.error("Miniature indisponible", error));
      }
    }
  }
}

// --------------------------------------------------------------------- pipette

/**
 * Prélève la couleur de l'image en un point du document. Elle va au sélecteur de couleur
 * ouvert s'il y en a un (couleur d'un texte, par exemple), sinon à la couleur de premier plan.
 */
export async function sampleColor(x: number, y: number, pointer: { clientX: number; clientY: number }): Promise<void> {
  const docId = app.activeId;
  if (docId === null) return;
  try {
    const rgb = await api.sampleColor(docId, Math.floor(x), Math.floor(y));
    if (!rgb) return; // hors du document, ou pixel transparent
    // Pastille ronde au-dessus du pointeur : on voit la couleur prélevée sans quitter l'image des yeux.
    app.sample = { x: pointer.clientX, y: pointer.clientY, rgb };
    if (app.eyedropperTarget) app.eyedropperTarget(rgb);
    else app.foreground = rgb;
  } catch (error) {
    app.fail(error);
  }
}

// --------------------------------------------------------------------- édition

/** Applique le résultat d'une modification à l'état de l'interface. */
export function adopt(info: DocumentInfo, dirty: DirtyInfo): void {
  app.update(info);
  notifyDirty(info.id, dirty, info.revision);
  // Miniatures : tout de suite pour les calques repeints. Un calque seulement déplacé (sa miniature, cadrée sur le
  // document, change avec sa position) attend un temps de repos : pendant un geste de déplacement, chaque pas
  // faisait relire la miniature de tous les calques déplacés (un dossier entier, par exemple), ce qui retardait
  // le pas suivant. Sans précision du moteur (`repainted` absent), tout est rechargé tout de suite.
  if (!dirty.repainted) {
    refreshThumbnails(info, dirty.layers);
    return;
  }
  const repainted = new Set(dirty.repainted);
  refreshThumbnails(info, dirty.layers.filter((id) => repainted.has(id)));
  deferThumbnails(info.id, dirty.layers.filter((id) => !repainted.has(id)));
}

/** Après ce temps sans nouveau déplacement, les miniatures des calques déplacés sont rechargées. */
const MOVED_THUMBNAILS_AFTER_MS = 250;
const deferredThumbnails = new Map<number, { layers: Set<number>; timer: ReturnType<typeof setTimeout> }>();

/** Miniatures de calques seulement déplacés : rechargées une fois le geste au repos, pas à chaque pas. */
function deferThumbnails(docId: number, layers: readonly number[]): void {
  if (layers.length === 0) return;
  const pending = deferredThumbnails.get(docId);
  if (pending) clearTimeout(pending.timer);
  const all = new Set(pending?.layers ?? []);
  for (const id of layers) all.add(id);
  const timer = setTimeout(() => {
    deferredThumbnails.delete(docId);
    const info = app.documents.find((d) => d.info.id === docId)?.info;
    if (info) refreshThumbnails(info, [...all]);
  }, MOVED_THUMBNAILS_AFTER_MS);
  deferredThumbnails.set(docId, { layers: all, timer });
}

/** Au-delà de ce délai, une modification en cours s'affiche dans la barre d'activité (et les raccourcis attendent). */
const BUSY_AFTER_MS = 400;
/** Libellé de la barre d'activité pour les modifications longues ; les autres restent muettes. */
const BUSY_LABELS: Partial<Record<EditOp["op"], string>> = { applyFilter: "Filtre…", delete: "Suppression…", duplicate: "Duplication…", importLayer: "Import…" };

/**
 * Crochet appelé avant chaque modification (voir `setBeforeEdit`). Enregistré par la séance de transformation :
 * engine.ts ne l'importe pas (elle importe déjà ce module), d'où ce crochet plutôt qu'un import croisé.
 */
let beforeEdit: ((op: EditOp | null) => void) | null = null;
export function setBeforeEdit(hook: ((op: EditOp | null) => void) | null): void {
  beforeEdit = hook;
}

/**
 * À appeler avant toute opération du moteur qui ne passe pas par `edit` (collage de calques, import…), avec
 * `null` : une transformation en attente est d'abord validée. Le crochet est SYNCHRONE et met la validation
 * dans la file du document : l'opération mise en file ensuite ne passe qu'une fois la transformation appliquée.
 */
export function settlePendingEdits(op: EditOp | null = null): void {
  beforeEdit?.(op);
}

/**
 * Envoie une modification au moteur. Les erreurs sont affichées, jamais avalées. Les modifications d'un même document
 * se suivent dans l'ordre ; si l'une d'elles dure (filtre sur un grand calque), la barre d'activité le montre et les
 * raccourcis sont retenus en attendant, plutôt que de paraître sans effet.
 */
export async function edit(op: EditOp): Promise<boolean> {
  return (await editDirty(op)) !== null;
}

/** Comme `edit`, mais rend ce que la modification a touché (`null` en cas d'échec). */
async function editDirty(op: EditOp): Promise<{ info: DocumentInfo; dirty: DirtyInfo } | null> {
  const docId = app.activeId;
  if (docId === null) return null;
  // Transformation en attente (cadre de l'outil Déplacement tiré, Ctrl+T) : toute autre opération la valide
  // d'abord. Rien n'est attendu ici (un `await` laisserait passer devant une opération demandée juste après) :
  // la validation entre dans la file AVANT cette opération, qui ne part donc qu'une fois elle appliquée.
  settlePendingEdits(op);
  let shownBusy = false;
  const timer = setTimeout(() => {
    if (app.busy === null) [app.busy, shownBusy] = [t(BUSY_LABELS[op.op] ?? "Modification…"), true];
  }, BUSY_AFTER_MS);
  try {
    const result = await enqueue(docId, () => api.edit(docId, op));
    adopt(result.document, result.dirty);
    return { info: result.document, dirty: result.dirty };
  } catch (error) {
    app.fail(error);
    return null;
  } finally {
    clearTimeout(timer);
    if (shownBusy) app.busy = null;
  }
}

/**
 * Fournie par le viewport : promesse tenue quand l'image affichée reflète le document. `layers` : n'attendre que
 * les textures de ces calques (sans précision : tout ce qui est en cours de chargement).
 */
let frameSettled: ((layers?: readonly number[]) => Promise<void>) | null = null;
export function provideFrameSettled(provider: ((layers?: readonly number[]) => Promise<void>) | null): void {
  frameSettled = provider;
}

/** Attend l'affichage complet de l'état courant, au plus une seconde (fenêtre masquée, document fermé…). */
export function waitForFrame(layers?: readonly number[]): Promise<void> {
  if (!frameSettled) return Promise.resolve();
  return Promise.race([frameSettled(layers), new Promise<void>((resolve) => setTimeout(resolve, 1000))]);
}

/**
 * Calques dont il faut attendre l'affichage après cette modification : les repeints et les dossiers qui les
 * contiennent (leurs effets se recalculent). `undefined` : le moteur n'a pas précisé, on attend tout.
 */
function repaintedLayers(info: DocumentInfo, dirty: DirtyInfo): number[] | undefined {
  if (!dirty.repainted) return undefined;
  const parents = new Map(info.layers.map((layer) => [layer.id, layer.parent]));
  const out = new Set<number>();
  for (const id of dirty.repainted) {
    for (let current: number | null | undefined = id; current != null && !out.has(current); current = parents.get(current)) out.add(current);
  }
  return [...out];
}

const latest = new Map<string, EditOp | null>();
const idleWaiters = new Map<string, (() => void)[]>();

/** Oublie la modification en attente de cette clé (celle en cours d'exécution, elle, se termine). */
export function dropLatest(key: string): void {
  if (latest.has(key)) latest.set(key, null);
}

/**
 * Résolue quand plus aucune modification de cette clé n'est en attente ni en cours. À utiliser
 * avant une action qui doit voir le dernier état (Ctrl+T juste après avoir tapé du texte).
 */
export function whenIdle(key: string): Promise<void> {
  if (!latest.has(key)) return Promise.resolve();
  return new Promise((resolve) => idleWaiters.set(key, [...(idleWaiters.get(key) ?? []), resolve]));
}

/**
 * Pour les curseurs : tant qu'une modification de même clé est en cours, seule la
 * dernière valeur demandée est retenue. L'interface ne prend jamais de retard.
 */
export function editLatest(key: string, op: EditOp): void {
  if (latest.has(key)) {
    latest.set(key, op);
    return;
  }
  latest.set(key, null);
  // La modification suivante n'est envoyée qu'une fois celle-ci entièrement affichée : chaque
  // image montrée est un état cohérent du document, jamais un mélange de deux positions.
  // Seules les textures des calques repeints par CETTE modification sont attendues : un geste commencé pendant le
  // chargement d'un gros document n'attend pas, à chaque pas, des textures sans rapport.
  void editDirty(op)
    .then((result) => waitForFrame(result ? repaintedLayers(result.info, result.dirty) : undefined))
    .finally(() => {
      const pending = latest.get(key);
      latest.delete(key);
      if (pending) {
        editLatest(key, pending);
      } else {
        idleWaiters.get(key)?.forEach((resolve) => resolve());
        idleWaiters.delete(key);
      }
    });
}
