// Sauvegarde automatique. Toutes les N minutes (3 par défaut, de 2 à 60), chaque document modifié depuis sa dernière
// copie est enregistré, historique compris, dans une copie de secours À PART (dossier de récupération de
// l'application) : le fichier .graf de l'utilisateur n'est jamais écrit à sa place. Après une fermeture imprévue,
// ces copies sont proposées au démarrage. Enregistrer (Ctrl+S) ou fermer un document supprime sa copie.
//
// Instantané : le moteur prend l'état du document au tout début de la sauvegarde (en quelques microsecondes) ; ce
// qui est fait pendant l'écriture n'y entre pas — ce sera dans la suivante. Rien n'est bloqué pendant ce temps.
import { listen } from "@tauri-apps/api/event";
import { app } from "../state/app.svelte";
import { api } from "./ipc";

export const AUTOSAVE_MIN = 2;
export const AUTOSAVE_MAX = 60;

let running = false;

async function saveOne(doc: (typeof app.documents)[number]): Promise<void> {
  const revision = doc.info.revision; // ce que l'instantané va contenir
  app.autosaveProgress = 0;
  const stop = await listen<number>("autosave-progress", ({ payload }) => (app.autosaveProgress = payload));
  try {
    await api.autosave(doc.info.id, doc.recoveryKey);
    [doc.autosavedRevision, doc.autosavedAt] = [revision, Date.now()];
  } catch (error) {
    // Une copie de secours manquée n'interrompt pas le travail : on retentera au prochain passage.
    console.warn("Sauvegarde automatique impossible :", error);
    doc.autosavedAt = Date.now();
  } finally {
    stop();
    app.autosaveProgress = null;
  }
}

/** Passe en revue les documents ; appelée toutes les 15 secondes. */
async function tick(): Promise<void> {
  if (running) return;
  running = true;
  try {
    const interval = Math.min(AUTOSAVE_MAX, Math.max(AUTOSAVE_MIN, app.autosaveMinutes)) * 60_000;
    for (const doc of [...app.documents]) {
      const changed = doc.info.revision !== doc.autosavedRevision && app.isModified(doc);
      if (changed && Date.now() - doc.autosavedAt >= interval && app.documents.includes(doc)) await saveOne(doc);
    }
  } finally {
    running = false;
  }
}

export function startAutosave(): () => void {
  const timer = setInterval(() => void tick(), 15_000);
  return () => clearInterval(timer);
}

/** Le document vient d'être enregistré ou fermé : sa copie de secours ne sert plus. */
export function forgetRecovery(doc: { recoveryKey: string; info: { revision: number }; autosavedRevision: number; autosavedAt: number }): void {
  [doc.autosavedRevision, doc.autosavedAt] = [doc.info.revision, Date.now()];
  void api.recoveryDelete(doc.recoveryKey).catch(() => undefined);
}

/** Rouvre une copie de secours, avec tout son historique ; elle continue ensuite sous la même clé. */
export async function recover(key: string): Promise<void> {
  const info = await api.recoveryOpen(key);
  app.add(info);
  const doc = app.documents.find((d) => d.info.id === info.id);
  if (doc) {
    doc.recoveryKey = key;
    doc.savedRevision = -1; // non enregistré : Ctrl+S le proposera, la fermeture le signalera
  }
}
