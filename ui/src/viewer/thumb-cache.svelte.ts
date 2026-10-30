// Vignettes de la pellicule, partagées avec la vue principale : la vignette d'une image s'affiche tout de suite
// (agrandie, floue) pendant que sa version « écran » se prépare.
import { viewerApi } from "./viewer-api";

export const thumbs = $state<Record<string, string>>({});
const asked = new Set<string>();

/** Demande la vignette d'une image (une seule fois). */
export function requestThumb(path: string): void {
  if (asked.has(path)) return;
  asked.add(path);
  viewerApi.thumb(path, 160).then(
    (buffer) => (thumbs[path] = URL.createObjectURL(new Blob([buffer]))),
    () => asked.delete(path),
  );
}

/** Refait la vignette (image pivotée) ou l'oublie (image supprimée). */
export function dropThumb(path: string, again = false): void {
  const url = thumbs[path];
  if (url) URL.revokeObjectURL(url);
  delete thumbs[path];
  asked.delete(path);
  if (again) requestThumb(path);
}
