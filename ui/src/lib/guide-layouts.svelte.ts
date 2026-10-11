// Dispositions de repères enregistrées par l'utilisateur (fenêtre Disposition de repères).
//
// Une disposition est gardée **en proportions** du document (0 à 1 sur chaque axe), comme les
// modèles livrés : la même disposition se rejoue sur n'importe quel format. Elle est enregistrée
// dans les réglages de l'application (`guide-layouts`), pas dans le document : on la retrouve
// d'un projet à l'autre.
import { api } from "./ipc";

export interface SavedLayout {
  id: string;
  name: string;
  /** Positions des repères verticaux, en fraction de la largeur. */
  vertical: number[];
  /** Positions des repères horizontaux, en fraction de la hauteur. */
  horizontal: number[];
}

const SETTINGS_NAME = "guide-layouts";

let list = $state<SavedLayout[]>([]);
let loaded = false;

export const savedLayouts = {
  get all(): SavedLayout[] {
    return list;
  },
};

/** Relit les dispositions enregistrées (une fois ; les appels suivants ne coûtent rien). */
export async function loadSavedLayouts(): Promise<void> {
  if (loaded) return;
  loaded = true;
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : [];
    if (Array.isArray(stored)) {
      list = stored.filter(
        (item): item is SavedLayout =>
          typeof item === "object" && item !== null && typeof (item as SavedLayout).id === "string" && typeof (item as SavedLayout).name === "string" && Array.isArray((item as SavedLayout).vertical) && Array.isArray((item as SavedLayout).horizontal),
      );
    }
  } catch {
    list = [];
  }
}

function persist(): void {
  void api.saveSettings(SETTINGS_NAME, JSON.stringify(list, null, 2)).catch(() => undefined);
}

/** Enregistre une disposition (positions en fraction), et la renvoie. Un nom déjà pris est remplacé. */
export function saveLayout(name: string, vertical: number[], horizontal: number[]): SavedLayout {
  const clean = (values: number[]): number[] => [...new Set(values.map((v) => Math.round(Math.min(1, Math.max(0, v)) * 10000) / 10000))].sort((a, b) => a - b);
  const trimmed = name.trim() || "Disposition";
  const existing = list.find((item) => item.name === trimmed);
  const layout: SavedLayout = { id: existing?.id ?? `saved-${Date.now().toString(36)}`, name: trimmed, vertical: clean(vertical), horizontal: clean(horizontal) };
  list = existing ? list.map((item) => (item.id === layout.id ? layout : item)) : [...list, layout];
  persist();
  return layout;
}

export function removeLayout(id: string): void {
  list = list.filter((item) => item.id !== id);
  persist();
}
