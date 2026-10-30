// Impression : unités de mesure, réglages d'affichage (Affichage › Impression), noirs à l'impression, et analyse
// d'encrage du document actif (alerte d'encrage, vérification avant export).
//
// Le moteur garde les tailles en pixels ; les unités ne sont qu'une façon de les saisir et de les lire, à la
// résolution du document (1 pouce = 25,4 mm = 72 pt).
import { api, type CmykSpace, type DocumentInfo, type InkReport, type PrintSetup } from "./ipc";

export type Unit = "px" | "mm" | "cm" | "in" | "pt";
export const UNITS: { id: Unit; label: string }[] = [
  { id: "px", label: "Pixels (px)" },
  { id: "mm", label: "Millimètres (mm)" },
  { id: "cm", label: "Centimètres (cm)" },
  { id: "in", label: "Pouces (in)" },
  { id: "pt", label: "Points (pt)" },
];

/** Taille d'une unité en pouces (le pixel dépend de la résolution). */
const INCHES: Record<Exclude<Unit, "px">, number> = { mm: 1 / 25.4, cm: 1 / 2.54, in: 1, pt: 1 / 72 };

const validDpi = (dpi: number) => (Number.isFinite(dpi) && dpi > 0 ? dpi : 72);

/** Valeur dans `unit` → pixels (non arrondis). */
export function toPx(value: number, unit: Unit, dpi: number): number {
  return unit === "px" ? value : value * INCHES[unit] * validDpi(dpi);
}

/** Pixels → valeur dans `unit`. */
export function fromPx(px: number, unit: Unit, dpi: number): number {
  return unit === "px" ? px : px / (INCHES[unit] * validDpi(dpi));
}

/** Décimales affichées par unité. */
export const DECIMALS: Record<Unit, number> = { px: 0, mm: 1, cm: 2, in: 3, pt: 1 };

/** Arrondi d'affichage (sans zéros inutiles). */
export function round(value: number, unit: Unit): number {
  const k = 10 ** DECIMALS[unit];
  return Math.round(value * k) / k;
}

/** « 85 mm », « 1004 px »… */
export function formatLength(px: number, unit: Unit, dpi: number): string {
  return `${round(fromPx(px, unit, dpi), unit).toLocaleString()} ${unit}`;
}

export const mmToPx = (mm: number, dpi: number) => Math.round((Math.max(0, mm) / 25.4) * validDpi(dpi));

export const DEFAULT_PRINT: PrintSetup = { bleedMm: 3, safeMm: 4, inkLimit: 300 };

interface PrintPrefs {
  /** Unité des champs de taille (Nouveau document, Taille de l'image, Format d'impression) et des règles. */
  unit: Unit;
  /** Affichage › Impression. */
  showBleed: boolean;
  showSafe: boolean;
  showMarks: boolean;
  inkAlert: boolean;
  separations: boolean;
  /** Plaque montrée seule (Séparations), ou toutes. */
  plate: "all" | "c" | "m" | "y" | "k";
  /** Noirs à l'impression (Paramètres de couleur). */
  textBlack: boolean;
  richBlack: boolean;
}

const DEFAULTS: PrintPrefs = { unit: "px", showBleed: true, showSafe: true, showMarks: true, inkAlert: false, separations: false, plate: "all", textBlack: true, richBlack: true };

export const printPrefs = $state<PrintPrefs>({ ...DEFAULTS });

let loaded = false;
let saveTimer: ReturnType<typeof setTimeout> | undefined;

export async function loadPrintPrefs(): Promise<void> {
  if (loaded) return;
  loaded = true;
  try {
    const text = await api.loadSettings("print");
    if (!text) return;
    const data = JSON.parse(text) as Partial<PrintPrefs>;
    if (UNITS.some((u) => u.id === data.unit)) printPrefs.unit = data.unit as Unit;
    for (const key of ["showBleed", "showSafe", "showMarks", "inkAlert", "separations", "textBlack", "richBlack"] as const) {
      if (typeof data[key] === "boolean") printPrefs[key] = data[key];
    }
  } catch {
    // Réglages illisibles : ceux par défaut.
  }
}

