// Composition sur carte graphique : le programme qui assemble le document, calque par calque. Module pur (aucune
// dépendance au navigateur) : il se teste sans affichage.
//
// C'est la traduction exacte de `composite_list` du moteur (crates/engine/src/render.rs) : mêmes opérations, dans le
// même ordre, sur des tampons numérotés de la même façon (le tampon 0 est l'image finale). La carte graphique exécute
// ce programme à la résolution de l'écran (voir gpu-layers.ts), le moteur l'exécute tuile par tuile : le résultat
// est le même, aux arrondis près. Masques, écrêtage, groupes isolés ou en Transfert, calques de réglage, les 21 modes
// de fusion et les effets de calque sont pris en charge. Les effets sont dessinés par le moteur (une fois, puis à
// chaque changement du calque) ; la carte graphique les fond dans l'image comme lui : ceux qui se fondent au calque
// font partie de ses pixels, les autres (ombre en Produit, contour en Incrustation…) sont fondus à part, dessous
// ou dessus, chacun avec son mode.
import type { Adjustment, BlendMode, DocumentInfo, LayerInfo } from "./ipc";

/** Calque porteur de pixels (pixels, texte, forme), du bas vers le haut. */
export interface GpuLayer {
  id: number;
  offset: readonly [number, number];
  /** Opacité effective : opacité × fond. */
  alpha: number;
  /** Fond seul : l'aperçu du curseur d'opacité remplace l'opacité, le fond reste. */
  fill: number;
}

/** Ce qui est fusionné : les pixels d'un calque, ou un tampon déjà composé (contenu d'un groupe). */
export type GpuSource = { layer: number } | { buffer: number };

/**
 * Une opération, comme dans le moteur (`plan.rs`). `layer` désigne le calque dont viennent l'opacité (opacité ×
 * fond) et le masque de l'opération : ils sont lus au moment de dessiner, ce qui permet les aperçus (curseur
 * d'opacité) sans refaire le programme.
 */
export type GpuOp =
  | { op: "clear"; target: number }
  | { op: "load"; target: number; layer: number }
  | { op: "copy"; target: number; source: number }
  | { op: "blend"; target: number; source: GpuSource; mode: BlendMode; layer: number }
  | { op: "mix"; target: number; before: number; layer: number }
  | { op: "adjust"; target: number; mode: BlendMode; layer: number }
  | {
      op: "clip";
      target: number;
      source: GpuSource;
      mode: BlendMode;
      layer: number;
      /**
       * Forme de la base quand elle porte des effets (qui débordent de sa forme) : ses pixels seuls (identifiant de
       * la base) ; `empty` pour une base sans pixels (groupe à effets). Absent : l'alpha de la base fait foi.
       */
      shape?: number | "empty";
    }
  /** Effets du calque fondus à part, dessous (avant le calque) ou dessus (après), chacun avec son mode. */
  | { op: "effects"; target: number; layer: number; side: "under" | "over" };

export interface GpuProgram {
  ops: GpuOp[];
  /** Nombre de tampons utilisés en même temps au plus fort (tampon 0 compris). */
  buffers: number;
}

export type GpuPlan =
  | {
      ok: true;
      /** Calques porteurs de pixels à charger en textures, du bas vers le haut. */
      layers: GpuLayer[];
      program: GpuProgram;
      /**
       * Programme réduit à une superposition de calques en mode Normal, sans masque : il se dessine directement à
       * l'écran, sans tampon intermédiaire (le cas le plus courant, et le plus rapide).
       */
      simple: boolean;
      /** Calques dont le masque (actif) sert au programme. */
      masks: number[];
      /** Calques de réglage appliqués, et leur réglage. */
      adjustments: { id: number; adjustment: Adjustment }[];
      /** Calques porteurs de pixels à effets actifs : leurs effets fondus à part sont à charger. */
      effects: number[];
      /** Bases d'écrêtage à effets : leurs pixels seuls (sans effets) sont à charger. */
      raws: number[];
    }
  | { ok: false; reason: string };

