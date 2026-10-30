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

/** Couche lue par la comparaison (options de fusion). */
export type BlendIfChannel = "gray" | "red" | "green" | "blue";

/**
 * Comparaison (« Blend If ») : plages `[noir début, noir fin, blanc début, blanc fin]` de 0 à 255, pour le calque
 * (`this`) et pour l'image dessous (`under`). Début ≠ fin : fondu (triangle scindé par Alt).
 */
export interface BlendIf {
  channel: BlendIfChannel;
  this: [number, number, number, number];
  under: [number, number, number, number];
}

/** Un filtre dynamique : le filtre, son œil et ses options de fusion. */
export interface SmartFilter {
  filter: Filter;
  enabled: boolean;
  blend: BlendMode;
  opacity: number;
}

/** Filtres dynamiques d'un calque. */
export interface FiltersInfo {
  list: SmartFilter[];
  enabled: boolean;
  hasMask: boolean;
  maskEnabled: boolean;
}

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
  /** « justify » : lignes pleines d'un bord à l'autre du cadre (texte de paragraphe ; un texte de point s'aligne à gauche). */
  align: "left" | "center" | "right" | "justify";
  color: Rgb;
  /** Graisse : 100 (Thin) à 900 (Black), 400 = Regular, 700 = Bold. */
  weight: number;
  italic: boolean;
  /** Portions dont le style diffère de celui du calque ; en cas de recouvrement, la dernière l'emporte. */
  runs?: StyleRun[];
  /**
   * Texte de paragraphe (texte en bloc) : son cadre, en pixels du repère du texte (origine au coin haut gauche du
   * cadre). Le texte revient à la ligne dans sa largeur ; ce qui dépasse sa hauteur est masqué. Absent : texte de point.
   */
  frame?: TextFrame | null;
  /** Texte dynamique (paragraphe seulement) : chaque ligne remplit la largeur du cadre. */
  dynamic?: DynamicText | null;
  /** Casse du calque. */
  caps?: TextCaps;
  /** Style de caractère appliqué au calque entier. */
  style?: number | null;
}

/** Cadre d'un texte de paragraphe. */
/** Réglages d'un texte dynamique. */
export interface DynamicText {
  /** Nombre de lignes ; 0 : automatique. */
  lines: number;
  /** Les retours à la ligne tapés font les lignes. */
  manual: boolean;
  /** Espace entre les lignes, en part de la hauteur moyenne d'une ligne. */
  gap: number;
}

export interface TextFrame {
  width: number;
  height: number;
}

/** Curseur, sélection et cadre d'un texte en cours de saisie, en repère document. */
export interface CaretInfo {
  caret: [[number, number], [number, number]];
  highlights: [number, number][][];
  /** Texte de paragraphe : coins de son cadre (haut gauche, haut droit, bas droit, bas gauche). */
  frame: [[number, number], [number, number], [number, number], [number, number]] | null;
  /** Une partie du texte dépasse du cadre et n'est pas dessinée. */
  overflow: boolean;
}

/** Style propre à une portion du texte ; `start..end` en unités UTF-16. Un champ absent reprend le réglage du calque. */
export interface StyleRun {
  start: number;
  end: number;
  fontFamily?: string;
  weight?: number;
  italic?: boolean;
  color?: Rgb;
  /** Corps de la portion, en pixels. */
  size?: number;
  /** Approche de la portion. */
  tracking?: number;
  caps?: TextCaps;
  /** Style de caractère appliqué à la portion. */
  style?: number;
}

/** Plan de travail : cadre en pixels du document, fond (`null` : transparent), découpe du contenu. */
export interface Artboard {
  x: number;
  y: number;
  width: number;
  height: number;
  background: Rgb | null;
  clip: boolean;
  preset?: string | null;
  backgroundLayer?: number | null;
}

/** Sélection avancée (crates/engine/src/select_advanced.rs). */
export type AdvancedOutput = "selection" | "layerMask" | "newLayer";
export interface AdvancedDab {
  x: number;
  y: number;
  radius: number;
  mode: "refine" | "add" | "remove";
}
export interface AdvancedParams {
  method: "channel" | "color" | "tones";
  mix: [number, number, number];
  black: number;
  gamma: number;
  white: number;
  invert: boolean;
  colors: [number, number, number][];
  fuzziness: number;
  toneLow: number;
  toneHigh: number;
  toneSoft: number;
  radius: number;
  smooth: number;
  feather: number;
  shift: number;
  decontaminate: boolean;
  amount: number;
  dabs: AdvancedDab[];
}
export interface AdvancedInfo {
  width: number;
  height: number;
  previewWidth: number;
  previewHeight: number;
  scale: number;
  /** Rouge, vert, bleu, luminosité. */
  scores: { score: number; threshold: number; black: number; white: number; invert: boolean }[];
}

/** Casse : telle que tapée, tout en capitales, petites capitales (fonction de la police). */
export type TextCaps = "normal" | "allCaps" | "smallCaps";

/**
 * Style de caractère (crates/engine/src/text_styles.rs). Un champ absent hérite de `basedOn`. Ses réglages sont
 * recopiés dans les textes qui l'utilisent ; `library` : lien vers la bibliothèque de l'application.
 */
