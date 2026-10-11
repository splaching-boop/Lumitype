// Géométrie de la transformation libre et du magnétisme. Tout est en pixels du document.
import type { Point } from "../state/app.svelte";

export const add = (a: Point, b: Point): Point => ({ x: a.x + b.x, y: a.y + b.y });
export const sub = (a: Point, b: Point): Point => ({ x: a.x - b.x, y: a.y - b.y });
export const scale = (a: Point, k: number): Point => ({ x: a.x * k, y: a.y * k });
export const dot = (a: Point, b: Point): number => a.x * b.x + a.y * b.y;
export const length = (a: Point): number => Math.hypot(a.x, a.y);
export const midpoint = (a: Point, b: Point): Point => scale(add(a, b), 0.5);
export const centroid = (points: readonly Point[]): Point => scale(points.reduce(add, { x: 0, y: 0 }), 1 / points.length);

export function unit(a: Point): Point {
  const norm = length(a);
  return norm > 1e-9 ? scale(a, 1 / norm) : { x: 1, y: 0 };
}

export function rotate(p: Point, center: Point, angle: number): Point {
  const d = sub(p, center);
  const [c, s] = [Math.cos(angle), Math.sin(angle)];
  return { x: center.x + d.x * c - d.y * s, y: center.y + d.x * s + d.y * c };
}

/** Coordonnées de `d` dans la base (u, v), non nécessairement orthogonale. */
export function decompose(d: Point, u: Point, v: Point): [number, number] {
  const det = u.x * v.y - u.y * v.x;
  if (Math.abs(det) < 1e-9) return [0, 0];
  return [(d.x * v.y - d.y * v.x) / det, (u.x * d.y - u.y * d.x) / det];
}

/**
 * Échelle d'un coin tiré à proportions conservées : le coin ne peut se déplacer que sur la diagonale qui le
 * relie au point fixe `anchor`, et l'échelle est la projection de la souris (`moved`) sur cette diagonale.
 * Un facteur unique pour les deux axes : le coin suit la souris au plus près, sans sauter d'un axe à
 * l'autre quand les écarts se croisent. Négatif si la souris passe de l'autre côté du point fixe
 * (retournement) ; 1 si la diagonale est dégénérée.
 */
export function diagonalScale(anchor: Point, handle: Point, moved: Point): number {
  const diagonal = sub(handle, anchor);
  const length2 = dot(diagonal, diagonal);
  return length2 < 1e-9 ? 1 : dot(sub(moved, anchor), diagonal) / length2;
}

/** Aire signée d'un quadrilatère ; positive si les sommets tournent dans le sens horaire à l'écran. */
export function signedArea(quad: readonly Point[]): number {
  return quad.reduce((sum, p, i) => {
    const q = quad[(i + 1) % quad.length] ?? p;
    return sum + (p.x * q.y - q.x * p.y);
  }, 0) / 2;
}

/** Un cadre exploitable : convexe, non retourné sur lui-même et d'aire non négligeable. */
export function isUsableQuad(quad: readonly Point[]): boolean {
  const turns = quad.map((p, i) => {
    const a = sub(quad[(i + 1) % 4] ?? p, p);
    const b = sub(quad[(i + 2) % 4] ?? p, quad[(i + 1) % 4] ?? p);
    return a.x * b.y - a.y * b.x;
  });
  const convex = turns.every((t) => t > 1e-6) || turns.every((t) => t < -1e-6);
  return convex && Math.abs(signedArea(quad)) > 1;
}

export function pointInQuad(p: Point, quad: readonly Point[]): boolean {
  const signs = quad.map((a, i) => {
    const b = quad[(i + 1) % 4] ?? a;
    return (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
  });
  return signs.every((s) => s >= 0) || signs.every((s) => s <= 0);
}

/**
 * Homographie 3x3 (en ligne) qui envoie les quatre points `from` sur `to`.
 * Système linéaire 8x8 résolu par élimination de Gauss avec pivot partiel ; `null` si dégénéré.
 */
export function homography(from: readonly Point[], to: readonly Point[]): number[] | null {
  const rows: number[][] = [];
  for (let i = 0; i < 4; i++) {
    const [s, d] = [from[i], to[i]];
    if (!s || !d) return null;
    rows.push([s.x, s.y, 1, 0, 0, 0, -s.x * d.x, -s.y * d.x, d.x]);
    rows.push([0, 0, 0, s.x, s.y, 1, -s.x * d.y, -s.y * d.y, d.y]);
  }
  for (let col = 0; col < 8; col++) {
    let pivot = col;
    for (let r = col + 1; r < 8; r++) {
      if (Math.abs(rows[r]?.[col] ?? 0) > Math.abs(rows[pivot]?.[col] ?? 0)) pivot = r;
    }
    const pivotRow = rows[pivot];
    const current = rows[col];
    if (!pivotRow || !current || Math.abs(pivotRow[col] ?? 0) < 1e-12) return null;
    [rows[col], rows[pivot]] = [pivotRow, current];
    for (let r = 0; r < 8; r++) {
      const row = rows[r];
      if (r === col || !row) continue;
      const factor = (row[col] ?? 0) / (pivotRow[col] ?? 1);
      for (let c = col; c < 9; c++) row[c] = (row[c] ?? 0) - factor * (pivotRow[c] ?? 0);
    }
  }
  const solution = rows.map((row, i) => (row[8] ?? 0) / (row[i] ?? 1));
  return solution.every(Number.isFinite) ? [...solution, 1] : null;
}

/**
 * Magnétisme sur un axe : plus petit décalage qui amène l'une des `positions` sur l'une
 * des `targets`, s'il est inférieur à `tolerance`. Retourne 0 sinon.
 */
export function snapOffset(positions: readonly number[], targets: readonly number[], tolerance: number): number {
  let best = 0;
  let bestDistance = tolerance;
  for (const position of positions) {
    for (const target of targets) {
      const distance = Math.abs(target - position);
      if (distance <= bestDistance) {
        bestDistance = distance;
        best = target - position;
      }
    }
  }
  return best;
}
