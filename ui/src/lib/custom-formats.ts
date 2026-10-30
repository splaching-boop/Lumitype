// Catégories de formats personnelles (« Client Dupont » : Bandeau site, Flyer A5…) : créées sur la page Formats de
// l'accueil, visibles aussi dans la fenêtre Nouveau document, après les plateformes.
//
// Gardées dans les réglages de l'application (« formats-perso », dossier de configuration : les mises à jour n'y
// touchent pas), relues avec prudence : une valeur hors bornes est ramenée dans les bornes, une entrée illisible est
// ignorée sans perdre le reste, un identifiant en double en reçoit un nouveau. Un fichier qui n'est pas du JSON
// (abîmé, modifié à la main) n'est jamais écrasé : les changements restent en mémoire. Et avant chaque écriture qui
// remplace des catégories, l'ancien contenu est copié dans « formats-perso-bak » (le nom « formats-perso.bak » n'est
// pas permis : les noms de réglages n'ont que des minuscules et des tirets, voir src-tauri/src/settings.rs).
//
// Export / import (page Formats) : le même texte que les réglages, dans un fichier .json choisi par l'utilisateur ;
// l'import AJOUTE les catégories du fichier (fusion, voir `mergeCustomFormats`).
//
// Les fonctions de modification sont pures (liste → nouvelle liste) : la page les applique par `editCustomFormats`,
// qui enregistre aussitôt. Le résultat est publié dans `customCategories` (lib/formats.ts), la source unique des
// deux écrans.
import { type CatalogCategory, type Format, ALL_FORMATS, FORMAT_CATEGORIES, customCategories } from "./formats";
import { api } from "./ipc";

/** Format personnel : une taille, une résolution, un fond. Pas de taille « HD » ni de note. */
export interface CustomFormat {
  id: string;
  label: string;
  width: number;
  height: number;
  dpi: number;
  transparent: boolean;
}

export interface CustomCategory {
  id: string;
  label: string;
  formats: CustomFormat[];
}

/** Ce que remplit le panneau « Nouveau format » (l'identifiant est donné à l'ajout). */
export type FormatDraft = Omit<CustomFormat, "id">;

const SETTINGS_NAME = "formats-perso";
/** Copie du contenu remplacé par la dernière écriture. */
export const BACKUP_SETTINGS_NAME = "formats-perso-bak";
/** Taille maximale d'un fichier importé (et exporté : la commande `write_json_file` refuse au-delà). */
export const MAX_FILE_BYTES = 1024 * 1024;
const VERSION = 1;
/** Bornes de la fenêtre Nouveau document : de 1 à 30000 px par côté, de 1 à 9600 ppp. */
export const MAX_SIDE = 30000;
export const MAX_DPI = 9600;
export const MAX_LABEL = 80;
/** Garde-fous : un fichier énorme (ou abîmé) ne fige pas l'accueil. */
export const MAX_CATEGORIES = 50;
export const MAX_FORMATS = 200;

const CATEGORY_PREFIX = "perso-cat-";
const FORMAT_PREFIX = "perso-";

/** Un entier dans [min, max] ; `null` si ce n'est pas un nombre du tout. */
function bounded(value: unknown, min: number, max: number): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** Nom nettoyé : une ligne, sans espaces superflus, longueur bornée. Vide si rien d'utilisable. */
export function cleanLabel(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/\s+/g, " ").trim().slice(0, MAX_LABEL).trim();
}

/** Le brouillon est-il prêt à être enregistré ? (nom non vide, côtés et résolution entiers dans les bornes) */
export function validDraft(draft: FormatDraft): boolean {
  const int = (v: number, max: number) => Number.isInteger(v) && v >= 1 && v <= max;
  return cleanLabel(draft.label) !== "" && int(draft.width, MAX_SIDE) && int(draft.height, MAX_SIDE) && int(draft.dpi, MAX_DPI);
}

/** Identifiants déjà pris : ceux du catalogue livré (jamais réutilisés) et ceux de la liste. */
function takenIds(list: CustomCategory[]): Set<string> {
  return new Set([...FORMAT_CATEGORIES.map((c) => c.id), ...ALL_FORMATS.map((f) => f.id), ...list.flatMap((c) => [c.id, ...c.formats.map((f) => f.id)])]);
}

