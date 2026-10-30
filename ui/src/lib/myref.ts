// « My ref » : bibliothèque d'images de référence de l'accueil (collections, favoris, notes).
// Tout est rangé par le processus Rust dans un dossier choisi par l'utilisateur, en fichiers ordinaires
// (src-tauri/src/myref.rs) ; ce module en est l'accès pour l'interface : appels, dossier mémorisé dans les
// réglages, lecture de ce qu'on dépose (fichiers, images de navigateur), vignettes.
import { invoke } from "@tauri-apps/api/core";
import { api } from "./ipc";

/** Une image de la bibliothèque. `id` = « collection/fichier ». */
export interface RefItem {
  id: string;
  collection: string;
  file: string;
  /** Chemin complet de la copie rangée dans la bibliothèque. */
  path: string;
  name: string;
  /** Chemin du fichier d'origine, adresse web, ou vide (glisser-déposer). */
  source: string;
  tags: string[];
  note: string;
  favorite: boolean;
  /** Date d'ajout, en millisecondes. */
  added: number;
  width: number | null;
  height: number | null;
  size: number;
  colors: string[];
}

export interface RefCollectionSummary {
  name: string;
  count: number;
  /** Image choisie comme couverture (« Changer la couverture… ») : montrée seule, en grand. */
  cover: RefItem | null;
  /** Sans couverture choisie : mosaïque des quatre derniers ajouts (vide si `cover` est donnée). */
  covers: RefItem[];
  favorites: number;
  modified: number;
}

export interface RefOverview {
  root: string;
  collections: RefCollectionSummary[];
  favorites: RefItem[];
  recent: RefItem[];
  total: number;
}

/** Tri d'une collection (menu « Trier »), mémorisé dans `myref.json`. */
export type RefSort = "recent" | "oldest" | "name";

export interface RefCollection {
  name: string;
  items: RefItem[];
  /** Fichier choisi comme couverture, `null` : mosaïque automatique. */
  cover: string | null;
  /** Tri coché dans le menu « Trier », `null` pour un ordre fait à la main (Organiser). */
  sort: RefSort | null;
  modified: number;
}

export interface RefPatch {
  name?: string;
  tags?: string[];
  note?: string;
  favorite?: boolean;
}

/** Appels au processus Rust (commandes `myref_*`). `root` : dossier de la bibliothèque. */
export const refApi = {
  overview: (root: string) => invoke<RefOverview>("myref_overview", { root }),
  collection: (root: string, name: string) => invoke<RefCollection>("myref_collection", { root, name }),
  search: (root: string, query: string) => invoke<RefItem[]>("myref_search", { root, query }),
  createCollection: (root: string, name: string) => invoke<string>("myref_create_collection", { root, name }),
  renameCollection: (root: string, name: string, newName: string) => invoke<string>("myref_rename_collection", { root, name, newName }),
  deleteCollection: (root: string, name: string) => invoke<void>("myref_delete_collection", { root, name }),
  reorderCollections: (root: string, names: string[]) => invoke<void>("myref_reorder_collections", { root, names }),
  importPaths: (root: string, collection: string, paths: string[]) => invoke<RefItem[]>("myref_import_paths", { root, collection, paths }),
  /** Contenu en corps binaire, paramètres en en-têtes (encodés : les en-têtes n'acceptent que l'ASCII). */
  importBytes: (root: string, collection: string, fileName: string, bytes: Uint8Array, source = "") =>
    invoke<RefItem>("myref_import_bytes", bytes, {
      headers: {
        "x-root": encodeURIComponent(root),
        "x-collection": encodeURIComponent(collection),
        "x-file-name": encodeURIComponent(fileName),
        "x-source": encodeURIComponent(source),
      },
    }),
  importUrl: (root: string, collection: string, url: string) => invoke<RefItem>("myref_import_url", { root, collection, url }),
  updateItem: (root: string, id: string, patch: RefPatch) => invoke<RefItem>("myref_update_item", { root, id, patch }),
  /** Renomme le fichier (extension gardée) : l'identifiant de l'image change. */
  renameItem: (root: string, id: string, name: string) => invoke<RefItem>("myref_rename_item", { root, id, name }),
  /** Collections qui contiennent cette image (copies identiques comprises). */
  memberships: (root: string, id: string) => invoke<string[]>("myref_memberships", { root, id }),
  /** Ajoute l'image à une collection (copie, sauf si elle y est déjà). */
  copyItem: (root: string, id: string, to: string) => invoke<RefItem>("myref_copy_item", { root, id, to }),
  /** Retire l'image d'une collection (copie mise à la corbeille de la bibliothèque). */
  removeFrom: (root: string, id: string, from: string) => invoke<void>("myref_remove_from", { root, id, from }),
  moveItem: (root: string, id: string, to: string) => invoke<RefItem>("myref_move_item", { root, id, to }),
  reorderItems: (root: string, collection: string, files: string[]) => invoke<void>("myref_reorder_items", { root, collection, files }),
  /** Trie la collection et retient ce tri (les images ajoutées ensuite y prennent leur place). */
  sortItems: (root: string, collection: string, sort: RefSort) => invoke<RefCollection>("myref_sort_items", { root, collection, sort }),
  /** `file` : une image de la collection ; `null` : retour à la mosaïque automatique. */
  setCover: (root: string, collection: string, file: string | null) => invoke<void>("myref_set_cover", { root, collection, file }),
  /** Ouvre le dossier de la collection dans le gestionnaire de fichiers (chemin résolu côté Rust d'après le nom). */
  openCollection: (root: string, name: string) => invoke<void>("myref_open_collection", { root, name }),
  deleteItem: (root: string, id: string) => invoke<void>("myref_delete_item", { root, id }),
  thumbnail: (root: string, id: string) => invoke<ArrayBuffer>("myref_thumbnail", { root, id }),
  image: (root: string, id: string) => invoke<ArrayBuffer>("myref_image", { root, id }),
};

