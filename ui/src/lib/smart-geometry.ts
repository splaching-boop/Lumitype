// Objets dynamiques : calculs du panneau Propriétés (taille, rotation, position), sans état ni moteur.
// Le moteur garde la transformation paramétrique ; l'interface lui envoie une transformation du calque (op
// `transform`, repère document) qui amène les coins actuels du contenu sur les coins voulus.
import { homography } from "./geometry";
import type { SmartInfo } from "./ipc";

type Corner = { x: number; y: number };
type Corners = SmartInfo["corners"];

const point = ([x, y]: readonly [number, number]): Corner => ({ x, y });

/** Taille affichée du contenu, en pixels du document (valeurs absolues : un retournement ne la rend pas négative). */
export function displayedSize(smart: Pick<SmartInfo, "width" | "height" | "scaleX" | "scaleY">): { width: number; height: number } {
  return { width: smart.width * Math.abs(smart.scaleX), height: smart.height * Math.abs(smart.scaleY) };
}

/** Matrice qui amène les coins `from` sur `to` (3 × 3 en ligne), ou `null` si le cadre visé est dégénéré. */
function between(from: Corners, to: Corner[]): number[] | null {
  return homography(from.map(point), to);
}

/**
 * Nouvelle taille affichée `width` × `height` : le coin haut gauche du contenu reste en place, rotation et
 * retournements sont gardés (l'étirement se fait le long des axes du contenu).
 */
export function resizeMatrix(smart: SmartInfo, width: number, height: number): number[] | null {
  const current = displayedSize(smart);
  if (current.width < 1e-9 || current.height < 1e-9 || width <= 0 || height <= 0) return null;
  const [kx, ky] = [width / current.width, height / current.height];
  const [c0, c1, , c3] = smart.corners.map(point) as [Corner, Corner, Corner, Corner];
  const u = { x: (c1.x - c0.x) * kx, y: (c1.y - c0.y) * kx };
  const v = { x: (c3.x - c0.x) * ky, y: (c3.y - c0.y) * ky };
  return between(smart.corners, [c0, { x: c0.x + u.x, y: c0.y + u.y }, { x: c0.x + u.x + v.x, y: c0.y + u.y + v.y }, { x: c0.x + v.x, y: c0.y + v.y }]);
}

/** Rotation absolue `degrees` (sens horaire à l'écran), autour du centre du contenu. */
export function rotateMatrix(smart: SmartInfo, degrees: number): number[] | null {
  const delta = ((degrees - smart.rotationDeg) * Math.PI) / 180;
  const corners = smart.corners.map(point);
  const center = { x: corners.reduce((s, p) => s + p.x, 0) / 4, y: corners.reduce((s, p) => s + p.y, 0) / 4 };
  const [c, s] = [Math.cos(delta), Math.sin(delta)];
  return between(
    smart.corners,
    corners.map((p) => ({ x: center.x + (p.x - center.x) * c - (p.y - center.y) * s, y: center.y + (p.x - center.x) * s + (p.y - center.y) * c })),
  );
}

/** Coin haut gauche du contenu amené en (`x`, `y`). */
export function moveMatrix(smart: SmartInfo, x: number, y: number): number[] {
  return [1, 0, x - smart.x, 0, 1, y - smart.y, 0, 0, 1];
}

/**
 * Taille liée par la chaîne : la dimension modifiée impose l'autre, au rapport de la taille affichée.
 * `changed` : la dimension saisie. L'autre n'est pas arrondie : l'échelle reste la même sur les deux axes
 * (le champ, lui, affiche l'arrondi).
 */
export function proportional(smart: SmartInfo, changed: "width" | "height", value: number): { width: number; height: number } {
  const { width, height } = displayedSize(smart);
  if (changed === "width") return { width: value, height: Math.max(1, (value * height) / Math.max(width, 1e-9)) };
  return { width: Math.max(1, (value * width) / Math.max(height, 1e-9)), height: value };
}

/** Pourcentage lisible (une décimale, virgule française), « 43,3 % ». */
export const percentLabel = (scale: number): string => `${(Math.round(Math.abs(scale) * 1000) / 10).toString().replace(".", ",")} %`;
