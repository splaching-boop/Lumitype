// Recherche dans le panneau Calques (components/LayersPanel.svelte) : par nom, insensible à la casse et aux accents,
// et « intelligente » — des mots-clés (texte, pixels, fx, masque, caché, mode:produit…) deviennent des pastilles,
// combinées en ET entre elles et avec le texte libre. Toutes les informations viennent de `LayerInfo`. Sans
// dépendance à l'interface, pour être vérifié sans navigateur (ui/tests/layer-search.test.mjs).
import { BLEND_MENU } from "./blend";
import type { BlendMode, LayerInfo } from "./ipc";

/** Événement de la fenêtre qui donne le focus au champ de recherche (action « layer.search », Ctrl+Maj+F). */
export const LAYER_SEARCH_EVENT = "lumitype:layer-search";

/** Ce que la recherche lit d'un calque. */
export type SearchLayer = Pick<
  LayerInfo,
  "id" | "parent" | "name" | "isGroup" | "expanded" | "visible" | "blend" | "locks" | "clipped" | "hasMask" | "text" | "shape" | "smart" | "adjustment" | "effects"
> & { fillLayer?: LayerInfo["fillLayer"] };

export type FilterKind = "text" | "pixels" | "adjustment" | "fill" | "shape" | "smart" | "group" | "fx" | "mask" | "hidden" | "locked" | "clipped";

/** Pastille : un type ou un état, ou un mode de fusion. */
export type Chip = { kind: FilterKind } | { kind: "mode"; blend: BlendMode };

export interface SearchQuery {
  chips: Chip[];
  /** Texte libre : mots cherchés dans le nom (tous doivent y figurer) ; un mot `mode:<nom>` vaut une pastille. */
  text: string;
}

/** Minuscules, sans accents ni ligatures : « Été » et « ete », « Œil » et « oeil » se valent. */
export function normalize(value: string): string {
  return value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/œ/gi, "oe")
    .replace(/æ/gi, "ae")
    .toLowerCase();
}

/** Calque de pixels : ni groupe, ni texte, ni forme, ni réglage, ni remplissage, ni objet dynamique. */
const isPixels = (l: SearchLayer) => !l.isGroup && !l.text && !l.shape && !l.adjustment && !l.smart && !l.fillLayer;

interface FilterSpec {
  kind: FilterKind;
  /** Libellé de la pastille. */
  chip: string;
  /** Libellé de la suggestion. */
  label: string;
  /** Mot-clé montré à droite de la suggestion (le premier de `keywords`, avec ses accents). */
  hint: string;
  /** Mots qui la désignent, en français et en anglais (normalisés à la comparaison). */
  keywords: string[];
  /** Pictogramme de la suggestion. */
  icon: string;
  test: (layer: SearchLayer) => boolean;
}