export interface CharStyle {
  id: number;
  name: string;
  basedOn?: number | null;
  fontFamily?: string | null;
  weight?: number | null;
  italic?: boolean | null;
  size?: number | null;
  lineHeight?: number | null;
  tracking?: number | null;
  color?: Rgb | null;
  caps?: TextCaps | null;
  shortcut?: string | null;
  library?: { uid: string; revision: number } | null;
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
  /** Comparaison (options de fusion) ; absente (`null`) quand les plages sont entières. */
  blendIf: BlendIf | null;
  hasMask: boolean;
  maskEnabled: boolean;
  /** Masque lié au calque (chaîne) : il le suit quand on déplace le calque. */
  maskLinked: boolean;
  /** Présent pour un calque de texte. */
  text: TextData | null;
  /** Présent pour un calque de réglage. */
  adjustment: Adjustment | null;
  /** Présent pour un calque de forme. */
  shape: ShapeData | null;
  /** Présent pour un calque de remplissage (couleur unie, dégradé, motif). */
  fillLayer?: FillContent | null;
  /** Effets de calque (fx). */
  effects: LayerEffect[];
  /** Couleur d'étiquette dans le panneau (`null` : aucune ; un dossier teinte ses enfants sans étiquette). */
  label: LayerLabel | null;
  /** Présent pour un objet dynamique. */
  smart: SmartInfo | null;
  /** Filtres dynamiques du calque, ou `null`. */
  filters: FiltersInfo | null;
  /** Plan de travail (groupe) : son cadre, son fond, sa découpe. */
  artboard?: Artboard | null;
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
  /** RAW « à redévelopper » : « Modifier le contenu » rouvre la fenêtre Développement RAW. */
  rawDevelop: boolean;
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
  // Motif de la bibliothèque (`patternCatalog`), échelle en %, couleurs du dessin et du fond.
  | { type: "patternOverlay"; enabled: boolean; pattern: string; scale: number; color: Rgb; background: Rgb; opacity: number }
  | { type: "satin"; enabled: boolean; color: Rgb; opacity: number; angle: number; distance: number; size: number; invert: boolean }
  // `depth` : 1 = 100 % ; `altitude` en degrés ; hautes lumières et ombres ont chacune couleur, opacité et mode.
  | {
      type: "bevelEmboss";
      enabled: boolean;
      style: BevelStyle;
      technique: "smooth" | "chisel_hard";
      depth: number;
      direction: "up" | "down";
      size: number;
      soften: number;
      angle: number;
      altitude: number;
      highlight: Rgb;
      highlightOpacity: number;
      highlightBlend: BlendMode;
      shadow: Rgb;
      shadowOpacity: number;
      shadowBlend: BlendMode;
    }
);
export type BevelStyle = "outer_bevel" | "inner_bevel" | "emboss" | "pillow_emboss";

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
  | ({ type: "develop" } & DevelopValues)
  /** Courbes : points [entrée, sortie] de 0 à 255 ; chaque couche passe par sa courbe, puis par la composite RVB. */
  | { type: "curves"; rgb: CurvePoint[]; red: CurvePoint[]; green: CurvePoint[]; blue: CurvePoint[] }
  /** Exposition en lumière linéaire : IL (-20 à 20), décalage (-0,5 à 0,5), gamma (0,01 à 9,99). */
  | { type: "exposure"; exposure: number; offset: number; gamma: number }
  /** Balance des couleurs : [cyan–rouge, magenta–vert, jaune–bleu] de -100 à 100 par plage de tons. */
  | { type: "colorBalance"; shadows: Triple; midtones: Triple; highlights: Triple; preserveLuminosity: boolean }
  /** Noir et blanc : poids en % (-200 à 300) ; teinte facultative (degrés, saturation 0 à 100). */
  | { type: "blackWhite"; reds: number; yellows: number; greens: number; cyans: number; blues: number; magentas: number; tint: boolean; tintHue: number; tintSaturation: number }
  /** Mélangeur de couches : [rouge, vert, bleu, constante] en % (-200 à 200) par couche de sortie. */
  | { type: "channelMixer"; red: MixerRow; green: MixerRow; blue: MixerRow; gray: MixerRow; monochrome: boolean }
  /** Correction sélective : [cyan, magenta, jaune, noir] en % (-100 à 100) par famille ; méthode Absolue ou Relative. */
  | ({ type: "selectiveColor"; absolute: boolean } & Record<SelectiveFamily, MixerRow>)
  /** Isohélie : niveaux par couche (2 à 255). */
  | { type: "posterize"; levels: number }
  /** Seuil : blanc à partir de cette luminosité (1 à 255). */
  | { type: "threshold"; level: number };

export type CurvePoint = [number, number];
export type Triple = [number, number, number];
export type MixerRow = [number, number, number, number];
export type SelectiveFamily = "reds" | "yellows" | "greens" | "cyans" | "blues" | "magentas" | "whites" | "neutrals" | "blacks";

/** Style d'un dégradé de remplissage. */
export type FillGradientStyle = "linear" | "radial" | "angle" | "reflected" | "diamond";

/** Contenu d'un calque de remplissage (engine::fill::FillContent). */
export type FillContent =
  | { type: "solidColor"; color: Rgb }
  /** Angle en degrés (90 : de bas en haut), échelle en % (10 à 150), décalage du centre en pixels. */
  | { type: "gradient"; stops: { position: number; color: [number, number, number, number] }[]; style: FillGradientStyle; angle: number; scale: number; reverse: boolean; offset: [number, number] }
  /** Motif de la bibliothèque du moteur ; échelle en % (10 à 1000), couleurs du motif et du fond. */
  | { type: "pattern"; pattern: string; scale: number; color: Rgb; background: Rgb };

