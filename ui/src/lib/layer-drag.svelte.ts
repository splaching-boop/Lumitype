// Glisser des calques (et des masques de fusion) dans le panneau Calques, mené par l'application elle-même.
//
// Le glisser natif du HTML (`draggable`, `DataTransfer`) n'est plus utilisé pour les glissers internes : sous
// Linux (Chromium sous X11), il lance la boucle de glisser imbriquée du navigateur, qui tient la fenêtre. Un clic
// suivi d'un tout petit mouvement laissait alors le calque « collé » au pointeur, comme si le bouton restait
// enfoncé, et la fenêtre pouvait geler. Ici, tout passe par les événements de pointeur ordinaires : rien ne bouge
// avant 4 px de course, le pointeur est ensuite capturé, et chaque issue possible (relâchement, annulation par le
// système, perte de la capture, Échap, fenêtre quittée, bouton relâché sans que l'événement arrive) termine le
// geste proprement. Aucun état ne peut rester accroché.
//
// Ce module ne connaît que le geste : le fantôme qui suit le pointeur, le curseur, le défilement automatique de la
// liste et les onglets de documents. Le panneau Calques lui confie, par `DropHandlers`, le calcul de la cible et le
// dépôt lui-même, qui gardent ses règles (zones avant/après/dans un dossier, refus d'un dossier dans lui-même…).
import { pasteFromOtherDocument } from "./actions";
import { t } from "./i18n.svelte";
import { app } from "../state/app.svelte";

/** Ce qui est glissé : des calques (la sélection du document d'origine) ou le masque de fusion d'un calque. */
export interface DragSource {
  kind: "layers" | "mask";
  /** Document d'où vient le glisser : un dépôt dans un autre document copie les calques au lieu de les déplacer. */
  docId: number;
  /** Calques glissés ; pour un masque, le seul calque qui le porte. */
  ids: number[];
  /** Texte du fantôme : nom du calque, « N calques » ou « Masque de fusion ». */
  label: string;
}

/** Ce que le panneau Calques fournit au geste. */
export interface DropHandlers {
  /** Le pointeur est en (x, y) : le panneau calcule la cible et affiche son liseré, ou l'efface hors cible. */
  hover(x: number, y: number, source: DragSource, alt: boolean): void;
  /** Relâché sur la dernière cible calculée par `hover` : le panneau envoie l'opération. */
  drop(source: DragSource, alt: boolean): void;
  /** Fin du geste, quelle qu'elle soit : plus de liseré. */
  clear(): void;
  /** Liste qui défile d'elle-même quand le pointeur approche de ses bords. */
  list(): HTMLElement | null;
}

/** Course minimale avant que le glisser démarre : en deçà, c'est un clic (sélection, renommage…). */
const THRESHOLD = 4;
/** Hauteur de la bande, en haut et en bas de la liste, où le pointeur la fait défiler. */
const SCROLL_EDGE = 28;
/** Pas de défilement maximal, en pixels par image, atteint au bord même de la liste (ou au-delà). */
const SCROLL_MAX = 14;
/** Temps passé sur l'onglet d'un autre document avant qu'il s'ouvre, comme avec l'ancien glisser natif. */
const TAB_DELAY = 450;

/** État visible du glisser en cours : les onglets de documents s'en servent pour marquer celui survolé. */
export const layerDrag = $state<{ source: DragSource | null; tab: number | null }>({ source: null, tab: null });

interface Gesture {
  pointerId: number;
  startX: number;
  startY: number;
  x: number;
  y: number;
  alt: boolean;
  begin: () => DragSource | null;
  handlers: DropHandlers;
  /** `null` tant que le seuil n'est pas franchi : le geste n'est encore qu'un clic possible. */
  source: DragSource | null;
  ghost: HTMLElement | null;
  frame: number;
  tabTimer: ReturnType<typeof setTimeout> | undefined;
}

let gesture: Gesture | null = null;

/** Élément qui reçoit la capture du pointeur : la racine du document, qui ne disparaît jamais en cours de geste. */
const captureRoot = () => document.documentElement;

const altOf = (event: PointerEvent | KeyboardEvent) => event.altKey || event.getModifierState("Alt");

/**
 * Prépare un glisser à partir d'un `pointerdown` du bouton gauche. Rien ne se passe tant que le pointeur n'a pas
 * parcouru `THRESHOLD` pixels ; alors `begin` dit ce qui est glissé (ou `null` pour renoncer) et le geste démarre.
 * Un relâchement avant le seuil laisse le clic suivre son cours normal.
 */
export function armLayerDrag(event: PointerEvent, begin: () => DragSource | null, handlers: DropHandlers): void {
  if (event.button !== 0 || !event.isPrimary) return;
  if (gesture) finish();
  gesture = {
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    x: event.clientX,
    y: event.clientY,
    alt: altOf(event),
    begin,
    handlers,
    source: null,
    ghost: null,
    frame: 0,
    tabTimer: undefined,
  };
  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", onUp, true);
  window.addEventListener("pointercancel", onCancel, true);
  window.addEventListener("keydown", onKey, true);
  window.addEventListener("keyup", onKey, true);
  window.addEventListener("blur", onBlur);
  document.addEventListener("visibilitychange", onCancel);
}