// ------------------------------------------------------------------ dossier de la bibliothèque

const SETTINGS_NAME = "myref";

/** Dossier choisi, relu des réglages de l'application (`myref.json` du dossier de configuration). */
export async function loadRefRoot(): Promise<string | null> {
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : null;
    const root = (stored as { root?: unknown } | null)?.root;
    return typeof root === "string" && root.trim() !== "" ? root : null;
  } catch {
    return null;
  }
}

export async function saveRefRoot(root: string): Promise<void> {
  await api.saveSettings(SETTINGS_NAME, JSON.stringify({ root }));
}

/** Chemin raccourci pour l'affichage : le dossier personnel devient « ~ ». */
export function shortPath(path: string): string {
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^[A-Za-z]:\\Users\\[^\\]+/, "~");
}

// ------------------------------------------------------------------ vignettes

/**
 * Vignettes déjà reçues (adresses `blob:`), par identifiant et date d'ajout : une image remplacée sous le même nom
 * n'hérite pas de l'ancienne. Les demandes partent quatre à la fois : une collection de 300 images n'inonde pas
 * le processus Rust, qui décode de toute façon deux images au plus en même temps. La table est bornée (les moins
 * récemment demandées sont oubliées, leur adresse révoquée) : parcourir des milliers d'images ne garde pas tout.
 */
const thumbs = new Map<string, Promise<string>>();
const THUMBS_MAX = 600;
const waiting: (() => void)[] = [];
let running = 0;

function slot<T>(work: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const start = () => {
      running += 1;
      work()
        .then(resolve, reject)
        .finally(() => {
          running -= 1;
          waiting.shift()?.();
        });
    };
    if (running < 4) start();
    else waiting.push(start);
  });
}

const blobUrl = (bytes: ArrayBuffer, type: string) => URL.createObjectURL(new Blob([bytes], { type }));

/**
 * Type d'une vignette d'après ses premiers octets : WebP (vignettes actuelles), ou JPEG (vignettes des versions
 * précédentes, servies tant qu'elles sont à jour).
 */
export function thumbnailMime(bytes: ArrayBuffer): string {
  const head = new Uint8Array(bytes, 0, Math.min(12, bytes.byteLength));
  const ascii = (from: number, to: number) => String.fromCharCode(...head.subarray(from, to));
  if (head.length >= 12 && ascii(0, 4) === "RIFF" && ascii(8, 12) === "WEBP") return "image/webp";
  if (head[0] === 0x89 && ascii(1, 4) === "PNG") return "image/png";
  return "image/jpeg";
}
const mimeOf = (file: string) => {
  const ext = file.split(".").pop()?.toLowerCase() ?? "";
  return ext === "png" ? "image/png" : ext === "webp" ? "image/webp" : ext === "avif" ? "image/avif" : "image/jpeg";
};

/** Adresse affichable de la vignette. Une AVIF (que le moteur ne décode pas) est montrée par la webview elle-même. */
export function thumbnailUrl(root: string, item: RefItem): Promise<string> {
  const key = `${root}|${item.id}|${item.added}|${item.size}`;
  let url = thumbs.get(key);
  if (url) {
    // Réinsérée en fin de table : l'ordre d'insertion sert d'ordre d'usage (la première est la plus ancienne).
    thumbs.delete(key);
    thumbs.set(key, url);
    return url;
  }
  url = slot(() => refApi.thumbnail(root, item.id))
    .then((bytes) => blobUrl(bytes, thumbnailMime(bytes)))
    .catch(() => slot(() => refApi.image(root, item.id)).then((bytes) => blobUrl(bytes, mimeOf(item.file))));
  url.catch(() => thumbs.delete(key));
  thumbs.set(key, url);
  while (thumbs.size > THUMBS_MAX) {
    const oldest = thumbs.keys().next().value;
    if (oldest === undefined) break;
    const evicted = thumbs.get(oldest);
    thumbs.delete(oldest);
    // Révoquée une fois résolue ; une vignette encore à l'écran se redemande d'elle-même (RefThumb, `onerror`).
    void evicted?.then((u) => URL.revokeObjectURL(u), () => undefined);
  }
  return url;
}

