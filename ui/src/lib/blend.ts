import type { BlendMode } from "./ipc";

/** Modes dans l'ordre et avec les séparations du menu de Photoshop. `null` = séparateur. */
export const BLEND_MENU: readonly ([BlendMode, string] | null)[] = [
  ["normal", "Normal"],
  ["dissolve", "Dissoudre"],
  null,
  ["darken", "Obscurcir"],
  ["multiply", "Produit"],
  ["color_burn", "Densité couleur +"],
  ["linear_burn", "Densité linéaire +"],
  null,
  ["lighten", "Éclaircir"],
  ["screen", "Superposition"],
  ["color_dodge", "Densité couleur -"],
  ["linear_dodge", "Densité linéaire - (ajout)"],
  null,
  ["overlay", "Incrustation"],
  ["soft_light", "Lumière tamisée"],
  ["hard_light", "Lumière crue"],
  null,
  ["difference", "Différence"],
  ["exclusion", "Exclusion"],
  ["subtract", "Soustraire"],
  ["divide", "Diviser"],
  null,
  ["hue", "Teinte"],
  ["saturation", "Saturation"],
  ["color", "Couleur"],
  ["luminosity", "Luminosité"],
];

export const BLEND_ORDER: readonly BlendMode[] = BLEND_MENU.flatMap((entry) => (entry ? [entry[0]] : []));
