// Seul point de contact avec le processus Rust. Le reste de l'interface ignore Tauri.
import { invoke } from "@tauri-apps/api/core";


export type BlendMode =
  | "pass_through"
  | "normal"
  | "dissolve"
  | "darken"
  | "multiply"
  | "color_burn"
  | "linear_burn"
  | "lighten"
  | "screen"
  | "color_dodge"
  | "linear_dodge"
  | "overlay"
  | "soft_light"
  | "hard_light"
  | "difference"
  | "exclusion"
  | "subtract"
  | "divide"
  | "hue"
  | "saturation"
  | "color"
  | "luminosity";

export interface Locks {
  transparency: boolean;
  pixels: boolean;
  position: boolean;
  all: boolean;
}

/** Contenu éditable d'un calque de texte. */
export interface TextData {
  content: string;
  /** Famille de police ; vide = police par défaut du système. */
  fontFamily: string;
  /** Corps en pixels. */
  size: number;
  /** Interlignage, en multiple du corps. */
  lineHeight: number;
  /** Approche, en millièmes de cadratin. */
  tracking: number;
  align: "left" | "center" | "right";
  color: Rgb;
  /** Graisse : 100 (Thin) à 900 (Black), 400 = Regular, 700 = Bold. */
  weight: number;
  italic: boolean;
  /** Portions dont le style diffère de celui du calque ; en cas de recouvrement, la dernière l'emporte. */
  runs?: StyleRun[];
}

/** Style propre à une portion du texte ; `start..end` en unités UTF-16. Un champ absent reprend le réglage du calque. */
export interface StyleRun {
  start: number;
  end: number;
  fontFamily?: string;
  weight?: number;
  italic?: boolean;
  color?: Rgb;
}

/** Une famille de polices installée et ses variantes réellement disponibles. */
export interface FontFamily {
  family: string;
  styles: { weight: number; italic: boolean }[];
  /** Police livrée avec Lumitype : identique sur toute machine (voir src-tauri/fonts). */
  bundled: boolean;
}

/** Couleur d'étiquette d'un calque ou dossier (panneau des calques). */
export type LayerLabel = "red" | "orange" | "yellow" | "green" | "blue" | "violet" | "gray";

/** Teinte de chaque étiquette, dans l'ordre du menu. */
export const LAYER_LABELS: { label: LayerLabel; color: string; name: string }[] = [
  { label: "red", color: "#ff6b6b", name: "Rouge" },
  { label: "orange", color: "#ffb84d", name: "Orange" },
  { label: "yellow", color: "#ffe14d", name: "Jaune" },
  { label: "green", color: "#5ed27a", name: "Vert" },
  { label: "blue", color: "#4d8dff", name: "Bleu" },
  { label: "violet", color: "#b07cff", name: "Violet" },
  { label: "gray", color: "#9a9aa3", name: "Gris" },
];

export interface LayerInfo {
  id: number;
  parent: number | null;
  /** Position parmi ses frères, du bas (0) vers le haut. */
  index: number;
  depth: number;
  /** Position du calque dans le document. */
  offset: [number, number];
  name: string;
  isGroup: boolean;
  expanded: boolean;
  childCount: number;
  visible: boolean;
  opacity: number;
  fill: number;
  blend: BlendMode;
  locks: Locks;
  clipped: boolean;
  hasMask: boolean;
  maskEnabled: boolean;
  /** Présent pour un calque de texte. */
  text: TextData | null;
  /** Présent pour un calque de réglage. */
  adjustment: Adjustment | null;
  /** Présent pour un calque de forme. */
  shape: ShapeData | null;
  /** Effets de calque (fx). */
  effects: LayerEffect[];
  /** Couleur d'étiquette dans le panneau (`null` : aucune ; un dossier teinte ses enfants sans étiquette). */
  label: LayerLabel | null;
  /** Présent pour un objet dynamique. */
  smart: SmartInfo | null;
}

/** Objet dynamique : son contenu (incorporé ou lié) et sa transformation paramétrique. */
export interface SmartInfo {
  /** Identifiant du contenu : les instances (Ctrl+J) partagent le même. */
  contentId: number;
  /** Lié à un fichier externe (sinon incorporé au projet). */
  linked: boolean;
  /** Fichier lié, chemin absolu. */
  path: string | null;
  /** Fichier lié introuvable à la dernière vérification : le dernier rendu reste affiché. */
  missing: boolean;
  /** Nom du contenu. */
  name: string;
  /** Dimensions du contenu en pixels. */
  width: number;
  height: number;
  /** Calques du document qui partagent ce contenu (lui compris). */
  instances: number;
  /** Image d'origine d'un contenu placé, tant qu'il n'a pas été modifié : nom du fichier. */
  originalName: string | null;
  /** Nom de fichier proposé par « Exporter le contenu ». */
  exportName: string;
  /** Échelles (1 = 100 %, négative : retourné) et rotation en degrés (sens horaire à l'écran). */
  scaleX: number;
  scaleY: number;
  rotationDeg: number;
  /** Coin haut gauche du contenu dans le document. */
  x: number;
  y: number;
  /** Coins du contenu dans le document : haut gauche, haut droit, bas droit, bas gauche. */
  corners: [[number, number], [number, number], [number, number], [number, number]];
  /** Transformation complète (repère contenu -> document), 3x3 en ligne. */
  transform: number[];
}

