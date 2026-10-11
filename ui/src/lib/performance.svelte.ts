// Réglages de performance : valeurs par défaut, lecture au démarrage, application au moteur, enregistrement.
import { app } from "../state/app.svelte";
import { api } from "./ipc";

export interface PerformanceSettings {
  /**
   * Part de la mémoire vive (10 à 90 %) que Lumitype s'autorise, documents, historique et cache de rendu compris, comme le
   * réglage « Utilisation de la mémoire » de Photoshop. Il n'en prend que ce dont il a besoin ; au-delà du plafond, il libère
   * le cache de rendu, puis envoie les états d'annulation anciens sur le disque de travail (ou, sans lui, les abandonne).
   */
  memoryPercent?: number;
  /**
   * Disque de travail de l'historique, comme celui de Photoshop : au-delà du plafond de mémoire, les états d'annulation
   * anciens sont écrits dans un fichier d'échange au lieu d'être abandonnés, et relus quand on y revient. Actif par défaut.
   */
  scratchEnabled?: boolean;
  /** Dossier du fichier d'échange ; vide = dossier de cache de l'application (`…/dev.imageeditor.app/scratch`). */
  scratchDir?: string;
  /** Place que le disque de travail peut prendre, en Gio (2 à 200) ; au-delà, l'historique ancien est abandonné. */
  scratchQuotaGb?: number;
  /** Cœurs utilisés par le moteur ; 0 = tous. */
  cores: number;
  /** Accélération matérielle : composition des calques et aperçus (pinceau, déplacement, opacité) par la carte graphique. */
  gpuPreviews: boolean;
  /** Part de la mémoire vidéo (10 à 80 %) réservée aux calques ; voir `gpuMemoryBudgetMb`. */
  gpuMemoryPercent?: number;
  historyStates: number;
  /** Compteur d'images dans la barre d'état. Coûte une mesure permanente : décoché par défaut. */
  showFps?: boolean;
  /**
   * Résolution dynamique : pendant un geste, la zone de travail est dessinée plus petite que l'écran si l'affichage ne
   * suit pas, puis redevient nette au relâchement. Activée par défaut ; sans effet sur une machine qui suit déjà.
   */
  adaptiveResolution?: boolean;
}

export const DEFAULT_PERFORMANCE: PerformanceSettings = {
  memoryPercent: 50,
  cores: 0,
  gpuPreviews: true,
  gpuMemoryPercent: 50,
  historyStates: 50,
  showFps: false,
  adaptiveResolution: true,
  scratchEnabled: true,
  scratchDir: "",
  scratchQuotaGb: 20,
};
/**
 * Anciens réglages, remplacés par les pourcentages : `cacheMb` (cache de rendu fixe par document, nul = un quart de la
 * mémoire vive) et `gpuMemoryMb` (mémoire vidéo en Mio, nul = 1 Gio). Encore présents dans un fichier enregistré par une
 * version précédente : ignorés à la lecture, et absents du fichier à la prochaine écriture.
 */
const LEGACY_KEYS = ["cacheMb", "gpuMemoryMb"] as const;
/** Mémoire vidéo supposée quand le système ne dit pas celle de la carte (2 Gio). */
export const ASSUMED_VRAM_MB = 2048;
/** Bornes du curseur « Mémoire vive utilisable », en pour cent. */
export const MEMORY_PERCENT_RANGE = { min: 10, max: 90 } as const;
/** Bornes du curseur « Mémoire vidéo utilisable », en pour cent. */
export const GPU_PERCENT_RANGE = { min: 10, max: 80 } as const;
/** Bornes du curseur « Place maximale » du disque de travail, en Gio. */
export const SCRATCH_QUOTA_RANGE = { min: 2, max: 200 } as const;

/** Quota du disque de travail en Gio, borné et entier (défaut 20). */
export function scratchQuotaOf(perf: PerformanceSettings): number {
  const value = Math.round(perf.scratchQuotaGb ?? DEFAULT_PERFORMANCE.scratchQuotaGb ?? 20);
  return Math.min(SCRATCH_QUOTA_RANGE.max, Math.max(SCRATCH_QUOTA_RANGE.min, Number.isFinite(value) ? value : 20));
}

export function memoryPercentOf(perf: PerformanceSettings): number {
  return Math.min(MEMORY_PERCENT_RANGE.max, Math.max(MEMORY_PERCENT_RANGE.min, Math.round(perf.memoryPercent ?? DEFAULT_PERFORMANCE.memoryPercent ?? 50)));
}

