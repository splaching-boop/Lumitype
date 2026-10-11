// Fichiers récents de l'écran d'accueil : chemin, nom, taille, date, épinglé. Gardés dans les réglages (« recents »),
// avec un petit aperçu PNG par fichier dans le dossier de données de l'application (voir save_preview).
import { type DocumentInfo, api } from "./ipc";

export interface Recent {
  path: string;
  name: string;
  width: number;
  height: number;
  /** Dernière ouverture ou dernier enregistrement, en millisecondes depuis 1970. */
  at: number;
  pinned: boolean;
  /** Clé de l'aperçu (dérivée du chemin). */
  key: string;
}

/**
 * Aucune limite de date, et aucune limite de nombre à l'usage : le garde-fou ne sert qu'à empêcher
 * le fichier de réglages de grossir sans fin. Les épinglés ne sont jamais retirés par ce garde-fou.
 */
const SAFETY_LIMIT = 500;

/** Ne garde que les plus récents au-delà du garde-fou, épinglés compris quoi qu'il arrive. */
function capped(list: Recent[]): Recent[] {
  if (list.length <= SAFETY_LIMIT) return list;
  const pinned = list.filter((r) => r.pinned);
  const rest = list.filter((r) => !r.pinned).slice(0, Math.max(0, SAFETY_LIMIT - pinned.length));
  return list.filter((r) => r.pinned || rest.includes(r));
}

export const recents = $state<{ list: Recent[]; previews: Record<string, string> }>({ list: [], previews: {} });

/** Clé stable d'un chemin (FNV-1a 64 bits, en hexadécimal) : nom de fichier de son aperçu. */
export function keyOf(path: string): string {
  let hash = 0xcbf29ce484222325n;
  for (const unit of new TextEncoder().encode(path)) hash = BigInt.asUintN(64, (hash ^ BigInt(unit)) * 0x100000001b3n);
  return hash.toString(16).padStart(16, "0");
}

const fileName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

function persist(): void {
  const list = recents.list.map(({ path, name, width, height, at, pinned }) => ({ path, name, width, height, at, pinned }));
  void api.saveSettings("recents", JSON.stringify(list, null, 2)).catch(() => undefined);
}

/** Au démarrage : liste lue dans les réglages, puis les aperçus. */
export async function loadRecents(): Promise<void> {
  const stored: unknown = JSON.parse((await api.loadSettings("recents")) ?? "[]");
  if (!Array.isArray(stored)) return;
  recents.list = stored
    .filter((entry): entry is Omit<Recent, "key"> => typeof entry?.path === "string")
    .map((entry) => ({ ...entry, name: entry.name || fileName(entry.path), pinned: !!entry.pinned, key: keyOf(entry.path) }));
  for (const recent of recents.list) void loadPreview(recent.key);
}

async function loadPreview(key: string): Promise<void> {
  const png = await api.loadPreview(key).catch(() => new ArrayBuffer(0));
  if (png.byteLength === 0) return;
  const previous = recents.previews[key];
  recents.previews[key] = URL.createObjectURL(new Blob([png], { type: "image/png" }));
  if (previous) URL.revokeObjectURL(previous);
}

/** Un fichier vient d'être ouvert ou enregistré : il passe en tête des récents, avec un aperçu à jour. */
export function recordRecent(path: string, info: DocumentInfo): void {
  const key = keyOf(path);
  const known = recents.list.find((r) => r.path === path);
  const entry: Recent = { path, name: fileName(path), width: info.width, height: info.height, at: Date.now(), pinned: known?.pinned ?? false, key };
  recents.list = capped([entry, ...recents.list.filter((r) => r.path !== path)]);
  persist();
  // Aperçu calculé en coulisse : l'ouverture ou l'enregistrement n'attend pas.
  void api.savePreview(info.id, key).then(() => loadPreview(key), () => undefined);
}

export function togglePinned(path: string): void {
  recents.list = recents.list.map((r) => (r.path === path ? { ...r, pinned: !r.pinned } : r));
  persist();
}

/** Aperçu déjà chargé pour ce chemin, ou une chaîne vide (document jamais enregistré, par exemple). */
export const previewFor = (path: string | null): string => (path ? (recents.previews[keyOf(path)] ?? "") : "");

/** Montre le fichier dans l'explorateur du système. */
export async function revealRecent(path: string): Promise<void> {
  await api.revealPath(path);
}

export function forgetRecent(path: string): void {
  const gone = recents.list.find((r) => r.path === path);
  recents.list = recents.list.filter((r) => r.path !== path);
  if (gone) {
    void api.deletePreview(gone.key).catch(() => undefined);
    const url = recents.previews[gone.key];
    if (url) URL.revokeObjectURL(url);
    delete recents.previews[gone.key];
  }
  persist();
}

/** « Aujourd'hui, 22:41 », « Hier », « 20 sept. » */
export function when(at: number): string {
  const date = new Date(at);
  const today = new Date();
  const days = Math.round((new Date(today.toDateString()).getTime() - new Date(date.toDateString()).getTime()) / 86_400_000);
  const time = date.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });
  if (days === 0) return `Aujourd'hui, ${time}`;
  if (days === 1) return `Hier, ${time}`;
  return date.toLocaleDateString("fr-FR", { day: "numeric", month: "short", year: date.getFullYear() === today.getFullYear() ? undefined : "numeric" });
}
