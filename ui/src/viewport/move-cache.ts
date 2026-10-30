// Déplacement de calques par images en cache (comme le cache de projection de groupe de Krita ou de Photoshop).
//
// Déplacer un groupe de cinquante calques ne doit rien recomposer pendant le geste. Dès le clic, le moteur compose
// l'image en quelques couches (voir crates/engine/src/move_preview.rs) au niveau de zoom affiché, sur la zone visible :
// « dessous », le groupe déplacé aplati (effets, opacité et masque compris), puis ce qui est au-dessus (une ou
// quelques couches fondues chacune dans son mode, et les réglages, que la carte graphique applique elle-même). Aucune
// couche ne dépend de la position du groupe : elles valent tout le geste. La carte graphique les assemble à chaque
// image, la couche déplacée décalée d'un bloc ; le moteur ne reçoit qu'un déplacement, au relâchement.
//
// Tout ou rien : l'image ne bouge qu'une fois TOUTES les couches arrivées (à la résolution de l'écran ; si elles
// tardent, d'abord à basse résolution, échangées ensemble ensuite) ; d'ici là, l'image reste figée et seul le cadre
// suit la souris.
// Les couches tiennent toujours en mémoire vidéo : la zone et la résolution sont choisies pour rester sous le budget.
import type { Adjustment, BlendMode, MoveLayout } from "../lib/ipc";
import { levelForScale } from "./camera";
import { paced, pendingUploads } from "./paced";

type Rect = [number, number, number, number];

/** Ce que le moteur fournit (voir lib/ipc.ts : `moveLayout`, `moveSlice`, `adjustmentGpu`). */
export interface MoveCacheSource {
  layout: (ids: number[]) => Promise<MoveLayout | null>;
  /** `maxPacks` : celui du découpage (même découpage côté moteur). */
  slice: (ids: number[], index: number, level: number, rect: Rect, revision: number, own: boolean, maxPacks: number) => Promise<ArrayBuffer>;
  adjust: (adjustment: Adjustment) => Promise<ArrayBuffer>;
}

/** Vue au début du geste : pixels physiques par pixel du document, coin du document, taille de la zone composée. */
export interface MoveView {
  scale: number;
  originX: number;
  originY: number;
  viewW: number;
  viewH: number;
}

/** Une couche prête sur la carte graphique. `rect` : emprise dans le document (pixels du document). */
export interface CachedSlice {
  kind: "below" | "moving" | "layers" | "adjust";
  mode: BlendMode;
  texture: WebGLTexture | null;
  rect: Rect;
  /** Réglage : type et valeurs (texture R32F, voir gpu-layers.ts), opacité, fond du masque hors de l'image. */
  adjust: { kind: number; texture: WebGLTexture } | null;
  opacity: number;
  maskBackground: number;
  /** Couche qui suit les calques déplacés : décalée (et transformée, Ctrl+T) par la carte graphique. */
  moving: boolean;
  bytes: number;
  /** Couche fixe à garder pour un prochain geste (voir `retained`) ; `null` sinon. */
  retainKey?: string | null;
}

/** Couche qui suit les calques déplacés (le « déplacé », ou un paquet du contenu d'un groupe en Transfert). */
export const movesWith = (slice: Pick<MoveLayout["slices"][number], "kind" | "moving">): boolean => slice.moving ?? slice.kind === "moving";

/**
 * `layers` : aperçu complet en couches (la carte graphique ne compose pas le document) ; `collapse` : seulement
 * l'aplat du groupe déplacé, sans son opacité ni son masque, qui remplace son sous-programme quand la carte
 * graphique compose déjà tout le document (voir `GpuLayers.setCollapse`).
 */
export type MoveCacheMode = "layers" | "collapse";

/** Marge autour de la zone visible : la couche déplacée (ce qui peut entrer dans la vue), les autres (petits écarts). */
const MOVING_MARGIN = 0.35;
const STATIC_MARGIN = 0.05;
/** Délai au-delà duquel des couches plus grossières sont demandées en attendant celles de la résolution de l'écran. */
const LOW_RES_AFTER_MS = 100;
/** Images de couches demandées en même temps au moteur : au-delà, elles attendent leur tour (l'affichage garde la main). */
const PARALLEL_SLICES = 2;

