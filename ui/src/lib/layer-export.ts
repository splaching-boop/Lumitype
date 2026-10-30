// Export d'un calque ou d'un groupe (clic droit › Exporter) : export rapide, « Exporter sous… », copie en PNG.
// Les réglages (dossier, format, tailles…) sont mémorisés dans les réglages « layer-export » : l'export rapide les
// reprend tels quels.
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { api, type ExportFormat, type ExportOptions } from "./ipc";
import { t } from "./i18n.svelte";
import { app } from "../state/app.svelte";

export type LayerExportZone = "content" | "document" | "selection" | "artboard";
/** Une taille demandée : une échelle, ou une largeur / hauteur fixe en pixels. `suffix` s'ajoute au nom du fichier. */
export interface ExportSize {
  kind: "scale" | "width" | "height";
  value: number;
  suffix: string;
}
export interface LayerExportSettings {
  folder: string;
  format: ExportFormat;
  quality: number;
  zone: LayerExportZone;
  margin: number;
  sizes: ExportSize[];
}

export const LAYER_FORMATS: { id: ExportFormat; label: string; extension: string; alpha: boolean }[] = [
  { id: "png", label: "PNG", extension: "png", alpha: true },
  { id: "jpeg", label: "JPEG", extension: "jpg", alpha: false },
  { id: "webp", label: "WebP", extension: "webp", alpha: true },
  { id: "avif", label: "AVIF", extension: "avif", alpha: true },
  { id: "tiff", label: "TIFF", extension: "tif", alpha: true },
];

const DEFAULTS: LayerExportSettings = { folder: "", format: "png", quality: 90, zone: "content", margin: 0, sizes: [{ kind: "scale", value: 1, suffix: "" }] };
/** Réglages mémorisés (relus à chaque fois : une autre fenêtre ou un autre export a pu les changer). */
export async function loadLayerExportSettings(): Promise<LayerExportSettings> {
  try {
    const stored = JSON.parse((await api.loadSettings("layer-export")) ?? "{}") as Partial<LayerExportSettings>;
    return { ...structuredClone(DEFAULTS), ...stored, sizes: Array.isArray(stored.sizes) && stored.sizes.length > 0 ? stored.sizes : structuredClone(DEFAULTS.sizes) };
  } catch {
    return structuredClone(DEFAULTS);
  }
}

export async function saveLayerExportSettings(settings: LayerExportSettings): Promise<void> {
  await api.saveSettings("layer-export", JSON.stringify(settings, null, 2)).catch(() => undefined);
}

/** Échelle d'une taille pour une image de `width` × `height`. */
export function scaleOf(size: ExportSize, width: number, height: number): number {
  if (size.kind === "width") return size.value / Math.max(1, width);
  if (size.kind === "height") return size.value / Math.max(1, height);
  return size.value;
}

export const extensionOf = (format: ExportFormat) => LAYER_FORMATS.find((f) => f.id === format)?.extension ?? "png";

/** Chemin d'un fichier dans `folder`, avec le séparateur du dossier. */
export function joinPath(folder: string, file: string): string {
  const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return `${folder.replace(/[\\/]+$/, "")}${separator}${file}`;
}

export function optionsOf(settings: LayerExportSettings): ExportOptions {
  return { format: settings.format, quality: settings.quality, lossless: false, scale: 1 };
}

export async function chooseFolder(current: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, defaultPath: current || app.saveFolder || undefined });
  return typeof picked === "string" ? picked : null;
}

/** Exporte `ids` (ensemble, sous le nom `stem`) selon `settings` ; `width`/`height` : taille de la zone à 1×. */
export async function exportLayersWith(ids: number[], stem: string, settings: LayerExportSettings, width: number, height: number): Promise<void> {
  const doc = app.active;
  if (!doc) return;
  const extension = extensionOf(settings.format);
  const safe = await api.layerFileStem(stem);
  const targets = settings.sizes.map((size) => ({ path: joinPath(settings.folder, `${safe}${size.suffix}.${extension}`), scale: scaleOf(size, width, height) }));
  await api.exportLayers(doc.info.id, ids, settings.zone, settings.margin, optionsOf(settings), targets);
}

/** Avancement affiché pendant `work` (barre d'export). */
export async function withProgress(work: () => Promise<void>): Promise<void> {
  app.exportProgress = { progress: 0, label: t("Préparation") };
  const stop = await listen<{ progress: number; label: string }>("export-progress", ({ payload }) => (app.exportProgress = payload));
  try {
    await work();
  } finally {
    stop();
    app.exportProgress = null;
  }
}

/** Export rapide : un fichier PNG à taille réelle par calque, dans le dossier mémorisé (demandé la première fois). */
export async function quickExportLayers(ids: number[], zone: LayerExportZone = "content"): Promise<void> {
  const doc = app.active;
  if (!doc || ids.length === 0) return;
  try {
    const settings = await loadLayerExportSettings();
    if (!settings.folder || !(await api.directoryExists(settings.folder).catch(() => false))) {
      const folder = await chooseFolder(settings.folder);
      if (!folder) return;
      settings.folder = folder;
      await saveLayerExportSettings(settings);
    }
    const quick: LayerExportSettings = { ...settings, format: "png", zone, sizes: [{ kind: "scale", value: 1, suffix: "" }] };
    const names = new Map(doc.info.layers.map((layer) => [layer.id, layer.name]));
    await withProgress(async () => {
      for (const id of ids) await exportLayersWith([id], names.get(id) ?? "calque", quick, 1, 1);
    });
  } catch (error) {
    app.fail(error);
  }
}

export async function copyLayersAsPng(ids: number[]): Promise<void> {
  const doc = app.active;
  if (!doc || ids.length === 0) return;
  try {
    await api.copyLayersPng(doc.info.id, ids);
  } catch (error) {
    app.fail(error);
  }
}
