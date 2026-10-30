// Fichiers de pinceaux (.abr Photoshop, .gbr GIMP) glissés dans la fenêtre : installés dans la bibliothèque des
// pinceaux, le premier pinceau du pack choisi aussitôt — au lieu d'être ouverts comme des images.
import { t } from "./i18n.svelte";
import { api } from "./ipc";
import { app } from "../state/app.svelte";
import { brushLibrary } from "./brush-library.svelte";

const BRUSH_FILE = /\.(abr|gbr)$/i;

/** Vrai pour un fichier de pinceaux, d'après son nom ou son chemin. */
export function isBrushFile(name: string): boolean {
  return BRUSH_FILE.test(name);
}

/** Nom du pack tel que la bibliothèque le range (nom du fichier sans extension, « _ » en espaces). */
const packName = (name: string) => name.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "").replaceAll("_", " ") ?? name;

async function installed(names: string[], count: number): Promise<void> {
  await brushLibrary.refresh();
  const folder = brushLibrary.folders.find((f) => f.source === "pack" && f.name === packName(names[names.length - 1] ?? ""));
  if (folder?.entries[0]) brushLibrary.apply(folder.entries[0]);
  // Le menu des pinceaux s'ouvre sur le pack installé : on voit tout de suite ce qui est arrivé.
  if (count > 0 && !app.brushPanel) app.brushPanel = { x: Math.max(16, window.innerWidth / 2 - 210), y: 90 };
  if (count === 0) app.error = t("Aucun pinceau lisible dans ce fichier.");
}

/** Fichiers déposés (contenu seul, sous Windows). */
export async function installBrushFiles(files: File[]): Promise<void> {
  let count = 0;
  try {
    for (const file of files) count += (await api.importBrushesDropped(new Uint8Array(await file.arrayBuffer()), file.name)).length;
    await installed(files.map((f) => f.name), count);
  } catch (error) {
    app.fail(error);
  }
}

/** Fichiers déposés par leur chemin (Linux, macOS). */
export async function installBrushPaths(paths: string[]): Promise<void> {
  let count = 0;
  try {
    for (const path of paths) count += (await api.importBrushes(path)).length;
    await installed(paths, count);
  } catch (error) {
    app.fail(error);
  }
}
