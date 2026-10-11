/** Placement du document à l'écran, en pixels CSS : écran = origine + doc * zoom. */
export interface Camera {
  zoom: number;
  x: number;
  y: number;
}

export const MIN_ZOOM = 0.01;
export const MAX_ZOOM = 64;

/** Paliers de zoom de Photoshop (Ctrl++ / Ctrl+-). */
const ZOOM_STEPS = [
  0.01, 0.02, 0.03, 0.04, 0.05, 0.0625, 0.0833, 0.125, 0.1667, 0.25, 0.3333, 0.5, 0.6667, 1, 2, 3, 4, 5, 6, 7, 8, 12,
  16, 32, 64,
];

export const clampZoom = (zoom: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom));

/** Change le zoom en gardant fixe le point d'écran (px, py). */
export function zoomAt(camera: Camera, zoom: number, px: number, py: number): Camera {
  const next = clampZoom(zoom);
  const ratio = next / camera.zoom;
  return { zoom: next, x: px - (px - camera.x) * ratio, y: py - (py - camera.y) * ratio };
}

export function steppedZoom(current: number, direction: 1 | -1): number {
  const epsilon = current * 1e-3;
  if (direction === 1) {
    return ZOOM_STEPS.find((step) => step > current + epsilon) ?? MAX_ZOOM;
  }
  return [...ZOOM_STEPS].reverse().find((step) => step < current - epsilon) ?? MIN_ZOOM;
}

/** Document entier visible et centré (Ctrl+0). */
export function fitCamera(docW: number, docH: number, viewW: number, viewH: number, margin = 32): Camera {
  const zoom = clampZoom(Math.min((viewW - margin * 2) / docW, (viewH - margin * 2) / docH));
  return centerCamera(docW, docH, viewW, viewH, zoom);
}

export function centerCamera(docW: number, docH: number, viewW: number, viewH: number, zoom: number): Camera {
  return { zoom, x: Math.round((viewW - docW * zoom) / 2), y: Math.round((viewH - docH * zoom) / 2) };
}

/**
 * Niveau de mip à demander : le plus réduit dont les texels restent au moins
 * aussi fins que les pixels physiques de l'écran.
 */
export function levelForScale(deviceScale: number, maxLevel: number): number {
  if (deviceScale >= 1) return 0;
  return Math.min(maxLevel, Math.max(0, Math.floor(Math.log2(1 / deviceScale))));
}
