// Images des pointes et des motifs de pinceau, pour le pinceau en direct sur la carte graphique (voir
// viewport/renderer.ts, `beginTipStrokePreview`). Lues une fois auprès du moteur, puis gardées : un trait commence
// sans attendre quand la pointe a déjà servi, ou a été préchargée au choix du pinceau.
import { api } from "./ipc";

export interface TipImage {
  width: number;
  height: number;
  /** Espacement propre de la pointe (fraction du diamètre). */
  spacing: number;
  /** Couverture, un octet par pixel, ligne par ligne. */
  mask: Uint8Array;
}

export interface PatternImage {
  width: number;
  height: number;
  gray: Uint8Array;
}

const tips = new Map<number, TipImage>();
const patterns = new Map<number, PatternImage>();
const loading = new Map<string, Promise<void>>();

/** Pointe déjà chargée, ou `null` (le chargement est alors lancé). */
export function tipImage(id: number): TipImage | null {
  const done = tips.get(id);
  if (done) return done;
  void loadTip(id);
  return null;
}

/** Motif déjà chargé, ou `null` (le chargement est alors lancé). */
export function patternImage(id: number): PatternImage | null {
  const done = patterns.get(id);
  if (done) return done;
  void loadPattern(id);
  return null;
}

export function loadTip(id: number): Promise<void> {
  const key = `t${id}`;
  if (tips.has(id)) return Promise.resolve();
  const pending = loading.get(key);
  if (pending) return pending;
  const job = api
    .brushTipImage(id)
    .then((buffer) => {
      const view = new DataView(buffer);
      const [width, height, spacing] = [view.getUint32(0, true), view.getUint32(4, true), view.getFloat32(8, true)];
      if (width > 0 && height > 0 && buffer.byteLength >= 12 + width * height) tips.set(id, { width, height, spacing, mask: new Uint8Array(buffer, 12, width * height) });
    })
    .catch(() => undefined)
    .finally(() => loading.delete(key));
  loading.set(key, job);
  return job;
}

export function loadPattern(id: number): Promise<void> {
  const key = `p${id}`;
  if (patterns.has(id)) return Promise.resolve();
  const pending = loading.get(key);
  if (pending) return pending;
  const job = api
    .brushPatternImage(id)
    .then((buffer) => {
      const view = new DataView(buffer);
      const [width, height] = [view.getUint32(0, true), view.getUint32(4, true)];
      if (width > 0 && height > 0 && buffer.byteLength >= 8 + width * height) patterns.set(id, { width, height, gray: new Uint8Array(buffer, 8, width * height) });
    })
    .catch(() => undefined)
    .finally(() => loading.delete(key));
  loading.set(key, job);
  return job;
}
