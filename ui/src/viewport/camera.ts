/**
 * Placement du document à l'écran, en pixels CSS. Deux temps, comme dans Photoshop :
 * 1. repère de la VUE (non tournée) : vue = origine + doc * zoom ;
 * 2. rotation de l'affichage (outil Rotation de l'affichage, R) : la vue entière tourne de `rotation` autour du
 *    centre de la zone de travail. Seul l'affichage tourne, jamais les pixels du document.
 * Déplacer, zoomer et cadrer travaillent dans le repère de la vue : la rotation, appliquée par-dessus autour d'un
 * point fixe, n'y change rien. Les conversions écran ↔ document passent TOUTES par les fonctions de ce module.
 */
export interface Camera {
  zoom: number;
  x: number;
  y: number;
  /** Rotation de l'affichage en radians, sens horaire à l'écran ; absente = 0. Propre à la vue, jamais enregistrée. */
  rotation?: number;
}

export interface ScreenPoint {
  x: number;
  y: number;
}

/** Rotation de l'affichage (0 si la caméra n'en porte pas). */
export const rotationOf = (camera: Camera | null | undefined): number => camera?.rotation ?? 0;

/** Angle ramené dans ]-π, π] ; un reste infime (erreur d'arrondi) redevient exactement 0. */
export function normalizeAngle(angle: number): number {
  let a = angle % (2 * Math.PI);
  if (a > Math.PI) a -= 2 * Math.PI;
  if (a <= -Math.PI) a += 2 * Math.PI;
  return Math.abs(a) < 1e-9 ? 0 : a;
}

/**
 * Point de l'écran (pixels CSS, origine au coin haut gauche de la zone de travail de taille `viewW` × `viewH`)
 * → repère de la vue, avant rotation.
 */
export function screenToView(camera: Camera, x: number, y: number, viewW: number, viewH: number): ScreenPoint {
  const r = rotationOf(camera);
  if (r === 0) return { x, y };
  const [cx, cy, c, s] = [viewW / 2, viewH / 2, Math.cos(r), Math.sin(r)];
  const [dx, dy] = [x - cx, y - cy];
  return { x: cx + dx * c + dy * s, y: cy - dx * s + dy * c };
}

/** Repère de la vue → écran : la rotation de l'affichage autour du centre de la zone de travail. */
export function viewToScreen(camera: Camera, x: number, y: number, viewW: number, viewH: number): ScreenPoint {
  const r = rotationOf(camera);
  if (r === 0) return { x, y };
  const [cx, cy, c, s] = [viewW / 2, viewH / 2, Math.cos(r), Math.sin(r)];
  const [dx, dy] = [x - cx, y - cy];
  return { x: cx + dx * c - dy * s, y: cy + dx * s + dy * c };
}

/** Point de l'écran → pixels du document (sous le pointeur, quelle que soit la rotation). */
export function screenToDoc(camera: Camera, x: number, y: number, viewW: number, viewH: number): ScreenPoint {
  const v = screenToView(camera, x, y, viewW, viewH);
  return { x: (v.x - camera.x) / camera.zoom, y: (v.y - camera.y) / camera.zoom };
}

/** Pixels du document → point de l'écran. */
export function docToScreen(camera: Camera, x: number, y: number, viewW: number, viewH: number): ScreenPoint {
  return viewToScreen(camera, camera.x + x * camera.zoom, camera.y + y * camera.zoom, viewW, viewH);
}

/** Déplacement à l'écran (main, molette) → déplacement de l'origine dans le repère de la vue : l'image suit la main. */
export function screenDeltaToView(camera: Camera, dx: number, dy: number): [number, number] {
  const r = rotationOf(camera);
  if (r === 0) return [dx, dy];
  const [c, s] = [Math.cos(r), Math.sin(r)];
  return [dx * c + dy * s, -dx * s + dy * c];
}

