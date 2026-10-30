// File des développements RAW (fenêtre « Développement RAW », voir lib/actions.ts) : une seule fenêtre à la fois, dans
// l'ordre des demandes, et le compte « n sur N » de la série en cours. Sans dépendance à l'interface, pour être vérifié
// sans navigateur (ui/tests/raw-queue.test.mjs).
import type { RawParams } from "./ipc";

/**
 * Série de RAW à développer : un dialogue par fichier, ou les mêmes réglages pour tous les suivants. Les RAW arrivés
 * pendant qu'une série est en cours (dépôt dans la fenêtre, fichiers ouverts depuis l'explorateur pendant que la
 * fenêtre Développement RAW est ouverte) la rejoignent : ils passent après le RAW courant, et « n sur N » en tient compte.
 */
export interface RawSeries {
  /** Fichiers de la série (ceux des appelants encore en cours compris). */
  count: number;
  /** Fichiers déjà commencés. */
  index: number;
  /** « Appliquer à tous les suivants » : réglages repris sans rouvrir le dialogue. */
  forRest: RawParams | null;
  /** Réglages du RAW précédent : point de départ du dialogue suivant. */
  last: RawParams | null;
  /** Case « redévelopper plus tard » du RAW précédent (reprise aussi par « Appliquer à tous les suivants »). */
  keepRaw: boolean;
  /** Appelants (ouverture, dépôt, import…) dont la boucle sur leurs fichiers n'est pas finie. */
  participants: number;
}

/** Part d'une série apportée par un appelant : ses `count` RAW, dont `done` déjà commencés. */
export interface RawBatch {
  series: RawSeries;
  count: number;
  done: number;
  ended: boolean;
}

/** Série en cours : tant qu'un appelant n'a pas fini, les nouveaux RAW la rejoignent. */
let currentSeries: RawSeries | null = null;
/** Prévenu quand le nombre de fichiers d'une série change (le dialogue affiché met à jour son « n sur N »). */
let countListener: (series: RawSeries) => void = () => {};
export function onSeriesCount(listener: (series: RawSeries) => void): void {
  countListener = listener;
}

/** Rejoint la série en cours (ou en commence une) avec `count` RAW. À terminer par `endRawBatch`, quoi qu'il arrive. */
export function rawBatch(count: number): RawBatch {
  const series = currentSeries ?? (currentSeries = { count: 0, index: 0, forRest: null, last: null, keepRaw: false, participants: 0 });
  series.count += count;
  series.participants += 1;
  countListener(series);
  return { series, count, done: 0, ended: false };
}

/** Fin de la boucle d'un appelant : ses RAW jamais commencés (format refusé…) sortent du compte ; la dernière part clôt la série. */
export function endRawBatch(batch: RawBatch): void {
  if (batch.ended) return;
  batch.ended = true;
  const { series } = batch;
  series.count -= batch.count - batch.done;
  series.participants -= 1;
  if (series.participants === 0 && currentSeries === series) currentSeries = null;
  countListener(series);
}

/** Un RAW de cette part commence : rend sa position dans la série (1 sur N). */
export function startRaw(batch: RawBatch): { index: number; count: number } {
  batch.done += 1;
  batch.series.index += 1;
  return { index: batch.series.index, count: Math.max(batch.series.count, batch.series.index) };
}

/**
 * Tour de rôle des fenêtres « Développement RAW » : une seule à la fois, dans l'ordre des demandes. Sans lui, un RAW
 * déposé pendant qu'une fenêtre était ouverte la remplaçait (Annuler implicite), puis la fermeture de la première
 * fermait aussi la seconde : les deux étaient perdus. Une erreur d'un tour ne bloque pas les suivants.
 */
let rawTurn: Promise<void> = Promise.resolve();
export function rawInTurn(work: () => Promise<void>): Promise<void> {
  const run = rawTurn.then(work);
  rawTurn = run.catch(() => undefined);
  return run;
}
