// Résolution des raccourcis clavier, indépendante de la disposition.
import { t } from "./i18n.svelte";
//
// Un raccourci se déclare de deux façons :
// - `key`  : le CARACTÈRE produit (event.key). Pour les lettres et + / - : Ctrl+Z
//            reste sur la touche gravée « Z », en AZERTY comme en QWERTY.
// - `code` : la POSITION physique (event.code). Pour les chiffres et [ ] : en
//            AZERTY, « 1 » demande Maj et « [ » demande AltGr ; la position, elle,
//            est accessible directement (touches & é " … et ^ $).
//
// L'étape 3 ajoutera les contextes, l'écran de configuration et la détection de
// conflits au-dessus de ce même modèle.

export const CTRL = 1;
export const SHIFT = 2;
export const ALT = 4;

export interface Chord {
  mods: number;
  match: "key" | "code";
  value: string;
  /** Maj ignorée : « + » s'obtient avec ou sans Maj selon la disposition. */
  shiftOptional?: boolean;
  /**
   * Raccourci par position de touche accepté aussi avec AltGr (Windows rapporte AltGr comme Ctrl+Alt). Sans effet
   * sur un raccourci par lettre : il est toujours accepté avec AltGr, voir `resolve`.
   */
  allowAltGr?: boolean;
}

export interface Binding<Action extends string> {
  action: Action;
  chord: Chord;
}

export const key = (value: string, mods = 0, shiftOptional = false): Chord => ({
  mods,
  match: "key",
  value: value.toLowerCase(),
  shiftOptional,
});

export const code = (value: string, mods = 0): Chord => ({ mods, match: "code", value });

function modifiersOf(event: KeyboardEvent): number {
  return (event.ctrlKey ? CTRL : 0) | (event.shiftKey ? SHIFT : 0) | (event.altKey ? ALT : 0);
}

/** La frappe va dans un champ de texte (champ, zone de texte, élément éditable) : les raccourcis se taisent. */
export function isTyping(event: KeyboardEvent): boolean {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLInputElement) {
    // Curseurs et cases à cocher ne reçoivent pas de texte : les raccourcis restent actifs.
    return !["range", "checkbox", "radio", "button"].includes(target.type);
  }
  return target.isContentEditable || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement;
}

export function resolve<Action extends string>(
  event: KeyboardEvent,
  bindings: readonly Binding<Action>[],
): Action | null {
  if (isTyping(event)) return null;
  // Sous Windows, Ctrl+Alt et AltGr sont une seule et même chose pour la webview : tout Ctrl+Alt+touche arrive avec
  // AltGr levé. Deux conséquences, traitées ici :
  //   - un raccourci par POSITION de touche (chiffres, crochets) ne doit pas se déclencher quand on tape « @ » ou « [ »
  //     en AZERTY : il n'est accepté avec AltGr que s'il le dit (`allowAltGr`) ;
  //   - un raccourci par LETTRE (Ctrl+Alt+M, Ctrl+Alt+Z) est sans danger : si AltGr produisait un caractère (« € »),
  //     c'est lui que la webview rapporte et la lettre ne correspondrait pas. Mais quand AltGr ne produit rien, la
  //     webview peut ne pas rapporter la lettre non plus : la touche est alors reconnue par sa position, d'après la
  //     disposition du clavier (ou, sans elle, d'après son nom : « KeyM » → « m »).
  const altGr = event.getModifierState("AltGraph");
  // Avec AltGr levé, la webview rapporte Ctrl et Alt RELÂCHÉS (`ctrlKey`, `altKey` à faux) : sans cette ligne,
  // Ctrl+Alt+M était lu comme un simple M (outil Rectangle de sélection). Même règle que `chordFromEvent`.
  const mods = modifiersOf(event) | (altGr ? CTRL | ALT : 0);
  const pressedKey = event.key.toLowerCase();
  const positional = altGr ? (layout?.get(event.code) ?? (/^Key[A-Z]$/.test(event.code) ? event.code.slice(3) : "")).toLowerCase() : "";
  const hits = (chord: Chord) => (chord.match === "key" ? pressedKey === chord.value || (positional !== "" && positional === chord.value) : event.code === chord.value);
  // Un raccourci aux modificateurs exacts l'emporte sur un raccourci « Maj facultatif » (Ctrl+Maj+; passe avant Ctrl+;).
  let loose: Action | null = null;
  for (const { action, chord } of bindings) {
    if (altGr && chord.match === "code" && !chord.allowAltGr) continue;
    if (!hits(chord)) continue;
    if (mods === chord.mods) return action;
    if (loose === null && chord.shiftOptional && (mods & ~SHIFT) === (chord.mods & ~SHIFT)) loose = action;
  }
  return loose;
}

/**
 * Deux raccourcis qui se disputeraient la même frappe. Mêmes modificateurs, ou tous deux « Maj facultatif » à Maj
 * près ; un raccourci exact ne gêne pas un raccourci « Maj facultatif » (l'exact l'emporte, voir `resolve`).
 */