/** Document ouvert par « Modifier le contenu » : Ctrl+S appelle `api.smartContentCommit` (et rien d'autre). */
export interface SmartParentInfo {
  /** Document qui contient l'objet dynamique. */
  docId: number;
  contentId: number;
  /** Nom du calque objet dynamique. */
  name: string;
}

/** Bilan de `api.smartLinksCheck`. */
export interface SmartLinksResult extends EditResult {
  /** Contenus relus (fichier modifié ou réapparu). */
  updated: number[];
  /** Contenus dont le fichier est introuvable. */
  missing: number[];
  /** Fichiers illisibles, avec la raison. */
  failed: string[];
}

/** Effet de calque. `opacity` de 0 à 1, longueurs en pixels, `angle` : direction d'où vient la lumière. */
/** Effet de calque. `blend` : son mode de fusion (Normal s'il manque, pour les fichiers d'avant). */
export type LayerEffect = { blend?: BlendMode } & (
  | { type: "dropShadow" | "innerShadow"; enabled: boolean; color: Rgb; opacity: number; angle: number; distance: number; size: number }
  | { type: "outerGlow" | "innerGlow"; enabled: boolean; color: Rgb; opacity: number; size: number }
  | { type: "stroke"; enabled: boolean; color: Rgb; opacity: number; width: number; position: "outside" | "inside" | "center" }
  | { type: "colorOverlay"; enabled: boolean; color: Rgb; opacity: number }
  | { type: "gradientOverlay"; enabled: boolean; from: Rgb; to: Rgb; opacity: number; angle: number; radial: boolean }
);

/** Forme vectorielle réglable ; sa boîte a son coin haut gauche à la position du calque. */
/** Un modèle livré : `src-tauri/templates/index.json`. */
export interface TemplateInfo {
  id: string;
  file: string;
  name: string;
  description: string;
}

export interface ShapeData {
  /** `polygon` : polygone régulier à `sides` côtés, pointe en haut (3 = triangle). */
  kind: "rect" | "ellipse" | "polygon";
  /** Nombre de côtés d'un polygone (3 à 64). */
  sides?: number;
  width: number;
  height: number;
  /** Rayon des coins d'un rectangle, en pixels. */
  radius: number;
  /** Couleurs RGBA à alpha droit ; `null` = absent. */
  fill: number[] | null;
  stroke: number[] | null;
  /** Épaisseur du contour, tracé vers l'intérieur. */
  strokeWidth: number;
  /**
   * Rotation, inclinaison ou perspective (matrice 3 × 3 en ligne), posée par Ctrl+T : la forme reste
   * un vecteur, redessinée à chaque changement. Absente ou identité pour une forme droite. À renvoyer
   * telle quelle avec `setShape`, sinon la forme se redresserait.
   */
  transform?: number[];
}

/** Réglage non destructif appliqué à tout ce qui se trouve sous le calque. */
export type Adjustment =
  | { type: "brightnessContrast"; brightness: number; contrast: number }
  // `rangeWidth` > 0 : seules les teintes à moins de `rangeWidth` degrés de `rangeCenter` sont modifiées.
  | {
      type: "hueSaturation";
      hue: number;
      saturation: number;
      lightness: number;
      /** Plage visée, en degrés : zone corrigée en entier de `rangeStart` à `rangeEnd`, transitions de part et d'autre. */
      rangeStart?: number;
      rangeEnd?: number;
      featherIn?: number;
      featherOut?: number;
      /** Ancienne plage symétrique, lue à l'ouverture d'un fichier d'avant les quatre poignées. */
      rangeCenter?: number;
      rangeWidth?: number;
    }
  | { type: "levels"; black: number; gamma: number; white: number }
  | { type: "invert" }
  | { type: "negative" }
  | {
      type: "gradientMap";
      /** Arrêts (position 0..1, couleur RGBA à alpha droit), le noir prenant celui de gauche. */
      stops: { position: number; color: [number, number, number, number] }[];
      /** Dégradé parcouru à l'envers. */
      reverse: boolean;
      /** Léger bruit qui casse les paliers. */
      dither: boolean;
      /** Transitions adoucies entre arrêts, 0 à 100 %. */
      smoothness: number;
    }
  | ({ type: "develop" } & DevelopValues);

/** Curseurs du réglage « Développement » : -100 à 100, sauf `exposure` (-5 à +5 IL). */
export interface DevelopValues {
  temperature: number;
  tint: number;
  exposure: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  clarity: number;
  dehaze: number;
  vibrance: number;
  saturation: number;
  redHue: number;
  greenHue: number;
  blueHue: number;
}

/** Filtre destructif appliqué aux pixels d'un calque. */
/** Pastille du flou de profondeur de champ : position relative au contenu du calque (0 à 1), flou en pixels. */
export interface BlurPin {
  x: number;
  y: number;
  blur: number;
}

