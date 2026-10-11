// Géométrie des tracés de la plume, en pixels du document. Fonctions pures, testées sans navigateur.
import type { Anchor, SubPath } from "./ipc";

export type Vec = [number, number];

const lerp = (a: Vec, b: Vec, t: number): Vec => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
export const distance = (a: Vec, b: Vec): number => Math.hypot(a[0] - b[0], a[1] - b[1]);
export const corner = (p: Vec): Anchor => ({ point: p, handleIn: p, handleOut: p });
export const isCorner = (a: Anchor): boolean => distance(a.point, a.handleIn) < 1e-6 && distance(a.point, a.handleOut) < 1e-6;

/** Point de la cubique au paramètre `t`. */
export function cubicAt(p: [Vec, Vec, Vec, Vec], t: number): Vec {
  const [a, b, c] = [lerp(p[0], p[1], t), lerp(p[1], p[2], t), lerp(p[2], p[3], t)];
  return lerp(lerp(a, b, t), lerp(b, c, t), t);
}

/** Nombre de segments d'un sous-tracé (un de plus s'il est fermé). */
export const segmentCount = (sub: SubPath): number => (sub.closed && sub.anchors.length > 1 ? sub.anchors.length : Math.max(0, sub.anchors.length - 1));

function controls(sub: SubPath, segment: number): [Vec, Vec, Vec, Vec] | null {
  const a = sub.anchors[segment];
  const b = sub.anchors[(segment + 1) % sub.anchors.length];
  return a && b ? [a.point, a.handleOut, b.handleIn, b.point] : null;
}

/** Commande SVG d'un sous-tracé, après conversion des points par `map` (document -> écran). */
export function toSvg(sub: SubPath, map: (p: Vec) => Vec): string {
  const first = sub.anchors[0];
  if (!first) return "";
  const at = (p: Vec) => map(p).join(" ");
  let d = `M${at(first.point)}`;
  for (let i = 0; i < segmentCount(sub); i++) {
    const c = controls(sub, i);
    if (c) d += `C${at(c[1])} ${at(c[2])} ${at(c[3])}`;
  }
  return sub.closed ? `${d}Z` : d;
}

/** Segment le plus proche d'un point : indice, paramètre et distance. */
export function nearestOnPath(sub: SubPath, p: Vec): { segment: number; t: number; distance: number } | null {
  let best: { segment: number; t: number; distance: number } | null = null;
  for (let segment = 0; segment < segmentCount(sub); segment++) {
    const c = controls(sub, segment);
    if (!c) continue;
    for (let step = 1; step < 40; step++) {
      const t = step / 40;
      const d = distance(cubicAt(c, t), p);
      if (!best || d < best.distance) best = { segment, t, distance: d };
    }
  }
  return best;
}

/** Insère un point sur un segment sans changer la forme de la courbe (découpe de De Casteljau). */
export function insertAnchor(sub: SubPath, segment: number, t: number): SubPath {
  const c = controls(sub, segment);
  if (!c) return sub;
  const [p01, p12, p23] = [lerp(c[0], c[1], t), lerp(c[1], c[2], t), lerp(c[2], c[3], t)];
  const [p012, p123] = [lerp(p01, p12, t), lerp(p12, p23, t)];
  const middle: Anchor = { point: lerp(p012, p123, t), handleIn: p012, handleOut: p123 };
  const anchors = sub.anchors.map((a) => ({ ...a }));
  const next = (segment + 1) % anchors.length;
  const [before, after] = [anchors[segment], anchors[next]];
  if (!before || !after) return sub;
  before.handleOut = p01;
  after.handleIn = p23;
  anchors.splice(segment + 1, 0, middle);
  return { ...sub, anchors };
}

/** Bascule un point entre angle (sans poignées) et point lisse (poignées alignées sur ses voisins). */
export function toggleSmooth(sub: SubPath, index: number): SubPath {
  const anchors = sub.anchors.map((a) => ({ ...a }));
  const anchor = anchors[index];
  if (!anchor) return sub;
  if (!isCorner(anchor)) {
    anchors[index] = corner(anchor.point);
    return { ...sub, anchors };
  }
  const count = anchors.length;
  const previous = anchors[sub.closed ? (index - 1 + count) % count : index - 1];
  const following = anchors[sub.closed ? (index + 1) % count : index + 1];
  const from = previous?.point ?? anchor.point;
  const to = following?.point ?? anchor.point;
  const span = distance(from, to);
  if (span < 1e-6) return sub;
  const direction: Vec = [(to[0] - from[0]) / span, (to[1] - from[1]) / span];
  const reach = (other: Vec | undefined) => (other ? distance(anchor.point, other) / 3 : 0);
  anchor.handleIn = [anchor.point[0] - direction[0] * reach(previous?.point), anchor.point[1] - direction[1] * reach(previous?.point)];
  anchor.handleOut = [anchor.point[0] + direction[0] * reach(following?.point), anchor.point[1] + direction[1] * reach(following?.point)];
  return { ...sub, anchors };
}