export function savePrintPrefs(): void {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void api.saveSettings("print", JSON.stringify(printPrefs, null, 2)).catch(() => undefined), 300);
}

export function setPref<K extends keyof PrintPrefs>(key: K, value: PrintPrefs[K]): void {
  printPrefs[key] = value;
  savePrintPrefs();
}

// ------------------------------------------------------------------ cadres d'impression

/** Cadres d'impression d'un document en pixels : format fini et zone de sécurité (`null` sans format d'impression). */
export function printFrames(info: DocumentInfo | null | undefined): { trim: [number, number, number, number]; safe: [number, number, number, number] | null; bleed: number } | null {
  if (!info?.print) return null;
  const trim = info.trim ?? [0, 0, info.width, info.height];
  const s = mmToPx(info.print.safeMm, info.dpi);
  const safe: [number, number, number, number] | null = s > 0 && trim[2] > 2 * s && trim[3] > 2 * s ? [trim[0] + s, trim[1] + s, trim[2] - 2 * s, trim[3] - 2 * s] : null;
  return { trim, safe, bleed: trim[0] };
}

// ------------------------------------------------------------------ analyse d'encrage

export const ink = $state<{ docId: number | null; revision: number; report: InkReport | null; busy: boolean }>({ docId: null, revision: -1, report: null, busy: false });

type InkRequest = { info: DocumentInfo; space: CmykSpace; limit: number };
let pending: { request: InkRequest; promise: Promise<InkReport | null> } | null = null;
/** Dernière demande reçue pendant qu'une analyse était en vol : relancée quand celle-ci se termine. */
let queued: { request: InkRequest; resolve: (report: InkReport | null) => void }[] = [];

const sameRequest = (a: InkRequest, b: InkRequest) => a.info.id === b.info.id && a.info.revision === b.info.revision && a.space === b.space && a.limit === b.limit;

/**
 * Analyse d'encrage du document (recalculée seulement si sa révision a changé). Une seule analyse à la fois : une
 * demande différente (autre limite, autre espace, autre révision) arrivée pendant le calcul est mise en attente et
 * relancée ensuite, plutôt que perdue.
 */
export function analyzeInk(info: DocumentInfo, space: CmykSpace, limit: number): Promise<InkReport | null> {
  if (ink.docId === info.id && ink.revision === info.revision && ink.report?.limit === limit) return Promise.resolve(ink.report);
  const request: InkRequest = { info, space, limit };
  if (pending) {
    if (sameRequest(pending.request, request)) return pending.promise;
    // Seule la demande la plus récente sera relancée : les précédentes en attente recevront son résultat.
    return new Promise((resolve) => queued.push({ request, resolve }));
  }
  ink.busy = true;
  const promise = api
    .inkReport(info.id, limit, space)
    .then((report) => {
      Object.assign(ink, { docId: info.id, revision: info.revision, report });
      return report;
    })
    .catch(() => null)
    .finally(() => {
      ink.busy = false;
      pending = null;
      const waiting = queued;
      queued = [];
      const last = waiting[waiting.length - 1];
      if (!last) return;
      const next = analyzeInk(last.request.info, last.request.space, last.request.limit);
      waiting.forEach((w) => void next.then(w.resolve));
    });
  pending = { request, promise };
  return promise;
}

/** Limite d'encrage du document (celle de son format d'impression, sinon 300 %). */
export const inkLimitOf = (info: DocumentInfo | null | undefined) => (info?.print?.inkLimit ? info.print.inkLimit : 300);

/**
 * Encrage conseillé pour un profil de presse : la limite d'usage des imprimeurs (papier couché offset 300 %, non couché
 * plus bas, GRACoL un peu plus haut). Profil inconnu : 300 %.
 */
export function recommendedInk(profile: string): number {
  const table: Record<string, number> = { fogra39: 300, fogra51: 300, fogra52: 280, gracol2006: 320, swop2006c3: 300 };
  return table[profile] ?? 300;
}

/** Limite d'encrage réglée dans le format d'impression du document, ou `null` s'il n'y en a pas. */
export const inkLimitSet = (info: DocumentInfo | null | undefined): number | null => (info?.print?.inkLimit ? info.print.inkLimit : null);