export type Filter =
  // Grille de déformation : décalages vers l'image d'origine, en fraction de la taille du contenu.
  | { type: "liquify"; cols: number; rows: number; dx: number[]; dy: number[] }
  | { type: "fieldBlur"; pins: BlurPin[] }
  | { type: "gaussianBlur"; radius: number }
  | { type: "sharpen"; radius: number; amount: number }
  | { type: "motionBlur"; angle: number; distance: number }
  | { type: "lensBlur"; radius: number; highlights: number }
  | { type: "noise"; amount: number; monochrome: boolean }
  | { type: "dither"; scale: number; levels: number; monochrome: boolean }
  // Seule la luminosité est tramée (Bayer 16 x 16) : les points allumés gardent la teinte de l'image.
  | { type: "colorDither"; scale: number; contrast: number; threshold: number; strength: number }
  // Rouge et bleu décalés en sens opposés ; `radial` : depuis le centre, sinon selon `angle`.
  | { type: "chromaticAberration"; amount: number; angle: number; radial: boolean }
  // Flou granuleux des affiches : flou, dissolution aléatoire des transitions, puis grain gaussien.
  // Texture, clarté, correction du voile : -100 à 100, avec un vrai calcul de voisinage.
  // Verre : l'image vue à travers un relief. `texture` "custom" = l'image chargée par `loadGlassTexture`.
  | { type: "glass"; distortion: number; smoothness: number; texture: "frosted" | "blocks" | "canvas" | "tinyLens" | "custom"; scale: number; invert: boolean }
  | { type: "presence"; texture: number; clarity: number; dehaze: number }
  | { type: "gaussianNoise"; radius: number; scatter: number; amount: number; grain: number; monochrome: boolean }
  | { type: "ascii"; cell: number; color: boolean };

export interface Guide {
  orientation: "horizontal" | "vertical";
  /** Pixels du document, le long de l'axe perpendiculaire au repère. */
  position: number;
}

export interface SystemInfo {
  cores: number;
  /** Mémoire vive totale, en Mio. */
  ramMb: number;
  /** Mémoire vidéo de la carte graphique, en Mio ; `null` : non détectée (l'interface suppose alors 2 Gio). */
  vramMb: number | null;
  /** Carte intégrée : sa mémoire vidéo est prise sur la mémoire vive (le plafond est alors borné au quart de celle-ci). */
  vramShared: boolean;
}

/** Répartition de la mémoire occupée par Lumitype (jauge des préférences), en Mio. `capMb` : plafond en vigueur. */
export interface MemoryStatus {
  capMb: number;
  ramMb: number;
  documentsMb: number;
  historyMb: number;
  cacheMb: number;
  /** Octets écrits sur le disque de travail (états d'historique anciens), hors mémoire vive. */
  scratchBytes: number;
  /** Dossier où le disque de travail écrit (celui réglé, ou celui par défaut). */
  scratchDir: string;
}

export type ExportFormat = "png" | "jpeg" | "webp" | "tiff";

/**
 * Métadonnées inscrites dans une image exportée (EXIF, XMP, textes PNG — voir `crates/engine/src/metadata.rs`).
 * Un champ vide n'est pas écrit. `created` au format EXIF (`AAAA:MM:JJ HH:MM:SS`), `createdIso` en ISO 8601.
 */
export interface ImageMetadata {
  author: string;
  copyright: string;
  description: string;
  keywords: string[];
  contact: string;
  software: string;
  created: string;
  createdIso: string;
}

/**
 * `quality` : JPEG et WebP avec perte (1 à 100) ; `lossless` : WebP sans perte (la qualité est alors ignorée) ;
 * `scale` : 1 = taille native ; `metadata` : absent = rien d'inscrit.
 */
export interface ExportOptions {
  format: ExportFormat;
  quality: number;
  lossless?: boolean;
  scale: number;
  metadata?: ImageMetadata | null;
}

/** Point d'ancrage d'un tracé : le point et ses deux poignées, en coordonnées absolues du document. */
export interface Anchor {
  point: [number, number];
  handleIn: [number, number];
  handleOut: [number, number];
}

export interface SubPath {
  anchors: Anchor[];
  closed: boolean;
}

/** `[x0, y0, x1, y1]` en pixels du document, bornes hautes exclues. */
export type Bounds = [number, number, number, number];

export interface DocumentInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  dpi: number;
  tileSize: number;
  maxLevel: number;
  projectPath: string | null;
  revision: number;
  hasSelection: boolean;
  guides: Guide[];
  guidesLocked: boolean;
  /** Tracé de travail de la plume. */
  paths: SubPath[];
  /** Du calque le plus haut au plus bas. */
  layers: LayerInfo[];
  history: string[];
  historyCursor: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Présent si ce document est le contenu d'un objet dynamique (« Modifier le contenu »). */
  smartParent: SmartParentInfo | null;
}

export interface DirtyInfo {
  all: boolean;
  /** Tuiles de niveau 0. */
  tiles: [number, number][];
  layers: number[];
  selection: boolean;
  /** Calques dont les pixels (ou le masque) ont changé, pas seulement la position. */
  repainted?: number[];
  /** Zones repeintes, dans le repère de chaque calque (`[x, y, largeur, hauteur]`), sans limite au document. */
  areas?: { layer: number; mask: boolean; rect: [number, number, number, number] }[];
}

