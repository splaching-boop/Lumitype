// Fermeture des menus contextuels (calques, image, cadre de transformation, réglages, menu du pinceau).
//
// Un menu se ferme quand on clique ailleurs ou quand la fenêtre perd le focus. Mais le clic droit qui
// l'ouvre produit lui-même des signaux qui ressemblent à une fermeture, selon le moteur d'affichage :
// - sous Linux, Chromium émet `contextmenu` dès l'appui ; selon le moteur embarqué (CEF sous X11), le
//   `pointerdown` du même clic peut arriver APRÈS l'ouverture, et le relâchement suit ;
// - sous CEF/X11, le clic donne le focus clavier à la fenêtre de la page (XSetInputFocus) : la fenêtre
//   reçoit alors un `blur` (souvent suivi aussitôt d'un `focus`) juste après l'ouverture.
// Le menu s'ouvrait donc et se refermait dans la foulée. Ce garde distingue ces signaux résiduels d'une
// vraie fermeture, sans rien supposer du moteur.

/** Délai après l'ouverture pendant lequel un `blur` de fenêtre ou un appui droit sont tenus pour résiduels. */
const SETTLE_MS = 300;

export interface DismissGuard {
  /** Vrai si cet appui doit fermer le menu : il vient d'un nouveau clic, postérieur à l'ouverture. */
  pointerDownCloses(event: PointerEvent): boolean;
  /** À appeler sur le `blur` de la fenêtre : ferme seulement si le document a réellement perdu le focus. */
  onWindowBlur(close: () => void): void;
  /** Annule une vérification de focus en attente (démontage du menu). */
  dispose(): void;
}

/** À créer à l'ouverture du menu (initialisation du composant). */
export function dismissGuard(): DismissGuard {
  // `event.timeStamp` et `performance.now()` partagent la même origine : on peut les comparer.
  const openedAt = performance.now();
  let pending: ReturnType<typeof setTimeout> | null = null;

  return {
    pointerDownCloses(event) {
      // Appui daté d'avant l'ouverture : c'est le clic droit qui a ouvert le menu, livré en retard.
      if (event.timeStamp <= openedAt) return false;
      // Appui droit juste après l'ouverture : écho du même clic (ordre des événements propre au moteur).
      if (event.button === 2 && event.timeStamp - openedAt < SETTLE_MS) return false;
      return true;
    },
    onWindowBlur(close) {
      // Juste après l'ouverture, le `blur` vient du changement de focus provoqué par le clic lui-même.
      if (performance.now() - openedAt < SETTLE_MS) return;
      // Au-delà, on vérifie au tick suivant que le focus a vraiment quitté le document (un `blur` aussitôt
      // suivi d'un `focus` ne compte pas : c'est un simple passage de focus entre fenêtres de la même application).
      if (pending !== null) clearTimeout(pending);
      pending = setTimeout(() => {
        pending = null;
        if (!document.hasFocus()) close();
      }, 0);
    },
    dispose() {
      if (pending !== null) clearTimeout(pending);
      pending = null;
    },
  };
}
