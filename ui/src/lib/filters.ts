// Catalogue des filtres : libellés, catégories de la galerie, réglages et vignettes.
// Les vignettes sont produites par le moteur lui-même (crates/engine/examples/filter_previews.rs) :
// la fougère de référence, entièrement filtrée.
import type { Filter } from "./ipc";

export interface FilterParam {
  key: string;
  label: string;
  min: number;
  max: number;
  step: number;
  value: number;
  unit: string;
}
export interface FilterToggle {
  key: string;
  label: string;
  value: boolean;
}
/** Réglage à choix : liste déroulante, boutons côte à côte (`segmented`) ou vignettes (`swatches`). */
export interface FilterChoice {
  key: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
  display?: "select" | "segmented" | "swatches";
}

/** Couleur réglable (encre, papier) : « #rrggbb » dans les réglages, [r, v, b] pour le moteur. */
export interface FilterColor {
  key: string;
  label: string;
  value: string;
}

/** Type d'un filtre de la galerie : ceux du moteur, plus la Fluidité (qui a sa propre fenêtre). */
export type GalleryKind = Filter["type"];

export interface FilterSpec {
  type: GalleryKind;
  label: string;
  hint: string;
  params: FilterParam[];
  toggles: FilterToggle[];
  choices?: FilterChoice[];
  colors?: FilterColor[];
  /** Valeurs sans curseur (centre du flou radial…), envoyées telles quelles au moteur. */
  fixed?: Record<string, number>;
  /** Le filtre tire au sort (grain, motif…) : un bouton « Variante » change le tirage. */
  seeded?: boolean;
}

const param = (key: string, label: string, min: number, max: number, value: number, step = 1, unit = ""): FilterParam => ({ key, label, min, max, step, value, unit });
const color = (key: string, label: string, value: string): FilterColor => ({ key, label, value });
const toggle = (key: string, label: string, value: boolean): FilterToggle => ({ key, label, value });
const choice = (key: string, label: string, value: string, options: [string, string][], display: FilterChoice["display"] = "select"): FilterChoice => ({
  key,
  label,
  value,
  options: options.map(([v, l]) => ({ value: v, label: l })),
  display,
});

/** Motifs du filtre Truchet, dans l'ordre de la grille de vignettes. */
export const TRUCHET_PATTERNS: [string, string][] = [
  ["arcs", "Arcs"],
  ["double", "Double voie"],
  ["triple", "Triple voie"],
  ["fan", "Éventail"],
  ["diagonals", "Diagonales"],
  ["ribbons", "Rubans"],
  ["bevels", "Biseaux"],
  ["rings", "Anneaux"],
  ["mixed", "Arcs et croisements"],
  ["weave", "Tissage"],
  ["maze", "Labyrinthe"],
  ["dotsArcs", "Points et arcs"],
  ["filledQuarters", "Quarts pleins"],
  ["triangles", "Triangles"],
  ["multiScale", "Multi-échelle"],
];

