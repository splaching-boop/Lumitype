// Compteur d'images de la webview : cadence moyenne et pire intervalle, mesurés sur l'horloge d'affichage.
//
// Deux chiffres, pas un seul. La moyenne dit à quelle fréquence l'affichage tourne ; la **pire image** de la
// dernière seconde dit s'il a bégayé. Une moyenne de 60 i/s peut très bien cacher une image à 40 ms : c'est elle
// qu'on sent au déplacement d'un calque, pas la moyenne.
//
// La mesure a un coût : une boucle `requestAnimationFrame` permanente empêche la webview de se reposer. Elle
// n'est donc lancée que si l'utilisateur coche la case dans les Préférences, et rien ne tourne quand elle est
// décochée.
import { app } from "../state/app.svelte";

/** Intervalle de rafraîchissement de l'affichage, en millisecondes. Assez court pour réagir, assez long pour être lisible. */
const WINDOW_MS = 500;

/** Pire coût d'une image de la zone de travail depuis la dernière lecture : temps JavaScript du dessin, temps carte. */
let worstDraw = { js: 0, gpu: 0 };

/**
 * La zone de travail vient de dessiner une image : `js` millisecondes passées dans le code (commandes WebGL incluses),
 * `gpu` millisecondes attendues ensuite par la carte (`gl.finish`). Lu par la barre d'état pour dire d'où vient le
 * temps : trop de JavaScript (tuiles, uniformes), ou trop de pixels à composer.
 */
export function noteDraw(js: number, gpu: number): void {
  worstDraw = { js: Math.max(worstDraw.js, js), gpu: Math.max(worstDraw.gpu, gpu) };
}

/**
 * Démarre la mesure et rend la fonction qui l'arrête. Pendant la mesure, `app.fps` porte la dernière lecture ;
 * à l'arrêt il repasse à `null`, ce qui retire l'affichage de la barre d'état.
 */
export function measureFps(): () => void {
  let handle = 0;
  let frames = 0;
  let worst = 0;
  let previous = 0;
  let since = 0;
  const tick = (now: number) => {
    handle = requestAnimationFrame(tick);
    // Première image : on n'a pas d'intervalle précédent, et le temps écoulé depuis le démarrage ne veut rien dire.
    if (previous === 0) {
      previous = now;
      since = now;
      return;
    }
    const gap = now - previous;
    previous = now;
    frames += 1;
    if (gap > worst) worst = gap;
    const elapsed = now - since;
    if (elapsed >= WINDOW_MS) {
      app.fps = { rate: Math.round((frames * 1000) / elapsed), worstMs: Math.round(worst * 10) / 10, drawMs: Math.round(worstDraw.js * 10) / 10, gpuMs: Math.round(worstDraw.gpu * 10) / 10 };
      worstDraw = { js: 0, gpu: 0 };
      frames = 0;
      worst = 0;
      since = now;
    }
  };
  handle = requestAnimationFrame(tick);
  return () => {
    cancelAnimationFrame(handle);
    app.fps = null;
  };
}