/** Interrompt le geste en cours sans rien déposer (panneau détruit, document fermé…). */
export function cancelLayerDrag(): void {
  if (gesture) finish();
}

function onMove(event: PointerEvent): void {
  const g = gesture;
  if (!g || event.pointerId !== g.pointerId) return;
  // Bouton relâché sans que `pointerup` nous soit parvenu (relâché hors de la fenêtre avant la capture, événement
  // perdu par le gestionnaire de fenêtres) : le geste s'arrête là, sans dépôt.
  if ((event.buttons & 1) === 0) {
    finish();
    return;
  }
  g.x = event.clientX;
  g.y = event.clientY;
  g.alt = altOf(event);
  if (!g.source) {
    if (Math.hypot(g.x - g.startX, g.y - g.startY) < THRESHOLD) return;
    start(g);
    if (!g.source) return;
  }
  event.preventDefault();
  update(g);
}

/** Seuil franchi : le geste devient un glisser. */
function start(g: Gesture): void {
  const source = g.begin();
  if (!source) {
    finish();
    return;
  }
  g.source = source;
  layerDrag.source = source;
  // Un texte que le geste aurait commencé à sélectionner est abandonné.
  window.getSelection()?.removeAllRanges();
  try {
    captureRoot().setPointerCapture(g.pointerId);
  } catch {
    // Pointeur déjà relâché entre-temps : `pointerup` ou `pointercancel` terminera le geste.
  }
  captureRoot().addEventListener("lostpointercapture", onLostCapture);
  captureRoot().classList.add("layer-dragging");
  ensureStyle();
  const ghost = document.createElement("div");
  ghost.className = "layer-drag-ghost";
  ghost.setAttribute("aria-hidden", "true");
  document.body.append(ghost);
  g.ghost = ghost;
  g.frame = requestAnimationFrame(() => autoScroll(g));
}

/** Position courante : fantôme, curseur, onglet survolé et cible dans le panneau. */
function update(g: Gesture): void {
  const source = g.source;
  if (!source) return;
  const copy = g.alt;
  captureRoot().classList.toggle("layer-drag-copy", copy);
  if (g.ghost) {
    const mode = copy ? (source.kind === "mask" ? t("Copier") : t("Dupliquer")) : "";
    g.ghost.textContent = mode ? `${mode} · ${source.label}` : source.label;
    g.ghost.style.transform = `translate(${Math.round(g.x + 14)}px, ${Math.round(g.y + 12)}px)`;
  }
  const element = document.elementFromPoint(g.x, g.y);
  const tab = source.kind === "layers" ? element?.closest<HTMLElement>("[data-doc]") : null;
  const tabId = tab ? Number(tab.dataset.doc) : null;
  if (tabId !== null && Number.isFinite(tabId)) {
    g.handlers.clear();
    hoverTab(g, tabId);
    return;
  }
  hoverTab(g, null);
  g.handlers.hover(g.x, g.y, source, g.alt);
}

/** S'attarder sur l'onglet d'un document l'ouvre : on dépose ensuite dans son panneau Calques, à l'endroit voulu. */
function hoverTab(g: Gesture, tabId: number | null): void {
  if (layerDrag.tab === tabId) return;
  layerDrag.tab = tabId;
  clearTimeout(g.tabTimer);
  g.tabTimer = tabId === null ? undefined : setTimeout(() => gesture === g && layerDrag.tab === tabId && (app.activeId = tabId), TAB_DELAY);
}

/** Défilement de la liste quand le pointeur s'approche de son haut ou de son bas, plus vite à mesure qu'il s'en approche. */
function autoScroll(g: Gesture): void {
  if (gesture !== g || !g.source) return;
  const list = g.handlers.list();
  if (list && layerDrag.tab === null) {
    const box = list.getBoundingClientRect();
    const across = g.x >= box.left && g.x <= box.right;
    let step = 0;
    // La bande s'étend un peu au-delà de la liste : on fait défiler sans devoir viser au pixel près.
    if (across && g.y < box.top + SCROLL_EDGE && g.y > box.top - SCROLL_EDGE * 2) step = -Math.min(1, (box.top + SCROLL_EDGE - g.y) / SCROLL_EDGE);
    else if (across && g.y > box.bottom - SCROLL_EDGE && g.y < box.bottom + SCROLL_EDGE * 2) step = Math.min(1, (g.y - (box.bottom - SCROLL_EDGE)) / SCROLL_EDGE);
    if (step !== 0) {
      const before = list.scrollTop;
      list.scrollTop += Math.round(step * SCROLL_MAX) || Math.sign(step);
      // Les lignes ont glissé sous le pointeur immobile : la cible est recalculée.
      if (list.scrollTop !== before) update(g);
    }
  }
  g.frame = requestAnimationFrame(() => autoScroll(g));
}

