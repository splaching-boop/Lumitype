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
   * Accepté aussi avec AltGr. Windows rapporte AltGr comme Ctrl+Alt : à réserver
   * aux raccourcis Ctrl+Alt dont la touche ne produit aucun caractère avec AltGr.
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

function isTyping(event: KeyboardEvent): boolean {
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
  // Sous Windows, AltGr est rapporté comme Ctrl+Alt : sans ce garde-fou, taper
  // « @ » ou « [ » en AZERTY déclencherait des raccourcis Ctrl+Alt.
  if (isTyping(event)) return null;
  const altGr = event.getModifierState("AltGraph");

  const mods = modifiersOf(event);
  const pressedKey = event.key.toLowerCase();
  for (const { action, chord } of bindings) {
    if (altGr && !chord.allowAltGr) continue;
    const ignore = chord.shiftOptional ? SHIFT : 0;
    if ((mods & ~ignore) !== (chord.mods & ~ignore)) continue;
    if (chord.match === "key" ? pressedKey === chord.value : event.code === chord.value) return action;
  }
  return null;
}

/** Deux raccourcis qui se déclencheraient sur la même frappe. */
export function chordsConflict(a: Chord, b: Chord): boolean {
  if (a.match !== b.match || a.value !== b.value) return false;
  const ignore = a.shiftOptional || b.shiftOptional ? SHIFT : 0;
  return (a.mods & ~ignore) === (b.mods & ~ignore);
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
  Backspace: "Retour arr.",
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
