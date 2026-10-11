// Aperçus calculés par la carte graphique, sans solliciter le moteur à chaque pas : le viewport
// fournit les fonctions, les panneaux (curseur d'opacité) et les outils (déplacement) les appellent.
import { app } from "../state/app.svelte";
import { api } from "./ipc";

export interface PreviewPlan {
  below: number[];
  moving: number[];
  above: number[];
}

export interface PreviewControls {
  /** Image plus grossière le temps d'un geste (0 = rendu normal). */
  draft: (levels: number) => void;
  /** Vrai si la carte graphique compose tout le document : tout aperçu d'opacité passe alors par elle. */
  composes: () => boolean;
  /** `opaque` : calque rendu à pleine opacité, son opacité étant appliquée par `setAlpha`. */
  begin: (plan: PreviewPlan, opaque: number | null) => void;
  setAlpha: (alpha: number) => void;
  end: () => void;
}

let controls: PreviewControls | null = null;
export function providePreview(provider: PreviewControls | null): void {
  controls = provider;
}

/**
 * Prépare l'aperçu d'opacité d'un calque. Retourne les commandes si l'image se décompose
 * exactement en « dessous / calque / dessus » (pile en mode Normal), sinon `null` : l'appelant
 * retombe alors sur la recomposition par le moteur.
 */
/**
 * Geste continu qui oblige le moteur à tout recomposer (opacité d'un groupe, pile non décomposable) :
 * l'image passe un cran plus grossière le temps du geste, ce qui la garde fluide, puis redevient nette.
 */
export function setDraftLevel(levels: number): void {
  controls?.draft(levels);
}

export async function beginOpacityPreview(docId: number, layer: number, isGroup = false): Promise<PreviewControls | null> {
  if (!controls || !app.performance.gpuPreviews) return null;
  // Composition par la carte graphique : l'opacité du calque (ou du groupe) y est appliquée directement.
  if (controls.composes()) {
    controls.begin({ below: [], moving: [], above: [] }, layer);
    return controls;
  }
  if (isGroup) return null;
  const plan = await api.movePlan(docId, [layer]).catch(() => null);
  if (!plan || plan.moving.length !== 1) return null;
  controls.begin(plan, layer);
  return controls;
}
