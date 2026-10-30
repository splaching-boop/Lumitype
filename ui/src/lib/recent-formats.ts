// Historique des formats utilisés : à chaque CRÉATION d'un document (formats rapides et page Formats de l'accueil,
// fenêtre Nouveau document, Ctrl+N), sa taille passe en tête. Ouvrir un fichier ne compte pas.
//
// Les formats rapides de l'accueil se rangent du plus récent au plus ancien : le format utilisé passe premier,
// les autres reculent d'un cran, sans doublon ; sans historique, l'ordre par défaut. Un format libre
// (1200 × 628, 72 ppp, fond transparent…) devient une carte comme les autres et recrée le même document.
//
// Gardé dans les réglages de l'application (« recent-formats »), comme les fichiers récents.
import { writable } from "svelte/store";
import { DEFAULT_QUICK_IDS, FORMAT_CATEGORIES, type Format, formatById } from "./formats";
import { api } from "./ipc";

export interface UsedFormat {
  width: number;
  height: number;
  dpi: number;
  transparent: boolean;
  /**
   * Format choisi (`formats.ts`), s'il y en a un : du catalogue, ou format personnel (lib/custom-formats.ts), qui
   * s'affiche alors avec son nom tant qu'il existe et garde sa taille.
   */
  formatId?: string;
  /** Taille « HD » de ce format. */
  hd?: boolean;
}

/** Une case des formats rapides : un format du catalogue, ou une taille libre déjà utilisée. */
export interface QuickFormat extends UsedFormat {
  /** Clé d'affichage, unique dans la liste. */
  key: string;
  /** Format du catalogue (libellé, note), absent pour une taille libre. */
  format: Format | null;
}

const SETTINGS_NAME = "recent-formats";
/** Assez pour la catégorie « Dernier utilisé » de la page Formats ; le fichier ne grossit pas au-delà. */
export const HISTORY_LIMIT = 12;
/** Cases des formats rapides, hors « Personnalisé… ». */
export const QUICK_COUNT = 5;

/** Même document : même taille, même résolution, même fond. C'est la règle « sans doublon ». */
export const sameDocument = (a: UsedFormat, b: UsedFormat): boolean =>
  a.width === b.width && a.height === b.height && a.dpi === b.dpi && a.transparent === b.transparent;

/** Ce format, en Standard ou en HD, sur son fond (blanc, sauf un format personnel transparent). */
export function fromFormat(format: Format, useHd = false): UsedFormat {
  const size = useHd && format.hd ? format.hd : format;
  return { width: size.width, height: size.height, dpi: format.dpi, transparent: !!format.transparent, formatId: format.id, ...(useHd && format.hd ? { hd: true } : {}) };
}

/**
 * Le format du catalogue qui correspond à une taille (création depuis une série de la fenêtre Nouveau document,
 * ou ancien historique) : le premier dans l'ordre du catalogue, Standard d'abord.
 */
function catalogFor(entry: UsedFormat): Format | null {
  if (entry.formatId) {
    const known = formatById(entry.formatId);
    const size = entry.hd ? known?.hd : known;
    if (known && size && size.width === entry.width && size.height === entry.height && known.dpi === entry.dpi) return known;
  }
  // Sans identifiant, une taille n'est reconnue que parmi les formats génériques (Essentiels, Impression, cartes) :
  // une taille libre de 1200 × 628 ne doit pas s'afficher « Image de lien · X » pour autant, ni prendre le nom
  // d'un format personnel de même taille. Un format personnel supprimé ou redimensionné redevient une taille libre.
  return GENERIC_FORMATS.find((f) => f.dpi === entry.dpi && f.width === entry.width && f.height === entry.height) ?? null;
}
const GENERIC_FORMATS: Format[] = FORMAT_CATEGORIES.filter((c) => c.group === "base").flatMap((c) => c.formats);

/** L'historique après une création : la nouvelle entrée en tête, l'ancienne identique retirée. */
export function pushHistory(history: UsedFormat[], used: UsedFormat): UsedFormat[] {
  return [used, ...history.filter((h) => !sameDocument(h, used))].slice(0, HISTORY_LIMIT);
}

/** Une entrée de l'historique, prête à afficher. */
export function describe(entry: UsedFormat): QuickFormat {
  const format = catalogFor(entry);
  const hd = !!format && !!entry.hd && !!format.hd && format.hd.width === entry.width && format.hd.height === entry.height;
  return {
    width: entry.width,
    height: entry.height,
    dpi: entry.dpi,
    transparent: entry.transparent,
    ...(format ? { formatId: format.id } : {}),
    ...(hd ? { hd: true } : {}),
    format,
    key: `${entry.width}x${entry.height}@${entry.dpi}${entry.transparent ? "t" : ""}`,
  };
}

/**
 * Les cinq formats rapides : l'historique d'abord, du plus récent au plus ancien, puis les formats par défaut dans
 * leur ordre — sans doublon (même taille, même résolution, même fond).
 */
export function quickFormats(history: UsedFormat[], count = QUICK_COUNT): QuickFormat[] {
  const defaults = DEFAULT_QUICK_IDS.map(formatById)
    .filter((f): f is Format => !!f)
    .map((f) => fromFormat(f));
  const out: QuickFormat[] = [];
  for (const entry of [...history, ...defaults]) {
    if (out.length >= count) break;
    if (!out.some((q) => sameDocument(q, entry))) out.push(describe(entry));
  }
  return out;
}

/** Lecture prudente du fichier de réglages : une entrée abîmée est ignorée, pas le reste. */
export function parseHistory(text: string | null): UsedFormat[] {
  let stored: unknown;
  try {
    stored = JSON.parse(text ?? "[]");
  } catch {
    return [];
  }
  if (!Array.isArray(stored)) return [];
  const side = (v: unknown): v is number => typeof v === "number" && Number.isInteger(v) && v >= 1 && v <= 30000;
  return stored
    .filter((e): e is UsedFormat => !!e && typeof e === "object" && side(e.width) && side(e.height) && side(e.dpi))
    .map((e) => ({
      width: e.width,
      height: e.height,
      dpi: e.dpi,
      transparent: !!e.transparent,
      ...(typeof e.formatId === "string" ? { formatId: e.formatId } : {}),
      ...(e.hd ? { hd: true } : {}),
    }))
    .slice(0, HISTORY_LIMIT);
}

/** L'historique, du plus récent au plus ancien. À lire avec `$formatHistory` dans un composant. */
export const formatHistory = writable<UsedFormat[]>([]);

let loading: Promise<void> | null = null;
/** Lu une seule fois par lancement ; les appels suivants attendent la même lecture. */
export function loadFormatHistory(): Promise<void> {
  loading ??= api.loadSettings(SETTINGS_NAME).then(
    (text) => formatHistory.set(parseHistory(text)),
    () => undefined,
  );
  return loading;
}

/** Un document vient d'être créé avec ce format : il passe en tête, et c'est enregistré. */
export async function recordFormat(used: UsedFormat): Promise<void> {
  await loadFormatHistory(); // ne jamais écraser un historique pas encore lu
  let next: UsedFormat[] = [];
  formatHistory.update((history) => (next = pushHistory(history, used)));
  await api.saveSettings(SETTINGS_NAME, JSON.stringify(next, null, 2)).catch(() => undefined);
}