/** Mots-clés compris, dans l'ordre de la liste des suggestions. */
export const FILTERS: readonly FilterSpec[] = [
  { kind: "text", chip: "Texte", label: "Type : Texte", hint: "texte", keywords: ["texte", "text"], icon: "T", test: (l) => !!l.text },
  { kind: "pixels", chip: "Pixels", label: "Type : Pixels", hint: "pixels", keywords: ["pixels", "pixel"], icon: "▣", test: isPixels },
  { kind: "adjustment", chip: "Réglage", label: "Type : Réglage", hint: "réglage", keywords: ["réglage", "reglages", "adjustment"], icon: "◐", test: (l) => !!l.adjustment },
  { kind: "fill", chip: "Remplissage", label: "Type : Remplissage", hint: "remplissage", keywords: ["remplissage", "fill"], icon: "▤", test: (l) => !!l.fillLayer },
  { kind: "shape", chip: "Forme", label: "Type : Forme", hint: "forme", keywords: ["forme", "formes", "shape"], icon: "◆", test: (l) => !!l.shape },
  { kind: "smart", chip: "Objet dynamique", label: "Type : Objet dynamique", hint: "objet dynamique", keywords: ["objet dynamique", "objet", "dynamique", "smart"], icon: "⧉", test: (l) => !!l.smart },
  { kind: "group", chip: "Groupe", label: "Type : Groupe", hint: "groupe", keywords: ["groupe", "dossier", "group"], icon: "▭", test: (l) => l.isGroup },
  { kind: "fx", chip: "Avec effets", label: "Avec effets", hint: "fx", keywords: ["fx", "effets", "effet", "effects"], icon: "fx", test: (l) => l.effects.length > 0 },
  { kind: "mask", chip: "Avec masque", label: "Avec masque", hint: "masque", keywords: ["masque", "mask"], icon: "◧", test: (l) => l.hasMask },
  { kind: "hidden", chip: "Cachés", label: "Masqués (œil fermé)", hint: "caché", keywords: ["caché", "cachés", "masqués", "invisible", "hidden"], icon: "◌", test: (l) => !l.visible },
  {
    kind: "locked",
    chip: "Verrouillés",
    label: "Verrouillés",
    hint: "verrouillé",
    keywords: ["verrouillé", "verrouillés", "verrou", "locked"],
    icon: "⊠",
    test: (l) => l.locks.all || l.locks.pixels || l.locks.position || l.locks.transparency,
  },
  { kind: "clipped", chip: "Écrêtés", label: "Écrêtés", hint: "écrêté", keywords: ["écrêté", "écrêtés", "écrêtage", "clipped"], icon: "↧", test: (l) => l.clipped },
];

const specOf = (kind: FilterKind) => FILTERS.find((f) => f.kind === kind)!;

/** Modes de fusion : nom français du menu (et « Transfert » pour les groupes), identifiant anglais. */
const MODES: readonly [BlendMode, string][] = [["pass_through", "Transfert"], ...BLEND_MENU.flatMap((entry) => (entry ? [entry] : []))];

/** Nom français d'un mode de fusion. */
export const modeLabel = (blend: BlendMode): string => MODES.find(([mode]) => mode === blend)?.[1] ?? blend;

/**
 * Clé de comparaison d'un nom de mode, tapé d'un seul mot après « mode: » : sans accents, parenthèses, espaces, « _ »
 * ni « - » intérieurs ; le signe final des densités reste (« Densité couleur + » -> `densitecouleur+`).
 */
export function modeKey(name: string): string {
  const plain = normalize(name).replace(/\(.*?\)/g, "").trim();
  const sign = /[+-]$/.exec(plain)?.[0] ?? "";
  return plain.slice(0, plain.length - sign.length).replace(/[\s_-]+/g, "") + sign;
}

/** Clés d'un mode : son nom français et son identifiant anglais (`multiply`, `color_burn`). */
const modeKeys = (blend: BlendMode, french: string): string[] => [modeKey(french), modeKey(blend)];

/**
 * Mode de fusion désigné par `name` (après « mode: ») : le nom français ou anglais exact, sinon le seul mode dont un nom
 * commence ainsi. `produit`, `multiply`, `densite-couleur+`, `color_burn`, `colorburn`, `prod` sont compris.
 */
export function resolveMode(name: string): BlendMode | null {
  const wanted = modeKey(name);
  if (!wanted) return null;
  const exact = MODES.find(([blend, french]) => modeKeys(blend, french).includes(wanted));
  if (exact) return exact[0];
  const starting = MODES.filter(([blend, french]) => modeKeys(blend, french).some((k) => k.startsWith(wanted)));
  return starting.length === 1 ? starting[0]![0] : null;
}

/** Libellé d'une pastille. */
export const chipLabel = (chip: Chip): string => (chip.kind === "mode" ? `Mode : ${modeLabel(chip.blend)}` : specOf(chip.kind).chip);

export const sameChip = (a: Chip, b: Chip): boolean => a.kind === b.kind && (a.kind !== "mode" || (b.kind === "mode" && a.blend === b.blend));

