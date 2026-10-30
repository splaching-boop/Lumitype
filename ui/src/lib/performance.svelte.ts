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
   * suit pas, puis redevient nette au relâchement. Désactivée par défaut (ancien réglage `adaptiveResolution`, actif, ignoré) ; à cocher sur une petite machine.
   */
  dynamicResolution?: boolean;
  /**
   * Mode portable choisi automatiquement (coché par défaut) : actif sur une carte graphique intégrée. Décoché, c'est
   * `laptopMode` qui décide. Voir `laptopModeActive`.
   */
  laptopAuto?: boolean;
  /** Mode portable choisi à la main (quand `laptopAuto` est décoché). */
  laptopMode?: boolean;
}

export const DEFAULT_PERFORMANCE: PerformanceSettings = {
  memoryPercent: 50,
  cores: 0,
  gpuPreviews: true,
  gpuMemoryPercent: 50,
  historyStates: 50,
  showFps: false,
  dynamicResolution: false,
  scratchEnabled: true,
  scratchDir: "",
  scratchQuotaGb: 20,
  laptopAuto: true,
  laptopMode: false,
};
/**
 * Anciens réglages, remplacés par les pourcentages : `cacheMb` (cache de rendu fixe par document, nul = un quart de la
 * mémoire vive) et `gpuMemoryMb` (mémoire vidéo en Mio, nul = 1 Gio). Encore présents dans un fichier enregistré par une
 * version précédente : ignorés à la lecture, et absents du fichier à la prochaine écriture.
 */
const LEGACY_KEYS = ["cacheMb", "gpuMemoryMb", "adaptiveResolution"] as const;
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
  /** Nom de la carte (celui que donne la webview ou le système), s'il est connu et n'est pas masqué. */
  gpuName?: string | null;
}

/** Noms de cartes dédiées (mémoire vidéo à part) et de cartes intégrées (mémoire prise sur la mémoire vive). */
const DEDICATED_NAME = /nvidia|geforce|quadro|tesla|rtx|gtx|radeon\s*(rx|pro|r9|r7|hd\s*[5-9])|firepro|arc\s*a\d|arc\s*b\d/i;
const INTEGRATED_NAME = /intel|iris|uhd|hd graphics|radeon(\(tm\))?\s*graphics|radeon\s*vega|vega\s*\d+\s*graphics|radeon\s*\d{3}m|apple\s*m\d|mali|adreno|powervr|llvmpipe|softpipe|swiftshader/i;

/**
 * Carte graphique intégrée (portable) : sa mémoire vidéo est prise sur la mémoire vive. Reconnue à son nom quand il
 * est connu (Intel UHD, Radeon Graphics, Apple M…), sinon à une mémoire dédiée petite (< 1 Gio), signalée comme
 * partagée par le système, ou inconnue de lui : dans le doute, la carte est supposée intégrée (le choix prudent).
 */
export function isIntegratedGpu(info: VideoMemoryInfo): boolean {
  const name = info.gpuName?.trim() ?? "";
  if (name && DEDICATED_NAME.test(name)) return false;
  if (name && INTEGRATED_NAME.test(name)) return true;
  return info.vramMb == null || info.vramShared === true || info.vramMb < 1024;
}

/** Part de la mémoire vive réservée à la carte graphique en mode portable (1/8), prise sur le plafond de Lumitype. */
export const LAPTOP_GPU_SHARE = 8;
/** En mode portable, le cache d'un groupe Transfert tient en 4 paquets au plus (12 sinon). */
export const LAPTOP_MAX_PACKS = 4;
export const DEFAULT_MAX_PACKS = 12;

/** Mode portable en vigueur : choisi automatiquement (carte intégrée) ou à la main. */
export function laptopModeActive(perf: PerformanceSettings, info: VideoMemoryInfo): boolean {
  return perf.laptopAuto !== false ? isIntegratedGpu(info) : perf.laptopMode === true;
}

/** Ce que l'interface sait de la machine (voir `VideoMemoryInfo`) ; le nom de la carte masqué (« Apple GPU » sous Linux) est ignoré. */
export function machineInfo(): VideoMemoryInfo {
  const renderer = app.gpuStatus?.renderer ?? "";
  const onMac = /Mac/i.test(navigator.userAgent) && !/Linux/i.test(navigator.userAgent);
  const masked = !onMac && /^apple gpu$/i.test(renderer.trim());
  return { vramMb: app.systemInfo?.vramMb, vramShared: app.systemInfo?.vramShared, ramMb: app.systemInfo?.ramMb, gpuName: masked ? null : renderer };
}

/** Mode portable en vigueur pour cette machine. */
export function laptopMode(): boolean {
  return laptopModeActive(app.performance, machineInfo());
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
  // Mode portable : un seul budget. La carte reçoit 1/8 de la mémoire vive, pris sur le plafond de Lumitype.
  if (laptopModeActive(perf, info) && info.ramMb) return Math.floor(info.ramMb / LAPTOP_GPU_SHARE);
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

/**
 * Part de la mémoire vive laissée au moteur, en pour cent : en mode portable, la carte graphique (intégrée : même
 * mémoire) y prend sa part (`LAPTOP_GPU_SHARE`), pour que Lumitype tienne en tout sous le plafond réglé.
 */
export function engineMemoryPercent(perf: PerformanceSettings, info: VideoMemoryInfo): number {
  const percent = memoryPercentOf(perf);
  if (!laptopModeActive(perf, info) || !info.ramMb) return percent;
  return Math.max(MEMORY_PERCENT_RANGE.min, Math.round(percent - 100 / LAPTOP_GPU_SHARE));
}

/** Envoie les réglages au moteur ; `persist` les enregistre aussi (après une courte pause : un curseur en envoie beaucoup). */
export function applyPerformance(persist: boolean): void {
  const { cores, historyStates } = app.performance;
  void api
    .setPerformance({
      memoryPercent: engineMemoryPercent(app.performance, machineInfo()),
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
  // La mémoire de la machine décide du mode portable et de la part laissée au moteur : réglages renvoyés à son arrivée.
  api.systemInfo().then((info) => {
    app.systemInfo = info;
    applyPerformance(false);
  }, () => undefined);
  try {
    const text = await api.loadSettings("performance");
    app.performance = mergeStoredPerformance(text ? JSON.parse(text) : {});
  } catch {
    app.performance = { ...DEFAULT_PERFORMANCE };
  }
  applyPerformance(false);
}
