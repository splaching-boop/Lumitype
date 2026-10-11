// Aperçu instantané des calques de réglage (onglet Réglages, Développement) : pendant qu'on tire un curseur, l'image
// n'est plus recalculée par le moteur à chaque pas, le rendu suit la souris.
//  - Document composé par la carte graphique : le réglage y est déjà calculé exactement comme par le moteur ; seules
//    ses valeurs changent (quelques octets, mis à plat par le moteur en un instant).
//  - Sinon : la carte graphique applique le réglage à l'image par une table de correspondance 3D des couleurs. Au
//    relâchement, l'image rendue par le moteur (identique) reprend la main dès qu'elle est prête.
import { app } from "../state/app.svelte";
import { whenIdle } from "./engine";
import { type Adjustment, api } from "./ipc";

export interface AdjustPreviewControls {
  begin: (plan: { below: number[]; target: number[]; above: number[] }) => void;
  setLut: (bytes: ArrayBuffer, size: number, mix: number) => void;
  /** Vrai si la carte graphique compose ce calque de réglage (aperçu par ses valeurs, sans plan). */
  composes: (layer: number) => boolean;
  /** Valeurs provisoires du réglage (`adjustment_gpu`), appliquées par la carte graphique. */
  setValues: (layer: number, bytes: ArrayBuffer) => void;
  finish: () => void;
}

let controls: AdjustPreviewControls | null = null;
export function provideAdjustPreview(provider: AdjustPreviewControls | null): void {
  controls = provider;
}

/** Côté de la table : 33³ couleurs (le standard des LUT d'étalonnage), interpolées par la carte graphique. */
const LUT_SIZE = 33;

type State = { layer: number; gpu: boolean; ready: Promise<boolean>; inFlight: boolean; pending: { adjustment: Adjustment; mix: number } | null };
let state: State | null = null;

/**
 * Plan préparé d'avance pour un calque de réglage (voir `warmAdjustment`). Le plan est ce qui coûte :
 * le moteur compose séparément ce qui est dessous, le calque visé et ce qui est dessus.
 */
let warmed: { layer: number; revision: number; plan: Promise<{ below: number[]; target: number[]; above: number[] } | null> } | null = null;

/**
 * Le calque de réglage vient d'être sélectionné : on prépare tout de suite ce que le premier
 * mouvement de curseur demanderait (le plan de composition, et une première table de couleurs).
 * Sans cela, le premier geste attendait ce calcul — d'où l'impression que « ça a du mal à partir ».
 */
export function warmAdjustment(layer: number, adjustment: Adjustment): void {
  const doc = app.active;
  // La pile de calques a pu changer depuis : un plan préparé avant ne vaut plus rien.
  if (!doc || (warmed && warmed.layer === layer && warmed.revision === doc.info.revision)) return;
  if (controls?.composes(layer)) return; // composition par la carte graphique : aucun plan ni table à préparer
  warmed = { layer, revision: doc.info.revision, plan: api.adjustPlan(doc.info.id, layer).catch(() => null) };
  // La table elle-même est mise en cache par le moteur : la demander une fois suffit à la rendre instantanée.
  void api.adjustmentLut(adjustment, LUT_SIZE).catch(() => undefined);
}

/** Le curseur bouge : nouvelle table pour ce réglage (la plus récente seulement si le moteur est occupé). */
export function previewAdjustment(layer: number, adjustment: Adjustment, mix: number): void {
  const doc = app.active;
  if (!doc || !controls) return;
  if (!state || state.layer !== layer) {
    const provider = controls;
    if (provider.composes(layer)) {
      // Composition par la carte graphique : rien à préparer, le réglage y est déjà.
      app.adjustPreviewing = true;
      state = { layer, gpu: true, ready: Promise.resolve(true), inFlight: false, pending: null };
    }
  }
  if (!state || state.layer !== layer) {
    const provider = controls;
    // Plan déjà demandé à la sélection du calque : le premier mouvement n'attend rien.
    const usable = warmed && warmed.layer === layer && warmed.revision === doc.info.revision;
    const plan = usable && warmed ? warmed.plan : api.adjustPlan(doc.info.id, layer);
    const ready = plan.then(
      (plan) => {
        if (!plan) return false;
        provider.begin(plan);
        app.adjustPreviewing = true;
        return true;
      },
      () => false,
    );
    state = { layer, gpu: false, ready, inFlight: false, pending: null };
  }
  const current = state;
  current.pending = { adjustment, mix };
  if (current.inFlight) return;
  void current.ready.then(async (ok) => {
    if (!ok) return;
    while (current.pending && state === current) {
      const { adjustment: next, mix: opacity } = current.pending;
      current.pending = null;
      current.inFlight = true;
      try {
        if (current.gpu) {
          const bytes = await api.adjustmentGpu(next);
          if (state === current) controls?.setValues(layer, bytes);
          continue;
        }
        const bytes = await api.adjustmentLut(next, LUT_SIZE);
        if (state === current) controls?.setLut(bytes, LUT_SIZE, opacity);
      } catch {
        // Table indisponible : l'image rendue par le moteur prendra le relais au relâchement.
      } finally {
        current.inFlight = false;
      }
    }
  });
}

/** Curseur relâché : une fois la dernière valeur appliquée par le moteur, son image remplace l'aperçu. */
export function endAdjustmentPreview(layer: number): void {
  const current = state;
  if (!current || current.layer !== layer) return;
  state = null;
  void current.ready.then((ok) => {
    if (!ok) return;
    void whenIdle(`reglage-${layer}`).then(() => {
      controls?.finish();
      app.adjustPreviewing = false;
    });
  });
}