/** Image en grand pour la visionneuse (le fichier lui-même, ou une réduction JPEG pour HEIC, TIFF, DNG). */
export async function fullImageUrl(root: string, item: RefItem): Promise<string> {
  const bytes = await refApi.image(root, item.id);
  const ext = item.file.split(".").pop()?.toLowerCase() ?? "";
  return blobUrl(bytes, ["png", "webp", "avif", "jpg", "jpeg", "jfif"].includes(ext) ? mimeOf(item.file) : "image/jpeg");
}

// ------------------------------------------------------------------ dépôts

/** Ce qu'un glisser venu de l'extérieur apporte : fichiers lus en mémoire, chemins locaux, ou adresse web. */
export interface Dropped {
  files: File[];
  paths: string[];
  urls: string[];
}

export const carriesDrop = (data: DataTransfer | null): boolean =>
  !!data && (data.types.includes("Files") || data.types.includes("text/uri-list") || data.types.includes("text/html"));

/**
 * Lit un dépôt de la page. Sous Linux, un fichier du gestionnaire de fichiers n'arrive souvent que par son adresse
 * file:// ; un navigateur fournit l'image comme fichier, ou seulement son adresse (liste d'URI, ou balise <img>).
 */
export function readDrop(data: DataTransfer): Dropped {
  const files = [...data.files].filter((file) => file.size > 0);
  const uris = data
    .getData("text/uri-list")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
  const paths = uris
    .filter((uri) => uri.toLowerCase().startsWith("file://"))
    .map((uri) => {
      try {
        const path = decodeURIComponent(new URL(uri).pathname);
        return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path; // file:///C:/… sous Windows
      } catch {
        return "";
      }
    })
    .filter((path) => path !== "");
  const fromTag = /<img[^>]+src=["']([^"']+)["']/i.exec(data.getData("text/html"))?.[1]?.replaceAll("&amp;", "&");
  const urls = [fromTag, ...uris].filter((url): url is string => !!url && /^https?:\/\//i.test(url));
  return { files, paths, urls: urls.slice(0, 1) };
}

/** Dépôt reçu par la FENÊTRE (Tauri, Linux avec Chromium) : chemins locaux, ou adresses web (l'image d'abord). */
export function readWindowDrop(paths: readonly string[]): Dropped {
  const urls = paths.filter((path) => /^https?:\/\//i.test(path));
  return { files: [], paths: paths.filter((path) => !urls.includes(path)), urls: urls.slice(0, 1) };
}

/** Range un dépôt dans une collection. Rend le nombre d'images ajoutées ; lève l'erreur si aucune ne l'a été. */
export async function importDrop(root: string, collection: string, drop: Dropped): Promise<number> {
  if (drop.files.length > 0) {
    let count = 0;
    let failure: unknown = null;
    for (const file of drop.files) {
      try {
        await refApi.importBytes(root, collection, file.name || "image", new Uint8Array(await file.arrayBuffer()), "");
        count += 1;
      } catch (error) {
        failure = error;
      }
    }
    if (count === 0 && failure) throw failure;
    return count;
  }
  if (drop.paths.length > 0) return (await refApi.importPaths(root, collection, drop.paths)).length;
  if (drop.urls[0]) return (await refApi.importUrl(root, collection, drop.urls[0]), 1);
  return 0;
}

/**
 * Aiguillage des dépôts reçus par la fenêtre (événements Tauri) : la vue My ref, quand elle est affichée, s'inscrit
 * ici. App.svelte lui demande d'abord si le dépôt la concerne (cible sous le pointeur) ; sinon, il fait son import
 * habituel (ouvrir l'image comme document). `position` : coordonnées CSS dans la fenêtre.
 */
export interface WindowDropTarget {
  hover(position: { x: number; y: number } | null): void;
  drop(paths: readonly string[], position: { x: number; y: number }): boolean;
}
let windowTarget: WindowDropTarget | null = null;

export function setWindowDropTarget(target: WindowDropTarget | null): void {
  windowTarget = target;
}

/** Pour App.svelte : position physique de Tauri → coordonnées CSS. */
const toCss = (position: { x: number; y: number }) => ({ x: position.x / devicePixelRatio, y: position.y / devicePixelRatio });

export function hoverWindowDrop(position: { x: number; y: number } | null): void {
  windowTarget?.hover(position ? toCss(position) : null);
}

/** `true` : le dépôt est pris par My ref, App.svelte ne l'importe pas. */
export function claimWindowDrop(paths: readonly string[], position: { x: number; y: number }): boolean {
  return windowTarget?.drop(paths, toCss(position)) ?? false;
}

// ------------------------------------------------------------------ affichage

export { formatBytes } from "./format-bytes";

export function formatKind(file: string): string {
  const ext = file.split(".").pop()?.toUpperCase() ?? "";
  return ext === "JPG" || ext === "JFIF" ? "JPEG" : ext === "TIF" ? "TIFF" : ext;
}