export interface GpuPlanOptions {
  /**
   * Calque dont l'opacité peut changer sans nouveau programme (aperçu du curseur d'opacité) : un groupe en Transfert
   * garde alors son interpolation même à 100 %.
   */
  live?: number | null;
}

/** Même opacité que le moteur (`Pass::opacity`) : opacité × fond. */
export const layerOpacity = (layer: Pick<LayerInfo, "opacity" | "fill">): number => layer.opacity * layer.fill;

const hasMask = (layer: LayerInfo) => layer.hasMask && layer.maskEnabled;
const hasEffects = (layer: LayerInfo) => layer.effects.some((e) => e.enabled);

export function gpuPlan(doc: DocumentInfo, options: GpuPlanOptions = {}): GpuPlan {
  const children = new Map<number | null, LayerInfo[]>();
  for (const layer of doc.layers) {
    const siblings = children.get(layer.parent) ?? [];
    siblings.push(layer);
    children.set(layer.parent, siblings);
  }
  // Indice 0 = tout en bas de la pile.
  for (const siblings of children.values()) siblings.sort((a, b) => a.index - b.index);

  const ops: GpuOp[] = [];
  const layers: GpuLayer[] = [];
  const masks = new Set<number>();
  const adjustments: { id: number; adjustment: Adjustment }[] = [];
  const effects = new Set<number>();
  const raws = new Set<number>();
  let next = 1;
  let peak = 1;
  const take = () => {
    const id = next++;
    peak = Math.max(peak, next);
    return id;
  };
  const release = (id: number) => {
    next = id;
  };
  /** Le calque sert à une opération : son masque et ses pixels devront être chargés. */
  const use = (layer: LayerInfo) => {
    if (hasMask(layer)) masks.add(layer.id);
  };

  /** Contenu d'un calque : ses pixels, ou pour un groupe un tampon où sa composition isolée est écrite. */
  const content = (layer: LayerInfo): { source: GpuSource; scratch: number | null } | null => {
    if (layer.adjustment) return null;
    if (!layer.isGroup) {
      if (!layers.some((l) => l.id === layer.id)) layers.push({ id: layer.id, offset: layer.offset, alpha: layerOpacity(layer), fill: layer.fill });
      return { source: { layer: layer.id }, scratch: null };
    }
    const buffer = take();
    ops.push({ op: "clear", target: buffer });
    compositeList(children.get(layer.id) ?? [], buffer);
    return { source: { buffer }, scratch: buffer };
  };

  const adjust = (layer: LayerInfo, target: number) => {
    if (!layer.adjustment) return;
    use(layer);
    if (!adjustments.some((a) => a.id === layer.id)) adjustments.push({ id: layer.id, adjustment: layer.adjustment });
    ops.push({ op: "adjust", target, mode: layer.blend, layer: layer.id });
  };

  const clipOnto = (layer: LayerInfo, target: number, shape: number | "empty" | undefined) => {
    const made = content(layer);
    if (!made) return;
    use(layer);
    ops.push({ op: "clip", target, source: made.source, mode: layer.blend, layer: layer.id, ...(shape === undefined ? {} : { shape }) });
    if (made.scratch !== null) release(made.scratch);
  };

  const passThrough = (group: LayerInfo, target: number) => {
    const full = layerOpacity(group) >= 1 && !hasMask(group) && options.live !== group.id;
    if (full) {
      compositeList(children.get(group.id) ?? [], target);
      return;
    }
    // Opacité ou masque : le groupe interpole entre l'image d'avant et celle d'après.
    use(group);
    const before = take();
    ops.push({ op: "copy", target: before, source: target });
    compositeList(children.get(group.id) ?? [], target);
    ops.push({ op: "mix", target, before, layer: group.id });
    release(before);
  };

  /** Compose une liste de frères (du bas vers le haut) dans le tampon `target`. */
  const compositeList = (ids: LayerInfo[], target: number) => {
    let i = 0;
    while (i < ids.length) {
      const base = ids[i]!;
      // Calques écrêtés sur `base` : ceux qui le suivent immédiatement.
      let end = i + 1;
      while (end < ids.length && ids[end]!.clipped) end += 1;
      const clipped = ids.slice(i + 1, end);
      i = end;

      if (!base.visible) continue;
      if (clipped.length === 0 && base.isGroup && base.blend === "pass_through") {
        passThrough(base, target);
        continue;
      }
      if (base.adjustment) {
        // Réglage : corrige tout ce qui a été composé jusqu'ici dans ce groupe.
        adjust(base, target);
        continue;
      }
      const made = content(base);
      if (!made) continue;
      let { source, scratch } = made;
      // Les calques écrêtés modifient le contenu de leur base : il lui faut alors un tampon à elle.
      if (clipped.length > 0 && "layer" in source) {
        const buffer = take();
        ops.push({ op: "load", target: buffer, layer: source.layer });
        source = { buffer };
        scratch = buffer;
      }
      // Base à effets : son contenu déborde de sa forme (ombre portée, lueur, contour extérieur). Les calques écrêtés
      // ne couvrent que la forme elle-même : ses pixels seuls. Une base sans pixels (groupe) n'en a pas : rien.
      let shape: number | "empty" | undefined;
      if (clipped.length > 0 && hasEffects(base)) {
        shape = base.isGroup ? "empty" : base.id;
        if (typeof shape === "number") raws.add(shape);
      }
      if ("buffer" in source) {
        for (const layer of clipped) {
          if (!layer.visible) continue;
          // Réglage écrêté : il ne corrige que le calque sur lequel il est écrêté.
          if (layer.adjustment) adjust(layer, source.buffer);
          else clipOnto(layer, source.buffer, shape);
        }
      }
      use(base);
      // Effets fondus à part : ceux du dessous se fondent d'abord dans l'image, puis le calque, puis ceux du dessus.
      const styled = !base.isGroup && hasEffects(base);
      if (styled) {
        effects.add(base.id);
        ops.push({ op: "effects", target, layer: base.id, side: "under" });
      }
      ops.push({ op: "blend", target, source, mode: base.blend, layer: base.id });
      if (styled) ops.push({ op: "effects", target, layer: base.id, side: "over" });
      if (scratch !== null) release(scratch);
    }
  };

  compositeList(children.get(null) ?? [], 0);

  const simple = ops.every((op) => op.op === "blend" && op.target === 0 && "layer" in op.source && op.mode === "normal" && !masks.has(op.layer));
  return { ok: true, layers, program: { ops, buffers: peak }, simple, masks: [...masks], adjustments, effects: [...effects], raws: [...raws] };
}

/** Rectangle `[x, y, largeur, hauteur]` en pixels du document couvert par des tuiles de niveau 0. */
export function tilesRect(tiles: readonly (readonly [number, number])[], tileSize: number): [number, number, number, number] | null {
  if (tiles.length === 0) return null;
  let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const [tx, ty] of tiles) {
    [x0, y0] = [Math.min(x0, tx * tileSize), Math.min(y0, ty * tileSize)];
    [x1, y1] = [Math.max(x1, (tx + 1) * tileSize), Math.max(y1, (ty + 1) * tileSize)];
  }
  return [x0, y0, x1 - x0, y1 - y0];
}

/** Intersection de deux rectangles `[x, y, largeur, hauteur]`, ou `null` si elle est vide. */
export function intersect(a: readonly number[], b: readonly number[]): [number, number, number, number] | null {
  const [ax, ay, aw, ah] = a as [number, number, number, number];
  const [bx, by, bw, bh] = b as [number, number, number, number];
  const [x0, y0, x1, y1] = [Math.max(ax, bx), Math.max(ay, by), Math.min(ax + aw, bx + bw), Math.min(ay + ah, by + bh)];
  return x1 > x0 && y1 > y0 ? [x0, y0, x1 - x0, y1 - y0] : null;
}