/** Maj = ajouter, Alt = soustraire, Maj+Alt = intersection. */
export type CombineMode = "replace" | "add" | "subtract" | "intersect";

/** Forme de sélection, en pixels du document. */
export type SelectionShape =
  | { kind: "rect" | "ellipse"; x: number; y: number; width: number; height: number }
  | { kind: "polygon"; points: [number, number][] };

export interface EditResult {
  document: DocumentInfo;
  dirty: DirtyInfo;
}

export type PropsPatch = Partial<Pick<LayerInfo, "name" | "visible" | "opacity" | "fill" | "blend" | "locks" | "clipped">>;

export type EditOp =
  | { op: "addLayer" | "addGroup"; above: number | null }
  | { op: "importLayer"; path: string; above: number | null }
  | { op: "delete" | "group"; ids: number[] }
  | { op: "move"; ids: number[]; parent: number | null; index: number }
  | { op: "duplicate" | "removeMask"; id: number }
  // `gesture` : aperçu en direct (survol des modes de fusion) ; `cancelGesture` le retire si rien n'est choisi.
  | { op: "setProps"; ids: number[]; patch: PropsPatch; gesture?: string }
  | { op: "setExpanded"; id: number; expanded: boolean }
  | { op: "setLabel"; ids: number[]; label: LayerLabel | null }
  | { op: "addMask"; id: number; revealAll: boolean }
  | { op: "setMaskEnabled"; id: number; enabled: boolean }
  | { op: "fill"; id: number; color: Rgb; onMask: boolean }
  | { op: "addText"; x: number; y: number; text: TextData; above: number | null }
  // `session` : les modifications d'une même séance d'édition ne font qu'un état d'historique.
  | { op: "setText"; id: number; text: TextData; session: string }
  // Pixellise un texte, une forme ou un objet dynamique (rendu actuel).
  | { op: "rasterize"; id: number }
  // Objets dynamiques. Placer : `parent` + `index` s'ils sont donnés, sinon au-dessus de `above` (ou au sommet).
  | { op: "convertToSmart"; ids: number[] }
  | { op: "placeSmartObject"; path: string; linked: boolean; above?: number | null; parent?: number | null; index?: number | null }
  // Remplacer (toutes les instances, taille affichée gardée) ; relier : le contenu devient lié à `path`.
  | { op: "replaceSmartContent" | "relinkSmart"; id: number; path: string }
  | { op: "embedSmartLink" | "resetSmartTransform" | "newSmartByCopy"; id: number }
  // `onMask` : Suppr sur le masque de fusion visé — la sélection y passe au noir (masquée).
  | { op: "clearPixels"; id: number; onMask?: boolean }
  | { op: "copy" | "cut"; id: number }
  | { op: "paste"; above: number | null }
  | { op: "bucketFill"; id: number; x: number; y: number; color: Rgb; tolerance: number; contiguous: boolean }
  // Arrêts en RGBA à alpha droit, positions de 0 à 1 ; points en pixels du document.
  // `onMask` : le dégradé s'inscrit dans le masque de fusion (sa clarté devient la valeur du masque).
  | { op: "gradientFill"; id: number; from: [number, number]; to: [number, number]; stops: { position: number; color: number[] }[]; radial: boolean; onMask?: boolean }
  // Masque de fusion copié (ou déplacé) d'un calque à un autre.
  | { op: "copyMask"; from: number; to: number; removeSource: boolean }
  // `clipped` : écrêté sur le calque `above`, pour ne corriger que lui.
  | { op: "addAdjustment"; adjustment: Adjustment; above: number | null; clipped: boolean }
  | { op: "invert"; id: number }
  | { op: "invertMask" | "mergeGroup"; id: number }
  | { op: "mergeLayers"; ids: number[] }
  // Alt + glisser dans le panneau Calques : copie déposée à l'emplacement visé.
  | { op: "duplicateTo"; ids: number[]; parent: number | null; index: number }
  // Alt + clic sur l'œil : isole le calque, ou rallume tout s'il l'était déjà.
  | { op: "solo"; id: number }
  // Correcteur localisé : trait en pixels du document ; la zone couverte est reconstruite.
  | { op: "heal"; id: number; points: [number, number][]; size: number; hardness: number }
  | { op: "setEffects"; id: number; effects: LayerEffect[]; gesture?: string }
  | { op: "addShape"; x: number; y: number; shape: ShapeData; above: number | null }
  | { op: "setShape"; id: number; shape: ShapeData }
  // Cadre de transformation tiré sur une forme sans rotation : redessinée à la nouvelle taille, coins intacts.
  | { op: "placeShape"; id: number; x: number; y: number; width: number; height: number; gesture: string }
  // Rectangle en pixels du document actuel ; il peut déborder pour agrandir la zone de travail.
  | { op: "crop"; x: number; y: number; width: number; height: number }
  | { op: "setAdjustment"; id: number; adjustment: Adjustment }
  // `gesture` : aperçu en direct depuis la boîte de dialogue ; `cancelGesture` retire tout si l'on annule.
  | { op: "applyFilter"; id: number; filter: Filter; gesture?: string }
  | { op: "select"; shape: SelectionShape; mode: CombineMode }
  // `sample` : côté du carré moyenné (1, 3 ou 5) ; `gesture` : tolérance réglée en glissant (un seul état d'historique).
  | { op: "magicWand"; layer: number; x: number; y: number; tolerance: number; contiguous: boolean; antiAlias: boolean; sample: number; allLayers: boolean; gesture?: string; mode: CombineMode }
  // Sélection rapide : la sélection s'étend à partir du trait peint (pixels du document).
  | { op: "quickSelect"; layer: number; points: [number, number][]; size: number; tolerance: number; allLayers: boolean; mode: CombineMode }
  // Dilater (> 0) ou contracter (< 0) la sélection de tant de pixels, puis arrondir son contour.
  | { op: "modifySelection"; amount: number; smooth: number }
  // Gestes continus : `dx`, `dy` = déplacement TOTAL depuis le début du geste `gesture`.
  // `parts` : calque à masque — « pixels » ou « mask » ne déplace que cette partie ; « both » : les deux.
  | { op: "translate"; ids: number[]; dx: number; dy: number; gesture: string; parts?: "pixels" | "mask" | "both" }
  | { op: "movePixels"; id: number; dx: number; dy: number; gesture: string }
  | { op: "translateSelection"; dx: number; dy: number; gesture: string }
  // Ctrl+T : `matrix` = transformation TOTALE (3x3 en ligne, repère document) depuis le début du geste.
  | { op: "transform"; ids: number[]; matrix: number[]; finalize: boolean; gesture: string }
  | { op: "cancelGesture"; gesture: string }
  | { op: "setPath"; paths: SubPath[]; label: string }
  | { op: "pathToSelection"; mode: CombineMode }
  | { op: "fillPath"; id: number; color: Rgb }
  | { op: "strokePath"; id: number; brush: BrushParams }
  // Disposition de repères posée d'un coup ; `clear` retire d'abord les repères existants.
  | { op: "setGuides"; guides: Guide[]; clear: boolean }
  | { op: "setGuidesLocked"; locked: boolean }
  | { op: "addGuide"; orientation: Guide["orientation"]; position: number }
  | { op: "moveGuide"; index: number; position: number }
  | { op: "removeGuide"; index: number }
  | { op: "clearGuides" }
  // Avec `layer` : le contenu opaque de ce calque ; sans : tout le document.
  | { op: "selectAll"; layer?: number }
  | { op: "deselect" | "invertSelection" }
  | { op: "feather"; radius: number }
  | { op: "undo" | "redo" }
  | { op: "jump"; index: number };

