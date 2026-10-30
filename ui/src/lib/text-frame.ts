// Cadre d'un texte de paragraphe (texte en bloc) : géométrie des poignées, sans navigateur ni moteur.
//
// Le cadre vit dans le repère du TEXTE (origine au coin haut gauche du cadre, un pixel du texte par unité) ; la
// transformation du calque (Ctrl+T) s'applique par-dessus. Le moteur donne les quatre coins du cadre en repère
// document : on en tire les axes du repère du texte, pour ramener la souris dans ce repère pendant qu'on tire une
// poignée. Tirer une poignée change la largeur et la hauteur du cadre (le texte se recompose) ; les poignées gauche
// et haute déplacent aussi l'origine, de `shift`.

export interface FramePoint {
  x: number;
  y: number;
}

/** Une poignée : position relative sur le cadre (0 = gauche / haut, 0,5 = milieu, 1 = droite / bas). */
export interface FrameHandle {
  hx: 0 | 0.5 | 1;
  hy: 0 | 0.5 | 1;
}

/** Les huit poignées, dans le sens des aiguilles d'une montre depuis le coin haut gauche. */
export const FRAME_HANDLES: readonly FrameHandle[] = [
  { hx: 0, hy: 0 },
  { hx: 0.5, hy: 0 },
  { hx: 1, hy: 0 },
  { hx: 1, hy: 0.5 },
  { hx: 1, hy: 1 },
  { hx: 0.5, hy: 1 },
  { hx: 0, hy: 1 },
  { hx: 0, hy: 0.5 },
];

/** Repère du texte vu du document : origine (coin haut gauche du cadre) et un pixel du texte le long de chaque axe. */
export interface FrameAxes {
  origin: FramePoint;
  ux: FramePoint;
  uy: FramePoint;
}

/** Axes du repère du texte, d'après les coins du cadre (haut gauche, haut droit, bas droit, bas gauche) et sa taille. */
export function frameAxes(quad: readonly FramePoint[], width: number, height: number): FrameAxes | null {
  const [a, b, , d] = quad;
  if (!a || !b || !d || !(width > 0) || !(height > 0)) return null;
  const ux = { x: (b.x - a.x) / width, y: (b.y - a.y) / width };
  const uy = { x: (d.x - a.x) / height, y: (d.y - a.y) / height };
  if (Math.abs(ux.x * uy.y - ux.y * uy.x) < 1e-12) return null; // cadre aplati
  return { origin: { x: a.x, y: a.y }, ux, uy };
}

/** Point du repère du texte -> document. */
export function fromFrame(axes: FrameAxes, p: FramePoint): FramePoint {
  return { x: axes.origin.x + p.x * axes.ux.x + p.y * axes.uy.x, y: axes.origin.y + p.x * axes.ux.y + p.y * axes.uy.y };
}

/** Point du document -> repère du texte. */
export function toFrame(axes: FrameAxes, p: FramePoint): FramePoint {
  const { ux, uy } = axes;
  const det = ux.x * uy.y - ux.y * uy.x;
  const [dx, dy] = [p.x - axes.origin.x, p.y - axes.origin.y];
  return { x: (dx * uy.y - dy * uy.x) / det, y: (ux.x * dy - ux.y * dx) / det };
}

/** Coins d'un rectangle du repère du texte, en document (même ordre que le moteur). */
export function frameQuad(axes: FrameAxes, x0: number, y0: number, width: number, height: number): FramePoint[] {
  return [
    fromFrame(axes, { x: x0, y: y0 }),
    fromFrame(axes, { x: x0 + width, y: y0 }),
    fromFrame(axes, { x: x0 + width, y: y0 + height }),
    fromFrame(axes, { x: x0, y: y0 + height }),
  ];
}

/** Nouveau cadre après avoir tiré une poignée : décalage de l'origine et taille, au pixel du texte. */
export interface ResizedFrame {
  shift: [number, number];
  width: number;
  height: number;
}