/** Nouvel identifiant, absent de `taken` (et ajouté à `taken`). */
export function freshId(prefix: string, taken: Set<string>): string {
  for (;;) {
    const id = `${prefix}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`;
    if (!taken.has(id)) {
      taken.add(id);
      return id;
    }
  }
}

/** Identifiant lu : gardé s'il est bien formé et libre, sinon remplacé (une copie du fichier à la main, par exemple). */
function keepOrRenew(value: unknown, prefix: string, taken: Set<string>): string {
  if (typeof value === "string" && value.startsWith(prefix) && /^[a-z0-9-]{1,64}$/.test(value) && !taken.has(value)) {
    taken.add(value);
    return value;
  }
  return freshId(prefix, taken);
}

function readFormat(raw: unknown, taken: Set<string>): CustomFormat | null {
  if (!raw || typeof raw !== "object") return null;
  const e = raw as Record<string, unknown>;
  const label = cleanLabel(e.label);
  const width = bounded(e.width, 1, MAX_SIDE);
  const height = bounded(e.height, 1, MAX_SIDE);
  const dpi = bounded(e.dpi, 1, MAX_DPI) ?? 72;
  if (!label || width === null || height === null) return null;
  // Les catégories utilisent un préfixe plus long : un identifiant de catégorie n'est pas un identifiant de format.
  const id = typeof e.id === "string" && e.id.startsWith(CATEGORY_PREFIX) ? freshId(FORMAT_PREFIX, taken) : keepOrRenew(e.id, FORMAT_PREFIX, taken);
  return { id, label, width, height, dpi, transparent: e.transparent === true };
}

/**
 * Lecture tolérante (réglages ou fichier importé) : `{ version, categories }`, ou directement la liste. Une catégorie
 * sans nom est ignorée, un format illisible aussi ; le reste est gardé. `taken` : identifiants déjà utilisés ailleurs.
 */
export function parseCustomFormats(text: string | null, taken: Set<string> = takenIds([])): CustomCategory[] {
  return readCustomFormats(text, taken).list;
}

/**
 * Comme `parseCustomFormats`, en disant si le texte était lisible : `readable` est faux quand ce n'est pas du JSON, ou
 * pas une liste de catégories (ni `{ categories: [...] }`). Pas de texte (réglages jamais écrits) : lisible, vide.
 */
export function readCustomFormats(text: string | null, taken: Set<string> = takenIds([])): { list: CustomCategory[]; readable: boolean } {
  if (text === null || text.trim() === "") return { list: [], readable: true };
  let stored: unknown;
  try {
    stored = JSON.parse(text);
  } catch {
    return { list: [], readable: false };
  }
  const list = Array.isArray(stored) ? stored : stored && typeof stored === "object" ? (stored as { categories?: unknown }).categories : null;
  if (!Array.isArray(list)) return { list: [], readable: false };
  const out: CustomCategory[] = [];
  for (const raw of list) {
    if (out.length >= MAX_CATEGORIES) break;
    if (!raw || typeof raw !== "object") continue;
    const e = raw as Record<string, unknown>;
    const label = cleanLabel(e.label);
    if (!label) continue;
    const id = keepOrRenew(e.id, CATEGORY_PREFIX, taken);
    const formats = (Array.isArray(e.formats) ? e.formats : [])
      .slice(0, MAX_FORMATS * 2)
      .map((f) => readFormat(f, taken))
      .filter((f): f is CustomFormat => f !== null)
      .slice(0, MAX_FORMATS);
    out.push({ id, label, formats });
  }
  return { list: out, readable: true };
}

/** Résultat d'un import (voir `mergeCustomFormats`). */
export interface MergeResult {
  list: CustomCategory[];
  /** Le fichier se lit (JSON, liste de catégories). */
  readable: boolean;
  /** Catégories ajoutées. */
  added: number;
  /** Noms de catégories ajoutées qui existaient déjà (sans tenir compte de la casse). */
  duplicates: string[];
  /** Catégories du fichier laissées de côté : la liste est pleine (`MAX_CATEGORIES`). */
  dropped: number;
}

/**
 * Import d'un fichier exporté : ses catégories sont AJOUTÉES après celles de la liste (rien n'est remplacé ni
 * fusionné nom à nom : deux catégories peuvent porter le même nom, l'interface demande alors confirmation). Les
 * identifiants déjà pris (liste, catalogue) sont renouvelés, comme à la lecture des réglages.
 */