/** Cadre tourné (voir `rotatedFrame`) : coin haut gauche par rapport à la zone de travail, et taille, en pixels CSS. */
export interface ViewFrame {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Cadre tourné de la zone de travail, dans le repère de la vue : les surcouches (sélection en cours, cadre de
 * transformation, repères, tracés, recadrage, texte…) y sont posées puis tournent ensemble autour du centre, par CSS.
 * L'image, elle, est composée droite dans une texture de la taille de ce cadre, puis dessinée tournée dans le canevas,
 * qui ne tourne pas et garde la taille de la zone de travail (voir renderer.ts, `draw`).
 * Sans rotation, c'est exactement la zone de travail. Tourné (quel que soit l'angle), c'est un cadre FIXE, centré,
 * au moins aussi grand que la diagonale de la zone de travail dans les deux sens : une fois tourné, il la recouvre
 * entièrement (aucun coin vide, aucun clic perdu), et tourner ne change jamais sa géométrie (auparavant il grandissait
 * par paliers de 16 px avec l'angle : le canevas était réalloué et étiré à chaque palier).
 * Les marges sont entières : `x`, `y` (négatifs ou nuls) restent des pixels CSS ronds.
 */
export function rotatedFrame(viewW: number, viewH: number, rotation: number): ViewFrame {
  if (rotation === 0 || viewW <= 0 || viewH <= 0) return { x: 0, y: 0, width: viewW, height: viewH };
  const diagonal = Math.hypot(viewW, viewH);
  // Un pixel de plus de chaque côté : le bord du cadre, lissé par la rotation, ne se voit jamais dans un coin.
  const pad = (own: number) => Math.ceil(Math.max(0, (diagonal - own) / 2)) + 1;
  const [padX, padY] = [pad(viewW), pad(viewH)];
  return { x: -padX, y: -padY, width: viewW + padX * 2, height: viewH + padY * 2 };
}

/**
 * Point de l'écran (pixels CSS de la zone de travail) → point du cadre tourné (pixels CSS du cadre, avant sa rotation).
 * C'est la formule du dessin final de l'image tournée (PRESENT_SHADER, renderer.ts) : le canevas y lit, pour chacun de
 * ses pixels, le pixel de la texture droite qui doit s'y trouver. Elle est l'inverse exacte de la rotation CSS du cadre
 * (`rotate(rotation)` autour de son centre, qui est le centre de la zone de travail).
 */
export function screenToFrame(x: number, y: number, viewW: number, viewH: number, frame: ViewFrame, rotation: number): ScreenPoint {
  const v = screenToView({ zoom: 1, x: 0, y: 0, rotation }, x, y, viewW, viewH);
  return { x: v.x - frame.x, y: v.y - frame.y };
}

/** Point du cadre tourné → écran : la rotation CSS du cadre autour de son centre (voir `screenToFrame`). */
export function frameToScreen(x: number, y: number, viewW: number, viewH: number, frame: ViewFrame, rotation: number): ScreenPoint {
  return viewToScreen({ zoom: 1, x: 0, y: 0, rotation }, x + frame.x, y + frame.y, viewW, viewH);
}

/** Caméra exprimée dans le cadre tourné (voir `rotatedFrame`) : même placement, origine décalée de la marge. */
export function frameCamera(camera: Camera, frame: { x: number; y: number }): Camera {
  return frame.x === 0 && frame.y === 0 ? camera : { ...camera, x: camera.x - frame.x, y: camera.y - frame.y };
}

/**
 * Conversion d'une position de la souris (coordonnées de la fenêtre) en pixels du document, fournie par la zone de
 * travail : les surcouches (recadrage, plume, transformation, texte) n'ont pas à connaître la rotation de l'affichage
 * ni la place de la zone de travail. `null` sans document affiché.
 */
let clientMapping: ((clientX: number, clientY: number) => ScreenPoint | null) | null = null;
export function provideClientMapping(mapping: typeof clientMapping): void {
  clientMapping = mapping;
}
export function clientToDoc(clientX: number, clientY: number): ScreenPoint | null {
  return clientMapping?.(clientX, clientY) ?? null;
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
  // (px, py) dans le repère de la vue : la rotation, autour du centre fixe, ne le fait pas bouger non plus.
  return { ...camera, zoom: next, x: px - (px - camera.x) * ratio, y: py - (py - camera.y) * ratio };
}

export function steppedZoom(current: number, direction: 1 | -1): number {
  const epsilon = current * 1e-3;
  if (direction === 1) {
    return ZOOM_STEPS.find((step) => step > current + epsilon) ?? MAX_ZOOM;
  }
  return [...ZOOM_STEPS].reverse().find((step) => step < current - epsilon) ?? MIN_ZOOM;
}

/**
 * Document entier visible et centré (Ctrl+0). Vue tournée : comme Photoshop, la rotation est gardée, et c'est le
 * document TOURNÉ (sa boîte englobante à l'écran) qui tient dans la fenêtre.
 */
export function fitCamera(docW: number, docH: number, viewW: number, viewH: number, margin = 32, rotation = 0): Camera {
  const [c, s] = [Math.abs(Math.cos(rotation)), Math.abs(Math.sin(rotation))];
  const [w, h] = [docW * c + docH * s, docW * s + docH * c];
  const zoom = clampZoom(Math.min((viewW - margin * 2) / w, (viewH - margin * 2) / h));
  return centerCamera(docW, docH, viewW, viewH, zoom, rotation);
}

/** Document centré au zoom donné. Centré dans la vue, il le reste une fois tourné autour du centre. */
export function centerCamera(docW: number, docH: number, viewW: number, viewH: number, zoom: number, rotation = 0): Camera {
  return { zoom, x: Math.round((viewW - docW * zoom) / 2), y: Math.round((viewH - docH * zoom) / 2), rotation };
}

/**
 * Niveau de mip à demander : le plus réduit dont les texels restent au moins
 * aussi fins que les pixels physiques de l'écran.
 */
export function levelForScale(deviceScale: number, maxLevel: number, bitmap = false): number {
  if (deviceScale >= 1) return 0;
  const level = Math.min(maxLevel, Math.max(0, Math.floor(Math.log2(1 / deviceScale))));
  // Bitmap sous 100 % : jamais le niveau 0, que le moteur seuille (voir `finishModeFor`).
  return bitmap ? Math.min(maxLevel, Math.max(1, level)) : level;
}

/**
 * Rendu final selon le mode du document (Image › Mode) : 0 tel quel, 1 en gris, 2 seuillé (bitmap). Règle unique,
 * tenue par le moteur sur ses tuiles (`color_mode::finish_tile` : seuil au niveau 0, gris aux niveaux réduits), par la
 * composition sur carte graphique (gpu-layers.ts) et par les aperçus en couches (renderer.ts) : le bitmap n'est
 * seuillé qu'au niveau 0 ET à 100 % ou plus (chaque pixel d'écran montre au plus un pixel du document) ; sinon il est
 * montré en gris (un seuil sur une image réduite n'aurait aucun sens). Pour que les tuiles du moteur suivent la même
 * règle, `levelForScale(…, bitmap = true)` ne demande jamais le niveau 0 sous 100 %. Seule exception : un document
 * d'une seule tuile (`maxLevel` 0) n'a pas de niveau réduit, ses tuiles restent seuillées en zoom arrière.
 * `deviceScale` : pixels physiques par pixel du document.
 */
export function finishModeFor(kind: string | undefined, level: number, deviceScale: number): 0 | 1 | 2 {
  if (kind === "grayscale") return 1;
  if (kind === "bitmap") return level === 0 && deviceScale >= 1 - 1e-6 ? 2 : 1;
  return 0;
}