export const FILTERS: FilterSpec[] = [
  // ---- Flou
  { type: "gaussianBlur", label: "Flou gaussien", hint: "Flou doux et régulier.", params: [param("radius", "Rayon", 0.1, 100, 4, 0.1, " px")], toggles: [] },
  { type: "fieldBlur", label: "Flou de profondeur de champ", hint: "Des pastilles fixent la force du flou zone par zone.", params: [], toggles: [] },
  { type: "motionBlur", label: "Flou directionnel", hint: "Filé de mouvement le long d'un angle.", params: [param("angle", "Angle", -90, 90, 0, 1, "°"), param("distance", "Distance", 1, 300, 30, 1, " px")], toggles: [] },
  {
    type: "trail",
    label: "Traînée",
    hint: "Flou de mouvement d'un seul côté, posé en mode Fondu : le sujet s'effrite en poussière fine dans la direction choisie. Idéal sur un objet détouré.",
    params: [
      param("angle", "Direction", -180, 180, 35, 1, "°"),
      param("length", "Longueur", 10, 2000, 650, 1, " px"),
      param("opacity", "Opacité du fondu", 0, 100, 80, 1, " %"),
      param("falloff", "Atténuation", 0, 100, 100, 1, " %"),
      param("sharp", "Sujet net", 0, 100, 55, 1, " %"),
    ],
    toggles: [],
    seeded: true,
  },
  {
    type: "blades",
    label: "Lames",
    hint: "L'image tourne, bascule en perspective, puis se découpe en lames verticales ondulées qui se fondent vers le noir. Idéal sur un logo ou une lettre détourés.",
    params: [
      param("rotation", "Rotation de l'image", -180, 180, 45, 1, "°"),
      param("tilt", "Perspective", 0, 80, 46, 1, "°"),
      param("orient", "Orientation des lames", -90, 90, 35, 1, "°"),
      param("density", "Densité des lames", 3, 120, 26, 1, ""),
      param("height", "Hauteur des lames", 0, 100, 17, 1, " %"),
      param("amp", "Ondulation", 0, 30, 4.5, 0.5, " %"),
      param("wavelength", "Longueur de vague", 10, 300, 70, 1, " %"),
      param("phase", "Phase de la vague", -180, 180, 0, 1, "°"),
      param("light", "Clarté", 10, 100, 85, 1, " %"),
    ],
    toggles: [toggle("color", "Couleur de l'image", true), toggle("transparent", "Fondu transparent", false)],
  },
  {
    type: "lensBlur",
    label: "Flou d'objectif",
    hint: "Bokeh : sélectionnez l'arrière-plan pour simuler une faible profondeur de champ.",
    params: [param("radius", "Rayon", 1, 60, 12, 0.5, " px"), param("highlights", "Hautes lumières", 0, 100, 40, 1, " %")],
    toggles: [],
  },
  {
    type: "radialBlur",
    label: "Flou radial",
    hint: "Zoom depuis un centre : cliquez dans l'aperçu pour le placer.",
    params: [param("strength", "Force", 0, 100, 25, 1, " %"), param("fringe", "Franges colorées", 0, 100, 15, 1, " %")],
    toggles: [],
    fixed: { x: 0.5, y: 0.5 },
  },
  // ---- Netteté
  { type: "sharpen", label: "Accentuation", hint: "Renforce la netteté des contours.", params: [param("radius", "Rayon", 0.1, 20, 1.5, 0.1, " px"), param("amount", "Gain", 1, 500, 100, 1, " %")], toggles: [] },
  {
    type: "presence",
    label: "Texture, clarté, voile",
    hint: "Les trois curseurs de présence de Lightroom, calculés sur le voisinage de chaque pixel.",
    params: [param("texture", "Texture", -100, 100, 0), param("clarity", "Clarté", -100, 100, 0), param("dehaze", "Correction du voile", -100, 100, 0)],
    toggles: [],
  },
  // ---- Grain
  { type: "noise", label: "Ajout de bruit", hint: "Grain de film.", params: [param("amount", "Quantité", 0, 100, 15, 1, " %")], toggles: [toggle("monochrome", "Monochromatique", true)] },
  {
    type: "gaussianNoise",
    label: "Bruit gaussien",
    hint: "Le flou granuleux des affiches : les transitions se dissolvent en poussière, sous un grain fin.",
    params: [param("radius", "Flou", 0, 150, 24, 0.5, " px"), param("scatter", "Dissolution", 0, 60, 10, 0.5, " px"), param("amount", "Grain", 0, 100, 22, 1, " %"), param("grain", "Taille du grain", 1, 8, 1, 1, " px")],
    toggles: [toggle("monochrome", "Grain monochrome", true)],
  },
  {
    type: "filmGrain",
    label: "Grain argentique",
    hint: "Grain de pellicule en amas, plus marqué dans les tons moyens.",
    params: [param("amount", "Quantité", 0, 100, 40, 1, " %"), param("size", "Taille du grain", 0.5, 8, 1.5, 0.5, " px")],
    toggles: [],
    seeded: true,
  },
  // ---- Déformation
  {
    type: "glass",
    label: "Verre",
    hint: "L'image vue à travers une plaque de verre : dépoli, pavés, toile, lentilles, ou votre propre texture.",
    params: [param("distortion", "Distorsion", 0, 20, 6, 0.5), param("smoothness", "Lissage", 1, 15, 3, 0.5), param("scale", "Échelle de la texture", 50, 200, 100, 1, " %")],
    toggles: [toggle("invert", "Inverser le relief", false)],
    choices: [
      choice("texture", "Texture", "frosted", [
        ["frosted", "Dépoli"],
        ["blocks", "Pavés de verre"],
        ["canvas", "Toile"],
        ["tinyLens", "Petites lentilles"],
        ["custom", "Texture chargée…"],
      ]),
    ],
  },
  {
    type: "flutedGlass",
    label: "Verre cannelé",
    hint: "L'image lue à travers des cannelures de verre : chaque bande la reprend, retournée et étirée.",
    params: [param("rib", "Largeur des cannelures", 2, 120, 14, 1, " px"), param("refraction", "Réfraction", 0, 8, 3, 0.1), param("wave", "Ondulation", 0, 20, 2, 0.5, " px")],
    toggles: [toggle("horizontal", "Cannelures horizontales", false)],
  },
  { type: "liquify", label: "Fluidité", hint: "Déformation libre au pinceau, dans sa propre fenêtre.", params: [], toggles: [] },
  {
    type: "chromaticAberration",
    label: "Aberration",
    hint: "Couleurs décalées comme par un objectif imparfait ; avec de l'ondulation ou des traînées, elles se séparent le long d'une lentille irrégulière.",
    params: [
      param("amount", "Décalage", 0, 60, 8, 0.5, " px"),
      param("angle", "Angle (mode uniforme)", -180, 180, 0, 1, "°"),
      param("warp", "Ondulation", 0, 60, 0, 0.5, " px"),
      param("streak", "Traînées", 0, 60, 0, 0.5, " px"),
    ],
    toggles: [toggle("radial", "Depuis le centre (radial)", true)],
  },
  // ---- Lumière
  {
    type: "vignette",
    label: "Vignettage",
    hint: "Bords assombris (ou éclaircis, en négatif) autour d'une zone épargnée.",
    params: [param("amount", "Quantité", -100, 100, 60, 1, " %"), param("size", "Taille", 0, 100, 40, 1, " %"), param("softness", "Douceur", 0, 100, 60, 1, " %")],
    toggles: [],
  },
  {
    type: "petrolSheen",
    label: "Film d'essence",
    hint: "Un film irisé sur l'image, plus marqué dans les tons moyens et les zones planes.",
    params: [param("strength", "Intensité", 0, 100, 85, 1, " %"), param("bands", "Bandes de couleur", 0.5, 5, 1.6, 0.1), param("scale", "Taille des nappes", 20, 2000, 400, 10, " px")],
    toggles: [],
    seeded: true,
  },
  {
    type: "chrome",
    label: "Chrome",
    hint: "Métal liquide : les formes deviennent des reflets polis.",
    params: [param("rings", "Anneaux", 2, 24, 9, 0.5), param("softness", "Douceur", 1, 40, 6, 0.5, " px")],
    toggles: [],
    choices: [
      choice(
        "tint",
        "Métal",
        "silver",
        [
          ["silver", "Argent"],
          ["gold", "Or"],
          ["copper", "Cuivre"],
        ],
        "segmented",
      ),
    ],
  },
  {
    type: "emboss",
    label: "Relief",
    hint: "Estampage : l'image en relief, éclairée de biais.",
    params: [param("angle", "Angle de la lumière", -180, 180, 135, 1, "°"), param("height", "Hauteur", 1, 20, 2, 0.5, " px"), param("amount", "Quantité", 1, 500, 200, 1, " %")],
    toggles: [toggle("color", "Garder les couleurs", false)],
  },
  // ---- Impression
  {
    type: "dither",
    label: "Tramage",
    hint: "Trame de points façon impression rétro.",
    params: [param("scale", "Taille du point", 1, 16, 2, 1, " px"), param("levels", "Tons par couche", 2, 8, 2)],
    toggles: [toggle("monochrome", "Noir et blanc", true)],
  },
  {
    type: "colorDither",
    label: "Tramage couleur",
    hint: "Trame de points sur noir : seule la luminosité est tramée, les couleurs sont conservées.",
    params: [param("scale", "Taille du point", 1, 16, 2, 1, " px"), param("contrast", "Contraste", 0, 200, 110, 1, " %"), param("threshold", "Seuil", 0, 100, 50), param("strength", "Force de l'effet", 0, 100, 100, 1, " %")],
    toggles: [],
  },
  {
    type: "halftone",
    label: "Demi-teinte",
    hint: "Trame comme dans la presse : points, cercles concentriques, lignes, ou les quatre encres CMJN.",
    params: [param("cell", "Taille de la trame", 3, 60, 8, 0.5, " px"), param("angle", "Angle", -90, 90, 45, 1, "°"), param("contrast", "Contraste", -100, 100, 0, 1)],
    toggles: [],
    choices: [
      choice(
        "pattern",
        "Motif",
        "dots",
        [
          ["dots", "Points"],
          ["circles", "Cercles"],
          ["lines", "Lignes"],
          ["cmyk", "CMJN couleur"],
        ],
        "swatches",
      ),
    ],
    colors: [color("ink", "Encre", "#1a1a1a"), color("paper", "Papier", "#ffffff")],
    fixed: { x: 0.5, y: 0.5 },
  },
  {
    type: "retroPrint",
    label: "Impression rétro",
    hint: "Une seule encre sur papier coloré, en grains d'impression irréguliers.",
    params: [param("density", "Densité", 0, 100, 50, 1, " %"), param("grain", "Grain", 0.5, 6, 1.5, 0.1, " px"), param("contrast", "Contraste", -100, 100, 10, 1), param("bleed", "Bavure de l'encre", 0, 100, 30, 1, " %")],
    toggles: [],
    colors: [color("ink", "Encre", "#262896"), color("paper", "Papier", "#f2d6e0")],
    seeded: true,
  },
  {
    type: "riso",
    label: "Risographie",
    hint: "Encres fluo tramées, légèrement décalées, sur papier crème.",
    params: [param("cell", "Trame", 3, 30, 6, 0.5, " px"), param("shift", "Décalage des encres", 0, 20, 3, 0.5, " px"), param("grain", "Grain", 0, 100, 35, 1, " %")],
    toggles: [],
    choices: [
      choice("inks", "Encres", "quad", [
        ["quad", "Bleu, rose, jaune, noir"],
        ["duo", "Bleu et rose"],
        ["greenOrange", "Vert et orange"],
        ["monoRed", "Rouge seul"],
      ]),
    ],
    seeded: true,
  },
  {
    type: "stipple",
    label: "Pointillisme",
    hint: "Points de couleur posés au hasard, plus nombreux dans les clairs.",
    params: [param("spacing", "Espacement", 3, 60, 9, 0.5, " px"), param("size", "Taille des points", 0.5, 20, 3.4, 0.1, " px")],
    toggles: [toggle("paper", "Sur papier clair", false)],
    choices: [
      choice(
        "shape",
        "Forme",
        "round",
        [
          ["round", "Ronds"],
          ["square", "Carrés"],
          ["stroke", "Traits"],
        ],
        "segmented",
      ),
    ],
    seeded: true,
  },
  // ---- Rétro
  {
    type: "crt",
    label: "Écran CRT",
    hint: "Les luminophores rouges, verts et bleus d'un vieil écran, vus de près.",
    params: [param("pitch", "Taille des phosphores", 3, 40, 9, 0.5, " px"), param("glow", "Halo", 0, 100, 100, 1, " %")],
    toggles: [],
    choices: [
      choice(
        "mask",
        "Masque",
        "triads",
        [
          ["triads", "Points"],
          ["grille", "Grille"],
          ["slots", "Fentes"],
        ],
        "segmented",
      ),
    ],
  },
  {
    type: "scanlines",
    label: "Lignes de balayage",
    hint: "Fines lignes horizontales et signal qui glisse, comme une vidéo.",
    params: [param("period", "Écart des lignes", 2, 24, 4, 0.5, " px"), param("depth", "Profondeur", 0, 100, 50, 1, " %"), param("jitter", "Décrochage", 0, 60, 6, 0.5, " px"), param("contrast", "Contraste", 50, 200, 112, 1, " %")],
    toggles: [],
    seeded: true,
  },
  { type: "pixelate", label: "Pixellisation", hint: "Grands pixels carrés.", params: [param("cell", "Taille des pixels", 2, 300, 16, 1, " px")], toggles: [] },
  {
    type: "ascii",
    label: "ASCII",
    hint: "L'image recomposée en caractères ; avec du halo, elle luit derrière le texte.",
    params: [param("cell", "Taille des caractères", 6, 64, 12, 1, " px"), param("glow", "Halo", 0, 100, 0, 1, " %")],
    toggles: [toggle("color", "En couleur", true)],
  },
  // ---- Glitch
  { type: "glitch", label: "Distorsion numérique", hint: "Le signal qui se déchire : bandes, canaux séparés, blocs.", params: [param("intensity", "Intensité", 0, 100, 70, 1, " %")], toggles: [], seeded: true },
  {
    type: "datamosh",
    label: "Datamosh",
    hint: "Vidéo corrompue : macroblocs, bavures et couleurs déplacées.",
    params: [param("block", "Taille des blocs", 4, 120, 22, 1, " px"), param("bleed", "Bavure", 0, 100, 35, 1, " %"), param("glitches", "Blocs déplacés", 0, 100, 50, 1, " %"), param("grain", "Grain", 0, 100, 30, 1, " %")],
    toggles: [],
    choices: [
      choice(
        "palette",
        "Palette",
        "violet",
        [
          ["violet", "Violet"],
          ["original", "D'origine"],
          ["acid", "Acide"],
        ],
        "segmented",
      ),
    ],
    seeded: true,
  },
  {
    type: "pixelSort",
    label: "Tri de pixels",
    hint: "Les zones claires s'étirent en traînées triées par luminosité.",
    params: [param("low", "Seuil bas", 0, 100, 45, 1, " %"), param("high", "Seuil haut", 0, 100, 100, 1, " %")],
    toggles: [],
    choices: [
      choice(
        "direction",
        "Sens",
        "right",
        [
          ["right", "→"],
          ["left", "←"],
          ["down", "↓"],
          ["up", "↑"],
        ],
        "segmented",
      ),
    ],
  },
  // ---- Artistique
  { type: "oilPaint", label: "Peinture à l'huile", hint: "Aplats et touches, contours gardés nets.", params: [param("radius", "Taille des touches", 2, 30, 8, 0.5, " px")], toggles: [] },
  { type: "posterize", label: "Postérisation", hint: "Peu de tons par couche, en aplats.", params: [param("levels", "Tons par couche", 2, 16, 4)], toggles: [] },
  { type: "contours", label: "Contours", hint: "Les contours de l'image, tracés sur blanc.", params: [param("amount", "Intensité", 10, 400, 120, 1, " %")], toggles: [toggle("color", "En couleur", false)] },
  {
    type: "truchet",
    label: "Truchet",
    hint: "Un motif de tuiles dont les traits prennent la couleur et l'éclat de l'image.",
    params: [param("cell", "Taille des cases", 4, 300, 24, 1, " px"), param("width", "Épaisseur", 2, 40, 9, 0.5, " %"), param("glow", "Halo", 0, 100, 60, 1, " %")],
    toggles: [],
    choices: [
      choice("pattern", "Motif", "arcs", TRUCHET_PATTERNS, "swatches"),
      choice(
        "color",
        "Couleur",
        "image",
        [
          ["image", "Image"],
          ["mono", "Mono"],
          ["neon", "Néon"],
          ["paper", "Papier"],
        ],
        "segmented",
      ),
    ],
    seeded: true,
  },
];