/**
 * Couches fixes gardées sur la carte graphique d'un geste à l'autre (« dessous », « dessus ») : un nouveau Ctrl+T ou
 * un nouveau déplacement du même groupe, à la même vue, les reprend sans rien redemander au moteur ni renvoyer d'image.
 * Rangées par empreinte de contenu (voir `MoveSlice.fingerprint` côté moteur), niveau et zone ; bornées en mémoire.
 */
interface Retained {
  gl: WebGL2RenderingContext;
  /** Document dont vient la couche : à sa fermeture, elle est rendue (voir `forgetRetained`). */
  docId: number;
  texture: WebGLTexture;
  rect: Rect;
  bytes: number;
}
const retained = new Map<string, Retained>();
/** Mémoire vidéo au plus pour les couches gardées (les plus anciennes partent d'abord). */
let retainBudget = 96 * 1048576;
export function setRetainBudget(bytes: number): void {
  retainBudget = Math.max(0, bytes);
  trimRetained();
}
function trimRetained(): void {
  let total = [...retained.values()].reduce((sum, r) => sum + r.bytes, 0);
  for (const [key, entry] of retained) {
    if (total <= retainBudget) break;
    entry.gl.deleteTexture(entry.texture);
    retained.delete(key);
    total -= entry.bytes;
  }
}
const retainKey = (slice: Pick<MoveLayout["slices"][number], "kind" | "mode" | "fingerprint">, level: number, rect: Rect) =>
  `${slice.fingerprint}|${slice.kind}|${slice.mode}|${level}|${rect.join(",")}`;
/** Mémoire vidéo des couches gardées (diagnostic, jauge). */
export function retainedBytes(): number {
  return [...retained.values()].reduce((sum, r) => sum + r.bytes, 0);
}
/**
 * Carte graphique perdue : tout est oublié (ou seulement ce qui tenait sur `gl`). Document fermé (`docId`) : ses
 * couches seules sont rendues, celles des autres documents ouverts restent prêtes pour leur prochain geste.
 */
export function forgetRetained(gl?: WebGL2RenderingContext, docId?: number): void {
  for (const [key, entry] of retained) {
    if (gl && entry.gl !== gl) continue;
    if (docId !== undefined && entry.docId !== docId) continue;
    entry.gl.deleteTexture(entry.texture);
    retained.delete(key);
  }
}

/**
 * Zone à composer pour une couche, en pixels du niveau `level` : la zone visible du document (plus une marge), limitée
 * au document pour les couches fixes, à l'étendue de ce qui bouge pour la couche déplacée (qui peut déborder du
 * document). `null` si elle est vide. Module pur : testé à part.
 */
export function sliceRegion(view: MoveView, doc: { width: number; height: number }, level: number, moving: boolean, bounds: readonly number[] | null): Rect | null {
  const [vx, vy] = [-view.originX / view.scale, -view.originY / view.scale];
  const [vw, vh] = [view.viewW / view.scale, view.viewH / view.scale];
  const margin = moving ? MOVING_MARGIN : STATIC_MARGIN;
  let [x0, y0, x1, y1] = [vx - vw * margin, vy - vh * margin, vx + vw * (1 + margin), vy + vh * (1 + margin)];
  if (moving) {
    if (!bounds) return null;
    [x0, y0, x1, y1] = [Math.max(x0, bounds[0]!), Math.max(y0, bounds[1]!), Math.min(x1, bounds[2]!), Math.min(y1, bounds[3]!)];
  } else {
    [x0, y0, x1, y1] = [Math.max(x0, 0), Math.max(y0, 0), Math.min(x1, doc.width), Math.min(y1, doc.height)];
  }
  const step = 2 ** level;
  const [lx0, ly0, lx1, ly1] = [Math.floor(x0 / step), Math.floor(y0 / step), Math.ceil(x1 / step), Math.ceil(y1 / step)];
  return lx1 > lx0 && ly1 > ly0 ? [lx0, ly0, lx1 - lx0, ly1 - ly0] : null;
}

