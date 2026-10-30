// Styles de caractère (voir crates/engine/src/text_styles.rs) : résolution « Basé sur », application à un texte,
// détection des remplacements (« + »), enregistrement dans le document et bibliothèque partagée entre projets.
// La bibliothèque (kit de marque) vit dans les réglages de l'application (« text-styles ») : chaque entrée a un
// identifiant stable (`uid`) et une révision ; un style de projet qui en vient garde ce lien pour proposer les mises
// à jour.
import { edit } from "./engine";
import { api, type CharStyle, type LayerInfo, type TextCaps, type TextData } from "./ipc";
import type { RangeStyle, RunStyle } from "./text-runs";
import { app } from "../state/app.svelte";

/** Réglages effectifs d'un style (héritage compris) ; seuls les champs définis sont imposés. */
export interface ResolvedStyle {
  fontFamily?: string;
  weight?: number;
  italic?: boolean;
  size?: number;
  lineHeight?: number;
  tracking?: number;
  color?: [number, number, number];
  caps?: TextCaps;
}

const FIELDS = ["fontFamily", "weight", "italic", "size", "lineHeight", "tracking", "color", "caps"] as const;

export function resolveStyle(list: readonly CharStyle[], id: number): ResolvedStyle | null {
  const out: ResolvedStyle = {};
  const seen = new Set<number>();
  let current: number | null | undefined = id;
  let found = false;
  while (current !== null && current !== undefined && !seen.has(current)) {
    seen.add(current);
    const style = list.find((s) => s.id === current);
    if (!style) break;
    found = true;
    for (const key of FIELDS) {
      const value = style[key];
      if (out[key] === undefined && value !== null && value !== undefined) Object.assign(out, { [key]: value });
    }
    current = style.basedOn;
  }
  return found ? out : null;
}

/** Ce que le style impose à une portion (sans l'interlignage, réglage de ligne). */
export function runPatchOf(resolved: ResolvedStyle, id: number): RunStyle {
  const { lineHeight: _lineHeight, ...rest } = resolved;
  return { ...rest, style: id };
}

/** Le texte (ou la plage affichée) diffère-t-il du style qu'il porte ? */
export function overridden(shown: RangeStyle, text: TextData, resolved: ResolvedStyle | null, wholeLayer: boolean): boolean {
  if (!resolved) return false;
  const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
  for (const key of FIELDS) {
    const wanted = resolved[key];
    if (wanted === undefined) continue;
    const actual = key === "lineHeight" ? (wholeLayer ? text.lineHeight : wanted) : shown[key];
    if (!same(actual, wanted)) return true;
  }
  return false;
}

export const stylesOf = (): CharStyle[] => app.active?.info.textStyles ?? [];

/** Nombre de calques de texte qui utilisent le style (calque entier ou portion), ou un style qui en hérite. */
export function usageOf(layers: readonly LayerInfo[], list: readonly CharStyle[], id: number): number {
  const family = dependentsOf(list, id);
  return layers.filter((l) => l.text && (family.has(l.text.style ?? -1) || (l.text.runs ?? []).some((r) => family.has(r.style ?? -1)))).length;
}

export function dependentsOf(list: readonly CharStyle[], id: number): Set<number> {
  const out = new Set([id]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const style of list) {
      if (style.basedOn !== null && style.basedOn !== undefined && out.has(style.basedOn) && !out.has(style.id)) {
        out.add(style.id);
        grew = true;
      }
    }
  }
  return out;
}

export const nextStyleId = (list: readonly CharStyle[]) => list.reduce((max, s) => Math.max(max, s.id), 0) + 1;

/** Enregistre la liste dans le document ; `update` : styles dont les textes en place doivent suivre. */
export function saveStyles(list: CharStyle[], update: number[], label: string): Promise<unknown> {
  return edit({ op: "setTextStyles", list: JSON.parse(JSON.stringify(list)) as CharStyle[], update, label });
}

/** Style tiré du texte affiché (« + Nouveau » : d'après la sélection). */
export function styleFromText(id: number, name: string, text: TextData, shown: RangeStyle): CharStyle {
  return {
    id,
    name,
    fontFamily: shown.fontFamily ?? text.fontFamily,
    weight: shown.weight ?? text.weight,
    italic: shown.italic ?? text.italic,
    size: shown.size ?? text.size,
    lineHeight: text.lineHeight,
    tracking: shown.tracking ?? text.tracking,
    color: shown.color ?? text.color,
    caps: shown.caps ?? text.caps ?? "normal",
  };
}

// ------------------------------------------------------------------ bibliothèque (kit de marque)

export type LibraryStyle = CharStyle & { uid: string; revision: number };
let library = $state<LibraryStyle[] | null>(null);

export async function loadLibrary(): Promise<LibraryStyle[]> {
  if (library) return library;
  try {
    const stored = JSON.parse((await api.loadSettings("text-styles")) ?? "{}") as { styles?: LibraryStyle[] };
    library = Array.isArray(stored.styles) ? stored.styles : [];
  } catch {
    library = [];
  }
  return library;
}

export const libraryNow = (): LibraryStyle[] => library ?? [];

async function writeLibrary(next: LibraryStyle[]): Promise<void> {
  library = next;
  await api.saveSettings("text-styles", JSON.stringify({ styles: next }, null, 2)).catch((error: unknown) => app.fail(error));
}

const uid = () => (crypto.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`);

/**
 * Copie (ou met à jour) un style du projet dans la bibliothèque. « Basé sur » est aplati : l'entrée porte ses
 * réglages effectifs, valables dans n'importe quel projet. Retourne le lien à garder dans le style du projet.
 */
export async function publishToLibrary(list: readonly CharStyle[], style: CharStyle): Promise<{ uid: string; revision: number }> {
  const current = await loadLibrary();
  const resolved = resolveStyle(list, style.id) ?? {};
  const link = { uid: style.library?.uid ?? uid(), revision: Date.now() };
  const entry: LibraryStyle = { id: 0, name: style.name, ...resolved, shortcut: style.shortcut ?? null, uid: link.uid, revision: link.revision };
  await writeLibrary([...current.filter((e) => e.uid !== link.uid), entry]);
  return link;
}

export async function removeFromLibrary(uidToRemove: string): Promise<void> {
  await writeLibrary((await loadLibrary()).filter((e) => e.uid !== uidToRemove));
}

/** Style du projet tiré d'une entrée de la bibliothèque (nouvel identifiant, lien gardé). */
export function fromLibrary(entry: LibraryStyle, id: number): CharStyle {
  const { uid: entryUid, revision, id: _id, ...rest } = entry;
  return { ...rest, id, basedOn: null, library: { uid: entryUid, revision } };
}

/** Styles du projet dont l'entrée de la bibliothèque est plus récente. */
export function updatesAvailable(list: readonly CharStyle[], entries: readonly LibraryStyle[]): { style: CharStyle; entry: LibraryStyle }[] {
  return list.flatMap((style) => {
    const entry = style.library ? entries.find((e) => e.uid === style.library?.uid) : undefined;
    return entry && style.library && entry.revision > style.library.revision ? [{ style, entry }] : [];
  });
}