/** Catégories de la galerie, dans l'ordre du rail. */
export const CATEGORIES: { id: string; label: string; types: GalleryKind[] }[] = [
  { id: "blur", label: "Flou", types: ["gaussianBlur", "fieldBlur", "motionBlur", "trail", "lensBlur", "radialBlur"] },
  { id: "sharpen", label: "Netteté", types: ["sharpen", "presence"] },
  { id: "grain", label: "Grain", types: ["noise", "gaussianNoise", "filmGrain"] },
  { id: "distort", label: "Déformation", types: ["glass", "flutedGlass", "blades", "liquify", "chromaticAberration"] },
  { id: "light", label: "Lumière", types: ["vignette", "petrolSheen", "chrome", "emboss"] },
  { id: "print", label: "Impression", types: ["dither", "colorDither", "halftone", "retroPrint", "riso", "stipple"] },
  { id: "retro", label: "Rétro", types: ["crt", "scanlines", "pixelate", "ascii"] },
  { id: "glitch", label: "Glitch", types: ["glitch", "datamosh", "pixelSort"] },
  { id: "art", label: "Artistique", types: ["oilPaint", "posterize", "contours", "truchet"] },
];

/** Filtres arrivés avec la galerie : un point les signale. */
export const NEW_FILTERS = new Set<GalleryKind>([
  "blades",
  "trail",
  "retroPrint",
  "radialBlur",
  "filmGrain",
  "flutedGlass",
  "vignette",
  "petrolSheen",
  "chrome",
  "emboss",
  "halftone",
  "riso",
  "stipple",
  "crt",
  "scanlines",
  "pixelate",
  "glitch",
  "datamosh",
  "pixelSort",
  "oilPaint",
  "posterize",
  "contours",
  "truchet",
]);

