// Dégradés : arrêts (position 0..1, couleur RGBA à alpha droit), préréglages, rendu CSS.
import type { Rgb } from "./ipc";

export interface GradientStop {
  position: number;
  color: [number, number, number, number];
}

/** `dynamic` : le dégradé suit les couleurs de premier plan et d'arrière-plan tant qu'on ne l'a pas modifié. */
export interface GradientState {
  stops: GradientStop[];
  dynamic: "foregroundToBackground" | "foregroundToTransparent" | null;
}

const stop = (position: number, hex: string, alpha = 255): GradientStop => {
  const v = Number.parseInt(hex.slice(1), 16);
  return { position, color: [(v >> 16) & 255, (v >> 8) & 255, v & 255, alpha] };
};

export const PRESETS: { name: string; stops: GradientStop[] }[] = [
  { name: "Noir, blanc", stops: [stop(0, "#000000"), stop(1, "#ffffff")] },
  { name: "Noir, transparent", stops: [stop(0, "#000000"), stop(1, "#000000", 0)] },
  { name: "Crépuscule", stops: [stop(0, "#1b1464"), stop(0.5, "#c2185b"), stop(1, "#ffb74d")] },
  { name: "Océan", stops: [stop(0, "#0f2027"), stop(0.5, "#2c5364"), stop(1, "#6dd5ed")] },
  { name: "Aurore", stops: [stop(0, "#00c9a7"), stop(0.5, "#845ec2"), stop(1, "#ff6f91")] },
  { name: "Braise", stops: [stop(0, "#200122"), stop(0.55, "#c31432"), stop(1, "#fdc830")] },
  { name: "Menthe", stops: [stop(0, "#0ba360"), stop(1, "#c6ffdd")] },
  { name: "Lavande", stops: [stop(0, "#654ea3"), stop(1, "#eaafc8")] },
  { name: "Acier", stops: [stop(0, "#232526"), stop(0.5, "#8e9eab"), stop(1, "#eef2f3")] },
  { name: "Néon", stops: [stop(0, "#12c2e9"), stop(0.5, "#c471ed"), stop(1, "#f64f59")] },
  { name: "Or", stops: [stop(0, "#3a2a00"), stop(0.45, "#d4a017"), stop(0.6, "#fff1b8"), stop(1, "#8a6100")] },
  { name: "Arc-en-ciel", stops: [stop(0, "#ff0000"), stop(0.2, "#ffd500"), stop(0.4, "#00d94a"), stop(0.6, "#00c8ff"), stop(0.8, "#3d3dff"), stop(1, "#d400ff")] },
];

/** Arrêts effectifs : ceux du dégradé, ou ceux déduits des couleurs courantes s'il est dynamique. */
export function resolveStops(state: GradientState, foreground: Rgb, background: Rgb): GradientStop[] {
  if (state.dynamic === "foregroundToBackground") return [{ position: 0, color: [...foreground, 255] }, { position: 1, color: [...background, 255] }];
  if (state.dynamic === "foregroundToTransparent") return [{ position: 0, color: [...foreground, 255] }, { position: 1, color: [...foreground, 0] }];
  return state.stops;
}

export const sorted = (stops: readonly GradientStop[]): GradientStop[] => [...stops].sort((a, b) => a.position - b.position);

/** Couleur interpolée à la position `t` (pour ajouter un arrêt sans changer l'aspect du dégradé). */
export function colorAt(stops: readonly GradientStop[], t: number): GradientStop["color"] {
  const list = sorted(stops);
  const after = list.findIndex((s) => s.position >= t);
  const b = list[after === -1 ? list.length - 1 : after];
  const a = list[Math.max(0, (after === -1 ? list.length : after) - 1)];
  if (!a || !b) return [0, 0, 0, 255];
  const span = b.position - a.position;
  const k = span > 1e-6 ? Math.min(1, Math.max(0, (t - a.position) / span)) : 1;
  return [0, 1, 2, 3].map((c) => Math.round((a.color[c] ?? 0) + ((b.color[c] ?? 0) - (a.color[c] ?? 0)) * k)) as GradientStop["color"];
}

export function toCss(stops: readonly GradientStop[]): string {
  const parts = sorted(stops).map(({ position, color: [r, g, b, a] }) => `rgba(${r}, ${g}, ${b}, ${(a / 255).toFixed(3)}) ${(position * 100).toFixed(1)}%`);
  return `linear-gradient(90deg, ${parts.join(", ")})`;
}
