// Favoris et filtres récents de la galerie : une préférence de confort, gardée dans le navigateur
// intégré. Si le stockage est indisponible, la galerie fonctionne simplement sans.
import type { GalleryKind } from "./filters";
import type { Filter } from "./ipc";

const FAVORITES = "lumitype.filters.favorites";
const RECENT = "lumitype.filters.recent";
const RECENT_MAX = 8;

function read(key: string): GalleryKind[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(key) ?? "[]");
    return Array.isArray(value) ? value.filter((v): v is GalleryKind => typeof v === "string") : [];
  } catch {
    return [];
  }
}

function write(key: string, list: GalleryKind[]): void {
  try {
    localStorage.setItem(key, JSON.stringify(list));
  } catch {
    // Stockage plein ou refusé : la liste ne sera simplement pas retenue.
  }
}

export const favorites = (): GalleryKind[] => read(FAVORITES);
export function toggleFavorite(kind: GalleryKind): GalleryKind[] {
  const list = read(FAVORITES);
  const next = list.includes(kind) ? list.filter((k) => k !== kind) : [...list, kind];
  write(FAVORITES, next);
  return next;
}

export const recentFilters = (): GalleryKind[] => read(RECENT);
export function recordFilter(kind: GalleryKind): void {
  write(RECENT, [kind, ...read(RECENT).filter((k) => k !== kind)].slice(0, RECENT_MAX));
}

/** Dernier filtre appliqué (hors Fluidité, qui a sa propre fenêtre). */
export function lastFilter(): Exclude<GalleryKind, "liquify"> | null {
  const last = read(RECENT).find((k) => k !== "liquify");
  return (last as Exclude<GalleryKind, "liquify"> | undefined) ?? null;
}

/** Dernier filtre appliqué, réglages compris (Ctrl+F le réapplique) : le temps de la séance seulement. */
let applied: Filter | null = null;
export function rememberApplied(filter: Filter): void {
  applied = filter;
}
export const lastApplied = (): Filter | null => applied;