const images = import.meta.glob<string>("../assets/filters/*.jpg", { eager: true, import: "default" });
/** Adresse de la vignette d'un filtre (vide si elle n'a pas été produite). */
export const previewOf = (type: string): string => images[`../assets/filters/${type}.jpg`] ?? "";
/** Vignette d'un choix à vignettes : `<filtre>-<valeur>.jpg` (motifs du Truchet, de la demi-teinte). */
export const swatchOf = (type: string, value: string): string => images[`../assets/filters/${type}-${value}.jpg`] ?? "";
export const specOf = (type: string): FilterSpec | undefined => FILTERS.find((f) => f.type === type);

/** Réglages par défaut d'un filtre (curseurs, cases, choix, valeurs fixes et graine). */
export function defaultsOf(spec: FilterSpec): Record<string, number | boolean | string> {
  return Object.fromEntries([
    ...[...spec.params, ...spec.toggles, ...(spec.choices ?? []), ...(spec.colors ?? [])].map((p) => [p.key, p.value]),
    ...Object.entries(spec.fixed ?? {}),
    ...(spec.seeded ? [["seed", 0]] : []),
  ]);
}

/** « #rrggbb » → [r, v, b]. */
export const hexToRgb = (hex: string): [number, number, number] => {
  const n = Number.parseInt(hex.replace("#", "").padEnd(6, "0").slice(0, 6), 16) || 0;
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
};

