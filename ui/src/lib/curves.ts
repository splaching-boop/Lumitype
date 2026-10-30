// Courbes : la même interpolation que le moteur (adjust.rs, `spline_at`), pour dessiner la courbe dans l'éditeur.
// Hermite cubique monotone (Fritsch–Carlson) : la courbe passe par chaque point sans dépasser ses voisins ; avant le
// premier point et après le dernier, elle reste plate. Points et valeurs de 0 à 255.
import type { CurvePoint } from "./ipc";

/** Courbe neutre : la diagonale. */
export const IDENTITY: CurvePoint[] = [
  [0, 0],
  [255, 255],
];

/** Points triés par entrée, sans doublon d'entrée (le dernier l'emporte, comme le moteur). */
export function normalized(points: readonly CurvePoint[]): CurvePoint[] {
  const sorted = points
    .filter(([x, y]) => Number.isFinite(x) && Number.isFinite(y))
    .map(([x, y]): CurvePoint => [Math.min(255, Math.max(0, x)), Math.min(255, Math.max(0, y))])
    .sort((a, b) => a[0] - b[0]);
  const out: CurvePoint[] = [];
  for (const point of sorted) {
    const last = out[out.length - 1];
    if (last && Math.abs(point[0] - last[0]) < 1e-6 * 255) continue;
    out.push(point);
  }
  return out;
}

/** Valeur de la courbe en `x` (0 à 255). Moins de deux points : la diagonale. */
export function curveAt(points: readonly CurvePoint[], x: number): number {
  const p = normalized(points).map(([a, b]) => [a / 255, b / 255] as const);
  const v = x / 255;
  const n = p.length;
  if (n < 2) return x;
  const first = p[0]!;
  const last = p[n - 1]!;
  if (v <= first[0]) return first[1] * 255;
  if (v >= last[0]) return last[1] * 255;
  const delta = p.slice(0, -1).map((a, k) => (p[k + 1]![1] - a[1]) / (p[k + 1]![0] - a[0]));
  const m = new Array<number>(n).fill(0);
  m[0] = delta[0]!;
  m[n - 1] = delta[n - 2]!;
  for (let k = 1; k < n - 1; k++) m[k] = delta[k - 1]! * delta[k]! <= 0 ? 0 : (delta[k - 1]! + delta[k]!) / 2;
  for (let k = 0; k < n - 1; k++) {
    const d = delta[k]!;
    if (Math.abs(d) < 1e-9) {
      m[k] = 0;
      m[k + 1] = 0;
      continue;
    }
    const a = m[k]! / d;
    const b = m[k + 1]! / d;
    const s = a * a + b * b;
    if (s > 9) {
      const t = 3 / Math.sqrt(s);
      m[k] = t * a * d;
      m[k + 1] = t * b * d;
    }
  }
  let k = p.findIndex((_, i) => i < n - 1 && v <= p[i + 1]![0]);
  if (k < 0) k = n - 2;
  const [x0, y0] = p[k]!;
  const [x1, y1] = p[k + 1]!;
  const h = x1 - x0;
  const t = (v - x0) / h;
  const t2 = t * t;
  const t3 = t2 * t;
  const y = (2 * t3 - 3 * t2 + 1) * y0 + (t3 - 2 * t2 + t) * h * m[k]! + (-2 * t3 + 3 * t2) * y1 + (t3 - t2) * h * m[k + 1]!;
  return Math.min(1, Math.max(0, y)) * 255;
}