/** Texte libre lu : mots du nom (normalisés) et modes `mode:<nom>`. Un `mode:` inconnu ne trouve rien. */
export function parseText(text: string): { words: string[]; modes: (BlendMode | null)[] } {
  const words: string[] = [];
  const modes: (BlendMode | null)[] = [];
  for (const word of text.trim().split(/\s+/)) {
    if (!word) continue;
    const mode = /^mode:(.*)$/i.exec(word);
    if (mode) {
      // « mode: » seul (en cours de frappe) ne filtre pas encore.
      if (mode[1]) modes.push(resolveMode(mode[1]));
    } else words.push(normalize(word));
  }
  return { words, modes };
}

/** La recherche filtre-t-elle quelque chose ? */
export const isActive = (query: SearchQuery): boolean => query.chips.length > 0 || query.text.trim().length > 0;

/** Le calque répond-il à toutes les pastilles, à tous les mots (dans son nom) et à tous les `mode:` ? */
export function matches(layer: SearchLayer, query: SearchQuery): boolean {
  const { words, modes } = parseText(query.text);
  const name = normalize(layer.name);
  return (
    query.chips.every((chip) => (chip.kind === "mode" ? layer.blend === chip.blend : specOf(chip.kind).test(layer))) &&
    modes.every((mode) => mode !== null && layer.blend === mode) &&
    words.every((word) => name.includes(word))
  );
}

export interface SearchResult {
  /** Calques trouvés, dans l'ordre du panneau. */
  ids: number[];
  /** Nombre de calques du document (le « N » de « n sur N »). */
  total: number;
  /** Groupes fermés à ouvrir le temps de la recherche : ceux qui contiennent un résultat. */
  open: Set<number>;
}

/** Recherche dans les calques (dans l'ordre du panneau) ; rien n'est trouvé ni ouvert sans recherche active. */
export function searchLayers(layers: readonly SearchLayer[], query: SearchQuery): SearchResult {
  const open = new Set<number>();
  if (!isActive(query)) return { ids: [], total: layers.length, open };
  const byId = new Map(layers.map((l) => [l.id, l]));
  const ids: number[] = [];
  for (const layer of layers) {
    if (!matches(layer, query)) continue;
    ids.push(layer.id);
    // Les ancêtres fermés s'ouvrent ; un id déjà vu arrête la remontée (parenté incohérente).
    const seen = new Set<number>([layer.id]);
    let parent = layer.parent === null ? undefined : byId.get(layer.parent);
    while (parent && !seen.has(parent.id)) {
      seen.add(parent.id);
      if (parent.isGroup && !parent.expanded) open.add(parent.id);
      parent = parent.parent === null ? undefined : byId.get(parent.parent);
    }
  }
  return { ids, total: layers.length, open };
}

/**
 * Groupe affiché ouvert dans le panneau : ouvert dans le document, ou ouvert le temps de la recherche (`open`) sans
 * avoir été refermé depuis par son chevron (`closed`, propre au panneau).
 */
export function groupShownOpen(id: number, expanded: boolean, open: ReadonlySet<number>, closed: ReadonlySet<number>): boolean {
  return expanded || (open.has(id) && !closed.has(id));
}

/**
 * Clic sur le chevron d'un groupe : il bascule selon l'état AFFICHÉ, comme le dit son libellé. Fermer un groupe ouvert
 * par la recherche retire seulement cette ouverture temporaire (`closeTemporary`) ; fermer un groupe ouvert dans le
 * document envoie `expanded: false` ; ouvrir envoie `expanded: true` (le groupe reste ouvert après la recherche) et
 * oublie une fermeture temporaire. `send` : valeur à envoyer au moteur, `null` pour ne rien envoyer.
 */
export function toggleGroup(id: number, expanded: boolean, open: ReadonlySet<number>, closed: ReadonlySet<number>): { send: boolean | null; closeTemporary: boolean; forgetClosed: boolean } {
  if (!groupShownOpen(id, expanded, open, closed)) return { send: true, closeTemporary: false, forgetClosed: true };
  return { send: expanded ? false : null, closeTemporary: open.has(id), forgetClosed: false };
}