export type Rgb = [number, number, number];

export interface BrushParams {
  size: number;
  hardness: number;
  opacity: number;
  color: Rgb;
  erase: boolean;
  pressureSize: boolean;
  /** La pression du stylet module l'opacité. */
  pressureOpacity?: boolean;
  /** Forme du pinceau ; absente = rond. */
  kind?: BrushKind;
  /** Dynamique de forme ; absente = comportement par défaut de la forme. */
  dynamics?: BrushDynamics;
  /** Trait esquissé à l'écran par la carte graphique : le moteur ne recompose le calque qu'à la fin du trait. */
  deferred?: boolean;
}

/** Comment les empreintes varient le long du trait ; tout à zéro = comportement naturel de la forme. */
export interface BrushDynamics {
  /** Espacement en fraction du diamètre ; 0 = celui de la forme. */
  spacing: number;
  /** Angle de la pointe, en degrés. */
  angle: number;
  followDirection: boolean;
  /** Dispersion autour du trait, en fraction du diamètre (0 à 3). */
  scatter: number;
  sizeJitter: number;
  angleJitter: number;
  /** Spray : densité des gouttelettes (1 = normale). Mélangeur : densité de la peinture (0 à 1). 0 = valeur par défaut. */
  amount: number;
  /** Flux (0 à 1 ; 0 = 100 %) : en dessous de 100 %, les empreintes qui se recouvrent s'additionnent. */
  flow: number;
  /** Mélangeur : quantité de peinture apportée par le pinceau (0 = il ne fait qu'étaler). */
  paint: number;
  /** Mélangeur : étirement des couleurs ramassées (0 à 1). */
  stretch: number;
}

export const NEUTRAL_DYNAMICS: BrushDynamics = { spacing: 0, angle: 0, followDirection: false, scatter: 0, sizeJitter: 0, angleJitter: 0, amount: 0, flow: 0, paint: 0, stretch: 0 };
/** Réglages de départ du mélangeur : un peu de peinture, des couleurs traînées assez loin. */
export const MIXER_DYNAMICS: BrushDynamics = { ...NEUTRAL_DYNAMICS, amount: 0.7, paint: 0.3, stretch: 0.7 };

/** Rond (net ou flou selon la dureté), spray, mélangeur, ou pointe importée désignée par son identifiant. */
export type BrushKind = { type: "round" | "spray" | "mixer" } | { type: "tip"; id: number };

export interface BrushTipInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  /** Espacement propre à la pointe, en fraction du diamètre. */
  spacing: number;
  /** Fournie avec le logiciel (craie, fusain…), par opposition aux pointes importées. */
  builtin: boolean;
}

export interface NewDocumentParams {
  name: string;
  width: number;
  height: number;
  dpi: number;
  transparent: boolean;
}