/** Un motif de la bibliothèque du moteur (taille de cellule à 100 %). */
export interface PatternInfo {
  id: string;
  name: string;
  width: number;
  height: number;
}

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
  // `warp` / `streak` > 0 : aberration « organique » (magenta et vert le long d'une lentille irrégulière).
  | { type: "chromaticAberration"; amount: number; angle: number; radial: boolean; warp: number; streak: number }
  // Flou granuleux des affiches : flou, dissolution aléatoire des transitions, puis grain gaussien.
  // Texture, clarté, correction du voile : -100 à 100, avec un vrai calcul de voisinage.
  // Verre : l'image vue à travers un relief. `texture` "custom" = l'image chargée par `loadGlassTexture`.
  | { type: "glass"; distortion: number; smoothness: number; texture: "frosted" | "blocks" | "canvas" | "tinyLens" | "custom"; scale: number; invert: boolean }
  | { type: "presence"; texture: number; clarity: number; dehaze: number }
  | { type: "gaussianNoise"; radius: number; scatter: number; amount: number; grain: number; monochrome: boolean }
  | { type: "ascii"; cell: number; color: boolean; glow: number }
  // Filtres de la galerie (moteur : crates/engine/src/creative.rs). Les longueurs sont en pixels, les
  // intensités de 0 à 100 ; `seed` : tirage au sort (bouton « Variante »).
  | { type: "radialBlur"; strength: number; x: number; y: number; fringe: number }
  | { type: "filmGrain"; amount: number; size: number; seed: number }
  | { type: "flutedGlass"; rib: number; refraction: number; wave: number; horizontal: boolean }
  | { type: "vignette"; amount: number; size: number; softness: number }
  | { type: "petrolSheen"; strength: number; bands: number; scale: number; seed: number }
  | { type: "chrome"; rings: number; softness: number; tint: "silver" | "gold" | "copper" }
  | { type: "emboss"; angle: number; height: number; amount: number; color: boolean }
  | { type: "halftone"; cell: number; angle: number; color?: boolean; pattern?: "dots" | "circles" | "lines" | "cmyk"; contrast?: number; ink?: Rgb; paper?: Rgb; x?: number; y?: number }
  | { type: "retroPrint"; ink: Rgb; paper: Rgb; density: number; grain: number; contrast: number; bleed: number; seed: number }
  | { type: "trail"; angle: number; length: number; opacity: number; falloff: number; sharp: number; seed: number }
  | {
      type: "blades";
      rotation: number;
      tilt: number;
      orient: number;
      density: number;
      height: number;
      amp: number;
      wavelength: number;
      phase: number;
      light: number;
      color: boolean;
      transparent: boolean;
    }
  | { type: "riso"; cell: number; shift: number; grain: number; inks: "quad" | "duo" | "greenOrange" | "monoRed"; seed: number }
  | { type: "stipple"; spacing: number; size: number; shape: "round" | "square" | "stroke"; paper: boolean; seed: number }
  | { type: "crt"; pitch: number; glow: number; mask: "triads" | "grille" | "slots" }
  | { type: "scanlines"; period: number; depth: number; jitter: number; contrast: number; seed: number }
  | { type: "pixelate"; cell: number }
  | { type: "glitch"; intensity: number; seed: number }
  | { type: "datamosh"; block: number; bleed: number; glitches: number; grain: number; palette: "violet" | "original" | "acid"; seed: number }
  | { type: "pixelSort"; low: number; high: number; direction: "right" | "left" | "down" | "up" }
  | { type: "oilPaint"; radius: number }
  | { type: "posterize"; levels: number }
  | { type: "contours"; amount: number; color: boolean }
  | { type: "truchet"; cell: number; width: number; glow: number; pattern: TruchetPattern; color: "image" | "mono" | "neon" | "paper"; seed: number }
  // Filtre › Filtre Camera Raw (hors galerie) : les réglages de la fenêtre Développement RAW, sur les pixels du calque.
  | { type: "cameraRaw"; params: RawParams };

/** Motifs du filtre Truchet (même ordre que le moteur). */
export type TruchetPattern =
  | "arcs"
  | "double"
  | "triple"
  | "fan"
  | "diagonals"
  | "ribbons"
  | "bevels"
  | "rings"
  | "mixed"
  | "weave"
  | "maze"
  | "dotsArcs"
  | "filledQuarters"
  | "triangles"
  | "multiScale";

export interface Guide {
  orientation: "horizontal" | "vertical";
  /** Pixels du document, le long de l'axe perpendiculaire au repère. */
  position: number;
  /** Plan de travail du repère (document à plans) : il n'est dessiné et n'aimante que dans son cadre. Absent sans plan. */
  board?: number | null;
}