/**
 * Cadre de `width` × `height` dont on tire la poignée `handle` jusqu'au point `at` (repère du texte d'AVANT le geste).
 * Le côté opposé reste en place ; un côté ne passe jamais au-delà de son opposé (taille minimale `min`). `keepRatio`
 * (Maj) : la proportion du cadre est gardée — par un coin, la plus grande des deux variations l'emporte ; par un bord,
 * l'autre dimension suit, centrée. Les valeurs sont arrondies au pixel : sans transformation, le texte reste net.
 */
export function resizeFrame(handle: FrameHandle, width: number, height: number, at: FramePoint, keepRatio: boolean, min = 8): ResizedFrame {
  let [x0, y0, x1, y1] = [0, 0, width, height];
  if (handle.hx === 0) x0 = Math.min(at.x, width - min);
  if (handle.hx === 1) x1 = Math.max(at.x, min);
  if (handle.hy === 0) y0 = Math.min(at.y, height - min);
  if (handle.hy === 1) y1 = Math.max(at.y, min);
  if (keepRatio && width > 0 && height > 0) {
    const ratio = width / height;
    if (handle.hx !== 0.5 && handle.hy !== 0.5) {
      const scale = Math.max((x1 - x0) / width, (y1 - y0) / height);
      const [w, h] = [width * scale, height * scale];
      if (handle.hx === 0) x0 = x1 - w;
      else x1 = x0 + w;
      if (handle.hy === 0) y0 = y1 - h;
      else y1 = y0 + h;
    } else if (handle.hy === 0.5) {
      const h = (x1 - x0) / ratio;
      [y0, y1] = [(height - h) / 2, (height + h) / 2];
    } else {
      const w = (y1 - y0) * ratio;
      [x0, x1] = [(width - w) / 2, (width + w) / 2];
    }
  }
  [x0, y0, x1, y1] = [Math.round(x0), Math.round(y0), Math.round(x1), Math.round(y1)];
  return { shift: [x0, y0], width: Math.max(1, x1 - x0), height: Math.max(1, y1 - y0) };
}

/**
 * Cadre tracé à l'outil Texte, du point `from` au point `to` (document) : coin haut gauche et taille, au pixel.
 * `null` si le glissé est trop court à l'écran (`screenTravel` < 4 px) : c'est alors un simple clic (texte de point).
 */
export function draggedFrame(from: FramePoint, to: FramePoint, screenTravel: number): { x: number; y: number; width: number; height: number } | null {
  if (screenTravel < 4) return null;
  const [x0, y0] = [Math.round(Math.min(from.x, to.x)), Math.round(Math.min(from.y, to.y))];
  const [x1, y1] = [Math.round(Math.max(from.x, to.x)), Math.round(Math.max(from.y, to.y))];
  return { x: x0, y: y0, width: Math.max(1, x1 - x0), height: Math.max(1, y1 - y0) };
}

/**
 * Curseur d'une poignée : direction dans laquelle elle tire, à l'écran (vue tournée, texte tourné par Ctrl+T compris).
 * Celle d'un bord est son axe ; celle d'un coin, la diagonale à 45° des axes, quelle que soit la forme du cadre.
 */
export function handleCursor(axes: FrameAxes, handle: FrameHandle): string {
  const unit = (v: FramePoint) => {
    const length = Math.hypot(v.x, v.y) || 1;
    return { x: v.x / length, y: v.y / length };
  };
  const [ux, uy] = [unit(axes.ux), unit(axes.uy)];
  const [dx, dy] = [(handle.hx - 0.5) * ux.x + (handle.hy - 0.5) * uy.x, (handle.hx - 0.5) * ux.y + (handle.hy - 0.5) * uy.y];
  const angle = ((Math.atan2(dy, dx) * 180) / Math.PI + 180) % 180; // 0..180, sens indifférent
  if (angle < 22.5 || angle >= 157.5) return "ew-resize";
  if (angle < 67.5) return "nwse-resize";
  if (angle < 112.5) return "ns-resize";
  return "nesw-resize";
}