export function mergeCustomFormats(list: CustomCategory[], text: string): MergeResult {
  const read = readCustomFormats(text, takenIds(list));
  if (!read.readable) return { list, readable: false, added: 0, duplicates: [], dropped: 0 };
  const kept = read.list.slice(0, Math.max(0, MAX_CATEGORIES - list.length));
  const names = new Set(list.map((c) => c.label.toLocaleLowerCase()));
  const duplicates = kept.filter((c) => names.has(c.label.toLocaleLowerCase())).map((c) => c.label);
  return { list: kept.length > 0 ? [...list, ...kept] : list, readable: true, added: kept.length, duplicates, dropped: read.list.length - kept.length };
}

/** Le texte enregistré dans les réglages. */
export function serializeCustomFormats(list: CustomCategory[]): string {
  return JSON.stringify({ version: VERSION, categories: list }, null, 2);
}

// ---- Modifications (pures) : chacune rend une nouvelle liste, l'ancienne n'est pas touchée.

/** Ajoute une catégorie à la fin ; rend la liste et l'identifiant de la nouvelle (`null` si le nom est vide). */
export function addCategory(list: CustomCategory[], label: string): { list: CustomCategory[]; id: string | null } {
  const name = cleanLabel(label);
  if (!name || list.length >= MAX_CATEGORIES) return { list, id: null };
  const id = freshId(CATEGORY_PREFIX, takenIds(list));
  return { list: [...list, { id, label: name, formats: [] }], id };
}

export function renameCategory(list: CustomCategory[], id: string, label: string): CustomCategory[] {
  const name = cleanLabel(label);
  if (!name) return list;
  return list.map((c) => (c.id === id ? { ...c, label: name } : c));
}

export function removeCategory(list: CustomCategory[], id: string): CustomCategory[] {
  return list.filter((c) => c.id !== id);
}

/** Le brouillon, nettoyé et borné. */
function fromDraft(draft: FormatDraft): FormatDraft {
  return {
    label: cleanLabel(draft.label),
    width: bounded(draft.width, 1, MAX_SIDE) ?? 1,
    height: bounded(draft.height, 1, MAX_SIDE) ?? 1,
    dpi: bounded(draft.dpi, 1, MAX_DPI) ?? 72,
    transparent: !!draft.transparent,
  };
}

/** Ajoute un format à la fin d'une catégorie ; rend la liste et l'identifiant du nouveau format. */
export function addFormat(list: CustomCategory[], categoryId: string, draft: FormatDraft): { list: CustomCategory[]; id: string | null } {
  const category = list.find((c) => c.id === categoryId);
  if (!category || !validDraft(draft) || category.formats.length >= MAX_FORMATS) return { list, id: null };
  const id = freshId(FORMAT_PREFIX, takenIds(list));
  return { list: list.map((c) => (c === category ? { ...c, formats: [...c.formats, { id, ...fromDraft(draft) }] } : c)), id };
}

/** Modifie un format : même identifiant, l'historique le retrouve (s'il garde sa taille). */
export function updateFormat(list: CustomCategory[], formatId: string, draft: FormatDraft): CustomCategory[] {
  if (!validDraft(draft)) return list;
  return list.map((c) => (c.formats.some((f) => f.id === formatId) ? { ...c, formats: c.formats.map((f) => (f.id === formatId ? { id: f.id, ...fromDraft(draft) } : f)) } : c));
}

/** Copie du format juste après lui, avec un nouvel identifiant ; `suffix` s'ajoute au nom (« (copie) », traduit). */
export function duplicateFormat(list: CustomCategory[], formatId: string, suffix: string): { list: CustomCategory[]; id: string | null } {
  const category = list.find((c) => c.formats.some((f) => f.id === formatId));
  const source = category?.formats.find((f) => f.id === formatId);
  if (!category || !source || category.formats.length >= MAX_FORMATS) return { list, id: null };
  const id = freshId(FORMAT_PREFIX, takenIds(list));
  const copy: CustomFormat = { ...source, id, label: cleanLabel(`${source.label} ${suffix}`) || source.label };
  const at = category.formats.indexOf(source) + 1;
  return { list: list.map((c) => (c === category ? { ...c, formats: [...c.formats.slice(0, at), copy, ...c.formats.slice(at)] } : c)), id };
}

