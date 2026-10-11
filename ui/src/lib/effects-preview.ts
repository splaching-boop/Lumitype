// Aperçu instantané des effets de calque (boîte de dialogue « Effets de calque ») : pendant qu'on règle un effet,
// la carte graphique le calcule elle-même à chaque pas (viewport/gpu-effects.ts), à partir des pixels seuls du calque
// déjà en texture. Le moteur ne rend l'effet exact qu'au relâchement (ou après une courte pause) ; son image remplace
// l'aperçu dès qu'elle est chargée. Quand la carte graphique ne compose pas le document (préférence, mémoire vidéo,
// calque démesuré), `preview` répond faux et chaque pas passe par le moteur, comme avant.
import type { LayerEffect } from "./ipc";

export interface EffectsPreviewControls {
  /** Affiche ces effets pour ce calque, calculés par la carte graphique. Faux : impossible ici. */
  preview: (layer: number, effects: readonly LayerEffect[]) => boolean;
  /** Le moteur a reçu les derniers effets : l'aperçu s'efface dès que son rendu est chargé. */
  finish: (layer: number) => void;
  /** Fin immédiate (annulation). */
  cancel: (layer: number) => void;
  /** Garde les pixels seuls de ce calque chargés (`null` : fin), le temps que la boîte de dialogue est ouverte. */
  hold: (layer: number | null) => void;
}

let controls: EffectsPreviewControls | null = null;

/** L'affichage s'enregistre ici (`null` : il disparaît). */
export function provideEffectsPreview(provider: EffectsPreviewControls | null): void {
  controls = provider;
}

/** Retire l'affichage `provider` s'il est encore celui enregistré. */
export function withdrawEffectsPreview(provider: EffectsPreviewControls): void {
  if (controls === provider) controls = null;
}

export function previewEffects(layer: number, effects: readonly LayerEffect[]): boolean {
  return controls?.preview(layer, effects) ?? false;
}

export function finishEffectsPreview(layer: number): void {
  controls?.finish(layer);
}

export function cancelEffectsPreview(layer: number): void {
  controls?.cancel(layer);
}

export function holdEffectsSource(layer: number | null): void {
  controls?.hold(layer);
}
