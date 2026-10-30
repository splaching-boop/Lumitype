// Commandes de la visionneuse (src-tauri/src/viewer.rs).
import { invoke } from "@tauri-apps/api/core";

export interface ViewerEntry {
  path: string;
  name: string;
  size: number;
  modified: number;
}

export interface ViewerInfo {
  width: number;
  height: number;
  format: string;
  depth: string | null;
  color: string | null;
  profile: string | null;
  size: number;
  modified: number;
  camera: string | null;
  lens: string | null;
  exposure: string | null;
  taken: string | null;
  orientation: number;
  rotatable: boolean;
}

export const viewerApi = {
  start: () => invoke<string | null>("viewer_start"),
  folder: (path: string) => invoke<{ folder: string; items: ViewerEntry[] }>("viewer_folder", { path }),
  image: (path: string) => invoke<ArrayBuffer>("viewer_image", { path }),
  thumb: (path: string, side: number) => invoke<ArrayBuffer>("viewer_thumb", { path, side }),
  /** Version réduite à la taille de l'écran : affichée d'abord, la pleine résolution venant au zoom. */
  screen: (path: string, side: number) => invoke<ArrayBuffer>("viewer_screen", { path, side }),
  info: (path: string) => invoke<ViewerInfo>("viewer_info", { path }),
  rotate: (path: string, quarters: number) => invoke<void>("viewer_rotate", { path, quarters }),
  trash: (path: string) => invoke<void>("viewer_trash", { path }),
  copy: (path: string) => invoke<void>("viewer_copy", { path }),
  edit: (path: string) => invoke<void>("viewer_edit", { path }),
  reveal: (path: string) => invoke<void>("reveal_path", { path }),
};
