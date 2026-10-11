// Cibles du magnétisme, communes à l'outil Déplacement, au cadre de transformation et aux repères.
import { app } from "../state/app.svelte";

/** Portée du magnétisme, en pixels d'écran. */
export const SNAP_PIXELS = 8;

/** Étendues `[x0, y0, x1, y1]` des autres objets du document, relevées au début d'un geste. */
let objects: readonly (readonly number[])[] = [];
export function setSnapObjects(bounds: readonly (readonly number[])[]): void {
  objects = bounds;
}

/**
 * Sur un axe : bords et centre du plan de travail, repères perpendiculaires à cet axe (s'ils sont
 * affichés), et bords et centre des autres objets.
 */
export function snapTargets(axis: "x" | "y"): number[] {
  const info = app.active?.info;
  if (!info) return [];
  const extent = axis === "x" ? info.width : info.height;
  const wanted = axis === "x" ? "vertical" : "horizontal";
  const guides = app.guidesVisible ? info.guides.filter((g) => g.orientation === wanted).map((g) => g.position) : [];
  const [lo, hi] = axis === "x" ? [0, 2] : [1, 3];
  const others = objects.flatMap((b) => edgesAndCenter(b[lo] ?? 0, b[hi] ?? 0));
  return [0, extent / 2, extent, ...guides, ...others];
}

/** Bords et milieu d'un intervalle : les trois positions d'un objet qui peuvent s'aimanter. */
export const edgesAndCenter = (from: number, to: number): number[] => [from, (from + to) / 2, to];

/**
 * Aimante un point isolé — un coin de tracé, un sommet de forme, un point de plume — sur les mêmes
 * cibles que le reste : bords et centre du document, repères, bords et centre des autres objets.
 * Trace au passage les lignes d'alignement, et les efface dès que rien n'est accroché.
 *
 * `zoom` sert à garder une portée constante à l'écran ; `suspended` (Ctrl enfoncé) rend la main.
 */
export function snapPoint(point: { x: number; y: number }, zoom: number, suspended = false): { x: number; y: number } {
  if (!app.snapEnabled || suspended || zoom <= 0) {
    app.smartGuides = { x: null, y: null };
    return point;
  }
  const reach = SNAP_PIXELS / zoom;
  const x = snapAxis([point.x], "x", reach);
  const y = snapAxis([point.y], "y", reach);
  app.smartGuides = { x: x.line, y: y.line };
  return { x: point.x + x.offset, y: point.y + y.offset };
}

/**
 * Aimante un objet sur un axe : `values` sont ses bords et son centre, déjà déplacés. Retourne le
 * décalage à ajouter et la position de la cible accrochée (pour tracer la ligne d'alignement), ou `null`.
 */
export function snapAxis(values: readonly number[], axis: "x" | "y", reach: number): { offset: number; line: number | null } {
  let best: { offset: number; line: number } | null = null;
  for (const target of snapTargets(axis)) {
    for (const value of values) {
      const offset = target - value;
      if (Math.abs(offset) <= reach && (!best || Math.abs(offset) < Math.abs(best.offset))) best = { offset, line: target };
    }
  }
  return best ?? { offset: 0, line: null };
}