/** Octets d'une texture RGBA8 de `w` × `h` avec ses niveaux de détail. */
const textureBytes = (w: number, h: number) => Math.round(w * h * 4 * (4 / 3));

/**
 * Niveaux à préparer, du plus grossier au plus fin : celui de l'écran (`levelForScale`), relevé tant que les couches
 * dépassent le budget de mémoire vidéo, précédé d'un niveau plus grossier (quatre fois moins de pixels) quand il
 * existe — demandé seulement si le premier tarde (voir `MoveCache.prepare`). Module pur : testé à part.
 */
export function planLevels(view: MoveView, doc: { width: number; height: number; maxLevel: number }, layout: Pick<MoveLayout, "slices" | "bounds">, budget: number, mode: MoveCacheMode): number[] {
  const cost = (level: number) =>
    layout.slices.reduce((sum, slice, index) => {
      if (!wanted(slice, index, mode)) return sum;
      const rect = sliceRegion(view, doc, level, movesWith(slice), layout.bounds);
      return sum + (rect ? textureBytes(rect[2], rect[3]) : 0);
    }, 0);
  let level = Math.min(doc.maxLevel, levelForScale(view.scale, doc.maxLevel));
  while (level < doc.maxLevel && cost(level) > budget) level += 1;
  if (cost(level) > budget) return [];
  // L'aplat seul (carte graphique déjà prête) n'est qu'une optimisation : directement à la résolution de l'écran.
  return mode === "layers" && level < doc.maxLevel ? [level + 1, level] : [level];
}

/** Couches à charger : toutes (sauf les réglages sans masque, qui n'ont pas d'image), ou la seule couche déplacée. */
function wanted(slice: MoveLayout["slices"][number], index: number, mode: MoveCacheMode): boolean {
  if (mode === "collapse") return index === 1;
  return slice.kind !== "adjust" || slice.masked;
}

/** Valeurs d'un réglage (type u32, puis f32) dans une texture R32F de 1024 valeurs par rangée (comme gpu-layers.ts). */
function uploadParams(gl: WebGL2RenderingContext, bytes: ArrayBuffer): { kind: number; texture: WebGLTexture } {
  const kind = new DataView(bytes, 0, 4).getUint32(0, true);
  const values = new Float32Array(bytes.slice(4));
  const rows = Math.max(1, Math.ceil(values.length / 1024));
  const padded = new Float32Array(rows * 1024);
  padded.set(values);
  const texture = gl.createTexture();
  gl.activeTexture(gl.TEXTURE4);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.R32F, 1024, rows, 0, gl.RED, gl.FLOAT, padded);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.bindTexture(gl.TEXTURE_2D, null);
  gl.activeTexture(gl.TEXTURE0);
  return { kind, texture };
}

/** Image d'une couche (en-tête x, y, largeur, hauteur en pixels du niveau, puis RGBA8 prémultiplié) en texture. */
function uploadSlice(gl: WebGL2RenderingContext, buffer: ArrayBuffer): { texture: WebGLTexture | null; w: number; h: number; x: number; y: number } {
  const view = new DataView(buffer, 0, 16);
  const [x, y, w, h] = [view.getInt32(0, true), view.getInt32(4, true), view.getUint32(8, true), view.getUint32(12, true)];
  if (w === 0 || h === 0 || buffer.byteLength < 16 + w * h * 4) return { texture: null, w: 0, h: 0, x, y };
  const texture = gl.createTexture();
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texStorage2D(gl.TEXTURE_2D, Math.floor(Math.log2(Math.max(w, h))) + 1, gl.RGBA8, w, h);
  gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(buffer, 16, w * h * 4));
  gl.generateMipmap(gl.TEXTURE_2D);
  gl.bindTexture(gl.TEXTURE_2D, null);
  return { texture, w, h, x, y };
}

/**
 * Les couches d'un geste de déplacement : demandées dès le clic, puis gardées telles quelles jusqu'au relâchement.
 * Après la préparation, plus aucune demande n'est faite au moteur, quoi que fasse la souris.
 */