export const PROJECT_EXTENSION = "graf";
/** Ancienne extension des projets, toujours acceptée à l'ouverture. */
export const LEGACY_PROJECT_EXTENSION = "iep";
/** Formats d'image ouverts ou importés : le moteur décode tout sauf l'AVIF, confié à la webview. */
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "tif", "tiff", "avif", "heic", "heif", "dng"];
/** Types MIME des mêmes formats, pour une image collée ou glissée sans extension. */
export const IMAGE_MIME_TYPES = ["image/png", "image/jpeg", "image/webp", "image/tiff", "image/avif", "image/heic", "image/heif", "image/heic-sequence", "image/x-adobe-dng", "image/dng"];

export const api = {
  newDocument: (params: NewDocumentParams) => invoke<DocumentInfo>("new_document", { ...params }),
  openPath: (path: string) => invoke<DocumentInfo>("open_path", { path }),
  /** 8 octets de révision, puis RGBA8 prémultiplié brut (rien si la tuile est transparente). */
  getTile: (docId: number, level: number, x: number, y: number) =>
    invoke<ArrayBuffer>("get_tile", { docId, level, x, y }),
  /** 8 octets de révision, 1 octet de valeur de fond, puis le masque brut (rien si la tuile est uniforme). */
  getSelectionTile: (docId: number, level: number, x: number, y: number) =>
    invoke<ArrayBuffer>("get_selection_tile", { docId, level, x, y }),
  /** Curseur et sélection d'un texte en cours de saisie, en repère document. Positions en unités UTF-16. */
  textCaret: (docId: number, layer: number, start: number, end: number) =>
    invoke<{ caret: [[number, number], [number, number]]; highlights: [number, number][][] }>("text_caret", { docId, layer, start, end }),
  /** Position du curseur (UTF-16) la plus proche d'un point du document. */
  textHit: (docId: number, layer: number, x: number, y: number) => invoke<number>("text_hit", { docId, layer, x, y }),
  /** Couleur de l'image composée en un point ; `null` hors document ou sur du transparent. */
  sampleColor: (docId: number, x: number, y: number) => invoke<Rgb | null>("sample_color", { docId, x, y }),
  /** Masque rapide : début d'un trait qui peint la sélection (suite par `strokePoints` et `strokeEnd`). */
  strokeBeginSelection: (docId: number, brush: BrushParams) => invoke<void>("stroke_begin_selection", { docId, brush }),
  /** PNG des derniers pixels copiés, pour le presse-papiers du système (vide si rien n'a été copié). */
  clipboardPng: () => invoke<ArrayBuffer>("clipboard_png"),
  /** Image glissée depuis un navigateur qui n'a fourni que son adresse : téléchargée par la coque. */
  importUrl: (url: string, docId: number | null, above: number | null) => invoke<EditResult>("import_url", { url, docId, above }),
  /** Aperçu d'un filtre sur une réduction du calque : 8 octets (largeur, hauteur) puis RGBA à alpha droit. */
  filterPreview: (docId: number, layer: number, filter: Filter, maxSide: number, inContext = false) =>
    invoke<ArrayBuffer>("filter_preview", { docId, layer, filter, maxSide, inContext }),
  /** Filtre Verre : charge l'image qui servira de relief ; retourne [nom, largeur, hauteur]. */
  loadGlassTexture: (path: string) => invoke<[string, number, number]>("load_glass_texture", { path }),
  /** Ctrl+C dans le panneau Calques : copie les calques (groupes et contenu compris). */
  copyLayers: (docId: number, ids: number[]) => invoke<number>("copy_layers", { docId, ids }),
  /** Ctrl+V : colle les calques copiés au-dessus de `above`, dans ce document ou un autre. */
  pasteLayers: (docId: number, above: number | null, at?: { parent: number | null; index: number }) =>
    invoke<EditResult>("paste_layers", { docId, above, parent: at?.parent ?? null, index: at?.index ?? null }),
  /** Fichiers passés au lancement (double-clic sur un projet dans l'explorateur). */
  startupFiles: () => invoke<string[]>("startup_files"),
  /** Pixels d'un calque (repère du calque) pour la composition sur carte graphique : 16 octets d'en-tête, puis RGBA prémultiplié. */
  /** `part` : absent = le calque avec ses effets fondus ; `raw` = ses pixels seuls ; `under:<n>` / `over:<n>` = un effet à part. */
  /** `limit` = `[côté maximal, nombre de pixels maximal]` : au-delà, le moteur ne renvoie que l'en-tête, sans lire les pixels. */
  layerPixels: (docId: number, layer: number, rect: [number, number, number, number] | null, part: string | null = null, limit: [number, number] | null = null) =>
    invoke<ArrayBuffer>("layer_pixels", { docId, layer, rect, part, limit }),
  /** Effets d'un calque fondus à part dans l'image : modes de ceux du dessous, puis du dessus. */
  layerEffects: (docId: number, layer: number) => invoke<{ under: BlendMode[]; over: BlendMode[] }>("layer_effects", { docId, layer }),
  /** Masque d'un calque (repère du calque) : 20 octets d'en-tête (x, y, largeur, hauteur, fond), puis un octet par pixel. `limit` : comme `layerPixels`. */
  layerMask: (docId: number, layer: number, rect: [number, number, number, number] | null, limit: [number, number] | null = null) =>
    invoke<ArrayBuffer>("layer_mask", { docId, layer, rect, limit }),
  /** Presse-papiers du système, lu nativement : empreinte du contenu (0 = vide), image en PNG, marque texte. */
  clipboardFingerprint: () => invoke<number>("clipboard_fingerprint"),
  clipboardImage: () => invoke<ArrayBuffer>("clipboard_image"),
  clipboardSetText: (text: string) => invoke<void>("clipboard_set_text", { text }),
  /** Sauvegarde automatique (historique compris) dans la copie de secours `key`. Avancement : événement « autosave-progress ». */
  autosave: (docId: number, key: string) => invoke<void>("autosave", { docId, key }),
  recoveryList: () => invoke<{ key: string; savedAt: number; size: number }[]>("recovery_list"),
  recoveryOpen: (key: string) => invoke<DocumentInfo>("recovery_open", { key }),
  recoveryDelete: (key: string) => invoke<void>("recovery_delete", { key }),
  /** Aperçu instantané d'un réglage : calques dessous et dessus, ou `null` (réglage non simple). */
  adjustPlan: (docId: number, layer: number) => invoke<{ below: number[]; target: number[]; above: number[] } | null>("adjust_plan", { docId, layer }),
  /** Table de correspondance 3D d'un réglage : `size`³ couleurs RGBA8. */
  adjustmentLut: (adjustment: Adjustment, size: number) => invoke<ArrayBuffer>("adjustment_lut", { adjustment, size }),
  /** Réglage mis à plat pour le shader de composition : type (u32), puis ses valeurs (f32). */
  adjustmentGpu: (adjustment: Adjustment) => invoke<ArrayBuffer>("adjustment_gpu", { adjustment }),
  /** Écran d'accueil : aperçu (PNG, 320 px) d'un fichier récent, sous sa clé. Réponse vide : pas d'aperçu. */
  savePreview: (docId: number, key: string) => invoke<void>("save_preview", { docId, key }),
  loadPreview: (key: string) => invoke<ArrayBuffer>("load_preview", { key }),
  /** Fichier d'une police incluse (vide pour une police du système) : échantillon du sélecteur. */
  bundledFontFile: (family: string) => invoke<ArrayBuffer>("bundled_font_file", { family }),
  /** Modèles livrés avec l'application (accueil › Modèles) : des projets de démonstration, ouverts comme copies. */
  listTemplates: () => invoke<TemplateInfo[]>("list_templates"),
  templatePreview: (id: string) => invoke<ArrayBuffer>("template_preview", { id }),
  openTemplate: (id: string) => invoke<DocumentInfo>("open_template", { id }),
  deletePreview: (key: string) => invoke<void>("delete_preview", { key }),
  fileExists: (path: string) => invoke<boolean>("file_exists", { path }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  /** Contenu brut d'un fichier (images AVIF, décodées par la webview). */
  readFileBytes: (path: string) => invoke<ArrayBuffer>("read_file_bytes", { path }),
  /** Le dossier existe-t-il encore ? */
  directoryExists: (path: string) => invoke<boolean>("directory_exists", { path }),
  /** Cartes graphiques lues dans le système (Linux ; vide ailleurs), la carte de démarrage en premier. */
  systemGpus: () => invoke<string[]>("system_gpus"),
  /** Cœurs, mémoire vive, et part que l'allocation automatique accorde au cache. */
  systemInfo: () => invoke<SystemInfo>("system_info"),
  /** Réglages de performance, appliqués à chaud. `memoryPercent` : part de la mémoire vive (10 à 90) ; `cores` nul = tous. */
  /**
   * Disque de travail de l'historique : `scratchEnabled`, `scratchDir` (nul = dossier de cache de l'application) et
   * `scratchQuotaMb` (place maximale, en Mio).
   */
  setPerformance: (settings: { memoryPercent: number; cores: number | null; historyStates: number; scratchEnabled: boolean; scratchDir: string | null; scratchQuotaMb: number }) =>
    invoke<void>("set_performance", { settings }),
  /** Mémoire occupée à cet instant (documents, historique, cache), pour la jauge des préférences. */
  memoryStatus: () => invoke<MemoryStatus>("memory_status"),
  /** Préférences de l'utilisateur : un fichier JSON par sujet (`name` : lettres minuscules et tirets). */
  loadSettings: (name: string) => invoke<string | null>("load_settings", { name }),
  saveSettings: (name: string, content: string) => invoke<void>("save_settings", { name, content }),
  /** Importe un fichier de pinceaux (.abr, .gbr, image) ; retourne les pointes ajoutées. */
  importBrushes: (path: string) => invoke<BrushTipInfo[]>("import_brushes", { path }),
  /** Bibliothèque de pointes (recharge au premier appel les fichiers importés auparavant). */
  listBrushes: () => invoke<BrushTipInfo[]>("list_brushes"),
  brushThumbnail: (id: number) => invoke<ArrayBuffer>("brush_thumbnail", { id }),
  /** Familles de polices installées (le premier appel recense les polices du système). */
  listFonts: () => invoke<FontFamily[]>("list_fonts"),
  /** Étendues des autres calques visibles : cibles du magnétisme entre objets. */
  otherBounds: (docId: number, exclude: number[]) => invoke<Bounds[]>("other_bounds", { docId, exclude }),
  /** Calque de texte sous un point, même recouvert par d'autres calques. */
  pickTextLayer: (docId: number, x: number, y: number) => invoke<number | null>("pick_text_layer", { docId, x, y }),
  /** Étendue exacte du contenu des calques, ou de la sélection active s'il y en a une. */
  contentBounds: (docId: number, ids: number[]) => invoke<Bounds | null>("content_bounds", { docId, ids }),
  /** Découpage « dessous / déplacé / dessus » pour l'aperçu de déplacement, ou `null` s'il n'est pas exact. */
  movePlan: (docId: number, ids: number[]) => invoke<{ below: number[]; moving: number[]; above: number[] } | null>("move_plan", { docId, ids }),
  /** Comme `getTile`, mais en ne composant que les calques `layers`. */
  getTilePart: (docId: number, level: number, x: number, y: number, layers: number[], opaque: number | null) =>
    invoke<ArrayBuffer>("get_tile_part", { docId, level, x, y, layers, opaque }),
  /** Calque visible le plus haut sous un point du document, pour la sélection automatique. */
  pickLayer: (docId: number, x: number, y: number) => invoke<number | null>("pick_layer", { docId, x, y }),
  /** Largeur et hauteur (u32), puis RGBA à alpha droit. Vide si la miniature n'existe pas. */
  thumbnail: (docId: number, layer: number, mask: boolean) => invoke<ArrayBuffer>("thumbnail", { docId, layer, mask }),
  /**
   * Fichier déposé : la webview n'en connaît pas le chemin, le contenu part donc en
   * corps binaire brut et les paramètres en en-têtes (nom encodé, car ASCII obligatoire).
   */
  importDropped: (bytes: Uint8Array, fileName: string, docId: number | null, above: number | null, asSmartObject = false) =>
    invoke<EditResult>("import_dropped", bytes, {
      headers: {
        "x-file-name": encodeURIComponent(fileName),
        ...(docId !== null && { "x-doc-id": String(docId) }),
        ...(above !== null && { "x-above": String(above) }),
        // Placé comme objet dynamique incorporé (avec un document cible seulement).
        ...(asSmartObject && { "x-smart": "1" }),
      },
    }),
  /** « Modifier le contenu » : ouvre le contenu comme un document à part (déjà ouvert : le même). */
  smartContentOpen: (docId: number, layer: number) => invoke<DocumentInfo>("smart_content_open", { docId, layer }),
  /** Ctrl+S d'un document `smartParent` : remplace le contenu dans le parent (toutes instances) ; renvoie le parent. */
  smartContentCommit: (docId: number) => invoke<EditResult>("smart_content_commit", { docId }),
  /** « Exporter le contenu… » : `.graf` = le document du contenu ; sinon l'image d'origine ou le fichier lié. */
  smartContentExport: (docId: number, layer: number, path: string) => invoke<void>("smart_content_export", { docId, layer, path }),
  /** Vérifie les fichiers liés : relit les modifiés, signale les introuvables. */
  smartLinksCheck: (docId: number) => invoke<SmartLinksResult>("smart_links_check", { docId }),
  saveProject: (docId: number, path: string) => invoke<DocumentInfo>("save_project", { docId, path }),
  exportPng: (docId: number, path: string) => invoke<void>("export_png", { docId, path }),
  /** Encode l'export en mémoire : 16 octets (poids exact en u64, largeur, hauteur) puis une image affichable. */
  exportPreview: (docId: number, options: ExportOptions) => invoke<ArrayBuffer>("export_preview", { docId, options }),
  /** Aperçu instantané de l'export : vue d'ensemble + poids estimé, zone visible à pleine résolution, poids exact. */
  exportOverview: (docId: number, options: ExportOptions, maxSide: number) => invoke<ArrayBuffer>("export_overview", { docId, options, maxSide }),
  exportRegion: (docId: number, options: ExportOptions, rect: [number, number, number, number]) => invoke<ArrayBuffer>("export_region", { docId, options, rect }),
  exportExactSize: (docId: number, options: ExportOptions) => invoke<number>("export_exact_size", { docId, options }),
  exportPreviewRelease: () => invoke<void>("export_preview_release"),
  exportImage: (docId: number, path: string, options: ExportOptions) => invoke<void>("export_image", { docId, path, options }),
  closeDocument: (docId: number) => invoke<void>("close_document", { docId }),
  edit: (docId: number, op: EditOp) => invoke<EditResult>("edit", { docId, op }),
  strokeBegin: (docId: number, layer: number, onMask: boolean, brush: BrushParams) =>
    invoke<void>("stroke_begin", { docId, layer, onMask, brush }),
  strokePoints: (docId: number, points: number[][]) => invoke<[number, DirtyInfo]>("stroke_points", { docId, points }),
  /** Le dernier bout du trait (le lissage garde un demi-segment d'avance) est tracé ici. */
  strokeEnd: (docId: number) => invoke<EditResult>("stroke_end", { docId }),
};
