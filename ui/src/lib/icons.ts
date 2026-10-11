// Icônes de trait dessinées pour le projet (grille 24 px, trait 1,6). Aucun actif tiers.
export const ICONS = {
  move: "M12 2v20M2 12h20M12 2l-3 3M12 2l3 3M12 22l-3-3M12 22l3-3M2 12l3-3M2 12l3 3M22 12l-3-3M22 12l-3 3",
  marquee: "M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4",
  lasso: "M12 4c5 0 9 2.5 9 6s-4 6-9 6-9-2.5-9-6 4-6 9-6zM7 15c-1 2 0 4 2 6",
  wand: "M4 20L15 9M14 3v3M18 10h3M18 4l-2 2M20 15l-2-1",
  crop: "M6 2v16h16M2 6h16v16",
  pipette: "M16 4l4 4-3 3-4-4zM13 9l-9 9v2h2l9-9",
  brush: "M18 3l3 3-9 9-3-3zM9 12c-3 0-4 2-4 4s-1 3-2 3c3 1 8 0 8-5",
  eraser: "M8 20l-5-5L14 4l7 7-9 9zM8 20h12M9 9l7 7",
  bucket: "M4 12l8-8 8 8-8 8zM20 16c1 1.5 1.5 2.5 1.5 3.5a1.5 1.5 0 01-3 0c0-1 .5-2 1.5-3.5",
  pen: "M12 2l7 11-7 9-7-9zM12 2v11",
  type: "M5 6V4h14v2M12 4v16M9 20h6",
  pathSelect: "M5 3l14 8-6 2-2 6z",
  shape: "M4 5h16v14H4z",
  hand: "M8 13V5a1.5 1.5 0 013 0v6M11 10V4a1.5 1.5 0 013 0v7M14 11V6a1.5 1.5 0 013 0v9c0 4-2 6-6 6s-5-2-7-6l-1-2a1.5 1.5 0 012.5-1.5L8 14",
  zoom: "M4 11a7 7 0 1014 0 7 7 0 10-14 0M16 16l5 5",
  eye: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM9 12a3 3 0 106 0 3 3 0 10-6 0",
  close: "M6 6l12 12M18 6L6 18",
  // ---- Panneau Calques
  chevron: "M9 6l6 6-6 6",
  clip: "M16 5h-6v13M5 13l5 5 5-5",
  textLayer: "M5 6V4h14v2M12 4v16M9 20h6",
  shapeRect: "M5 7h14v10H5z",
  shapeEllipse: "M12 6a8 6 0 100 12 8 6 0 000-12z",
  shapePolygon: "M12 5l8 14H4z",
  group: "M3 6h6l2 2h10v11H3z",
  adjustment: "M12 3a9 9 0 100 18zM12 3a9 9 0 010 18",
  lockTransparency: "M4 4h16v16H4zM4 12h16M12 4v16",
  lockPixels: "M18 3l3 3-9 9-3-3zM9 12c-3 0-4 2-4 4s-1 3-2 3c3 1 8 0 8-5",
  lockPosition: "M12 2v20M2 12h20M12 2l-3 3M12 2l3 3M12 22l-3-3M12 22l3-3M2 12l3-3M2 12l3 3M22 12l-3-3M22 12l-3 3",
  lockAll: "M6 11h12v9H6zM8 11V8a4 4 0 018 0v3",
  link: "M9 15l6-6M10 6l1-1a4 4 0 016 6l-1 1M14 18l-1 1a4 4 0 01-6-6l1-1",
  // « fx » : dessiné en lettres tant qu'aucun fichier fx.svg / fx.png n'est fourni (ce tracé ne sert que de filigrane au gabarit).
  fx: "M10 5c-2 0-3 1-3 3v11M5 11h5M13 11l6 8M19 11l-6 8",
  mask: "M4 5h16v14H4zM8.5 12a3.5 3.5 0 107 0 3.5 3.5 0 10-7 0",
  newLayer: "M5 4h14v16H5zM12 9v6M9 12h6",
  trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
  // ---- Divers
  caret: "M6 9l6 6 6-6",
  plus: "M12 5v14M5 12h14",

  quickSelect: "M14 4l6 6-7 7-4 1 1-4zM4 14a3 3 0 013-3M4 19a3 3 0 003 2M10 21a3 3 0 002-1",
  heal: "M4.5 13.5l9-9a4.2 4.2 0 016 6l-9 9a4.2 4.2 0 01-6-6zM9 9l6 6M11.5 12.5h.01M13 11h.01M10 14h.01",
} as const;

export type IconName = keyof typeof ICONS;

/**
 * Icônes personnalisées : tout fichier `ui/src/assets/icons/<nom>.svg` (ou `.png`) remplace l'icône
 * intégrée du même nom. Voir `ui/src/assets/icons/LISEZ-MOI.md` pour les noms et le gabarit.
 */
// Les fichiers de travail (gabarits, modèles), dont le nom commence par « _ », ne sont ni lus ni embarqués dans l'application.
const files = import.meta.glob<string>(["../assets/icons/*.{svg,png}", "!../assets/icons/_*"], { eager: true, import: "default", query: "?url" });
export const CUSTOM_ICONS: Partial<Record<IconName, string>> = Object.fromEntries(
  Object.entries(files)
    .filter(([path]) => !path.split("/").pop()?.startsWith("_")) // `_gabarit-icones.svg` et autres fichiers de travail
    .map(([path, url]) => [path.split("/").pop()?.replace(/\.(svg|png)$/, "") ?? "", url]),
);