export function gpuPercentOf(perf: PerformanceSettings): number {
  return Math.min(GPU_PERCENT_RANGE.max, Math.max(GPU_PERCENT_RANGE.min, Math.round(perf.gpuMemoryPercent ?? DEFAULT_PERFORMANCE.gpuMemoryPercent ?? 50)));
}

/** Ce que le système dit de la carte graphique : mémoire vidéo détectée (`null` : inconnue) et, si elle est intégrée, partagée avec la mémoire vive. */
export interface VideoMemoryInfo {
  vramMb: number | null | undefined;
  vramShared?: boolean;
  ramMb?: number | null;
}

/**
 * Carte graphique intégrée (portable) : sa mémoire vidéo est prise sur la mémoire vive. Reconnue à une mémoire dédiée
 * petite (< 1 Gio), signalée comme partagée par le système, ou inconnue de lui.
 */
export function isIntegratedGpu(info: VideoMemoryInfo): boolean {
  return info.vramMb == null || info.vramShared === true || info.vramMb < 1024;
}

/** Mémoire vidéo sur laquelle le pourcentage s'applique, en Mio : celle que le système a détectée, ou 2 Gio supposés. */
export function detectedVramMb(info: VideoMemoryInfo): number {
  return info.vramMb ?? ASSUMED_VRAM_MB;
}

/**
 * Plafond de mémoire vidéo pour les calques, en Mio : la part réglée de la mémoire vidéo détectée (ou supposée). Sur une
 * carte intégrée, jamais plus d'un quart de la mémoire vive (si elle est connue) : la mémoire « vidéo » y est la même.
 * Au-delà de ce plafond, les calques sont composés par le processeur (voir `GpuLayers`) : rien ne plante.
 */
export function gpuMemoryBudgetMb(perf: PerformanceSettings, info: VideoMemoryInfo): number {
  const share = Math.floor((detectedVramMb(info) * gpuPercentOf(perf)) / 100);
  return isIntegratedGpu(info) && info.ramMb ? Math.min(share, Math.floor(info.ramMb / 4)) : share;
}

/** Réglages lus dans un fichier : les clés inconnues ou anciennes sont écartées, les absentes gardent leur défaut. */
export function mergeStoredPerformance(stored: unknown): PerformanceSettings {
  const perf: PerformanceSettings = { ...DEFAULT_PERFORMANCE };
  if (!stored || typeof stored !== "object") return perf;
  const known = new Set(Object.keys(DEFAULT_PERFORMANCE));
  for (const [key, value] of Object.entries(stored as Record<string, unknown>)) {
    if (!known.has(key) || (LEGACY_KEYS as readonly string[]).includes(key) || value === undefined) continue;
    (perf as unknown as Record<string, unknown>)[key] = value;
  }
  return perf;
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;

/** Envoie les réglages au moteur ; `persist` les enregistre aussi (après une courte pause : un curseur en envoie beaucoup). */
export function applyPerformance(persist: boolean): void {
  const { cores, historyStates } = app.performance;
  void api
    .setPerformance({
      memoryPercent: memoryPercentOf(app.performance),
      cores: cores > 0 ? cores : null,
      historyStates,
      scratchEnabled: app.performance.scratchEnabled !== false,
      scratchDir: app.performance.scratchDir?.trim() || null,
      scratchQuotaMb: scratchQuotaOf(app.performance) * 1024,
    })
    .catch((error: unknown) => app.fail(error));
  if (!persist) return;
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void api.saveSettings("performance", JSON.stringify(app.performance, null, 2)).catch(() => undefined), 400);
}

/** Au démarrage : relit les réglages enregistrés (les valeurs absentes gardent leur défaut) et les applique. */
export async function loadPerformance(): Promise<void> {
  // Ce que la machine offre, pour le budget de mémoire vidéo et la fenêtre des préférences. Sans réponse, les
  // budgets se calculent sur des valeurs supposées.
  api.systemInfo().then((info) => (app.systemInfo = info), () => undefined);
  try {
    const text = await api.loadSettings("performance");
    app.performance = mergeStoredPerformance(text ? JSON.parse(text) : {});
  } catch {
    app.performance = { ...DEFAULT_PERFORMANCE };
  }
  applyPerformance(false);
}
