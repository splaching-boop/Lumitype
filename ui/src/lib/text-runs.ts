// Portions stylées d'un texte (un mot dans une autre police ou une autre couleur).
// Positions en unités UTF-16, celles de la sélection d'un champ de saisie. Fonctions pures.
import type { Rgb, StyleRun, TextData } from "./ipc";

export type RunStyle = Omit<StyleRun, "start" | "end">;
/** Style effectif d'une plage ; `null` = valeurs différentes dans la plage (affiché « ? »). */
export interface RangeStyle {
  fontFamily: string | null;
  weight: number | null;
  italic: boolean | null;
  color: Rgb | null;
}

const KEYS = ["fontFamily", "weight", "italic", "color"] as const;
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/**
 * Recale les portions après une modification du texte. La partie modifiée est trouvée en comparant
 * l'ancien et le nouveau texte (préfixe et suffixe communs). Taper à la fin d'une portion la
 * prolonge ; taper juste avant la repousse ; supprimer tout son contenu la fait disparaître.
 */
export function adjustRuns(runs: readonly StyleRun[], before: string, after: string): StyleRun[] {
  let prefix = 0;
  while (prefix < before.length && prefix < after.length && before[prefix] === after[prefix]) prefix++;
  let suffix = 0;
  while (suffix < before.length - prefix && suffix < after.length - prefix && before[before.length - 1 - suffix] === after[after.length - 1 - suffix]) suffix++;
  const [removedFrom, removedTo] = [prefix, before.length - suffix];
  const inserted = after.length - prefix - suffix;
  const shift = inserted - (removedTo - removedFrom);
  const mapStart = (x: number) => (x < removedFrom ? x : x >= removedTo ? x + shift : removedFrom + inserted);
  const mapEnd = (x: number) => (x < removedFrom ? x : x >= removedTo && x > removedFrom ? x + shift : removedFrom + inserted);
  return runs.map((run) => ({ ...run, start: mapStart(run.start), end: mapEnd(run.end) })).filter((run) => run.end > run.start);
}

/** Style de chaque unité du texte : réglages du calque, surchargés par les portions. */
function expand(text: TextData, length: number): Required<RunStyle>[] {
  const base: Required<RunStyle> = { fontFamily: text.fontFamily, weight: text.weight, italic: text.italic, color: text.color };
  const units = Array.from({ length }, () => ({ ...base }));
  for (const run of text.runs ?? []) {
    for (let i = Math.max(0, run.start); i < Math.min(length, run.end); i++) {
      const unit = units[i];
      if (!unit) continue;
      for (const key of KEYS) if (run[key] !== undefined) Object.assign(unit, { [key]: run[key] });
    }
  }
  return units;
}

/** Regroupe les unités voisines de même style ; ce qui est identique au calque n'a pas besoin de portion. */
function collapse(text: TextData, units: Required<RunStyle>[]): StyleRun[] {
  const runs: StyleRun[] = [];
  const diff = (unit: Required<RunStyle>): RunStyle => {
    const out: RunStyle = {};
    if (unit.fontFamily !== text.fontFamily) out.fontFamily = unit.fontFamily;
    if (unit.weight !== text.weight) out.weight = unit.weight;
    if (unit.italic !== text.italic) out.italic = unit.italic;
    if (!same(unit.color, text.color)) out.color = unit.color;
    return out;
  };
  units.forEach((unit, i) => {
    const style = diff(unit);
    const last = runs.at(-1);
    if (Object.keys(style).length === 0) return;
    const { start: _s, end: _e, ...lastStyle } = last ?? { start: 0, end: -1 };
    if (last && last.end === i && same(lastStyle, style)) last.end = i + 1;
    else runs.push({ start: i, end: i + 1, ...style });
  });
  return runs;
}

/** Applique un style à la plage `start..end` du texte : retourne les nouvelles portions. */
export function applyStyle(text: TextData, start: number, end: number, patch: RunStyle): StyleRun[] {
  const units = expand(text, text.content.length);
  for (let i = Math.max(0, start); i < Math.min(units.length, end); i++) Object.assign(units[i] ?? {}, patch);
  return collapse(text, units);
}

/** Change un réglage pour TOUT le calque : les portions perdent cette propriété, les autres sont gardées. */
export function applyToLayer(text: TextData, patch: Partial<TextData>): TextData {
  const next = { ...text, ...patch };
  const cleared = (text.runs ?? []).map((run) => {
    const copy: StyleRun = { ...run };
    for (const key of KEYS) if (key in patch) delete copy[key];
    return copy;
  });
  // Repasser par expand/collapse fusionne et retire les portions devenues identiques au calque.
  return { ...next, runs: collapse(next, expand({ ...next, runs: cleared }, next.content.length)) };
}

/** Style commun à une plage, `null` là où il varie. Plage vide : style à la position du curseur. */
export function styleOver(text: TextData, start: number, end: number): RangeStyle {
  const units = expand(text, text.content.length);
  const from = Math.min(start, end);
  const slice = from === Math.max(start, end) ? units.slice(Math.max(0, from - 1), Math.max(1, from)) : units.slice(from, Math.max(start, end));
  const first = slice[0] ?? { fontFamily: text.fontFamily, weight: text.weight, italic: text.italic, color: text.color };
  const common = <K extends (typeof KEYS)[number]>(key: K) => (slice.every((unit) => same(unit[key], first[key])) ? first[key] : null);
  return { fontFamily: common("fontFamily"), weight: common("weight"), italic: common("italic"), color: common("color") };
}