export class MoveCache {
  state: "pending" | "ready" | "failed" = "pending";
  /** Découpage du moteur (révision, couches, exactitude). */
  layout: MoveLayout | null = null;
  /** Couches affichées, toutes de la même résolution (l'échange d'un niveau à l'autre se fait d'un coup). */
  slices: CachedSlice[] | null = null;
  /** Niveau des couches affichées (0 = pleine résolution). */
  level: number | null = null;
  /** Délai entre la demande et les premières couches prêtes (mesure), puis jusqu'à la pleine résolution. */
  readyMs: number | null = null;
  fullMs: number | null = null;
  /** Pourquoi l'aperçu en couches est impossible (`failed`). */
  reason: string | null = null;
  private disposed = false;
  /** Avancement de la préparation, pour le diagnostic : images demandées, reçues, posées sur la carte graphique. */
  private progress = { asked: 0, received: 0, uploaded: 0, kept: 0 };
  private readonly started = performance.now();
  /** Numéro de la préparation (diagnostic : une préparation relancée sans cesse se voit). */
  static created = 0;
  private readonly serial = ++MoveCache.created;
  private readonly waiters: ((ok: boolean) => void)[] = [];

  constructor(
    private readonly gl: WebGL2RenderingContext,
    private readonly source: MoveCacheSource,
    readonly docId: number,
    readonly ids: readonly number[],
    private readonly doc: { width: number; height: number; maxLevel: number },
    private readonly view: MoveView,
    private readonly budget: () => number,
    readonly mode: MoveCacheMode,
    private readonly changed: () => void,
  ) {
    void this.prepare();
  }

