// Catalogue des filtres : libellés, réglages de la boîte de dialogue et vignette du menu.
// Les vignettes sont produites par le moteur lui-même (crates/engine/examples/filter_previews.rs) :
// moitié gauche filtrée, moitié droite d'origine.
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
/** Réglage à choix (liste déroulante). */
export interface FilterChoice {
  key: string;
  label: string;
  value: string;
  options: { value: string; label: string }[];
}

export interface FilterSpec {
  type: Filter["type"];
  label: string;
  hint: string;
  params: FilterParam[];
  toggles: FilterToggle[];
  choices?: FilterChoice[];
}

const param = (key: string, label: string, min: number, max: number, value: number, step = 1, unit = ""): FilterParam => ({ key, label, min, max, step, value, unit });

export const FILTERS: FilterSpec[] = [
  { type: "gaussianBlur", label: "Flou gaussien", hint: "Flou doux et régulier.", params: [param("radius", "Rayon", 0.1, 100, 4, 0.1, " px")], toggles: [] },
  { type: "fieldBlur", label: "Flou de profondeur de champ", hint: "Des pastilles fixent la force du flou zone par zone.", params: [], toggles: [] },
  { type: "motionBlur", label: "Flou directionnel", hint: "Filé de mouvement le long d'un angle.", params: [param("angle", "Angle", -90, 90, 0, 1, "°"), param("distance", "Distance", 1, 300, 30, 1, " px")], toggles: [] },
  {
    type: "lensBlur",
    label: "Flou d'objectif",
    hint: "Bokeh : sélectionnez l'arrière-plan pour simuler une faible profondeur de champ.",
    params: [param("radius", "Rayon", 1, 60, 12, 0.5, " px"), param("highlights", "Hautes lumières", 0, 100, 40, 1, " %")],
    toggles: [],
  },
  { type: "sharpen", label: "Accentuation", hint: "Renforce la netteté des contours.", params: [param("radius", "Rayon", 0.1, 20, 1.5, 0.1, " px"), param("amount", "Gain", 1, 500, 100, 1, " %")], toggles: [] },
  { type: "noise", label: "Ajout de bruit", hint: "Grain de film.", params: [param("amount", "Quantité", 0, 100, 15, 1, " %")], toggles: [{ key: "monochrome", label: "Monochromatique", value: true }] },
  {
    type: "dither",
    label: "Tramage",
    hint: "Trame de points façon impression rétro.",
    params: [param("scale", "Taille du point", 1, 16, 2, 1, " px"), param("levels", "Tons par couche", 2, 8, 2)],
    toggles: [{ key: "monochrome", label: "Noir et blanc", value: true }],
  },
  {
    type: "glass",
    label: "Verre",
    hint: "L'image vue à travers une plaque de verre : dépoli, pavés, toile, lentilles, ou votre propre texture.",
    params: [param("distortion", "Distorsion", 0, 20, 6, 0.5), param("smoothness", "Lissage", 1, 15, 3, 0.5), param("scale", "Échelle de la texture", 50, 200, 100, 1, " %")],
    toggles: [{ key: "invert", label: "Inverser le relief", value: false }],
    choices: [
      {
        key: "texture",
        label: "Texture",
        value: "frosted",
        options: [
          { value: "frosted", label: "Dépoli" },
          { value: "blocks", label: "Pavés de verre" },
          { value: "canvas", label: "Toile" },
          { value: "tinyLens", label: "Petites lentilles" },
          { value: "custom", label: "Texture chargée…" },
        ],
      },
    ],
  },
  {
    type: "presence",
    label: "Texture, clarté, voile",
    hint: "Les trois curseurs de présence de Lightroom, calculés sur le voisinage de chaque pixel.",
    params: [param("texture", "Texture", -100, 100, 0), param("clarity", "Clarté", -100, 100, 0), param("dehaze", "Correction du voile", -100, 100, 0)],
    toggles: [],
  },
  {
    type: "gaussianNoise",
    label: "Bruit gaussien",
    hint: "Le flou granuleux des affiches : les transitions se dissolvent en poussière, sous un grain fin.",
    params: [
      param("radius", "Flou", 0, 150, 24, 0.5, " px"),
      param("scatter", "Dissolution", 0, 60, 10, 0.5, " px"),
      param("amount", "Grain", 0, 100, 22, 1, " %"),
      param("grain", "Taille du grain", 1, 8, 1, 1, " px"),
    ],
    toggles: [{ key: "monochrome", label: "Grain monochrome", value: true }],
  },
  {
    type: "colorDither",
    label: "Tramage couleur",
    hint: "Trame de points sur noir : seule la luminosité est tramée, les couleurs sont conservées.",
    params: [param("scale", "Taille du point", 1, 16, 2, 1, " px"), param("contrast", "Contraste", 0, 200, 110, 1, " %"), param("threshold", "Seuil", 0, 100, 50), param("strength", "Force de l'effet", 0, 100, 100, 1, " %")],
    toggles: [],
  },
  {
    type: "chromaticAberration",
    label: "Aberration chromatique",
    hint: "Franges rouges et bleues, comme un objectif imparfait.",
    params: [param("amount", "Décalage", 0, 60, 8, 0.5, " px"), param("angle", "Angle (mode uniforme)", -180, 180, 0, 1, "°")],
    toggles: [{ key: "radial", label: "Depuis le centre (radial)", value: true }],
  },
  { type: "ascii", label: "ASCII", hint: "L'image recomposée en caractères.", params: [param("cell", "Taille des caractères", 6, 64, 12, 1, " px")], toggles: [{ key: "color", label: "En couleur", value: true }] },
];

const images = import.meta.glob<string>("../assets/filters/*.jpg", { eager: true, import: "default" });
/** Adresse de la vignette d'un filtre (vide si elle n'a pas été produite). */
export const previewOf = (type: string): string => images[`../assets/filters/${type}.jpg`] ?? "";
export const specOf = (type: string): FilterSpec | undefined => FILTERS.find((f) => f.type === type);
