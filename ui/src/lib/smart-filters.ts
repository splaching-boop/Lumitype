// Filtres dynamiques : libellés, réouverture d'un filtre sur ses réglages, et modifications de la liste d'un calque.
// La liste est toujours renvoyée entière au moteur (`setSmartFilters`) : un seul état d'historique par changement.
import { editCameraRawFilter } from "./actions";
import { edit, editLatest } from "./engine";
import { specOf } from "./filters";
import type { BlendMode, Filter, LayerInfo, SmartFilter } from "./ipc";
import { app } from "../state/app.svelte";

/** Nom affiché d'un filtre (en français, traduit à l'affichage). */
export function filterLabel(filter: Filter): string {
  if (filter.type === "liquify") return "Fluidité";
  if (filter.type === "cameraRaw") return "Filtre Camera Raw";
  return specOf(filter.type)?.label ?? filter.type;
}

/** Copie modifiable de la liste d'un calque. */
const listOf = (layer: LayerInfo): SmartFilter[] => (layer.filters ? (JSON.parse(JSON.stringify(layer.filters.list)) as SmartFilter[]) : []);

/** Rouvre la fenêtre d'un filtre sur ses réglages : « Appliquer » le remplacera dans la liste. */
export function openFilterEditor(layer: LayerInfo, index: number): void {
  const item = layer.filters?.list[index];
  if (!item || !app.active) return;
  app.active.selection = [layer.id];
  app.filterEdit = { layerId: layer.id, index };
  if (item.filter.type === "liquify") app.liquifyOpen = true;
  else if (item.filter.type === "cameraRaw") void editCameraRawFilter(index).finally(() => (app.filterEdit = null));
  else app.filterDialog = item.filter.type;
}

function send(layer: LayerInfo, list: SmartFilter[], patch: { enabled?: boolean; maskEnabled?: boolean } = {}, gesture?: string): void {
  const op = { op: "setSmartFilters" as const, id: layer.id, list, enabled: patch.enabled ?? layer.filters?.enabled ?? true, ...(patch.maskEnabled === undefined ? {} : { maskEnabled: patch.maskEnabled }) };
  if (gesture) editLatest(gesture, { ...op, gesture });
  else void edit(op);
}

/** Remplace le filtre `index` (réglages rouverts). */
export function replaceFilter(layer: LayerInfo, index: number, filter: Filter): void {
  const list = listOf(layer);
  if (!list[index]) return;
  list[index] = { ...list[index], filter, enabled: true };
  send(layer, list);
}

/** Œil d'un filtre. */
export function toggleFilter(layer: LayerInfo, index: number): void {
  const list = listOf(layer);
  if (!list[index]) return;
  list[index].enabled = !list[index].enabled;
  send(layer, list);
}

/** Œil de « Filtres dynamiques » : tous les filtres à la fois. */
export function toggleAllFilters(layer: LayerInfo): void {
  send(layer, listOf(layer), { enabled: !(layer.filters?.enabled ?? true) });
}

/** Masque de filtre activé ou non. */
export function toggleFilterMask(layer: LayerInfo): void {
  send(layer, listOf(layer), { maskEnabled: !(layer.filters?.maskEnabled ?? true) });
}

export function removeFilter(layer: LayerInfo, index: number): void {
  const list = listOf(layer);
  list.splice(index, 1);
  send(layer, list);
}

export function duplicateFilter(layer: LayerInfo, index: number): void {
  const list = listOf(layer);
  const item = list[index];
  if (!item) return;
  list.splice(index + 1, 0, structuredClone(item));
  send(layer, list);
}

/** Change l'ordre : `delta` +1 = plus haut dans la liste affichée (appliqué plus tard). */
export function moveFilter(layer: LayerInfo, index: number, delta: number): void {
  const list = listOf(layer);
  const to = index + delta;
  if (!list[index] || to < 0 || to >= list.length) return;
  const [item] = list.splice(index, 1);
  list.splice(to, 0, item as SmartFilter);
  send(layer, list);
}

/** Options de fusion d'un filtre ; `gesture` : pas d'un curseur (un seul état d'historique pour le geste). */
export function setFilterBlending(layer: LayerInfo, index: number, patch: { blend?: BlendMode; opacity?: number }, gesture?: string): void {
  const list = listOf(layer);
  if (!list[index]) return;
  list[index] = { ...list[index], ...patch };
  send(layer, list, {}, gesture);
}

export function removeAllFilters(layer: LayerInfo): void {
  send(layer, []);
}

export function bakeFilters(layer: LayerInfo): void {
  void edit({ op: "bakeFilters", id: layer.id });
}

export function deleteFilterMask(layer: LayerInfo): void {
  void edit({ op: "deleteFilterMask", id: layer.id });
}