/** Déplace un format à la fin d'une autre catégorie personnelle (même identifiant). */
export function moveFormat(list: CustomCategory[], formatId: string, targetId: string): CustomCategory[] {
  const source = list.find((c) => c.formats.some((f) => f.id === formatId));
  const target = list.find((c) => c.id === targetId);
  const format = source?.formats.find((f) => f.id === formatId);
  if (!source || !target || !format || source === target || target.formats.length >= MAX_FORMATS) return list;
  return list.map((c) => (c === source ? { ...c, formats: c.formats.filter((f) => f !== format) } : c === target ? { ...c, formats: [...c.formats, format] } : c));
}

export function removeFormat(list: CustomCategory[], formatId: string): CustomCategory[] {
  return list.map((c) => (c.formats.some((f) => f.id === formatId) ? { ...c, formats: c.formats.filter((f) => f.id !== formatId) } : c));
}

/** Forme du catalogue (lib/formats.ts) : groupe « custom », formats marqués `custom` (nom jamais traduit). */
export function toCatalog(list: CustomCategory[]): CatalogCategory[] {
  return list.map((c) => ({
    id: c.id,
    label: c.label,
    group: "custom",
    formats: c.formats.map((f): Format => ({ id: f.id, label: f.label, width: f.width, height: f.height, dpi: f.dpi, transparent: f.transparent, custom: true })),
  }));
}

// ---- Réglages

/** La liste telle qu'enregistrée, gardée ici pour les modifications (le magasin public est sa forme catalogue). */
let current: CustomCategory[] = [];
let loading: Promise<void> | null = null;
/**
 * Réglages illisibles (disque, droits, ou texte qui n'est pas du JSON) : on ne les écrase pas avec une liste vide ;
 * les changements restent en mémoire.
 */
let unreadable = false;
/** Texte des réglages tel que lu ou écrit en dernier (ce qu'une écriture va remplacer). */
let stored: string | null = null;

/** Lu une seule fois par lancement ; les appels suivants attendent la même lecture. */
export function loadCustomFormats(): Promise<void> {
  loading ??= api.loadSettings(SETTINGS_NAME).then(
    (text) => {
      const read = readCustomFormats(text);
      if (!read.readable) unreadable = true;
      stored = text;
      publish(read.list);
    },
    () => {
      unreadable = true;
    },
  );
  return loading;
}

/** Les réglages n'ont pas pu être lus : rien n'y sera écrit pendant ce lancement. */
export const customFormatsUnreadable = (): boolean => unreadable;

/**
 * Écrit les réglages. Si l'écriture remplace des catégories, l'ancien texte est d'abord copié dans
 * `BACKUP_SETTINGS_NAME` ; si cette copie échoue, rien n'est écrit (les changements restent en mémoire).
 */
async function save(text: string): Promise<void> {
  if (text === stored) return;
  if (stored !== null && parseCustomFormats(stored).length > 0) {
    try {
      await api.saveSettings(BACKUP_SETTINGS_NAME, stored);
    } catch {
      return;
    }
  }
  try {
    await api.saveSettings(SETTINGS_NAME, text);
    stored = text;
  } catch {
    // Disque plein, droits : la liste reste en mémoire, la prochaine modification réessaiera.
  }
}

function publish(list: CustomCategory[]): void {
  current = list;
  customCategories.set(toCatalog(list));
}

/** La liste actuelle (après lecture des réglages). */
export const customFormatList = (): CustomCategory[] => current;

/**
 * Applique une modification et l'enregistre. Rend ce que rend `change` (l'identifiant créé, par exemple).
 * N'écrase jamais des réglages pas encore lus.
 */
export async function editCustomFormats<T = void>(change: (list: CustomCategory[]) => { list: CustomCategory[]; result?: T } | CustomCategory[]): Promise<T | undefined> {
  await loadCustomFormats();
  const out = change(current);
  const next = Array.isArray(out) ? out : out.list;
  const result = Array.isArray(out) ? undefined : out.result;
  if (next === current) return result;
  publish(next);
  if (!unreadable) await save(serializeCustomFormats(next));
  return result;
}