/** Joint automatiquement à un rapport de bug (Aide > Signaler un bug). */
export interface BugReportContext {
  /** Système lisible : « Linux (Fedora Linux 44) », « Windows »… */
  os: string;
  arch: string;
  /** Dossier des journaux (cache de l'application, `chromium.log` sous Linux). */
  logsDir: string;
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

export type ExportFormat = "png" | "jpeg" | "webp" | "tiff" | "pdf" | "avif";

/** Vitesse d'encodage de l'AVIF : préréglage 9 (rapide), 6 (équilibrée) ou 4 (compression maximale) de rav1e. */
export type AvifSpeed = "fast" | "balanced" | "max";

/**
 * Couleurs du fichier exporté : RVB, CMJN selon un profil de presse (PDF, TIFF et JPEG seulement), niveaux de gris
 * (vrai fichier gris 8 bits en PNG, TIFF, JPEG et PDF ; pixels gris en RVB ailleurs) ou bitmap (PNG, TIFF et PDF
 * 1 bit ; JPEG gris 8 bits ; noir et blanc en RVB ailleurs).
 */
export type ExportColor = "rgb" | "cmyk" | "gray" | "bitmap";

/** Qualité d'un PDF : sans perte (Flate), ou JPEG 92 / 80 / 60. */
export type PdfQuality = "lossless" | "high" | "medium" | "low";

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
 * `quality` : JPEG, WebP avec perte et AVIF (1 à 100) ; `lossless` : WebP sans perte (la qualité est alors ignorée) ;
 * `scale` : 1 = taille native ; `metadata` : absent = rien d'inscrit.
 * `color`, `cmykSpace` : CMJN dans cet espace (PDF, TIFF, JPEG ; ignoré ailleurs) ; `embedProfile` : incorporer le
 * profil ICC (de presse en CMJN, sRGB en RVB) ; `pdfQuality` : qualité du PDF ; `avifSpeed` : vitesse d'encodage de
 * l'AVIF. Absents : RVB, sans profil, PDF « haute », AVIF « rapide ».
 */
export interface ExportOptions {
  format: ExportFormat;
  quality: number;
  lossless?: boolean;
  scale: number;
  metadata?: ImageMetadata | null;
  color?: ExportColor;
  cmykSpace?: CmykSpace;
  embedProfile?: boolean;
  pdfQuality?: PdfQuality;
  avifSpeed?: AvifSpeed;
  /** PDF d'impression (PDF/X-4). */
  pdfPrint?: PdfPrint | null;
  /** Plan de travail exporté seul (`null` : le document entier). */
  artboard?: number | null;
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
  /** Mode colorimétrique (Image › Mode). Absent des descriptions écrites avant son existence (tests) : RVB. */
  colorMode?: ColorMode;
  /** Styles de caractère du document. */
  textStyles?: CharStyle[];
  /** Format d'impression (absent ou `null` : aucun) et format fini `[x, y, l, h]` en pixels. */
  print?: PrintSetup | null;
  trim?: [number, number, number, number];
  /** Temps de travail actif enregistré avec le projet, en secondes. */
  workSeconds?: number;
}

/**
 * Mode colorimétrique d'un document (voir crates/engine/src/color_mode.rs). Les pixels restent en RVB : en gris et
 * en bitmap, le rendu final est désaturé (et seuillé) ; en CMJN, l'épreuve écran du profil reste allumée.
 */
/**
 * Espace CMJN (voir lib/color-settings.svelte.ts) : profil (`fogra39`, …, ou `icc-<empreinte>` pour un profil chargé
 * ou installé sur l'ordinateur), intention de rendu vers le CMJN, compensation du point noir.
 */
export interface CmykSpace {
  profile: string;
  intent: "relative" | "perceptual";
  bpc: boolean;
}

/** Profil CMJN de la liste des Paramètres de couleur. */
export interface CmykProfileEntry {
  id: string;
  name: string;
  short: string;
  builtin: boolean;
  origin: "builtin" | "user" | "system";
  path: string | null;
}

export type ColorMode = { kind: "rgb" } | { kind: "grayscale" } | { kind: "bitmap" } | ({ kind: "cmykWorkflow" } & CmykSpace);

/** Méthode de la fenêtre Bitmap. */
export type BitmapMethod = { kind: "threshold" } | { kind: "diffusion" } | { kind: "halftone"; frequency: number; angle: number; shape?: "round" };

export interface BitmapOptions {
  method: BitmapMethod;
  /** Résolution de sortie, en pixels par pouce (l'image est rééchantillonnée si elle diffère du document). */
  outputDpi: number;
}

/** Gamme d'impression d'une couleur : équivalent imprimable, écart ΔE 1976, alerte hors gamme. */
export interface GamutInfo {
  printable: Rgb;
  deltaE: number;
  outOfGamut: boolean;
}

/** Couche de l'aperçu de déplacement par images en cache (voir crates/engine/src/move_preview.rs). */
export interface MoveSlice {
  kind: "below" | "moving" | "layers" | "adjust";
  /** Mode de fusion de la couche sur ce qui est dessous. */
  mode: BlendMode;
  /** Calque de réglage (`adjust`) ; calque ou groupe déplacé seul (`moving`). */
  layer: number | null;
  adjustment: Adjustment | null;
  opacity: number;
  /** Réglage à masque : l'image de la couche est la couverture du masque. */
  masked: boolean;
  maskBackground: number;
  /** Couche qui suit les calques déplacés (le « déplacé », un paquet d'un groupe en Transfert, le masque d'un réglage du groupe). */
  moving?: boolean;
  /** Empreinte du contenu d'une couche fixe (absente pour une couche qui bouge) : même empreinte, même image. */
  fingerprint?: string | null;
}

export interface MoveLayout {
  revision: number;
  /** Paquets au plus du découpage (à redonner pour les images des couches). */
  maxPacks?: number;
  moving: number[];
  slices: MoveSlice[];
  /** L'aperçu est l'image exacte du document déplacé ; sinon `notes` dit pourquoi. */
  exact: boolean;
  notes: string[];
  /** Groupe déplacé seul dont l'aplat peut remplacer tout son sous-programme sur la carte graphique. */
  collapsible: number | null;
  /** Étendue de ce qui est déplacé, effets compris : `[x0, y0, x1, y1]` (repère document). */
  bounds: [number, number, number, number] | null;
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

/** Pixels d'une zone d'un calque envoyés avec une annulation (voir `api.historyStep`). */
export interface HistoryPatch {
  layer: number;
  /** `[x, y, largeur, hauteur]`, repère du calque. */
  rect: [number, number, number, number];
  pixels: Uint8Array;
}

export interface HistoryStepResult extends EditResult {
  patches: HistoryPatch[];
}

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
  | { op: "setMaskLinked"; id: number; linked: boolean }
  | { op: "fill"; id: number; color: Rgb; onMask: boolean }
  | { op: "addText"; x: number; y: number; text: TextData; above: number | null }
  // `session` : les modifications d'une même séance d'édition ne font qu'un état d'historique.
  | { op: "setText"; id: number; text: TextData; session: string }
  | { op: "setTextStyles"; list: CharStyle[]; update: number[]; label: string }
  | { op: "addArtboard"; name: string; board: Artboard }
  | { op: "setArtboard"; id: number; board: Artboard; fit: boolean; gesture: string | null }
  | { op: "moveArtboard"; id: number; dx: number; dy: number; fit: boolean; gesture: string | null }
  | { op: "duplicateArtboard"; id: number; dx: number; dy: number; content: boolean }
  | { op: "removeArtboard"; id: number }
  | { op: "fitArtboard"; id: number }
  | { op: "applyArtboards"; boards: { id: number | null; name: string; board: Artboard }[]; removed: number[] }
  | { op: "advancedSelection"; layer: number | null; params: AdvancedParams; output: AdvancedOutput }
  /** Poignée du cadre d'un paragraphe : `shift` = déplacement de l'origine depuis le début du geste (repère du texte). */
  | { op: "setTextFrame"; id: number; frame: TextFrame | null; shift: [number, number]; gesture: string }
  | { op: "convertText"; id: number; paragraph: boolean }
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
  // Style complet (effets + options de fusion) d'un ou plusieurs calques en un seul état : fenêtre des effets
  // (`gesture`), Coller / Effacer le style. Les options absentes ne changent pas.
  | { op: "setStyle"; ids: number[]; effects: LayerEffect[]; blend?: BlendMode; opacity?: number; fill?: number; blendIf?: BlendIf; gesture?: string }
  | { op: "addShape"; x: number; y: number; shape: ShapeData; above: number | null }
  | { op: "setShape"; id: number; shape: ShapeData }
  // Calques de remplissage ; `gesture` : boîte de dialogue ouverte (Annuler = `cancelGesture`).
  | { op: "addFill"; fill: FillContent; above: number | null; gesture?: string }
  | { op: "setFill"; id: number; fill: FillContent; gesture?: string }
  // Cadre de transformation tiré sur une forme sans rotation : redessinée à la nouvelle taille, coins intacts.
  | { op: "placeShape"; id: number; x: number; y: number; width: number; height: number; gesture: string }
  // Rectangle en pixels du document actuel ; il peut déborder pour agrandir la zone de travail.
  | { op: "crop"; x: number; y: number; width: number; height: number }
  | { op: "setWorkArea"; board: number | null }
  | { op: "setPrintSetup"; setup: PrintSetup | null; extend: boolean; dpi?: number | null }
  | { op: "correctInk"; limit: number; space: CmykSpace; layer?: number | null }
  | { op: "setAdjustment"; id: number; adjustment: Adjustment }
  // `gesture` : aperçu en direct depuis la boîte de dialogue ; `cancelGesture` retire tout si l'on annule.
  | { op: "applyFilter"; id: number; filter: Filter; gesture?: string }
  // Camera Raw sur un objet dynamique : pixellisé puis filtré en un seul état d'historique (rien ne change en cas d'échec).
  | { op: "rasterizeAndFilter"; id: number; filter: Filter }
  | { op: "setSmartFilters"; id: number; list: SmartFilter[]; enabled: boolean; maskEnabled?: boolean; gesture?: string }
  | { op: "bakeFilters"; id: number }
  | { op: "deleteFilterMask"; id: number }
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
  | { op: "setGuides"; guides: Guide[]; clear: boolean; board: number | null }
  | { op: "setGuidesLocked"; locked: boolean }
  | { op: "addGuide"; orientation: Guide["orientation"]; position: number; board: number | null }
  | { op: "moveGuide"; index: number; position: number }
  | { op: "removeGuide"; index: number }
  | { op: "clearGuides"; board: number | null }
  // Avec `layer` : le contenu opaque de ce calque ; sans : tout le document.
  | { op: "selectAll"; layer?: number }
  | { op: "deselect" | "invertSelection" }
  | { op: "feather"; radius: number }
  /** Image › Mode : un seul état d'historique ; `bitmap` exigé pour ce mode (après confirmation de l'aplatissement). */
  | { op: "setColorMode"; mode: ColorMode; bitmap?: BitmapOptions }
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
  /** Pinceau double d'un préréglage importé (Photoshop). */
  dual?: DualParam | null;
  /** Texture d'un préréglage importé (Photoshop). */
  texture?: TextureParam | null;
}

/** Mode de combinaison du pinceau double ou de la texture (modes de Photoshop). */
export type TipBlendMode = "multiply" | "darken" | "lighten" | "overlay" | "colorDodge" | "colorBurn" | "linearBurn" | "hardMix" | "subtract" | "height" | "linearHeight" | "screen";

/** Pinceau double : seconde pointe tamponnée le long du trait, qui découpe la première. */
export interface DualParam {
  tip: number;
  /** Diamètre de la seconde pointe rapporté à celui du pinceau. */
  ratio: number;
  spacing: number;
  scatter: number;
  bothAxes: boolean;
  count: number;
  mode: TipBlendMode;
  flip: boolean;
}

/** Texture : motif fixé à la toile qui module le trait. */
export interface TextureParam {
  pattern: number;
  scale: number;
  depth: number;
  mode: TipBlendMode;
  invert: boolean;
  brightness: number;
  contrast: number;
}

/** Pinceau d'un fichier importé (préréglage Photoshop, ou pointe seule d'un fichier sans préréglage). */
export interface BrushPresetInfo {
  /** `dossier#rang`, stable d'un lancement à l'autre. */
  key: string;
  folder: string;
  name: string;
  kind: "round" | "tip";
  tip: number | null;
  size: number;
  hardness: number;
  opacity: number;
  pressureSize: boolean;
  pressureOpacity: boolean;
  dynamics: BrushDynamics;
  dual: DualParam | null;
  texture: TextureParam | null;
}

export interface BrushLibrary {
  tips: BrushTipInfo[];
  presets: BrushPresetInfo[];
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
  /** Rondeur de la pointe (0 à 1 ; 0 = ronde). */
  roundness: number;
  roundnessJitter: number;
  minRoundness: number;
  flipX: boolean;
  flipY: boolean;
  /** Diamètre minimal sous faible pression (fraction du diamètre). */
  minSize: number;
  /** Dispersion perpendiculaire au trait seulement, et nombre d'empreintes par pas. */
  scatterPerpendicular: boolean;
  count: number;
  opacityJitter: number;
  flowJitter: number;
  /** La pression module aussi le flux. */
  pressureFlow: boolean;
}

export const NEUTRAL_DYNAMICS: BrushDynamics = {
  // Espacement des empreintes : 1 % du diamètre par défaut (trait continu) ; un pinceau importé garde le sien.
  spacing: 0.01,
  angle: 0,
  followDirection: false,
  scatter: 0,
  sizeJitter: 0,
  angleJitter: 0,
  amount: 0,
  flow: 0,
  paint: 0,
  stretch: 0,
  roundness: 0,
  roundnessJitter: 0,
  minRoundness: 0,
  flipX: false,
  flipY: false,
  minSize: 0,
  scatterPerpendicular: false,
  count: 0,
  opacityJitter: 0,
  flowJitter: 0,
  pressureFlow: false,
};
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

/** Réglages de la fenêtre « Développement RAW » (`engine::raw::RawParams`). */
export interface RawParams {
  /** asShot : coefficients de l'appareil ; auto : gris moyen (calculé pour chaque fichier) ; custom : température et teinte. */
  whiteBalance: "asShot" | "auto" | "custom";
  temperature: number;
  tint: number;
  exposure: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  clarity: number;
  vibrance: number;
  saturation: number;
  noiseLuma: number;
  noiseColor: number;
  sharpen: number;
  vignette: number;
}
export type RawClipView = "none" | "highlights" | "shadows";
export type RawFinishMode = "open" | "layer" | "smart";
/** Ce que la fenêtre de développement prépare : une ouverture (voir `RawFinishMode`) ou un objet dynamique à redévelopper. */
/** `filter` : Filtre › Filtre Camera Raw, sur les pixels du calque actif (session de `rawOpenLayer`). */
export type RawDialogMode = RawFinishMode | "redevelop" | "filter";
export interface RawAutoTone {
  exposure: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
}
export interface RawInfo {
  fileName: string;
  camera: string;
  width: number;
  height: number;
  iso: number | null;
  aperture: number | null;
  /** Secondes. */
  shutter: number | null;
  focalLength: number | null;
  asShotTemperature: number;
  asShotTint: number;
  autoTemperature: number;
  autoTint: number;
  monochrome: boolean;
}
export interface RawOpened {
  id: number;
  info: RawInfo;
  previewWidth: number;
  previewHeight: number;
}
/** Le fichier est-il un RAW d'appareil (d'après son extension, casse ignorée) ? */
export function isRawPath(name: string): boolean {
  const base = name.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  return dot > 0 && RAW_EXTENSIONS.includes(base.slice(dot + 1).toLowerCase());
}

export interface NewDocumentParams {
  name: string;
  width: number;
  height: number;
  dpi: number;
  transparent: boolean;
  /** Document d'impression : `width` × `height` est alors le format fini, les fonds perdus s'y ajoutent. */
  print?: PrintSetup | null;
  /** Travail en CMJN dès la création, dans cet espace. */
  cmyk?: CmykSpace | null;
}

/** Format d'impression d'un document (Image › Format d'impression…). 0 : désactivé. */
export interface PrintSetup {
  bleedMm: number;
  safeMm: number;
  inkLimit: number;
}

/** Analyse d'encrage (alerte d'encrage, vérification avant export). */
export interface InkReport {
  limit: number;
  maxPercent: number;
  overPixels: number;
  cell: number;
  cols: number;
  rows: number;
  mask: number[];
  /** `[x, y, largeur, hauteur]` en pixels du document, les plus grandes d'abord. */
  regions: [number, number, number, number][];
}

/** PDF d'impression (PDF/X-4). */
export interface PdfPrint {
  bleedMm: number;
  includeBleed: boolean;
  marks: boolean;
  inkLimit: number;
  title: string;
  pureBlack: boolean;
  richBlack: boolean;
  dpi: number;
}

/** Vérification avant impression. */
export interface Preflight {
  ink: InkReport;
  /** Haut, droite, bas, gauche ; `null` sans fonds perdus. */
  bleedFilled: [boolean, boolean, boolean, boolean] | null;
  textsOutside: [number, string, [number, number, number, number]][];
}

export const PROJECT_EXTENSION = "graf";
/** Ancienne extension des projets, toujours acceptée à l'ouverture. */
export const LEGACY_PROJECT_EXTENSION = "iep";
/** RAW d'appareil photo, développés par le moteur (même liste que `engine::io::RAW_EXTENSIONS`). */
export const RAW_EXTENSIONS = ["dng", "arw", "srf", "sr2", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "ori", "rw2", "rwl", "raw", "pef", "srw", "3fr", "fff", "iiq", "mos", "erf", "dcr", "dcs", "kdc", "mef", "mrw", "x3f", "ari"];
/** Formats d'image ouverts ou importés : le moteur décode tout sauf l'AVIF, confié à la webview. */
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "tif", "tiff", "avif", "heic", "heif", ...RAW_EXTENSIONS];
/** Types MIME des mêmes formats, pour une image collée ou glissée sans extension. */
export const IMAGE_MIME_TYPES = ["image/png", "image/jpeg", "image/webp", "image/tiff", "image/avif", "image/heic", "image/heif", "image/heic-sequence", "image/x-adobe-dng", "image/dng"];
/**
 * Extensions pour un filtre de fenêtre d'ouverture : sous Linux, le filtre compare la casse (`*.arw` ne montre pas
 * `DSC0001.ARW`, nom que donnent la plupart des appareils). Chaque extension y figure donc aussi en majuscules.
 */