export function chordsConflict(a: Chord, b: Chord): boolean {
  if (a.match !== b.match || a.value !== b.value) return false;
  if (a.mods === b.mods) return true;
  return Boolean(a.shiftOptional && b.shiftOptional) && (a.mods & ~SHIFT) === (b.mods & ~SHIFT);
}

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "AltGraph", "Meta"]);
/** Touches identifiées par leur position : leur caractère dépend trop de la disposition. */
const POSITIONAL = /^(Digit\d|Numpad.+|Bracket(Left|Right)|Backspace|Delete|Enter|Tab|Space|Arrow.+|F\d{1,2}|Home|End|Page(Up|Down)|Insert)$/;

/**
 * Construit un raccourci depuis une frappe, pour l'écran de configuration.
 * `null` tant que seule une touche de modification est enfoncée.
 */
export function chordFromEvent(event: KeyboardEvent): Chord | null {
  if (MODIFIER_KEYS.has(event.key)) return null;
  const altGr = event.getModifierState("AltGraph");
  const mods = (event.ctrlKey || altGr ? CTRL : 0) | (event.shiftKey ? SHIFT : 0) | (event.altKey || altGr ? ALT : 0);
  if (POSITIONAL.test(event.code)) return { mods, match: "code", value: event.code, ...(altGr && { allowAltGr: true }) };
  // Avec AltGr, `key` est le caractère composé (« @ », « [ »…) : on retient la position de la touche.
  if (altGr || event.key.length !== 1) return { mods, match: "code", value: event.code, ...(altGr && { allowAltGr: true }) };
  const symbol = !/[a-z0-9]/i.test(event.key);
  // Un symbole s'obtient avec ou sans Maj selon la disposition : Maj ne compte pas pour lui.
  return { mods: symbol ? mods & ~SHIFT : mods, match: "key", value: event.key.toLowerCase(), ...(symbol && { shiftOptional: true }) };
}

/** Caractère gravé sur chaque touche, quand la webview sait le dire (WebView2 oui, WebKitGTK non). */
let layout: ReadonlyMap<string, string> | null = null;
type KeyboardApi = { getLayoutMap?: () => Promise<ReadonlyMap<string, string>> };
void (navigator as Navigator & { keyboard?: KeyboardApi }).keyboard?.getLayoutMap?.().then(
  (map) => (layout = map),
  () => undefined,
);

const CODE_LABELS: Record<string, string> = {
  Backspace: "⌫",
  Delete: "Suppr",
  // Le pavé numérique est nommé : « Ctrl++ » et « Ctrl+Pavé + » sont deux raccourcis distincts.
  NumpadAdd: "Pavé +",
  NumpadSubtract: "Pavé -",
  Space: "Espace",
  Enter: "Entrée",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
};

/** Repli quand la disposition est inconnue : libellés de la disposition américaine. */
const US_LABELS: Record<string, string> = { BracketLeft: "[", BracketRight: "]", Minus: "-", Equal: "=" };

/** Libellé affiché dans les menus, avec les noms de touches français. */
export function chordLabel(chord: Chord): string {
  const parts: string[] = [];
  // Les noms des touches passent par le dictionnaire : « Maj » devient « Shift », « Umschalt », « Mayús ».
  if (chord.mods & CTRL) parts.push(t("Ctrl"));
  if (chord.mods & ALT) parts.push(t("Alt"));
  if (chord.mods & SHIFT) parts.push(t("Maj"));
  const raw =
    chord.match === "key"
      ? chord.value
      : (CODE_LABELS[chord.value] ? t(CODE_LABELS[chord.value] ?? "") : (layout?.get(chord.value) ?? US_LABELS[chord.value] ?? chord.value.replace(/^Numpad/, t("Pavé ")).replace(/^(Digit|Key)/, "")));
  parts.push(raw.length === 1 ? raw.toUpperCase() : raw);
  return parts.join("+");
}

/**
 * Vrai si une boîte ou un panneau a la main sur le clavier : une fenêtre modale (`<dialog>` ouvert) ou
 * un panneau flottant non modal (`role="dialog"` hors d'un `<dialog>` : Effets de calque, éditeur de
 * dégradé, sélecteur de couleur, menu du pinceau). Les gestionnaires globaux de l'espace de travail
 * (raccourcis, flèches, Entrée et Suppr des surcouches) s'effacent alors : Suppr ne supprime pas un
 * calque derrière le panneau, Entrée n'y valide pas un recadrage.
 */
export function panelOwnsKeyboard(): boolean {
  if (document.querySelector("dialog[open]")) return true;
  return floatingPanelOpen();
}

/** Vrai si un panneau flottant non modal (`role="dialog"` hors d'un vrai `<dialog>`) est affiché. */
export function floatingPanelOpen(): boolean {
  return Array.from(document.querySelectorAll('[role="dialog"]:not(dialog)')).some((panel) => !panel.closest("dialog") && panel.getClientRects().length > 0);
}
