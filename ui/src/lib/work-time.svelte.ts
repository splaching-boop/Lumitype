// Temps de travail par projet (barre d'état) : seul le temps ACTIF compte. Une seconde est ajoutée au document affiché
// tant qu'on s'en sert ; après 5 minutes sans geste (souris, clavier, molette, stylet), ou fenêtre cachée, le compteur
// se met en pause, et repart au premier geste. Le total est enregistré avec le projet (.graf, et sa copie de secours) et
// reprend à l'ouverture.
import { app } from "../state/app.svelte";

export const IDLE_MS = 5 * 60_000;

export const workTime = $state({ idle: false });

let last = Date.now();

function noteActivity(): void {
  last = Date.now();
  if (workTime.idle) workTime.idle = false;
}

/** Démarre l'horloge (une fois, au montage de l'application) ; rend de quoi l'arrêter. */
export function startWorkClock(): () => void {
  const events = ["pointerdown", "pointermove", "keydown", "wheel"] as const;
  for (const name of events) window.addEventListener(name, noteActivity, { capture: true, passive: true });
  const timer = setInterval(() => {
    const idle = Date.now() - last > IDLE_MS || document.hidden;
    if (workTime.idle !== idle) workTime.idle = idle;
    const doc = app.active;
    if (!idle && doc && !app.homeOpen) doc.workSeconds += 1;
  }, 1000);
  return () => {
    clearInterval(timer);
    for (const name of events) window.removeEventListener(name, noteActivity, { capture: true });
  };
}

/** « 12 h 47 », « 47 min », « 0 min ». */
export function formatWork(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")}`;
}
