// Aperçus de transformation par la carte graphique (cadre de transformation, Ctrl+T), tenus par la zone de travail
// (renderer.ts). Séparés du rendu WebGL pour être vérifiés sans navigateur (ui/tests/gpu-layers.test.mjs).
//
// Chaque aperçu appartient au document de son geste (`docId`) : les identifiants de calques ne valent que dans un
// document, et deux onglets ont souvent les mêmes (1, 2, 3…). Un aperçu ne s'applique donc qu'à son document ; changer
// d'onglet ne le jette pas : au retour, il est encore là tant que le rendu final du moteur n'est pas arrivé.
import type { DocumentInfo } from "../lib/ipc";
import { type TransformMatrix, previewInverse, relativeMatrix } from "./gpu-layers";

interface TransformPreview {
  gesture: string;
  /** Document du geste : l'aperçu ne s'applique qu'à lui. */
  docId: number;
  /** Calques transformés, groupes et descendants compris. */
  ids: Set<number>;
  /** État à montrer : homographie du cadre (document d'avant le geste → document), et un point du cadre transformé. */
  current: readonly number[];
  inside: { x: number; y: number };
  /**
   * État que porte chaque texture de ces calques : celui de l'étape du moteur dont elle vient (au relâchement de chaque
   * poignée, voir lib/transform-session.ts). Absente : texture d'avant le geste. Chaque texture reçoit donc
   * `current · base⁻¹` — l'identité, et plus aucune matrice, dès que les textures de la dernière étape sont arrivées ;
   * une réponse périmée (nouvelle poignée saisie entre-temps) est simplement replacée, sans saut.
   */
  bases: WeakMap<object, readonly number[]>;
  /** État du moteur auquel appartiennent les textures posées à partir de maintenant (`null` : celui d'avant le geste). */
  expected: readonly number[] | null;
  /** Matrices relatives déjà calculées pour `current`, par état de base. */
  cache: Map<readonly number[] | null, TransformMatrix | null>;
}

/** Calques et tout leur contenu (un groupe transformé emporte ses calques). */
export function withDescendants(doc: DocumentInfo, ids: Iterable<number>): Set<number> {
  const out = new Set(ids);
  for (let grew = true; grew; ) {
    grew = false;
    for (const layer of doc.layers) {
      if (layer.parent !== null && out.has(layer.parent) && !out.has(layer.id)) {
        out.add(layer.id);
        grew = true;
      }
    }
  }
  return out;
}

/** Aperçus en cours (un par geste ; celui d'un geste validé reste le temps du rendu final). */
export class TransformPreviews {
  private readonly previews = new Map<string, TransformPreview>();
  /** Document affiché (voir `setDocument`) : seuls ses aperçus s'appliquent. */
  private doc: DocumentInfo | null = null;

  /** Document affiché. Calques disparus de ce document : ses aperçus les oublient ; ceux des autres onglets restent. */
  setDocument(doc: DocumentInfo | null): void {
    this.doc = doc;
    if (!doc) return;
    const present = new Set(doc.layers.map((l) => l.id));
    for (const preview of this.previews.values()) {
      if (preview.docId === doc.id) preview.ids = withDescendants(doc, [...preview.ids].filter((id) => present.has(id)));
    }
  }

  /** Un aperçu s'applique au document affiché (la composition doit être refaite à chaque image). */
  get active(): boolean {
    const id = this.doc?.id;
    if (id === undefined) return false;
    for (const preview of this.previews.values()) if (preview.docId === id) return true;
    return false;
  }

  /**
   * Montre `ids` du document `docId` transformés par `forward`. Dans le document affiché, il faut `canCompose` (la carte
   * compose tout le document en ce moment) ; un geste déjà montré dans un autre onglet change simplement d'état (abandon
   * ramené à l'identité, vu au retour). Faux : impossible, le moteur doit rendre chaque pas.
   */
  show(gesture: string, docId: number, ids: readonly number[], forward: readonly number[], inside: { x: number; y: number }, canCompose: boolean): boolean {
    if (!previewInverse(forward, inside)) return false;
    const doc = this.doc;
    const shown = doc !== null && doc.id === docId;
    if (shown && !canCompose) return false;
    const preview = this.previews.get(gesture);
    if (preview) {
      if (preview.docId !== docId) return false;
      Object.assign(preview, { current: [...forward], inside: { ...inside }, cache: new Map() });
      return true;
    }
    if (!shown) return false;
    this.previews.set(gesture, { gesture, docId, ids: withDescendants(doc, ids), current: [...forward], inside: { ...inside }, bases: new WeakMap(), expected: null, cache: new Map() });
    return true;
  }

  /** Une étape du moteur pour ce geste vient d'être appliquée (`null` : retour à l'état d'avant le geste). */
  step(gesture: string, forward: readonly number[] | null): void {
    const preview = this.previews.get(gesture);
    if (preview) preview.expected = forward ? [...forward] : null;
  }

  /** Texture posée par la composition du document affiché : elle porte l'état attendu du moteur. */
  committed(id: number, texture: object): void {
    const docId = this.doc?.id;
    for (const preview of this.previews.values()) {
      if (preview.docId !== docId || !preview.ids.has(id)) continue;
      if (preview.expected) preview.bases.set(texture, preview.expected);
      else preview.bases.delete(texture);
    }
  }

  /** Fin de l'aperçu (rendu final affiché, ou abandon). Vrai s'il existait. */
  end(gesture: string): boolean {
    return this.previews.delete(gesture);
  }

  /** Matrice d'aperçu d'une texture de calque du document affiché (voir `GpuOverrides.matrix`) : `current · base⁻¹`, ou rien. */
  matrix(id: number, texture: object): TransformMatrix | null {
    const docId = this.doc?.id;
    for (const preview of this.previews.values()) {
      if (preview.docId !== docId || !preview.ids.has(id)) continue;
      const base = preview.bases.get(texture) ?? null;
      if (!preview.cache.has(base)) preview.cache.set(base, relativeMatrix(preview.current, base, preview.inside));
      return preview.cache.get(base) ?? null;
    }
    return null;
  }
}
