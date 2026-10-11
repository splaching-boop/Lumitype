/**
 * Curseur de rotation du cadre de transformation : un arc ouvert avec une flèche à chaque extrémité,
 * tourné pour épouser l'angle survolé (l'arc enveloppe le coin, son sommet pointe vers l'extérieur du cadre).
 * Le dessin est produit une fois par degré et mis en cache : la souris ne génère pas un SVG à chaque mouvement.
 */
const SIZE = 28;
const HOT = 14;
// Arc de 8,5 px de rayon ouvert vers le bas, flèches aux deux bouts, point chaud au centre (sommet vers le haut).
const SHAPE = "<path d='M6.5 18.5A8.5 8.5 0 1 1 21.5 18.5'/><path d='M3.5 15.5L6.5 19L10 16.5'/><path d='M24.5 15.5L21.5 19L18 16.5'/>";
const cache = new Map<number, string>();

/** Valeur CSS `cursor` pour un angle en degrés (0 = sommet de l'arc vers le haut, sens horaire). */
export function rotateCursor(angleDeg: number): string {
  const angle = ((Math.round(angleDeg) % 360) + 360) % 360;
  let value = cache.get(angle);
  if (value === undefined) {
    const svg =
      `<svg xmlns='http://www.w3.org/2000/svg' width='${SIZE}' height='${SIZE}' viewBox='0 0 ${SIZE} ${SIZE}' fill='none' stroke-linecap='round' stroke-linejoin='round'>` +
      `<g transform='rotate(${angle} ${HOT} ${HOT})'><g stroke='black' stroke-width='4.5'>${SHAPE}</g><g stroke='white' stroke-width='2'>${SHAPE}</g></g></svg>`;
    value = `url("data:image/svg+xml;utf8,${encodeURIComponent(svg)}") ${HOT} ${HOT}, alias`;
    cache.set(angle, value);
  }
  return value;
}

/** Angle du curseur pour un coin dont la direction vers l'extérieur du cadre est (dx, dy), en pixels écran. */
export function rotateCursorFor(dx: number, dy: number): string {
  return rotateCursor((Math.atan2(dy, dx) * 180) / Math.PI + 90);
}
