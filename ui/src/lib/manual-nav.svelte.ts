// Ouvrir la notice à un endroit précis, depuis n'importe où (menu Aide, recherche de l'accueil) : l'accueil
// s'affiche sur l'onglet « Notice » et fait défiler jusqu'à l'ancre demandée (section, sous-section ou ligne).
import { app } from "../state/app.svelte";

let sequence = 0;

/** Dernière demande d'ouverture, pas encore prise en compte par l'accueil. `seq` distingue deux demandes identiques. */
export const manualNav = $state<{ request: { anchor: string; seq: number } | null }>({ request: null });

/** Affiche l'accueil sur la notice ; `anchor` : la section (ou la ligne) où se placer, sinon le début. */
export function manualOpen(anchor = ""): void {
  app.homeOpen = true;
  sequence += 1;
  manualNav.request = { anchor, seq: sequence };
}
