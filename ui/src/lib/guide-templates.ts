// Modèles de repères. Chaque modèle décrit sa disposition en proportions du plan de travail (0 → 1),
// jamais en pixels : la même disposition suit donc n'importe quel format de document.
//
// Un modèle peut aussi décrire des « indices » — spirale, bandes, points forts. Ce sont des aides
// d'aperçu uniquement : seuls `vertical` et `horizontal` deviennent de vrais repères.

/** Quart d'ellipse de la spirale d'or, en proportions du plan de travail. */
export type Arc = { sx: number; sy: number; ex: number; ey: number; rx: number; ry: number; sweep: 0 | 1 };

/** Ce que l'aperçu dessine en plus des repères, et que la validation jette. */
export type Hints = {
  /** Spirale du nombre d'or. */
  arcs?: Arc[];
  /** Œil de la spirale, ou sujet suggéré. */
  eye?: [number, number];
  /** Coupes calculées mais non retenues : elles montrent où la découpe continue. */
  fadedV?: number[];
  fadedH?: number[];
  /** Bandes horizontales teintées (titre, corps, appel à l'action). */
  bands?: [number, number][];
  /** Points forts aux croisements. */
  dots?: [number, number][];
  /** Cadre intérieur teinté. */
  inner?: [number, number, number, number];
  /** Cadre rouge de zone perdue. */
  danger?: boolean;
  /** Parcours de lecture en Z ou en F. */
  path?: string;
};

export type Layout = { vertical: number[]; horizontal: number[]; hints?: Hints };

export type Template = {
  id: string;
  name: string;
  note: string;
  group: string;
  /** Vrai si le modèle change quand on choisit un coin (aujourd'hui : la spirale d'or seule). */
  turnable?: boolean;
  build(corner: Corner): Layout;
};

/** Coin où se referme la spirale. `flipX`/`flipY` disent comment retourner la découpe. */
export type Corner = "bottomLeft" | "bottomRight" | "topRight" | "topLeft";

export const CORNERS: { id: Corner; label: string }[] = [
  { id: "bottomLeft", label: "Bas gauche" },
  { id: "bottomRight", label: "Bas droite" },
  { id: "topRight", label: "Haut droite" },
  { id: "topLeft", label: "Haut gauche" },
];

const PHI = (1 + Math.sqrt(5)) / 2;

/** Nombre de coupes retenues comme repères. Au-delà elles se tassent sur quelques pixels. */
const KEPT_CUTS = 6;

type Cut = { axis: "v" | "h"; at: number };

/**
 * Découpe du rectangle d'or : on retire un carré à gauche, en haut, à droite, en bas, en tournant.
 *
 * Elle est calculée une fois pour toutes sur le rectangle φ : 1, puis étirée au format du document —
 * c'est aussi ce que fait Photoshop. Sans cet étirement, un plan carré ne se découperait jamais
 * (retirer un carré d'un carré ne laisse rien) et la spirale disparaîtrait.
 *
 * L'arc de chaque carré part du coin où finissait le précédent et tourne autour du coin qui touche
 * le carré suivant : c'est ce qui raccorde les tangentes et donne une spirale continue, sans pointe.
 */
function goldenCuts(): { cuts: Cut[]; arcs: Arc[]; eye: [number, number] } {
  let [x0, y0, x1, y1] = [0, 0, PHI, 1];
  const cuts: Cut[] = [];
  const raw: { sx: number; sy: number; ex: number; ey: number; rad: number }[] = [];
  for (let step = 0; step < 9; step++) {
    const side = Math.min(x1 - x0, y1 - y0);
    if (side < 0.012) break;
    if (step % 4 === 0) {
      raw.push({ sx: x0, sy: y1, ex: x0 + side, ey: y0, rad: side });
      x0 += side;
      cuts.push({ axis: "v", at: x0 });
    } else if (step % 4 === 1) {
      raw.push({ sx: x0, sy: y0, ex: x1, ey: y0 + side, rad: side });
      y0 += side;
      cuts.push({ axis: "h", at: y0 });
    } else if (step % 4 === 2) {
      raw.push({ sx: x1, sy: y0, ex: x1 - side, ey: y1, rad: side });
      x1 -= side;
      cuts.push({ axis: "v", at: x1 });
    } else {
      raw.push({ sx: x1, sy: y1, ex: x0, ey: y1 - side, rad: side });
      y1 -= side;
      cuts.push({ axis: "h", at: y1 });
    }
  }
  const nx = (x: number): number => x / PHI;
  return {
    cuts: cuts.map((cut) => ({ axis: cut.axis, at: cut.axis === "v" ? nx(cut.at) : cut.at })),
    arcs: raw.map((a) => ({ sx: nx(a.sx), sy: a.sy, ex: nx(a.ex), ey: a.ey, rx: nx(a.rad), ry: a.rad, sweep: 1 as const })),
    eye: [nx((x0 + x1) / 2), (y0 + y1) / 2],
  };
}

const GOLDEN = goldenCuts();

/** Retourne la découpe pour amener l'œil de la spirale dans le coin demandé. */
function turn(corner: Corner): Layout {
  const flipX = corner === "bottomRight" || corner === "topRight";
  const flipY = corner === "topRight" || corner === "topLeft";
  const fx = (x: number): number => (flipX ? 1 - x : x);
  const fy = (y: number): number => (flipY ? 1 - y : y);
  // Un seul miroir inverse le sens de rotation de la spirale : l'arc doit alors tourner dans l'autre sens.
  const sweep: 0 | 1 = flipX === flipY ? 1 : 0;

  const kept = GOLDEN.cuts.slice(0, KEPT_CUTS);
  const rest = GOLDEN.cuts.slice(KEPT_CUTS);
  const along = (list: Cut[], axis: "v" | "h"): number[] =>
    list.filter((cut) => cut.axis === axis).map((cut) => (axis === "v" ? fx(cut.at) : fy(cut.at)));

  return {
    vertical: along(kept, "v"),
    horizontal: along(kept, "h"),
    hints: {
      arcs: GOLDEN.arcs.map((a) => ({ ...a, sx: fx(a.sx), sy: fy(a.sy), ex: fx(a.ex), ey: fy(a.ey), sweep })),
      eye: [fx(GOLDEN.eye[0]), fy(GOLDEN.eye[1])],
      fadedV: along(rest, "v"),
      fadedH: along(rest, "h"),
    },
  };
}