/** Réglages tels que le moteur les attend : couleurs « #rrggbb » converties en [r, v, b]. */
export function engineValues(spec: FilterSpec, values: Record<string, number | boolean | string>): Record<string, unknown> {
  const out: Record<string, unknown> = { ...values };
  for (const c of spec.colors ?? []) if (typeof values[c.key] === "string") out[c.key] = hexToRgb(values[c.key] as string);
  return out;
}

/** Réglages d'un filtre existant (filtre dynamique rouvert), dans la forme des réglages de la galerie. */
export function valuesOf(spec: FilterSpec, filter: Filter): Record<string, number | boolean | string> {
  const values = defaultsOf(spec);
  const given = filter as unknown as Record<string, unknown>;
  const hex = (rgb: number[]) => `#${rgb.map((v) => Math.round(v).toString(16).padStart(2, "0")).join("")}`;
  for (const key of Object.keys(values)) {
    const value = given[key];
    if ((spec.colors ?? []).some((c) => c.key === key) && Array.isArray(value)) values[key] = hex(value as number[]);
    else if (typeof value === "number" || typeof value === "boolean" || typeof value === "string") values[key] = value;
  }
  return values;
}

/** Filtre montré quand aucun n'est demandé : le flou gaussien. */
export const FALLBACK_SPEC: FilterSpec = FILTERS[0] as FilterSpec;