  /** Résolue à vrai dès que des couches sont prêtes, à faux si l'aperçu est impossible. */
  whenReady(): Promise<boolean> {
    if (this.state !== "pending") return Promise.resolve(this.state === "ready");
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  get ready(): boolean {
    return this.state === "ready" && this.slices !== null;
  }

  /** Mémoire vidéo occupée par les couches. */
  get bytes(): number {
    return (this.slices ?? []).reduce((sum, s) => sum + s.bytes, 0);
  }

  private settle(ok: boolean, reason: string | null = null): void {
    if (this.state !== "pending") return;
    this.state = ok ? "ready" : "failed";
    this.reason = reason;
    for (const resolve of this.waiters.splice(0)) resolve(ok);
    this.changed();
  }

  private async prepare(): Promise<void> {
    const ids = [...this.ids];
    try {
      const layout = await this.source.layout(ids);
      if (this.disposed) return;
      if (!layout) return this.settle(false, "découpage impossible (écrêtage, calques non voisins, verrou…)");
      if (this.mode === "collapse" && layout.collapsible !== ids[0]) return this.settle(false, "groupe non aplatissable");
      this.layout = layout;
      const levels = planLevels(this.view, this.doc, layout, this.budget(), this.mode);
      if (levels.length === 0) return this.settle(false, "mémoire vidéo insuffisante");
      // Résolution de l'écran d'abord : le plus souvent le moteur a déjà les réductions et le contenu du groupe en
      // cache (il affiche ce document), et elle arrive en une centaine de millisecondes sur un document 4K. Si elle
      // tarde (cache froid), un niveau plus grossier est demandé en plus, montré en attendant.
      const full = levels[levels.length - 1]!;
      const low = levels.length > 1 ? levels[0]! : null;
      const show = (level: number, slices: CachedSlice[]) => {
        // Échange d'un coup : les couches d'un niveau remplacent toutes celles du précédent ; jamais un retour en arrière.
        if (this.disposed || (this.level === full && level !== full)) {
          this.release(slices);
          return;
        }
        this.release(this.slices ?? []);
        this.slices = slices;
        this.level = level;
        const elapsed = Math.round(performance.now() - this.started);
        this.readyMs ??= elapsed;
        if (level === full) this.fullMs = elapsed;
        this.settle(true);
        this.changed();
      };
      const timer = low === null ? null : setTimeout(() => {
        if (this.disposed || this.level === full) return;
        this.load(layout, low).then((slices) => show(low, slices), () => undefined);
      }, LOW_RES_AFTER_MS);
      try {
        show(full, await this.load(layout, full));
      } finally {
        if (timer !== null) clearTimeout(timer);
      }
    } catch (error) {
      if (this.disposed) return;
      // Couches déjà prêtes (niveau grossier) : elles restent ; sinon l'aperçu en couches est abandonné.
      if (this.state === "pending") this.settle(false, error instanceof Error ? error.message : String(error));
    }
  }

  /** Toutes les couches d'un niveau : `PARALLEL_SLICES` demandes à la fois, envois étalés (voir `paced`). */
  private async load(layout: MoveLayout, level: number): Promise<CachedSlice[]> {
    const step = 2 ** level;
    let running = 0;
    const queue: (() => void)[] = [];
    const turn = async <T>(task: () => Promise<T>): Promise<T> => {
      if (running >= PARALLEL_SLICES) await new Promise<void>((resolve) => queue.push(resolve));
      running += 1;
      try {
        return await task();
      } finally {
        running -= 1;
        queue.shift()?.();
      }
    };
    const jobs = layout.slices.map(async (slice, index): Promise<CachedSlice | null> => {
      if (this.mode === "collapse" && index !== 1) return null;
      const adjust = slice.kind === "adjust" && slice.adjustment ? await this.source.adjust(slice.adjustment) : null;
      const base: CachedSlice = { kind: slice.kind, mode: slice.mode, texture: null, rect: [0, 0, 0, 0], adjust: null, opacity: slice.opacity, maskBackground: slice.maskBackground, moving: movesWith(slice), bytes: 0 };
      if (this.disposed) return base;
      if (adjust) base.adjust = uploadParams(this.gl, adjust);
      if (!wanted(slice, index, this.mode)) return base;
      const rect = sliceRegion(this.view, this.doc, level, movesWith(slice), layout.bounds);
      if (!rect) return base;
      // Couche fixe gardée d'un geste précédent (même contenu, même niveau, même zone) : reprise telle quelle.
      const key = slice.fingerprint && !movesWith(slice) ? retainKey(slice, level, rect) : null;
      const kept = key ? retained.get(key) : undefined;
      if (key && kept && kept.gl === this.gl) {
        retained.delete(key);
        this.progress.kept += 1;
        return { ...base, texture: kept.texture, rect: kept.rect, bytes: kept.bytes, retainKey: key };
      }
      this.progress.asked += 1;
      const buffer = await turn(() => this.source.slice([...this.ids], index, level, rect, layout.revision, this.mode === "layers", layout.maxPacks ?? 12));
      this.progress.received += 1;
      if (this.disposed) return base;
      const image = await paced(() => (this.disposed ? null : uploadSlice(this.gl, buffer)));
      if (!image) return base;
      this.progress.uploaded += 1;
      return { ...base, texture: image.texture, rect: [image.x * step, image.y * step, image.w * step, image.h * step], bytes: textureBytes(image.w, image.h), retainKey: key };
    });
    const all = await Promise.all(jobs);
    return all.filter((s): s is CachedSlice => s !== null);
  }

  private release(slices: readonly CachedSlice[]): void {
    for (const slice of slices) {
      // Couche fixe : gardée pour un prochain geste (voir `retained`), dans la limite de sa mémoire.
      if (slice.texture && slice.retainKey && retainBudget > 0) {
        const old = retained.get(slice.retainKey);
        if (old && old.texture !== slice.texture) old.gl.deleteTexture(old.texture);
        retained.delete(slice.retainKey);
        retained.set(slice.retainKey, { gl: this.gl, docId: this.docId, texture: slice.texture, rect: slice.rect, bytes: slice.bytes });
        trimRetained();
        if (slice.adjust) this.gl.deleteTexture(slice.adjust.texture);
        continue;
      }
      if (slice.texture) this.gl.deleteTexture(slice.texture);
      if (slice.adjust) this.gl.deleteTexture(slice.adjust.texture);
    }
  }

  /** Fin du geste (ou abandon) : textures libérées, chargements en cours ignorés. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.release(this.slices ?? []);
    this.slices = null;
    this.settle(false, "abandonné");
  }

  /** Résumé pour l'indicateur de diagnostic (barre d'état, avec l'option « afficher les images par seconde »). */
  describe(): string {
    if (this.state === "failed") return `déplacement : moteur (${this.reason ?? "?"})`;
    if (!this.ready) {
      const { asked, received, uploaded, kept } = this.progress;
      return `déplacement : préparation n° ${this.serial} depuis ${Math.round(performance.now() - this.started)} ms (${this.layout ? "découpage reçu" : "découpage demandé"}, ${asked} demandées, ${received} reçues, ${uploaded} posées, ${kept} reprises, ${pendingUploads()} envois en file)`;
    }
    const what = this.mode === "collapse" ? "groupe aplati" : `${this.slices?.length ?? 0} images en cache`;
    const exact = this.layout && !this.layout.exact ? ` · approché : ${this.layout.notes.join(" ; ")}` : "";
    return `déplacement : ${what}, niveau ${this.level}, ${Math.round(this.bytes / 1048576)} Mio, prêt en ${this.readyMs} ms (net en ${this.fullMs} ms)${exact}`;
  }
}

/** Aperçu d'un déplacement composé par la carte graphique (voir renderer.ts, `GpuPreview`). */
export interface ShiftState {
  moving: ReadonlySet<number>;
  parts: "both" | "pixels" | "mask";
  dx: number;
  dy: number;
  /** Position de chaque calque déplacé au début du geste. */
  start: ReadonlyMap<number, readonly [number, number]>;
  /** Textures posées par le moteur après le relâchement : déjà à leur place. */
  fresh: { has: (texture: object) => boolean };
}

/**
 * Décalage d'une texture pendant un aperçu de déplacement : elle est montrée à `départ + décalage`, quelle que soit la
 * position à laquelle elle est posée. Quand la réponse du moteur au relâchement replace les textures (nouvelle
 * position), le décalage restant tombe à zéro dans la même image — auparavant le décalage de l'aperçu s'ajoutait
 * encore à la nouvelle position jusqu'à la fin de l'aperçu (double décalage, un saut visible). Une texture posée après
 * cette réponse (relue par le moteur) est déjà juste : pas de décalage. Module pur : testé à part.
 */
export function moveShift(state: ShiftState | null, id: number, kind: "pixels" | "mask", texture?: { readonly offset: readonly [number, number] }): readonly [number, number] {
  if (!state?.moving.has(id) || !(state.parts === "both" || state.parts === kind)) return [0, 0];
  if (texture && state.fresh.has(texture)) return [0, 0];
  const start = state.start.get(id);
  if (!start || !texture) return [state.dx, state.dy];
  return [start[0] + state.dx - texture.offset[0], start[1] + state.dy - texture.offset[1]];
}

/** Ce dont un geste en couches a besoin de la zone de travail (voir Viewport.svelte). */
export interface MoveSessionHost {
  /** Décalage courant de la couche déplacée (aucune demande au moteur). */
  offset: (dx: number, dy: number) => void;
  /** Le seul envoi au moteur : le déplacement final. */
  translate: (dx: number, dy: number) => Promise<unknown>;
  /** Attend que l'image du moteur (déplacement appliqué) soit affichée. */
  settled: () => Promise<void>;
  /** Fin de l'aperçu : l'image du moteur prend la place. */
  end: () => void;
}

/**
 * Un geste de déplacement en couches : chaque pas ne fait que décaler la couche déplacée ; au relâchement, un seul
 * déplacement part au moteur, et l'aperçu reste affiché jusqu'à ce que le rendu exact soit prêt (pas de saut).
 */
export class MoveSession {
  dx = 0;
  dy = 0;
  private finished = false;

  /** `cache` : les images en cache du geste (aucune quand la carte graphique compose déjà tout le document). */
  constructor(
    readonly cache: MoveCache | null,
    private readonly host: MoveSessionHost,
  ) {}

  move(dx: number, dy: number): void {
    if (this.finished) return;
    [this.dx, this.dy] = [dx, dy];
    this.host.offset(dx, dy);
  }

  /** Relâchement : rien n'a bougé, rien n'est envoyé ; sinon un seul déplacement, puis l'image exacte. */
  async finish(): Promise<void> {
    if (this.finished) return;
    this.finished = true;
    try {
      if (this.dx !== 0 || this.dy !== 0) {
        await this.host.translate(this.dx, this.dy);
        await this.host.settled();
      }
    } finally {
      this.host.end();
    }
  }
}
