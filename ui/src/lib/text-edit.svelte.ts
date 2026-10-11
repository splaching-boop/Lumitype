// Mise en forme du texte en cours de saisie, utilisable depuis n'importe quel panneau
// (Propriétés, mais aussi le sélecteur de couleur principal).
import { app } from "../state/app.svelte";
import { dropLatest, edit, editLatest, whenIdle } from "./engine";
import type { TextData } from "./ipc";
import { type RunStyle, applyStyle, applyToLayer } from "./text-runs";

/**
 * Applique un style au texte en cours de saisie : à la sélection s'il y en a une, sinon à tout
 * le texte. Retourne faux si aucun texte n'est en cours de saisie (rien n'est fait).
 */
export function restyleEditingText(patch: RunStyle): boolean {
  const editing = app.textEdit;
  const layer = app.active?.info.layers.find((l) => l.id === editing?.layerId);
  if (!editing || !layer?.text) return false;
  // Ce qui est tapé fait foi : il peut être en avance sur ce que le moteur a confirmé.
  const current: TextData = { ...$state.snapshot(layer.text), content: editing.draft, runs: $state.snapshot(editing.runs) };
  const { start, end } = editing.selection;
  const next = end > start ? { ...current, runs: applyStyle(current, start, end, patch) } : applyToLayer(current, patch);
  editing.runs = next.runs ?? [];
  editLatest(editing.id, { op: "setText", id: layer.id, text: next, session: editing.id });
  return true;
}

/**
 * Valide la saisie de texte en cours : la séance se ferme, et un calque resté vide est retiré, comme
 * dans Photoshop. Utilisable sans la surcouche de saisie : quand on clique un autre calque, celle-ci
 * est démontée avant que l'espace de travail ne réagisse, et c'est d'ici que la saisie est close.
 * Tout est lu dans un instantané pris AVANT de vider `app.textEdit`.
 */
export function finishTextEdit(): void {
  const current = app.textEdit;
  if (!current) return;
  const { id, docId, layerId } = current;
  const empty = current.draft.trim() === "";
  app.textEdit = null;
  // `edit` vise le document actif : si l'on vient d'en changer, le calque vide reste (le supprimer ici
  // toucherait un calque de même id dans l'autre document).
  if (!empty || app.activeId !== docId) return;
  // Calque resté vide : la frappe encore en attente ne sert plus (elle viserait un calque retiré) ; on
  // laisse finir celle en cours d'envoi, puis le calque est supprimé.
  dropLatest(id);
  void whenIdle(id).then(() => edit({ op: "delete", ids: [layerId] }));
}