/** Repères d'une grille de `n` colonnes : les traits intérieurs seulement, les bords n'en sont pas. */
const split = (n: number): number[] => Array.from({ length: n - 1 }, (_, i) => (i + 1) / n);

export const TEMPLATES: Template[] = [
  {
    id: "thirds",
    name: "Règle des tiers",
    note: "4 repères",
    group: "Composition",
    build: () => ({
      vertical: [1 / 3, 2 / 3],
      horizontal: [1 / 3, 2 / 3],
      hints: {
        dots: [
          [1 / 3, 1 / 3],
          [2 / 3, 1 / 3],
          [1 / 3, 2 / 3],
          [2 / 3, 2 / 3],
        ],
      },
    }),
  },
  {
    id: "goldenGrid",
    name: "Nombre d'or — grille",
    note: "4 repères · 0,382 et 0,618",
    group: "Composition",
    build: () => ({ vertical: [0.382, 0.618], horizontal: [0.382, 0.618] }),
  },
  {
    id: "goldenSpiral",
    name: "Nombre d'or — spirale",
    note: "6 repères · pivotable",
    group: "Composition",
    turnable: true,
    build: turn,
  },
  {
    id: "frameInFrame",
    name: "Cadre dans le cadre",
    note: "4 repères · marge 12 %",
    group: "Composition",
    build: () => ({ vertical: [0.12, 0.88], horizontal: [0.12, 0.88], hints: { inner: [0.12, 0.12, 0.76, 0.76] } }),
  },
  {
    id: "safeArea",
    name: "Marges de sécurité",
    note: "4 repères · 5 %",
    group: "Composition",
    build: () => ({ vertical: [0.05, 0.95], horizontal: [0.05, 0.95], hints: { danger: true } }),
  },

  {
    id: "symmetryV",
    name: "Symétrie verticale",
    note: "1 repère",
    group: "Symétrie et centrage",
    build: () => ({ vertical: [0.5], horizontal: [] }),
  },
  {
    id: "symmetryH",
    name: "Symétrie horizontale",
    note: "1 repère",
    group: "Symétrie et centrage",
    build: () => ({ vertical: [], horizontal: [0.5] }),
  },
  {
    id: "centerAxes",
    name: "Centre et axes",
    note: "4 repères",
    group: "Symétrie et centrage",
    build: () => ({ vertical: [0.25, 0.5, 0.75], horizontal: [0.5], hints: { eye: [0.5, 0.5] } }),
  },
  {
    id: "quarters",
    name: "Quarts",
    note: "6 repères",
    group: "Symétrie et centrage",
    build: () => ({ vertical: [0.25, 0.5, 0.75], horizontal: [0.25, 0.5, 0.75] }),
  },

  {
    id: "titleAndCall",
    name: "Titre et appel",
    note: "3 repères",
    group: "Hiérarchie",
    build: () => ({
      vertical: [0.08],
      horizontal: [0.22, 0.78],
      hints: {
        bands: [
          [0, 0.22],
          [0.78, 1],
        ],
      },
    }),
  },
  {
    id: "readingZ",
    name: "Lecture en Z",
    note: "4 repères",
    group: "Hiérarchie",
    build: () => ({
      vertical: [0.125, 0.875],
      horizontal: [0.25, 0.75],
      hints: { path: "M.125 .25L.875 .25L.125 .75L.875 .75" },
    }),
  },
  {
    id: "readingF",
    name: "Lecture en F",
    note: "4 repères",
    group: "Hiérarchie",
    build: () => ({
      vertical: [0.09, 0.68],
      horizontal: [0.22, 0.48],
      hints: { path: "M.09 .12L.92 .12M.09 .35L.68 .35M.09 .35L.09 .9" },
    }),
  },
  {
    id: "triptych",
    name: "Triptyque",
    note: "2 repères · 20 / 60 / 20",
    group: "Hiérarchie",
    build: () => ({ vertical: [], horizontal: [0.2, 0.8], hints: { bands: [[0.2, 0.8]] } }),
  },
  {
    id: "poster",
    name: "Affiche",
    note: "4 repères · 30 / 50 / 20",
    group: "Hiérarchie",
    build: () => ({ vertical: [0.08, 0.92], horizontal: [0.3, 0.8] }),
  },
  {
    id: "offCenter",
    name: "Sujet décentré",
    note: "2 repères",
    group: "Hiérarchie",
    build: () => ({ vertical: [1 / 3], horizontal: [1 / 3], hints: { eye: [1 / 6, 0.62] } }),
  },

  { id: "cols12", name: "Grille 12 colonnes", note: "11 repères", group: "Grilles", build: () => ({ vertical: split(12), horizontal: [] }) },
  { id: "grid12x8", name: "Grille 12 × 8", note: "18 repères", group: "Grilles", build: () => ({ vertical: split(12), horizontal: split(8) }) },
];

export const GROUPS = [...new Set(TEMPLATES.map((template) => template.group))];

export const templateById = (id: string): Template | undefined => TEMPLATES.find((template) => template.id === id);
