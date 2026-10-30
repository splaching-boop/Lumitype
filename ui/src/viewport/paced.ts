// Envois à la carte graphique étalés sur plusieurs images affichées (couches du cache de déplacement, textures des
// calques par bandes) : quelques millisecondes par image au plus, le reste attend l'image suivante. Un gros envoi
// d'un coup figeait l'affichage, surtout sur une carte graphique intégrée. Module sans dépendance : testé à part.

/** Temps d'envoi à la carte graphique par image affichée, en ms : au-delà, la suite attend l'image suivante. */
export const UPLOAD_MS_PER_FRAME = 6;
/** Taille d'une bande de texture envoyée en une fois (4 Mio). */
export const UPLOAD_STRIP_BYTES = 4 * 1048576;

/**
 * Envois à la carte graphique étalés sur plusieurs images affichées : une couche de la taille de l'écran (8 Mio, avec
 * ses réductions) prend plusieurs millisecondes à envoyer, et toutes envoyées d'un coup figeaient l'affichage le temps
 * de la préparation. Ici, au plus `UPLOAD_MS_PER_FRAME` ms par image ; la suite passe à l'image suivante.
 */
const uploads: { run: () => void }[] = [];
let uploadsScheduled = false;
/** Fenêtre de temps en cours (≈ une image affichée) et temps d'envoi déjà dépensé dedans. */
let windowStart = 0;
let spent = 0;
const FRAME_MS = 16;
function pumpUploads(): void {
  uploadsScheduled = false;
  while (uploads.length > 0 && budgetLeft()) runTimed(uploads.shift()!.run);
  if (uploads.length > 0) schedulePump();
}
function budgetLeft(): boolean {
  const now = performance.now();
  if (now - windowStart >= FRAME_MS) [windowStart, spent] = [now, 0];
  return spent < UPLOAD_MS_PER_FRAME;
}
function runTimed(run: () => void): void {
  const started = performance.now();
  run();
  spent += performance.now() - started;
}
/** Suite des envois à l'image suivante ; un minuteur prend le relais si l'image tarde (fenêtre cachée, onglet en veille). */
function schedulePump(): void {
  if (uploadsScheduled) return;
  uploadsScheduled = true;
  let done = false;
  const go = () => {
    if (done) return;
    done = true;
    pumpUploads();
  };
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(go);
  setTimeout(go, FRAME_MS * 2);
}
/** Exécute `task` tout de suite s'il reste du temps d'envoi dans cette image, sinon lors d'une image suivante. */
export function paced<T>(task: () => T): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const run = () => {
      try {
        resolve(task());
      } catch (error) {
        reject(error);
      }
    };
    if (uploads.length === 0 && budgetLeft()) runTimed(run);
    else {
      uploads.push({ run });
      schedulePump();
    }
  });
}


/** Envois encore en attente (diagnostic). */
export function pendingUploads(): number {
  return uploads.length;
}