/** Nom découpé pour le surlignage : les mots cherchés sont `mark` (comparaison sans accents, texte d'origine gardé). */
export function highlight(name: string, query: SearchQuery): { text: string; mark: boolean }[] {
  const { words } = parseText(query.text);
  if (words.length === 0) return [{ text: name, mark: false }];
  // Correspondance caractère par caractère entre le nom et sa forme normalisée (un « É » peut donner un « e »).
  const chars = Array.from(name);
  const map: number[] = [];
  let flat = "";
  chars.forEach((c, i) => {
    const n = normalize(c);
    flat += n;
    for (let k = 0; k < n.length; k++) map.push(i);
  });
  const marked = new Array<boolean>(chars.length).fill(false);
  for (const word of words) {
    for (let at = flat.indexOf(word); at >= 0; at = flat.indexOf(word, at + 1)) {
      for (let k = at; k < at + word.length; k++) marked[map[k]!] = true;
    }
  }
  const parts: { text: string; mark: boolean }[] = [];
  chars.forEach((c, i) => {
    const last = parts[parts.length - 1];
    if (last && last.mark === marked[i]) last.text += c;
    else parts.push({ text: c, mark: marked[i]! });
  });
  return parts;
}

export interface Suggestion {
  /** Libellé affiché (« Type : Texte », « Mode : Produit »). */
  label: string;
  /** Mot-clé à droite. */
  hint: string;
  icon: string;
  /** Pastille ajoutée ; `null` : le texte devient `fill` (« mode: », pour choisir ensuite le mode). */
  chip: Chip | null;
  fill?: string;
}

/** Mot en cours de frappe : le dernier du texte (vide après une espace). */
export const currentWord = (text: string): string => (/(\S*)$/.exec(text)?.[1] ?? "");

/**
 * Suggestions pour le mot en cours : les mots-clés qui commencent ainsi (tous si le mot est vide), « Mode : … » qui
 * prépare `mode:` ; après `mode:`, les modes de fusion qui conviennent. Les pastilles déjà posées n'y sont plus.
 */
export function suggestions(word: string, chips: readonly Chip[]): Suggestion[] {
  const wanted = normalize(word);
  const has = (chip: Chip) => chips.some((c) => sameChip(c, chip));
  const mode = /^mode:(.*)$/.exec(wanted);
  if (mode) {
    const rest = modeKey(mode[1]!);
    return MODES.filter(([blend, french]) => !has({ kind: "mode", blend }) && (!rest || modeKeys(blend, french).some((k) => k.startsWith(rest)))).map(([blend, french]) => ({
      label: `Mode : ${french}`,
      hint: `mode:${modeKey(french)}`,
      icon: "⊗",
      chip: { kind: "mode", blend },
    }));
  }
  const list: Suggestion[] = FILTERS.filter((f) => !has({ kind: f.kind }) && (!wanted || f.keywords.some((k) => normalize(k).startsWith(wanted)))).map((f) => ({
    label: f.label,
    hint: f.hint,
    icon: f.icon,
    chip: { kind: f.kind },
  }));
  if (!wanted || "mode".startsWith(wanted)) list.push({ label: "Mode : Produit…", hint: "mode:", icon: "⊗", chip: null, fill: "mode:" });
  return list;
}

/** Le texte une fois la suggestion choisie : le mot en cours est remplacé (par `fill`, ou retiré pour une pastille). */
export function acceptSuggestion(query: SearchQuery, suggestion: Suggestion): SearchQuery {
  const before = query.text.slice(0, query.text.length - currentWord(query.text).length);
  if (!suggestion.chip) return { chips: query.chips, text: before + (suggestion.fill ?? "") };
  const chips = query.chips.some((c) => sameChip(c, suggestion.chip!)) ? query.chips : [...query.chips, suggestion.chip];
  return { chips, text: before.trimEnd() ? `${before.trimEnd()} ` : "" };
}

/** Résultat suivant (`step` = 1) ou précédent (-1) après `current`, en bouclant ; le premier (ou le dernier) sinon. */
export function stepResult(ids: readonly number[], current: number | null, step: 1 | -1): number | null {
  if (ids.length === 0) return null;
  const at = current === null ? -1 : ids.indexOf(current);
  if (at < 0) return step === 1 ? ids[0]! : ids[ids.length - 1]!;
  return ids[(at + step + ids.length) % ids.length]!;
}