export const dialogExtensions = (extensions: readonly string[]): string[] => extensions.flatMap((e) => [e, e.toUpperCase()]);

export const api = {
  newDocument: (params: NewDocumentParams) => invoke<DocumentInfo>("new_document", { ...params, print: params.print ?? null, cmyk: params.cmyk ?? null }),
  cmykOf: (space: CmykSpace, color: Rgb) => invoke<[number, number, number, number]>("cmyk_of", { space, color }),
  separations: (docId: number, space: CmykSpace, maxSide: number) => invoke<ArrayBuffer>("separations", { docId, space, maxSide }),
  preflight: (docId: number, limit: number, space: CmykSpace) => invoke<Preflight>("preflight", { docId, limit, space }),
  inkReport: (docId: number, limit: number, space: CmykSpace) => invoke<InkReport>("ink_report", { docId, limit, space }),
  openPath: (path: string) => invoke<DocumentInfo>("open_path", { path }),
  /** 8 octets de révision, puis RGBA8 prémultiplié brut (rien si la tuile est transparente). */
  getTile: (docId: number, level: number, x: number, y: number) =>
    invoke<ArrayBuffer>("get_tile", { docId, level, x, y }),
  /** 8 octets de révision, 1 octet de valeur de fond, puis le masque brut (rien si la tuile est uniforme). */
  getSelectionTile: (docId: number, level: number, x: number, y: number) =>
    invoke<ArrayBuffer>("get_selection_tile", { docId, level, x, y }),
  /** Curseur et sélection d'un texte en cours de saisie, en repère document. Positions en unités UTF-16. */
  textCaret: (docId: number, layer: number, start: number, end: number) =>
    invoke<CaretInfo>("text_caret", { docId, layer, start, end }),
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
  /** `edit` : rang d'un filtre dynamique rouvert (aperçu avec le filtre réglé à sa place) ; sinon un nouveau filtre. */
  filterPreview: (docId: number, layer: number, filter: Filter, maxSide: number, inContext = false, edit: number | null = null) =>
    invoke<ArrayBuffer>("filter_preview", { docId, layer, filter, maxSide, inContext, edit }),
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
  /**
   * Annuler / Rétablir `count` fois d'un coup. Avec la réponse, les pixels des zones repeintes (repère de chaque
   * calque, RGBA prémultiplié) : posés directement dans les textures de la composition, sans relecture.
   */
  historyStep: async (docId: number, op: "undo" | "redo", count: number): Promise<HistoryStepResult> => {
    const buffer = await invoke<ArrayBuffer>("history_step", { docId, op, count });
    const length = new DataView(buffer).getUint32(0, true);
    const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, 4, length))) as EditResult & { patches: { layer: number; rect: [number, number, number, number] }[] };
    let at = 4 + length;
    const patches = head.patches.map(({ layer, rect }) => {
      const size = rect[2] * rect[3] * 4;
      const pixels = new Uint8Array(buffer, at, size);
      at += size;
      return { layer, rect, pixels };
    });
    return { document: head.document, dirty: head.dirty, patches };
  },
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
  autosave: (docId: number, key: string, workSeconds?: number) => invoke<void>("autosave", { docId, key, workSeconds: workSeconds === undefined ? null : Math.floor(workSeconds) }),
  recoveryList: () => invoke<{ key: string; savedAt: number; size: number }[]>("recovery_list"),
  recoveryOpen: (key: string) => invoke<DocumentInfo>("recovery_open", { key }),
  recoveryDelete: (key: string) => invoke<void>("recovery_delete", { key }),
  /** Aperçu instantané d'un réglage : calques dessous et dessus, ou `null` (réglage non simple). */
  adjustPlan: (docId: number, layer: number) => invoke<{ below: number[]; target: number[]; above: number[] } | null>("adjust_plan", { docId, layer }),
  /** Table de correspondance 3D d'un réglage : `size`³ couleurs RGBA8. */
  adjustmentLut: (adjustment: Adjustment, size: number) => invoke<ArrayBuffer>("adjustment_lut", { adjustment, size }),
  /** Réglage mis à plat pour le shader de composition : type (u32), puis ses valeurs (f32). */
  adjustmentGpu: (adjustment: Adjustment) => invoke<ArrayBuffer>("adjustment_gpu", { adjustment }),
  /** Bibliothèque de motifs des calques de remplissage. */
  patternCatalog: () => invoke<PatternInfo[]>("pattern_catalog"),
  /** Fenêtre Bitmap : extrait central avant / après (en-tête 8 octets : largeur, hauteur ; puis gris, puis noir et blanc). */
  bitmapPreview: (docId: number, options: BitmapOptions, width: number, height: number) => invoke<ArrayBuffer>("bitmap_preview", { docId, options, width, height }),
  /** Mode CMJN : la couleur est-elle imprimable avec ce profil ? */
  cmykGamut: (space: CmykSpace, color: Rgb) => invoke<GamutInfo>("cmyk_gamut", { space, color }),
  /** Table de l'épreuve CMJN de cet espace : 33³ triplets RVB, rouge le plus rapide. */
  cmykProofTable: (space: CmykSpace) => invoke<ArrayBuffer>("cmyk_proof_table", { space }),
  cmykProfiles: () => invoke<CmykProfileEntry[]>("cmyk_profiles"),
  cmykImportProfile: (path: string) => invoke<{ id: string; name: string; short: string; builtin: boolean }>("cmyk_import_profile", { path }),
  cmykProfileInfo: (profile: string) => invoke<{ id: string; name: string; short: string; builtin: boolean }>("cmyk_profile_info", { profile }),
  /** Vignette d'un motif : `size`² pixels RGBA (noir sur blanc). */
  patternSwatch: (id: string, size: number, scale: number) => invoke<ArrayBuffer>("pattern_swatch", { id, size, scale }),
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
  /** Page du dépôt du projet dans le navigateur ; toute autre adresse est refusée par le moteur. */
  openProjectLink: (url: string) => invoke<void>("open_project_link", { url }),
  bugReportContext: () => invoke<BugReportContext>("bug_report_context"),
  openLogsDir: () => invoke<void>("open_logs_dir"),
  /** Contenu brut d'un fichier (images AVIF, décodées par la webview). */
  readFileBytes: (path: string) => invoke<ArrayBuffer>("read_file_bytes", { path }),
  /** Écrit un fichier .json choisi dans une fenêtre d'enregistrement (JSON valide, 1 Mo au plus ; autre extension refusée). */
  writeJsonFile: (path: string, content: string) => invoke<void>("write_json_file", { path, content }),
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
  /** Pinceaux déposés dans la fenêtre (contenu seul : la webview ne donne pas le chemin). */
  importBrushesDropped: (bytes: Uint8Array, fileName: string) => invoke<BrushTipInfo[]>("import_brushes_dropped", bytes, { headers: { "x-file-name": encodeURIComponent(fileName) } }),
  /** Bibliothèque de pointes (recharge au premier appel les fichiers importés auparavant). */
  listBrushes: () => invoke<BrushTipInfo[]>("list_brushes"),
  brushLibrary: () => invoke<BrushLibrary>("brush_library"),
  /** Trait d'essai tracé par le moteur (PNG, encre blanche sur transparent). */
  brushPreview: (brush: BrushParams, width: number, height: number, ramp = false) => invoke<ArrayBuffer>("brush_preview", { brush, width, height, ramp }),
  removeBrushPack: (folder: string) => invoke<void>("remove_brush_pack", { folder }),
  brushThumbnail: (id: number) => invoke<ArrayBuffer>("brush_thumbnail", { id }),
  /** Pointe pour la carte graphique : largeur, hauteur (u32), espacement (f32), puis la couverture (octets). */
  brushTipImage: (id: number) => invoke<ArrayBuffer>("brush_tip_image", { id }),
  /** Motif d'une texture de pinceau : largeur, hauteur (u32), puis les gris (octets). */
  brushPatternImage: (id: number) => invoke<ArrayBuffer>("brush_pattern_image", { id }),
  /** Empreintes d'un trait de pointe calculées par l'interface (lib/tip-stamps.ts), posées telles quelles. */
  strokeStamps: (docId: number, stamps: number[][], duals: number[][]) => invoke<[number, DirtyInfo]>("stroke_stamps", { docId, stamps, duals }),
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
  /** Aperçu de déplacement par images en cache : découpage en couches (voir `MoveLayout`), ou `null` s'il ne convient pas. */
  /** `maxPacks` : paquets au plus pour un groupe Transfert déplacé (12 ; 4 en mode portable). */
  moveLayout: (docId: number, ids: number[], maxPacks: number) => invoke<MoveLayout | null>("move_layout", { docId, ids, maxPacks }),
  /**
   * Image d'une couche de cet aperçu sur `rect` (pixels du niveau `level`) : 16 octets d'en-tête (x, y, largeur,
   * hauteur), puis RGBA8 prémultiplié. Refusée si le document n'est plus à la révision du découpage.
   */
  moveSlice: (docId: number, ids: number[], index: number, level: number, rect: [number, number, number, number], revision: number, own: boolean, maxPacks: number) =>
    invoke<ArrayBuffer>("move_slice", { docId, ids, index, level, rect, revision, own, maxPacks }),
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
  /** Développement RAW : décode le capteur une fois ; la session vit jusqu'à `rawClose` (ou `rawFinish`). */
  rawOpen: (path: string) => invoke<RawOpened>("raw_open", { path }),
  /** Comme `rawOpen`, pour un fichier déposé (contenu en corps binaire). */
  rawOpenDropped: (bytes: Uint8Array, fileName: string) => invoke<RawOpened>("raw_open_dropped", bytes, { headers: { "x-file-name": encodeURIComponent(fileName) } }),
  /** Aperçu réduit : largeur et hauteur (u32), histogramme RVB (768 u32), puis JPEG. */
  rawPreview: (id: number, params: RawParams, clip: RawClipView = "none") => invoke<ArrayBuffer>("raw_preview", { id, params, clip }),
  /** Pipette : [température, teinte] qui rendent neutre le point (0 à 1 sur l'image), ou null. */
  rawRegion: (id: number, params: RawParams, clip: RawClipView, x: number, y: number, width: number, height: number, factor: number) =>
    invoke<ArrayBuffer>("raw_region", { id, params, clip, x, y, width, height, factor }),
  rawPickWhite: (id: number, x: number, y: number) => invoke<[number, number] | null>("raw_pick_white", { id, x, y }),
  rawAutoTone: (id: number, params: RawParams) => invoke<RawAutoTone>("raw_auto_tone", { id, params }),
  /**
   * Développe en pleine taille : nouveau document, calque ou objet dynamique du document `docId`. Libère la session.
   * `keepRaw` : objet dynamique qui garde le RAW et ses réglages (« redévelopper plus tard ») ; toujours vrai en `smart`.
   */
  rawFinish: (id: number, params: RawParams, mode: RawFinishMode, docId: number | null, above: number | null, keepRaw = false) =>
    invoke<EditResult>("raw_finish", { id, params, mode, docId, above, keepRaw }),
  /** « Modifier le contenu » d'un objet dynamique RAW : session neuve depuis le RAW gardé, et ses réglages. */
  rawOpenSmart: (docId: number, layer: number) => invoke<{ opened: RawOpened; params: RawParams }>("raw_open_smart", { docId, layer }),
  /** Redéveloppe l'objet dynamique RAW (toutes ses instances, une étape d'historique). Libère la session. */
  rawRedevelop: (id: number, params: RawParams, docId: number, layer: number) => invoke<EditResult>("raw_redevelop", { id, params, docId, layer }),
  /** Filtre Camera Raw : session de développement construite depuis les pixels du calque (à fermer par `rawClose`). */
  /** `upto` : filtre dynamique rouvert (la fenêtre part de ce qu'il reçoit) ; `null` : nouveau filtre. */
  rawOpenLayer: (docId: number, layer: number, upto: number | null = null) => invoke<RawOpened>("raw_open_layer", { docId, layer, upto }),
  rawClose: (id: number) => invoke<void>("raw_close", { id }),
  /** Au chargement de l'interface : libère les RAW d'une page précédente (webview rechargée). */
  rawCloseAll: () => invoke<void>("raw_close_all"),
  /** « Modifier le contenu » : ouvre le contenu comme un document à part (déjà ouvert : le même). */
  smartContentOpen: (docId: number, layer: number) => invoke<DocumentInfo>("smart_content_open", { docId, layer }),
  /** Ctrl+S d'un document `smartParent` : remplace le contenu dans le parent (toutes instances) ; renvoie le parent. */
  smartContentCommit: (docId: number) => invoke<EditResult>("smart_content_commit", { docId }),
  /** « Exporter le contenu… » : `.graf` = le document du contenu ; sinon l'image d'origine ou le fichier lié. */
  smartContentExport: (docId: number, layer: number, path: string) => invoke<void>("smart_content_export", { docId, layer, path }),
  /** Vérifie les fichiers liés : relit les modifiés, signale les introuvables. */
  smartLinksCheck: (docId: number) => invoke<SmartLinksResult>("smart_links_check", { docId }),
  saveProject: (docId: number, path: string, workSeconds?: number) => invoke<DocumentInfo>("save_project", { docId, path, workSeconds: workSeconds === undefined ? null : Math.floor(workSeconds) }),
  exportPng: (docId: number, path: string) => invoke<void>("export_png", { docId, path }),
  /** Encode l'export en mémoire : 16 octets (poids exact en u64, largeur, hauteur) puis une image affichable. */
  exportPreview: (docId: number, options: ExportOptions) => invoke<ArrayBuffer>("export_preview", { docId, options }),
  /** Aperçu instantané de l'export : vue d'ensemble + poids estimé, zone visible à pleine résolution, poids exact. */
  exportOverview: (docId: number, options: ExportOptions, maxSide: number) => invoke<ArrayBuffer>("export_overview", { docId, options, maxSide }),
  exportRegion: (docId: number, options: ExportOptions, rect: [number, number, number, number]) => invoke<ArrayBuffer>("export_region", { docId, options, rect }),
  exportExactSize: (docId: number, options: ExportOptions) => invoke<number>("export_exact_size", { docId, options }),
  exportPreviewRelease: () => invoke<void>("export_preview_release"),
  exportImage: (docId: number, path: string, options: ExportOptions) => invoke<void>("export_image", { docId, path, options }),
  /** Export d'un calque / groupe : voir lib/layer-export.ts. Réponse de l'aperçu : taille à 1× (u32 × 2), taille de la vue (u32 × 2), type (u8), image. */
  layerExportOverview: (docId: number, ids: number[], zone: "content" | "document" | "selection" | "artboard", margin: number, options: ExportOptions, maxSide: number) =>
    invoke<ArrayBuffer>("layer_export_overview", { docId, ids, zone, margin, options, maxSide }),
  exportLayers: (docId: number, ids: number[], zone: "content" | "document" | "selection" | "artboard", margin: number, options: ExportOptions, targets: { path: string; scale: number }[]) =>
    invoke<void>("export_layers", { docId, ids, zone, margin, options, targets }),
  copyLayersPng: (docId: number, ids: number[]) => invoke<void>("copy_layers_png", { docId, ids }),
  layerFileStem: (name: string) => invoke<string>("layer_file_stem", { name }),
  advancedSelectionOpen: (docId: number, layer: number | null, maxSide: number) => invoke<AdvancedInfo>("advanced_selection_open", { docId, layer, maxSide }),
  advancedSelectionImage: () => invoke<ArrayBuffer>("advanced_selection_image"),
  advancedSelectionPreview: (params: AdvancedParams) => invoke<ArrayBuffer>("advanced_selection_preview", { params }),
  advancedSelectionClose: () => invoke<void>("advanced_selection_close"),
  documentPreview: (docId: number, maxSide: number) => invoke<ArrayBuffer>("document_preview", { docId, maxSide }),
  projectTextStyles: (path: string) => invoke<CharStyle[]>("project_text_styles", { path }),
  closeDocument: (docId: number) => invoke<void>("close_document", { docId }),
  edit: (docId: number, op: EditOp) => invoke<EditResult>("edit", { docId, op }),
  strokeBegin: (docId: number, layer: number, onMask: boolean, brush: BrushParams) =>
    invoke<void>("stroke_begin", { docId, layer, onMask, brush }),
  strokePoints: (docId: number, points: number[][]) => invoke<[number, DirtyInfo]>("stroke_points", { docId, points }),
  /** Le dernier bout du trait (le lissage garde un demi-segment d'avance) est tracé ici. */
  strokeEnd: (docId: number) => invoke<EditResult>("stroke_end", { docId }),
};