function onUp(event: PointerEvent): void {
  const g = gesture;
  if (!g || event.pointerId !== g.pointerId) return;
  if (!g.source) {
    // Relâché avant le seuil : c'est un clic, qui suit son cours.
    finish();
    return;
  }
  event.preventDefault();
  g.x = event.clientX;
  g.y = event.clientY;
  g.alt = altOf(event);
  update(g);
  const source = g.source;
  const tab = layerDrag.tab;
  const alt = g.alt;
  const handlers = g.handlers;
  finish(false);
  suppressClick();
  if (tab !== null) {
    // Lâché sur l'onglet lui-même : les calques sont copiés en haut de la pile de ce document.
    if (source.kind === "layers" && source.docId !== tab) void pasteFromOtherDocument({ docId: source.docId, ids: source.ids }, tab).then(() => (app.activeId = tab));
  } else {
    handlers.drop(source, alt);
  }
  handlers.clear();
}

function onCancel(event: Event): void {
  const g = gesture;
  if (!g) return;
  if (event instanceof PointerEvent && event.pointerId !== g.pointerId) return;
  if (event.type === "visibilitychange" && document.visibilityState === "visible") return;
  finish();
}

function onLostCapture(event: PointerEvent): void {
  if (gesture && event.pointerId === gesture.pointerId) finish();
}

function onKey(event: KeyboardEvent): void {
  const g = gesture;
  if (!g) return;
  if (event.key === "Escape" && event.type === "keydown") {
    // Échap annule : rien n'est déposé, et la touche ne sert à rien d'autre (désélection, fermeture d'un menu…).
    if (g.source) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
    finish();
    return;
  }
  if (event.key !== "Alt" && event.key !== "AltGraph") return;
  // Alt pressé ou relâché sans bouger : le fantôme et le curseur annoncent tout de suite la copie.
  g.alt = event.type === "keydown";
  if (g.source) {
    event.preventDefault();
    update(g);
  }
}

/**
 * Fenêtre quittée pour de bon (autre application, bureau virtuel…). Une simple pression d'Alt peut, sous certains
 * gestionnaires de fenêtres, envoyer un `blur` aussitôt suivi d'un retour : seul compte un focus vraiment perdu.
 */
function onBlur(): void {
  const g = gesture;
  setTimeout(() => {
    if (gesture === g && !document.hasFocus()) finish();
  }, 0);
}

/** Après un vrai glisser, le `click` qui suit le relâchement ne doit ni sélectionner ni ouvrir quoi que ce soit. */
function suppressClick(): void {
  const stop = (event: MouseEvent) => {
    event.stopPropagation();
    event.preventDefault();
  };
  window.addEventListener("click", stop, { capture: true, once: true });
  setTimeout(() => window.removeEventListener("click", stop, true), 0);
}

/** Fin du geste, quelle qu'en soit la cause : tout ce qu'il avait posé est retiré. */
function finish(clear = true): void {
  const g = gesture;
  if (!g) return;
  gesture = null;
  window.removeEventListener("pointermove", onMove, true);
  window.removeEventListener("pointerup", onUp, true);
  window.removeEventListener("pointercancel", onCancel, true);
  window.removeEventListener("keydown", onKey, true);
  window.removeEventListener("keyup", onKey, true);
  window.removeEventListener("blur", onBlur);
  document.removeEventListener("visibilitychange", onCancel);
  const root = captureRoot();
  root.removeEventListener("lostpointercapture", onLostCapture);
  try {
    if (root.hasPointerCapture(g.pointerId)) root.releasePointerCapture(g.pointerId);
  } catch {
    // Pointeur déjà disparu : rien à relâcher.
  }
  root.classList.remove("layer-dragging", "layer-drag-copy");
  cancelAnimationFrame(g.frame);
  clearTimeout(g.tabTimer);
  g.ghost?.remove();
  layerDrag.source = null;
  layerDrag.tab = null;
  if (clear && g.source) g.handlers.clear();
}

/**
 * Styles du fantôme et du curseur, posés une fois dans la page : le fantôme vit directement dans `body`, hors de
 * tout panneau (une transformation ou un `overflow` d'un parent ne peut ni le décaler ni le rogner).
 */
function ensureStyle(): void {
  if (document.getElementById("layer-drag-style")) return;
  const style = document.createElement("style");
  style.id = "layer-drag-style";
  style.textContent = `
    html.layer-dragging, html.layer-dragging * { cursor: grabbing !important; user-select: none !important; }
    html.layer-dragging.layer-drag-copy, html.layer-dragging.layer-drag-copy * { cursor: copy !important; }
    .layer-drag-ghost {
      position: fixed; left: 0; top: 0; z-index: 10000; pointer-events: none;
      max-width: 240px; padding: 4px 9px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
      background: var(--bg-menu); border: 1px solid var(--border-strong); border-radius: 0;
      box-shadow: var(--shadow-menu); color: var(--text); font-size: 11.5px;
    }`;
  document.head.append(style);
}
