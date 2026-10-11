// Résolution dynamique de la zone de travail : pendant un geste (déplacement, zoom, pinceau, curseur), l'image est
// dessinée dans un canevas plus petit que l'écran, que la webview agrandit. Sur un écran 4K, c'est jusqu'à quatre fois
// moins de pixels à calculer ET à transmettre à l'écran par la webview — ce qui, sous WebKitGTK, est le vrai goulot.
// Au relâchement, l'image redevient nette en une seule image.
//
// Le facteur s'ajuste tout seul, d'après le temps réellement mesuré entre deux images du geste : s'il dépasse
// l'intervalle de rafraîchissement de l'écran, la résolution baisse d'un cran ; si les images repassent largement
// sous le budget, elle remonte. Module pur, testable sans navigateur.

/** Facteurs possibles, du plus fin au plus grossier (côté du canevas, pas surface : 0,5 = quatre fois moins de pixels). */
export const STEPS = [1, 0.75, 0.5] as const;

/** Images consécutives trop lentes avant de baisser, et images rapides avant de remonter : évite les oscillations. */
const SLOW_FRAMES = 3;
const FAST_FRAMES = 24;

export class DynamicResolution {
  private step = 0;
  /** Intervalle d'une image de l'écran, estimé par l'intervalle le plus court observé (ms). */
  private refreshMs = 1000 / 60;
  private slow = 0;
  private fast = 0;
  private lastFrame = 0;
  /** Facteur retenu à la fin du dernier geste : le suivant repart de là, sans refaire la descente. */
  private remembered = 0;

  constructor(private enabled = true) {}

  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    if (!enabled) this.endGesture();
  }

  /** Facteur courant (1 = résolution de l'écran). */
  get factor(): number {
    return STEPS[this.step] ?? 1;
  }

  /**
   * Une image vient d'être demandée pendant un geste, à l'instant `now` (ms). Retourne vrai si le facteur change
   * (le canevas est alors à redimensionner avant de dessiner).
   */
  gestureFrame(now: number): boolean {
    if (!this.enabled) return false;
    const previous = this.lastFrame;
    this.lastFrame = now;
    let changed = false;
    if (previous === 0) {
      // Début du geste : on repart du facteur qui convenait la dernière fois.
      if (this.step !== this.remembered) {
        this.step = this.remembered;
        changed = true;
      }
      return changed;
    }
    const gap = now - previous;
    if (gap <= 0) return false;
    // L'écran ne peut pas aller plus vite que son rafraîchissement : le plus court intervalle vu en est une bonne mesure
    // (bornée : 240 Hz au plus, 30 Hz au moins).
    this.refreshMs = Math.max(1000 / 240, Math.min(this.refreshMs, gap, 1000 / 30));
    if (gap > this.refreshMs * 1.15) {
      this.slow += 1;
      this.fast = 0;
      if (this.slow >= SLOW_FRAMES && this.step < STEPS.length - 1) {
        this.step += 1;
        this.slow = 0;
        changed = true;
      }
    } else if (gap < this.refreshMs * 1.02) {
      this.fast += 1;
      this.slow = 0;
      if (this.fast >= FAST_FRAMES && this.step > 0) {
        this.step -= 1;
        this.fast = 0;
        changed = true;
      }
    } else {
      this.slow = 0;
      this.fast = 0;
    }
    return changed;
  }

  /** Fin du geste (ou image hors geste) : retour à la résolution de l'écran. Vrai si le facteur change. */
  endGesture(): boolean {
    this.lastFrame = 0;
    this.slow = 0;
    this.fast = 0;
    this.remembered = this.step;
    if (this.step === 0) return false;
    this.step = 0;
    return true;
  }
}
