import { t } from "../lib/i18n.svelte";
// Affichage WebGL2 des tuiles composées par le moteur et du contour de sélection.
//
// Tout ce qui est purement visuel (zoom, déplacement, animation du contour) se fait
// ici, sans aucun aller-retour vers Rust : seules les tuiles absentes sont demandées.
import type { Adjustment, CmykSpace, DirtyInfo, DocumentInfo, LayerEffect } from "../lib/ipc";
import { type Camera, type ViewFrame, finishModeFor, levelForScale } from "./camera";
import type { Dab as StrokeDab } from "../lib/brush-path";
import type { DualTipStamp, TipStamp } from "../lib/tip-stamps";
import { loadPattern, loadTip, patternImage, tipImage } from "../lib/tip-images";
import { FINISH_SHADER, PRESENT_SHADER, PROOF_SHADER, VERTEX_SHADER } from "./shaders";
import { type EffectsFetcher, GpuLayers, type GpuStatus, type LayerFetcher, type TexturePart, relativeMatrix } from "./gpu-layers";
import { TransformPreviews } from "./transform-previews";
import { LivePaint, type LiveSpec } from "./live-paint";
import { GpuEffects } from "./gpu-effects";
import { type EffectsPreviewControls, provideEffectsPreview, withdrawEffectsPreview } from "../lib/effects-preview";
import { type Resolved, TileCache, type TileFetcher } from "./tile-cache";
import { MoveCache, type MoveCacheMode, type MoveCacheSource, type MoveView, moveShift } from "./move-cache";


/** Pixels, masque ou effets d'un calque pour la composition sur carte graphique (voir gpu-layers.ts). */
export type DocLayerFetcher = (docId: number, layer: number, rect: [number, number, number, number] | null, part: TexturePart, limit: [number, number] | null) => Promise<ArrayBuffer>;

/** Aperçu de déplacement ou d'opacité quand la carte graphique compose : rien à charger, seulement des décalages. */
interface GpuPreview {
  moving: Set<number>;
  /** Ce qui bouge des calques déplacés : pixels et masque ensemble, ou l'un des deux seulement. */
  parts: "both" | "pixels" | "mask";
  opaque: number | null;
  dx: number;
  dy: number;
  alpha: number;
  /**
   * Position de départ de chaque calque déplacé : l'aperçu montre chaque texture à `départ + décalage`, quelle que
   * soit la position à laquelle elle est posée. Quand le moteur applique le déplacement au relâchement, les textures
   * suivent la nouvelle position et le décalage restant tombe à zéro dans la même image : plus de double décalage
   * le temps que l'aperçu se termine (le calque sautait un instant deux fois plus loin).
   */
  start: Map<number, readonly [number, number]>;
  /** Le déplacement est parti au moteur (relâchement) ; `applied` : sa réponse est arrivée. */
  releasing: boolean;
  applied: boolean;
  /** Textures posées après la réponse du moteur : déjà à la bonne place, elles ne sont pas décalées. */
  fresh: WeakSet<object>;
}

/** `group` et tous ses descendants (identifiants), d'après la description du document. */
function withDescendantIds(doc: DocumentInfo, group: number): number[] {
  const out = [group];
  for (let i = 0; i < out.length; i++) for (const layer of doc.layers) if (layer.parent === out[i]) out.push(layer.id);
  return out;
}

/** Parties de l'image pendant un aperçu de déplacement : calques de pixels de chacune. */
export interface MovePlan {
  below: number[];
  moving: number[];
  above: number[];
}

export type PartFetcher = (layers: number[], opaque: number | null) => TileFetcher;

/**
 * Vue tournée (outil Rotation de l'affichage) : l'image est composée DROITE dans une texture hors écran de la taille du
 * cadre tourné (`frame`, voir camera.ts `rotatedFrame`), puis dessinée tournée dans le canevas, qui garde la taille de
 * la zone de travail (`viewW` × `viewH`, pixels CSS). `maxDpr` : rapport de pixels de l'écran sans réduction (la
 * texture est allouée à cette taille une fois pour toutes ; la résolution dynamique n'en utilise qu'une partie).
 */
export interface RotatedView {
  rotation: number;
  frame: ViewFrame;
  viewW: number;
  viewH: number;
  maxDpr: number;
}

interface MovePreview {
  below: TileCache;
  /** Absente pour l'aperçu d'un réglage (il n'y a rien à déplacer). */
  moving: TileCache | null;
  above: TileCache;
  /** Aperçu d'un réglage : table de correspondance 3D appliquée par la carte graphique à la couche du milieu
   *  (réglage écrêté : sa base), ou à « dessous » s'il n'y a pas de couche du milieu (réglage libre). */
  lut?: { texture: WebGLTexture; size: number; mix: number } | null;
  /** Aperçu d'un réglage (et non d'un déplacement ou d'une opacité). */
  adjusting?: boolean;
  /** Curseur relâché : l'aperçu cède la place dès que l'image rendue par le moteur est prête. */
  releasing?: boolean;
  /** Déplacement courant, en pixels du document. */
  dx: number;
  dy: number;
  /** Opacité appliquée à la couche du milieu (aperçu du curseur d'opacité) ; 1 pour un déplacement. */
  alpha: number;
}

// Sert aux tuiles d'image (RGBA) comme aux tuiles de sélection (canal rouge seul).
// u_solid >= 0 : tuile uniforme, aucune texture n'est lue.
const TILE_SHADER = `#version 300 es
precision mediump float;
precision mediump sampler3D;
uniform sampler2D u_tex;
uniform vec4 u_clamp;  // limites d'UV : ne jamais lire hors de la zone utile d'une tuile de bord
uniform float u_solid;
uniform float u_alpha; // opacité globale ; en alpha prémultiplié, elle multiplie les quatre canaux
// Aperçu d'un calque de réglage : table de correspondance 3D des couleurs (calculée par le moteur), mêlée à
// proportion de l'opacité du réglage. 0 = pas de réglage.
uniform sampler3D u_lut;
uniform float u_lutMix;
uniform float u_lutSize;
in vec2 v_uv;
out vec4 o_color;
void main() {
  vec4 c = u_solid >= 0.0 ? vec4(u_solid) : texture(u_tex, clamp(v_uv, u_clamp.xy, u_clamp.zw));
  if (u_lutMix > 0.0 && c.a > 0.0) {
    // Comme le moteur : le réglage agit sur la couleur non prémultipliée ; l'alpha ne change pas.
    vec3 rgb = clamp(c.rgb / c.a, 0.0, 1.0);
    vec3 adjusted = texture(u_lut, (rgb * (u_lutSize - 1.0) + 0.5) / u_lutSize).rgb;
    c.rgb = mix(rgb, adjusted, u_lutMix) * c.a;
  }
  o_color = c * u_alpha;
}`;

// Damier de transparence, calculé en pixels physiques : il ne bouge pas avec le zoom.
const CHECKER_SHADER = `#version 300 es
precision mediump float;
uniform float u_cell;
in vec2 v_uv;
out vec4 o_color;
void main() {
  vec2 c = floor(gl_FragCoord.xy / u_cell);
  float v = mod(c.x + c.y, 2.0) < 1.0 ? 1.0 : 0.8;
  o_color = vec4(v, v, v, 1.0);
}`;

// Contour de sélection animé. Le masque a d'abord été rendu à la taille de l'écran :
// un pixel est sur le contour si le seuil de 50 % est franchi avec l'un de ses voisins.
// Les jointures de tuiles et les bords du document sont ainsi traités sans cas particulier.
const ANTS_SHADER = `#version 300 es
precision mediump float;
uniform sampler2D u_mask;
uniform float u_time;
uniform float u_dash;
in vec2 v_uv;
out vec4 o_color;
float inside(ivec2 p) {
  // Hors de l'écran, le voisin vaut le pixel de bord : pas de faux contour le long de la fenêtre.
  ivec2 q = clamp(p, ivec2(0), textureSize(u_mask, 0) - 1);
  return step(0.5, texelFetch(u_mask, q, 0).r);
}
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  float here = inside(p);
  float around = inside(p + ivec2(1, 0)) + inside(p - ivec2(1, 0)) + inside(p + ivec2(0, 1)) + inside(p - ivec2(0, 1));
  // Le contour est tracé sur le côté intérieur de la frontière.
  if (here < 0.5 || around > 3.5) discard;
  float phase = mod((gl_FragCoord.x + gl_FragCoord.y) / u_dash + u_time, 2.0);
  o_color = vec4(vec3(phase < 1.0 ? 0.0 : 1.0), 1.0);
}`;

// Segment de trait en « gélule » (cercle balayé), même profil que le moteur (crates/engine/src/paint.rs) :
// plein jusqu'au rayon dur, décroissance en cosinus ensuite, un pixel d'antialiasing au bord.
const DAB_SHADER = `#version 300 es
precision highp float;
uniform vec3 u_a;        // extrémités : centre (pixels physiques, origine en haut à gauche) et rayon (pixels du document)
uniform vec3 u_b;
uniform float u_hardness;
uniform float u_deposit;   // 1 : part déposée par empreinte (accumulation) ; 0 : empreinte entière
uniform float u_scale;   // pixels physiques par pixel du document
uniform vec2 u_view;
in vec2 v_uv;
out vec4 o_color;
void main() {
  vec2 p = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y);
  vec2 ab = u_b.xy - u_a.xy;
  float len2 = dot(ab, ab);
  float t = len2 > 1e-6 ? clamp(dot(p - u_a.xy, ab) / len2, 0.0, 1.0) : 0.0;
  float d = distance(p, u_a.xy + ab * t) / u_scale;
  float radius = mix(u_a.z, u_b.z, t);
  float hard = radius * u_hardness;
  float edge = clamp(radius - d + 0.5, 0.0, 1.0);
  float value = edge;
  if (d > hard && radius - hard >= 1.0) {
    float s = clamp((d - hard) / (radius - hard), 0.0, 1.0);
    // Même profil que le moteur (soft_falloff) : cloche gaussienne exp(-(s / 0,55)^2), ramenée à zéro au bord.
    float u = s / 0.55;
    float tail = exp(-1.0 / (0.55 * 0.55));
    value = min(edge, max(0.0, (exp(-(u * u)) - tail) / (1.0 - tail)));
  }
  // Pinceau flou : chaque empreinte ne dépose qu'un tiers (SOFT_FLOW du moteur), et elles s'additionnent.
  o_color = vec4(u_deposit > 0.5 ? value * 0.33 : value);
}`;

// Trait en cours : couleur du pinceau pondérée par la couverture accumulée, posée sur l'image.
const STROKE_SHADER = `#version 300 es
precision mediump float;
uniform sampler2D u_coverage;
uniform vec4 u_ink;      // couleur RGB (0..1) et opacité du trait
in vec2 v_uv;
out vec4 o_color;
uniform highp vec2 u_view; // même précision que dans le vertex shader, sinon l'édition de liens échoue
void main() {
  // Lecture interpolée : la couverture peut être calculée à résolution réduite (très gros pinceaux).
  float a = texture(u_coverage, gl_FragCoord.xy / u_view).r * u_ink.a;
  o_color = vec4(u_ink.rgb * a, a);
}`;

// Pinceau en direct, pointe importée : une empreinte de la pointe (tournée, écrasée, retournée), posée dans la
// couverture du trait au maximum (flux 100 %) ou par-dessus (flux plus faible), comme `stamp_tip` dans le moteur. La
// pointe est lue dans sa version réduite la plus proche de sa taille à l'écran (niveaux de détail de la texture).
const TIP_DAB_SHADER = `#version 300 es
precision highp float;
uniform sampler2D u_tip;
uniform vec2 u_center;   // centre (pixels physiques, origine en haut à gauche)
uniform vec2 u_size;     // largeur, hauteur de la pointe à l'écran (pixels physiques)
uniform vec2 u_rot;      // cosinus, sinus de la rotation
uniform vec2 u_flip;     // 1 : retournée horizontalement, verticalement
uniform float u_value;   // force de l'empreinte (× flux)
uniform float u_lod;     // version réduite de la pointe, celle que lit le moteur (BrushTip::level_for)
uniform vec2 u_view;
in vec2 v_uv;
out vec4 o_color;
void main() {
  vec2 p = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y);
  vec2 d = p - u_center;
  vec2 uv = vec2((d.x * u_rot.x + d.y * u_rot.y) / u_size.x + 0.5, (d.y * u_rot.x - d.x * u_rot.y) / u_size.y + 0.5);
  if (u_flip.x > 0.5) uv.x = 1.0 - uv.x;
  if (u_flip.y > 0.5) uv.y = 1.0 - uv.y;
  float inside = step(0.0, uv.x) * step(uv.x, 1.0) * step(0.0, uv.y) * step(uv.y, 1.0);
  o_color = vec4(textureLod(u_tip, uv, u_lod).r * inside * u_value);
}`;

/**
 * Version réduite d'une pointe de `width` × `height` lue pour une empreinte de `side` pixels du document (plus grand
 * côté) : la plus petite qui garde au moins un pixel de pointe par pixel du trait — même choix que le moteur
 * (`BrushTip::level_for`, versions de moitié en moitié, arrondies au-dessus).
 */
export function tipLevel(width: number, height: number, side: number): number {
  let [w, h, level] = [width, height, 0];
  while (Math.max(w, h) > 2) {
    [w, h] = [Math.ceil(w / 2), Math.ceil(h / 2)];
    if (Math.max(w, h) < side) break;
    level += 1;
  }
  return level;
}

// Trait de pointe posé sur l'image : couverture combinée au pinceau double et à la texture (mêmes formules que
// `Stroke::effective` et `TipBlend::apply` dans le moteur), puis la couleur du pinceau.
const TIP_STROKE_SHADER = `#version 300 es
precision highp float;
uniform sampler2D u_coverage;
uniform sampler2D u_dual;
uniform sampler2D u_pattern;
uniform vec4 u_ink;
uniform int u_dualMode;     // -1 : pas de pinceau double
uniform int u_texMode;      // -1 : pas de texture
uniform vec4 u_tex;         // échelle, profondeur, luminosité (÷ 255), contraste (÷ 100)
uniform float u_texInvert;
uniform vec2 u_patternSize;
uniform vec3 u_camera;      // coin du document à l'écran, pixels physiques par pixel du document
uniform highp vec2 u_view;
in vec2 v_uv;
out vec4 o_color;
float tipBlend(int mode, float a, float b) {
  a = clamp(a, 0.0, 1.0);
  b = clamp(b, 0.0, 1.0);
  float o;
  if (mode == 0) o = a * b;
  else if (mode == 1) o = min(a, b);
  else if (mode == 2) o = a > 0.0 ? max(a, b) : 0.0;
  else if (mode == 3) o = a < 0.5 ? 2.0 * a * b : 1.0 - 2.0 * (1.0 - a) * (1.0 - b);
  else if (mode == 4) o = b >= 1.0 ? (a > 0.0 ? 1.0 : 0.0) : min(a / (1.0 - b), 1.0);
  else if (mode == 5) o = b <= 0.0 ? 0.0 : max(1.0 - (1.0 - a) / b, 0.0);
  else if (mode == 6) o = max(a + b - 1.0, 0.0);
  else if (mode == 7) o = a + b > 1.0 ? a : 0.0;
  else if (mode == 8) o = max(a - (1.0 - b), 0.0);
  else if (mode == 9 || mode == 10) o = min(clamp((a - (1.0 - b)) * 2.0, 0.0, 1.0), a);
  else o = a > 0.0 ? 1.0 - (1.0 - a) * (1.0 - b) : 0.0;
  return clamp(o, 0.0, 1.0);
}
void main() {
  vec2 uv = gl_FragCoord.xy / u_view;
  float c = texture(u_coverage, uv).r;
  if (c <= 0.0) { o_color = vec4(0.0); return; }
  if (u_dualMode >= 0) c = tipBlend(u_dualMode, c, texture(u_dual, uv).r);
  if (u_texMode >= 0) {
    // Le motif est fixé à la toile : repère du document.
    vec2 p = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y);
    vec2 cell = floor(((p - u_camera.xy) / u_camera.z) / max(u_tex.x, 0.01));
    float t = texelFetch(u_pattern, ivec2(mod(cell, u_patternSize)), 0).r;
    if (u_texInvert > 0.5) t = 1.0 - t;
    t = clamp((t + u_tex.z - 0.5) * (1.0 + u_tex.w) + 0.5, 0.0, 1.0);
    c += (tipBlend(u_texMode, c, t) - c) * clamp(u_tex.y, 0.0, 1.0);
  }
  float a = clamp(c, 0.0, 1.0) * u_ink.a;
  o_color = vec4(u_ink.rgb * a, a);
}`;

/** Codes des modes de combinaison du pinceau double et de la texture (ordre de `tipBlend` ci-dessus). */
const TIP_BLEND_CODES: Record<string, number> = { multiply: 0, darken: 1, lighten: 2, overlay: 3, colorDodge: 4, colorBurn: 5, linearBurn: 6, hardMix: 7, subtract: 8, height: 9, linearHeight: 10, screen: 11 };

/** Pinceau en direct d'une pointe importée (voir `beginTipStrokePreview`). */
export interface TipPreviewSpec {
  tip: number;
  /** Flux (0 à 1) : à 100 %, les empreintes se confondent (maximum) ; en dessous, elles s'additionnent. */
  flow: number;
  dual: { tip: number; mode: string } | null;
  texture: { pattern: number; scale: number; depth: number; mode: string; invert: boolean; brightness: number; contrast: number } | null;
}

interface TipPreview extends TipPreviewSpec {
  stamps: TipStamp[];
  duals: DualTipStamp[];
  dualDrawn: number;
}

export type { Dab as StrokeDab } from "../lib/brush-path";

/** Aperçu d'un trait de pinceau, dessiné par la carte graphique pendant que le moteur travaille en coulisse. */
interface StrokePreview {
  color: readonly [number, number, number];
  /** Opacité du trait multipliée par celle du calque. */
  alpha: number;
  hardness: number;
  dabs: StrokeDab[];
  /** Nombre d'empreintes déjà dessinées dans la texture de couverture. */
  drawn: number;
  /** Caméra et taille au moment du dernier dessin : si elles changent, tout est redessiné. */
  signature: string;
  /** Trait terminé : l'aperçu disparaît dès que les vraies tuiles ont pris place. */
  releasing: boolean;
  /** Empreintes accumulées plutôt que fondues au maximum (pinceau flou). */
  accumulate: boolean;
  /** Pointe importée : ses empreintes (calculées par lib/tip-stamps.ts, rejouées telles quelles par le moteur). */
  tip: TipPreview | null;
}

// Masque rapide : la sélection en cours de peinture, en rouge translucide par-dessus l'image.
const TINT_SHADER = `#version 300 es
precision mediump float;
uniform sampler2D u_mask;
in vec2 v_uv;
out vec4 o_color;
void main() {
  float a = texelFetch(u_mask, ivec2(gl_FragCoord.xy), 0).r * 0.5;
  o_color = vec4(1.0 * a, 0.12 * a, 0.16 * a, a);
}`;

interface Program {
  program: WebGLProgram;
  uniform: (name: string) => WebGLUniformLocation | null;
}

interface Frame {
  doc: DocumentInfo;
  level: number;
  span: number;
  scale: number;
  originX: number;
  originY: number;
  first: [number, number];
  last: [number, number];
}

export class ViewportRenderer {
  private readonly gl: WebGL2RenderingContext;
  private readonly tileProgram: Program;
  private readonly checkerProgram: Program;
  private readonly antsProgram: Program;
  private readonly nearest: WebGLSampler;
  private readonly linear: WebGLSampler;
  private readonly image: TileCache;
  private readonly selection: TileCache;
  private readonly maskTarget: WebGLTexture;
  private readonly maskFramebuffer: WebGLFramebuffer;
  private maskSize: [number, number] = [0, 0];
  private preview: MovePreview | null = null;
  /**
   * Niveau d'ébauche pendant un geste continu (curseur d'opacité, par exemple) quand l'image ne se
   * décompose pas en couches : les tuiles sont demandées un cran plus grossier, soit seize fois moins
   * de travail pour le moteur à chaque pas. Le rendu fin revient au relâchement.
   */
  private draft = 0;
  private stroke: StrokePreview | null = null;
  /**
   * Pinceau et gomme peints directement dans la texture du calque (voir live-paint.ts), créé au premier trait.
   * `liveLayer` : calque du trait en cours.
   */
  private livePaint: LivePaint | null = null;
  private liveLayer: number | null = null;
  private readonly dabProgram: Program;
  private readonly tintProgram: Program;
  private readonly proofProgram: Program;
  /** Copie de l'écran pour la passe d'épreuve CMJN (allouée à la taille du canevas, à la demande). */
  private proofTexture: { texture: WebGLTexture; width: number; height: number } | null = null;
  /**
   * Tables 3D sRGB → presse → sRGB de l'épreuve, par espace CMJN (calculées par le moteur, voir engine::cmyk) ;
   * chacune chargée à sa première épreuve puis gardée : repasser d'un espace à l'autre ne recharge rien.
   */
  private readonly proofLuts = new Map<string, { texture: WebGLTexture; size: number } | "loading" | "failed">();
  /** Épreuve CMJN : l'image affichée simule une impression quadri dans cet espace, `null` sans épreuve. */
  proofProfile: CmykSpace | null = null;
  /** Fournie par la vue : la table d'épreuve d'un espace (le moteur la calcule). */
  proofTables: ((space: CmykSpace) => Promise<ArrayBuffer>) | null = null;
  /** Masque rapide : la sélection s'affiche en rouge translucide au lieu du contour animé. */
  quickMask = false;
  private readonly strokeProgram: Program;
  private readonly coverageTarget: WebGLTexture;
  /** La carte sait dessiner (et mélanger) dans une texture en demi-flottant. */
  private readonly floatCoverage: boolean;
  private readonly coverageFramebuffer: WebGLFramebuffer;
  private coverageSize: [number, number] = [0, 0];
  /** Pinceau double du trait de pointe en direct : sa couverture, à la même taille que celle du trait. */
  private dualTarget: WebGLTexture | null = null;
  private dualSize: [number, number] = [0, 0];
  private readonly tipDabProgram: Program;
  private readonly tipStrokeProgram: Program;
  /** Pointes et motifs chargés sur la carte graphique, par identifiant. */
  private readonly tipTextures = new Map<number, WebGLTexture>();
  private readonly patternTextures = new Map<number, { texture: WebGLTexture; width: number; height: number }>();
  /** Attentes de `whenSettled` ; `layers` : seulement les textures de ces calques (sinon tout ce qui se charge). */
  private settledWaiters: { resolve: () => void; layers?: readonly number[] }[] = [];
  private doc: DocumentInfo | null = null;
  private frame = 0;
  private readonly gpu: GpuLayers;
  private gpuPreview: GpuPreview | null = null;
  /**
   * Déplacement par images en cache (move-cache.ts) : `moveCache` quand la carte graphique ne compose pas le document
   * (aperçu en couches), `collapseCache` quand elle le compose (aplat du groupe déplacé, voir `GpuLayers.setCollapse`).
   */
  private moveCache: MoveCache | null = null;
  private collapseCache: MoveCache | null = null;
  /**
   * Aplat du groupe gardé APRÈS le relâchement, tant que le groupe reste sélectionné et que rien d'autre que sa
   * position ne change (voir `keepCollapse`) : enchaîner les déplacements d'un groupe ne recharge ni ne recompose
   * ses calques à chaque relâchement — ils ne reprennent leur place, un par un, qu'une fois le groupe désélectionné.
   * `origin` : position du groupe quand l'aplat a été préparé (l'aplat est rangé relativement à elle) ; `layout` :
   * position de chaque calque du groupe relativement à lui, qui doit rester la même.
   */
  private collapseKept = false;
  /**
   * Même chose quand la carte graphique ne compose pas le document (images en cache « dessous / déplacé / dessus ») :
   * après le relâchement, les images restent affichées, la couche déplacée à sa nouvelle place, tant que la sélection
   * ne change pas et que rien d'autre ne bouge. Le moteur ne recompose l'image qu'une fois, quand on passe à autre
   * chose — et non plus à chaque relâchement. `moveOrigin` : position de chaque calque quand les images ont été
   * préparées ; `moveLead` : le calque déplacé qui sert de repère ; `gestureStart` : sa position au début du geste.
   */
  private moveKept = false;
  private moveOrigin = new Map<number, readonly [number, number]>();
  private moveLead: number | null = null;
  private moveMoved = new Set<number>();
  private gestureStart: readonly [number, number] | null = null;
  private collapseOrigin: readonly [number, number] | null = null;
  private collapseLayout = new Map<number, readonly [number, number]>();
  /** Décalage du déplacement en cours, pour les images en cache. */
  private moveOffset: [number, number] = [0, 0];
  /** Dernier déplacement par images en cache (indicateur de diagnostic) : voie, délai de préparation, détail. */
  private lastMove: MoveStatus | null = null;
  /** Dernière préparation refusée par le moteur (mode, document, révision, calques). */
  private refusedMove: string | null = null;
  /**
   * Groupe optimisé (Ctrl+T sur un groupe, voir `optimizeGroup`) : une image du groupe (son aplat, ou ses paquets pour
   * un groupe en Transfert) sert à tous ses déplacements, transformations et annulations, tant qu'il reste la sélection.
   * L'intention survit à l'image : vue changée, contenu retouché… l'image est refaite (`onReoptimize`).
   */
  private optimized: { docId: number; group: number } | null = null;
  /** Demande à la zone de travail de refaire l'image du groupe optimisé (elle seule connaît la vue). */
  onReoptimize: (() => void) | null = null;
  private reoptimizeAt = 0;
  /**
   * Transformation (Ctrl+T) du groupe optimisé montrée par ses images en couches : homographie courante (document
   * d'avant le geste → document) et décalage de la couche du groupe au début du geste. Les images montrent l'état
   * d'avant le geste : les étapes du moteur (au relâchement de chaque poignée) ne les retirent pas.
   */
  private cachedTransform: { gesture: string; current: readonly number[]; inside: { x: number; y: number }; shift: readonly [number, number] } | null = null;
  /** Transformation du groupe optimisé en cours (une étape du moteur est arrivée) : son image n'est pas refaite avant la fin. */
  private transformStep: string | null = null;
  /** Aperçu d'un réglage relâché : à retirer dès que la carte graphique a les valeurs définitives. */
  private gpuAdjustReleasing = false;
  /** Aperçu des effets de calque, enregistré auprès de la boîte de dialogue des effets (lib/effects-preview.ts). */
  private readonly effectsControls: EffectsPreviewControls;
  /** Redessin demandé : la composition de la vue tournée n'est plus à jour (voir `sceneStale`). */
  private readonly requestRedraw = (): void => {
    this.sceneStale = true;
    this.notifyRedraw();
  };
  /**
   * Aperçus de transformation en cours (un par geste ; celui d'un geste validé reste le temps du rendu final), chacun
   * lié au document de son geste : voir transform-previews.ts.
   */
  private readonly transforms = new TransformPreviews();
  /** Redessin demandé par le rendu lui-même (tuile ou texture arrivée…) : voir `requestRedraw`. */
  private readonly notifyRedraw: () => void;
  /**
   * Cible du dessin de l'image : l'écran (`null`), ou la texture droite de la vue tournée ; et sa taille utile en pixels.
   * Tout ce que le rendu « pose à l'écran » (damier, calques, trait, sélection, épreuve) va dans cette cible.
   */
  private target: WebGLFramebuffer | null = null;
  private targetSize: [number, number] = [1, 1];
  /** Texture droite de la vue tournée (allouée à la taille du cadre tourné au rapport de pixels de l'écran). */
  private rotated: { texture: WebGLTexture; framebuffer: WebGLFramebuffer; width: number; height: number } | null = null;
  private readonly presentProgram: Program;
  /** Mode Gris ou Bitmap : rendu final d'un aperçu en couches (voir `drawPreview`, FINISH_SHADER). */
  private readonly finishProgram: Program;
  /** Texture hors écran où les couches d'un aperçu sont assemblées avant ce rendu final (allouée à la demande). */
  private finishTarget: { texture: WebGLTexture; framebuffer: WebGLFramebuffer; width: number; height: number } | null = null;
  /**
   * Vue tournée : l'image droite de la dernière image reste valable tant que rien d'autre que l'angle n'a changé.
   * `sceneKey` : caméra, échelle et taille de la dernière composition ; `sceneStale` : quelque chose a demandé un
   * redessin depuis (tuile, texture, document…) ; `reuseNext` : la zone de travail signale que seul l'angle change
   * (glisser de l'outil R) — tourner ne recompose alors rien, seul le dessin final tourné est refait.
   */
  private sceneKey = "";
  /** Pixels de la cible par pixel CSS lors du dernier dessin (voir `isSelectedAt`). */
  private drawnScale = 1;
  private sceneStale = true;
  private reuseNext = false;

  constructor(
    canvas: HTMLCanvasElement,
    fetchImage: TileFetcher,
    fetchSelection: TileFetcher,
    private readonly fetchPart: PartFetcher,
    requestRedraw: () => void,
    private canvasColor: readonly [number, number, number],
    fetchLayer: DocLayerFetcher,
    fetchAdjust: (adjustment: Adjustment) => Promise<ArrayBuffer>,
    fetchEffects: (docId: number, layer: number) => ReturnType<EffectsFetcher>,
    private readonly moveSource: ((docId: number) => MoveCacheSource) | null = null,
  ) {
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: false });
    if (!gl) {
      throw new Error(t("WebGL2 indisponible : vérifiez le pilote graphique."));
    }
    this.gl = gl;
    this.notifyRedraw = requestRedraw;
    this.tileProgram = this.link(TILE_SHADER);
    this.presentProgram = this.link(PRESENT_SHADER);
    this.finishProgram = this.link(FINISH_SHADER);
    this.checkerProgram = this.link(CHECKER_SHADER);
    this.antsProgram = this.link(ANTS_SHADER);
    this.dabProgram = this.link(DAB_SHADER);
    this.tipDabProgram = this.link(TIP_DAB_SHADER);
    this.tipStrokeProgram = this.link(TIP_STROKE_SHADER);
    this.tintProgram = this.link(TINT_SHADER);
    this.proofProgram = this.link(PROOF_SHADER);
    this.strokeProgram = this.link(STROKE_SHADER);
    this.coverageTarget = gl.createTexture();
    this.floatCoverage = gl.getExtension("EXT_color_buffer_float") !== null;
    this.coverageFramebuffer = gl.createFramebuffer();
    this.nearest = this.sampler(gl.NEAREST);
    this.linear = this.sampler(gl.LINEAR);
    this.maskTarget = gl.createTexture();
    this.maskFramebuffer = gl.createFramebuffer();

    // 512 tuiles RGBA = 128 Mio de VRAM au plus ; les masques pèsent quatre fois moins.
    this.image = new TileCache(gl, fetchImage, (payload) => ({ texture: this.upload(payload, gl.RGBA8, gl.RGBA), uniform: 0 }), this.requestRedraw, 512);
    this.selection = new TileCache(
      gl,
      fetchSelection,
      // Premier octet : valeur de fond du masque, celle des tuiles sans données.
      (payload) => ({ texture: this.upload(payload.subarray(1), gl.R8, gl.RED), uniform: payload[0] ?? 0 }),
      this.requestRedraw,
      256,
    );

    const layerFetcher: LayerFetcher = (layer, rect, part, limit) => (this.doc ? fetchLayer(this.doc.id, layer, rect, part, limit) : Promise.reject(new Error(t("aucun document"))));
    const effectsFetcher: EffectsFetcher = (layer) => (this.doc ? fetchEffects(this.doc.id, layer) : Promise.reject(new Error(t("aucun document"))));
    this.gpu = new GpuLayers(gl, (fragment) => this.link(fragment), layerFetcher, fetchAdjust, effectsFetcher, this.requestRedraw);
    this.gpu.onTextureCommitted = (id, texture) => {
      this.transforms.committed(id, texture);
      // Après la réponse du moteur au relâchement d'un déplacement : texture déjà à sa place (voir `GpuPreview`).
      if (this.gpuPreview?.applied) this.gpuPreview.fresh.add(texture);
    };
    // Effets de calque calculés ici pendant un réglage. Un pilote qui refuse ces shaders laisse le moteur s'en charger.
    try {
      this.gpu.effectsCompute = new GpuEffects(gl, (fragment) => this.link(fragment));
    } catch (error) {
      console.error("Effets de calque sur carte graphique indisponibles", error);
    }
    this.effectsControls = {
      preview: (layer, effects) => this.previewEffects(layer, effects),
      finish: (layer) => this.finishEffectsPreview(layer),
      cancel: (layer) => this.endEffectsPreview(layer),
      hold: (layer) => this.gpu.holdEffectsSource(layer),
    };
    provideEffectsPreview(this.effectsControls);

    gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    // Les tuiles sont en alpha prémultiplié.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  }

  setDocument(doc: DocumentInfo | null): void {
    // Autre document, ou même document recadré : la grille de tuiles n'est plus la même.
    const resized = doc !== null && this.doc !== null && (doc.width !== this.doc.width || doc.height !== this.doc.height);
    if (doc?.id !== this.doc?.id || resized) {
      this.releaseHold();
      this.stroke = null;
      this.image.hold = false;
      this.image.reset();
      this.selection.reset();
      this.endMovePreview();
    } else if (doc && this.collapseKept && !this.collapseIntact(doc)) {
      this.dropCollapse();
    }
    // (Transformation du groupe optimisé en cours : ses images montrent l'état d'avant le geste, quoi qu'aient fait les
    // étapes du moteur depuis.)
    if (doc && this.moveKept && doc.id === this.doc?.id && !this.transformingOptimized() && !this.moveIntact(doc)) this.dropMove();
    // Autre document affiché : l'optimisation de celui d'avant prend fin.
    if (doc && this.optimized && this.optimized.docId !== doc.id) this.releaseOptimized();
    // Même description qu'à l'image précédente (la zone de travail la redonne à chaque mouvement de la caméra) : la
    // composition de la vue tournée reste valable.
    if (doc !== this.doc) this.sceneStale = true;
    // Déplacement relâché : la description qui place les calques déplacés ailleurs qu'au départ est la réponse du
    // moteur (voir `GpuPreview.applied`).
    const preview = this.gpuPreview;
    if (preview?.releasing && !preview.applied && doc) {
      preview.applied = doc.layers.some((l) => {
        const start = preview.start.get(l.id);
        return !!start && (start[0] !== l.offset[0] || start[1] !== l.offset[1]);
      });
    }
    this.doc = doc;
    this.gpu.setDocument(doc);
    // Aperçus de transformation : seuls ceux de ce document s'appliquent (ceux d'un autre onglet attendent son retour) ;
    // calques disparus : leurs aperçus aussi.
    this.transforms.setDocument(doc);
  }

  /** Préférences : composition sur carte graphique permise ou non, et mémoire vidéo réservée aux calques. */
  setGpuOptions(enabled: boolean, budgetMb: number): void {
    const changed = this.gpu.enabled !== enabled;
    this.gpu.enabled = enabled;
    this.gpu.budgetBytes = budgetMb * 1024 * 1024;
    if (changed) {
      if (!enabled) this.gpu.reset();
      this.gpu.setDocument(this.doc);
      this.requestRedraw();
    }
  }


  /** Fenêtre native active : cet affichage WebGL cesse de dessiner (c'est ce qui libère la webview). */
  suspended = false;

  gpuStatus(): GpuStatus & { move: MoveStatus | null } {
    const status = this.gpu.status();
    // Images en cache d'un déplacement : la composition passe par elles pendant le geste.
    const reason = this.moveCache?.ready ? null : status.reason;
    const live = this.moveCache ?? this.collapseCache;
    return { ...status, active: status.active || !!this.moveCache?.ready, reason, move: live ? moveStatus(live) : this.lastMove };
  }

  /** Couleur du fond de la zone de travail (thème, Préférences › Apparence). */
  setCanvasColor(color: readonly [number, number, number]): void {
    this.canvasColor = color;
    this.requestRedraw();
  }

  /** Chargement des calques en cours (jauge de la barre d'état), ou `null` au repos. */
  loadProgress(): { done: number; total: number } | null {
    return this.gpu.progress();
  }

  /** Le moteur signale ce qui a changé : les tuiles concernées seront rechargées au prochain rendu. */
  invalidate(dirty: DirtyInfo, revision: number, immediate: boolean): void {
    const doc = this.doc;
    if (!doc) return;
    this.sceneStale = true;
    if (dirty.all) this.image.invalidateAll(revision);
    else this.image.invalidate(dirty.tiles, doc.maxLevel, revision, immediate);
    if (dirty.selection) this.selection.invalidateAll(revision);
    // Première modification arrivée après le relâchement d'un déplacement : c'est lui (la file du document les tient
    // dans l'ordre). Masque déplacé seul : les positions ne changent pas, c'est son contenu qui est repeint.
    if (this.gpuPreview?.releasing) this.gpuPreview.applied = true;
    // Aplat gardé : le moindre pixel repeint (ou une structure changée) peut le concerner, il cède la place.
    // Déplacement (ou son annulation) d'un groupe tenu : seuls ses remplissages sont « repeints » (ils couvrent toujours
    // le document) ; l'aperçu et la suspension des chargements tiennent. Toute autre retouche les lève.
    const held = this.held;
    const pureMove = !!held && !dirty.all && dirty.repainted !== undefined && dirty.repainted.every((id) => held.all.has(id) && !!doc.layers.find((l) => l.id === id)?.fillLayer);
    // Transformation (Ctrl+T) du groupe optimisé montrée par ses images : les étapes du moteur (au relâchement de
    // chaque poignée) les laissent en place ; elles montrent l'état d'avant le geste, transformé par le cadre.
    if (!pureMove && !this.transformingOptimized() && (dirty.all || dirty.repainted === undefined || dirty.repainted.length > 0)) {
      if (this.collapseKept) this.dropCollapse();
      if (this.moveKept) this.dropMove();
      if (held) this.releaseHold();
      // Groupe optimisé retouché : son image est refaite, d'après le document à jour.
      this.requestReoptimize();
      // Image préparée d'avance (groupe sélectionné, pas encore déplacé) : son contenu a pu changer, elle est refaite.
      if (this.collapseCache && !this.gpuPreview) this.dropCollapse();
      if (this.moveCache && !this.gestureStart && !this.moveKept) this.dropMove();
    }
    this.gpu.invalidate(dirty);
  }

  /**
   * Aperçu de déplacement : l'image est affichée en trois couches (dessous, déplacé, dessus)
   * et seule la couche déplacée est décalée. Le geste ne demande alors plus rien au moteur
   * jusqu'au relâchement : le calque suit la souris à la fréquence de l'écran.
   */
  beginMovePreview(plan: MovePlan, opaque: number | null = null): void {
    this.endMovePreview();
    // La carte graphique compose : décaler ou atténuer les calques concernés suffit, rien n'est chargé.
    if (this.gpu.ready()) {
      this.gpuPreview = this.newGpuPreview(plan.moving, "both", opaque);
      this.gpu.setLive(opaque);
      return;
    }
    const part = (layers: number[], full: number | null) =>
      new TileCache(this.gl, this.fetchPart(layers, full), (payload) => ({ texture: this.upload(payload, this.gl.RGBA8, this.gl.RGBA), uniform: 0 }), this.requestRedraw, 256);
    this.preview = { below: part(plan.below, null), moving: part(plan.moving, opaque), above: part(plan.above, null), dx: 0, dy: 0, alpha: 1 };
  }

  private newGpuPreview(ids: Iterable<number>, parts: GpuPreview["parts"], opaque: number | null): GpuPreview {
    // Un remplissage d'un groupe déplacé couvre toujours le document : décalé, il laisserait une bande vide au bord.
    // Il reste en place pendant le geste (une couleur unie est de toute façon partout, voir `solidFillColor`) ; le moteur le recalcule.
    const all = new Set(ids);
    const inMovedGroup = (id: number) => {
      const layer = this.doc?.layers.find((l) => l.id === id);
      return !!layer?.fillLayer && layer.parent != null && all.has(layer.parent);
    };
    const moving = new Set([...all].filter((id) => opaque === id || !inMovedGroup(id)));
    const start = new Map<number, readonly [number, number]>();
    for (const layer of this.doc?.layers ?? []) if (moving.has(layer.id)) start.set(layer.id, [layer.offset[0], layer.offset[1]]);
    return { moving, parts, opaque, dx: 0, dy: 0, alpha: 1, start, releasing: false, applied: false, fresh: new WeakSet() };
  }

  /** Un aperçu est en cours (déplacement, opacité, réglage, trait, ébauche) : l'image change à chaque instant. */
  previewing(): boolean {
    // Images de déplacement gardées hors geste : l'image est stable (pas de résolution réduite).
    const moving = this.moveCache !== null && !(this.moveKept && !this.gestureStart);
    return this.preview !== null || this.gpuPreview !== null || moving || this.stroke !== null || this.draft > 0;
  }

  /** Vrai si la carte graphique compose tout le document en ce moment : les aperçus n'ont alors besoin de rien d'autre. */
  gpuComposes(): boolean {
    return this.gpu.ready() && !this.preview;
  }

  /**
   * Aperçu de déplacement composé par la carte graphique : `ids` (groupes et descendants compris) sont décalés,
   * pixels et masques, ou seulement l'un des deux. Faux si la carte graphique ne compose pas le document.
   */
  beginGpuMovePreview(ids: readonly number[], parts: GpuPreview["parts"]): boolean {
    if (!this.gpuComposes()) return false;
    // Aplat gardé : préparé dès le clic pour ce geste, ou gardé depuis le déplacement précédent du même groupe.
    const [collapse, origin, layout] = [this.collapseCache, this.collapseOrigin, new Map(this.collapseLayout)];
    this.collapseCache = null;
    this.endMovePreview();
    [this.collapseCache, this.collapseOrigin, this.collapseLayout] = [collapse, origin, layout];
    this.gpuPreview = this.newGpuPreview(ids, parts, null);
    this.applyCollapse();
    return true;
  }

  // ---- Déplacement par images en cache (move-cache.ts). Préparé dès le clic, avant même la sélection automatique.

  /**
   * Prépare les images d'un déplacement de `ids` (document `docId`) pour la vue `view` : aperçu en couches si la carte
   * graphique ne compose pas le document, sinon (groupe seul) son aplat. Une préparation identique en cours est
   * gardée. Retourne l'objet de la préparation, ou `null` si rien n'est à préparer.
   */
  prepareMove(docId: number, ids: readonly number[], view: MoveView, force: MoveCacheMode | null = null): MoveCache | null {
    const doc = this.doc;
    if (!this.moveSource || !doc || doc.id !== docId || ids.length === 0) return null;
    // Groupe optimisé : son image décide (aplat ou couches) ; un geste sur lui la reprend telle quelle.
    if (!force && this.isOptimized(ids)) {
      const kept = this.collapseCache && this.collapseKept && sameIds(this.collapseCache.ids, ids) ? this.collapseCache : this.moveCache && sameIds(this.moveCache.ids, ids) ? this.moveCache : null;
      if (kept && kept.state !== "failed") return kept;
    }
    // La carte graphique compose le document : elle décale elle-même les calques déplacés, à chaque image, sans rien
    // préparer (ni aplat, ni couches). Les images en cache ne servent plus que de secours, quand elle ne compose pas.
    if (!force && this.gpuComposes()) return null;
    const mode: MoveCacheMode = force ?? "layers";
    // Aplat : seulement pour un groupe seul (sinon les calques déplacés sont déjà chacun une texture).
    if (mode === "collapse" && !(ids.length === 1 && doc.layers.some((l) => l.id === ids[0] && l.isGroup))) return null;
    // Groupe avec un remplissage : pas d'aplat, qui figerait le remplissage là où il était (une bande vide apparaissait
    // au bord du document pendant le geste). La carte graphique décale chaque calque ; une couleur unie, elle, couvre
    // toujours tout le document (voir `solidFillColor`).
    if (mode === "collapse" && withDescendantIds(doc, ids[0]!).some((id) => doc.layers.find((l) => l.id === id)?.fillLayer)) return null;
    // Couches préparées avant que la carte graphique compose le document (chargement des textures) : périmées, elles
    // ne serviraient plus et gardent de la mémoire vidéo. Rendues, sauf pendant un geste ou gardées après lui.
    if (mode === "collapse" && this.moveCache && !this.moveKept && !this.gestureStart) this.dropMove();
    // Préparation déjà refusée par le moteur pour ces calques, document inchangé : pas redemandée à chaque clic.
    const key = `${mode}:${docId}:${doc.revision}:${ids.join(",")}`;
    if (this.refusedMove === key) return null;
    const current = mode === "collapse" ? this.collapseCache : this.moveCache;
    // Image encore en préparation : gardée (la relancer à chaque demande l'empêchait d'aboutir) ; une fois prête, si
    // elle ne couvre pas la vue, `drawScene` la remplace.
    const preparing = current?.state === "pending";
    const fits = preparing || (mode === "layers" ? !this.moveKept || this.moveCovers(view) : !this.collapseKept || this.collapseCovers(view));
    // L'autre genre d'image, préparé pour ce groupe optimisé avant que la voie change : il cède la place.
    if (this.isOptimized(ids)) {
      if (mode === "layers" && this.collapseCache) this.dropCollapse();
      if (mode === "collapse" && this.moveCache && !this.gestureStart) this.dropMove();
    }
    if (current && fits && current.docId === docId && current.mode === mode && current.state !== "failed" && sameIds(current.ids, ids)) return current;
    if (current === this.collapseCache && current) this.dropCollapse();
    else if (current === this.moveCache && current) this.dropMove();
    else current?.dispose();
    const size = { width: doc.width, height: doc.height, maxLevel: doc.maxLevel };
    // Budget. Aplat (la carte graphique compose déjà) : ce que les calques laissent de la mémoire réservée. Couches :
    // la moitié de la mémoire réservée, quoi qu'occupent les textures déjà chargées — quand la carte graphique ne
    // compose pas, c'est le plus souvent qu'elles la remplissent, et la réserve n'est elle-même qu'une part de la
    // mémoire vidéo (au pire 1,5 fois la réserve en tout).
    const budget = () => (mode === "layers" ? this.gpu.budgetBytes / 2 : Math.max(0, this.gpu.budgetBytes - this.gpu.status().memoryMb * 1048576));
    const cache = new MoveCache(this.gl, this.moveSource(docId), docId, [...ids], size, view, budget, mode, () => {
      this.lastMove = moveStatus(cache);
      // Refus du moteur seulement (la mémoire vidéo, elle, dépend de la vue).
      if (cache.state === "failed" && cache.layout === null && cache.reason !== "abandonné") this.refusedMove = key;
      // Groupe optimisé que la carte graphique ne peut pas aplatir (Transfert aux modes variés, remplissage…) : ses
      // images en couches (paquets) prennent le relais.
      if (cache.state === "failed" && cache.mode === "collapse" && cache.reason !== "abandonné" && this.isOptimized(cache.ids)) this.requestReoptimize(true);
      if (cache === this.collapseCache) this.applyCollapse();
      this.requestRedraw();
    });
    if (mode === "collapse") {
      this.collapseCache = cache;
      this.collapseKept = false;
      this.collapseOrigin = null;
      this.collapseLayout.clear();
      const group = doc.layers.find((l) => l.id === ids[0]);
      if (group) {
        this.collapseOrigin = [group.offset[0], group.offset[1]];
        for (const id of withDescendantIds(doc, group.id)) {
          const layer = doc.layers.find((l) => l.id === id);
          if (layer) this.collapseLayout.set(id, [layer.offset[0] - group.offset[0], layer.offset[1] - group.offset[1]]);
        }
      }
    } else {
      this.moveCache = cache;
      this.moveKept = false;
      this.moveOrigin = new Map(doc.layers.map((l) => [l.id, [l.offset[0], l.offset[1]] as const]));
      this.moveMoved = new Set(ids.flatMap((id) => withDescendantIds(doc, id)));
      this.moveLead = ids[0] ?? null;
    }
    this.lastMove = moveStatus(cache);
    return cache;
  }

  /** Préparation en couches du geste en cours (voir `prepareMove`), s'il y en a une. */
  get cachedMove(): MoveCache | null {
    return this.moveCache;
  }

  // ---- Groupe optimisé (Ctrl+T sur un groupe) : une image du groupe, gardée tant qu'il reste la sélection.

  /** Groupe optimisé, s'il y en a un (document affiché). */
  get optimizedGroup(): number | null {
    return this.optimized && this.optimized.docId === this.doc?.id ? this.optimized.group : null;
  }

  /** Vrai si `ids` est exactement le groupe optimisé. */
  private isOptimized(ids: readonly number[]): boolean {
    const o = this.optimized;
    return !!o && o.docId === this.doc?.id && ids.length === 1 && ids[0] === o.group;
  }

  /** Voie de l'image du groupe optimisé `ids` : aplat (la carte graphique compose), couches (paquets), ou aucune. */
  optimizedMode(ids: readonly number[]): MoveCacheMode | null {
    if (!this.isOptimized(ids)) return null;
    if (this.collapseKept && this.collapseCache && sameIds(this.collapseCache.ids, ids) && this.collapseCache.state !== "failed") return "collapse";
    if (this.moveCache && sameIds(this.moveCache.ids, ids) && this.moveCache.state !== "failed") return "layers";
    return null;
  }

  /** État de l'image du groupe optimisé `ids`, pour la pastille : en préparation, prête, ou pas d'optimisation. */
  optimizedState(ids: readonly number[]): "pending" | "ready" | null {
    if (!this.isOptimized(ids)) return null;
    const cache = this.optimizedMode(ids) === "collapse" ? this.collapseCache : this.optimizedMode(ids) === "layers" ? this.moveCache : null;
    // Une image affinée (la grossière d'abord, puis la fine) repasse un instant « en cours » : elle compte encore.
    return cache && (cache.ready || cache.level !== null) ? "ready" : "pending";
  }

  /**
   * Ctrl+T sur un groupe : son image est préparée pour la vue `view` — l'aplat si la carte graphique compose le
   * document et que le groupe s'y prête, sinon ses images en couches (« dessous / groupe / dessus » ; un groupe en
   * Transfert aux modes variés : ses paquets, voir crates/engine/src/move_preview.rs). Elle sert ensuite à tout :
   * déplacements, relâchements, annulations, transformation du cadre. `releaseOptimized` y met fin.
   */
  optimizeGroup(docId: number, group: number, view: MoveView): MoveCache | null {
    const doc = this.doc;
    if (!doc || doc.id !== docId || !doc.layers.some((l) => l.id === group && l.isGroup)) return null;
    if (!this.isOptimized([group])) {
      this.releaseOptimized();
      this.optimized = { docId, group };
    }
    return this.reoptimize(view);
  }

  /** Refait (ou garde, si elle convient) l'image du groupe optimisé pour la vue `view`. */
  reoptimize(view: MoveView): MoveCache | null {
    const o = this.optimized;
    const doc = this.doc;
    if (!o || !doc || doc.id !== o.docId) return null;
    const ids = [o.group];
    // Tant que le groupe est optimisé, son image le remplace : les textures de ses calques ne sont pas rechargées (le
    // moteur, déjà occupé à préparer cette image ou à rendre une transformation, ne les recalcule pas en même temps).
    // Elles reprennent une fois, quand la sélection change (`releaseOptimized`).
    const all = new Set(withDescendantIds(doc, o.group));
    this.held = { ids, all };
    this.gpu.holdLoads(all);
    const hasFill = [...all].some((id) => doc.layers.find((l) => l.id === id)?.fillLayer);
    const flatKey = `${o.docId}:${doc.revision}:${o.group}`;
    if (this.gpuComposes() && !hasFill && this.flatRefused !== flatKey) {
      const cache = this.prepareMove(o.docId, ids, view, "collapse");
      if (cache) {
        this.collapseKept = true;
        this.applyCollapse();
        this.requestRedraw();
        return cache;
      }
    }
    const cache = this.prepareMove(o.docId, ids, view, "layers");
    if (cache) {
      this.moveKept = true;
      this.requestRedraw();
    }
    return cache;
  }

  /** Fin de l'optimisation (autre sélection, autre document) : les calques du groupe reprennent, rechargés une fois. */
  releaseOptimized(): void {
    if (!this.optimized) return;
    this.optimized = null;
    this.cachedTransform = null;
    this.transformStep = null;
    this.flatRefused = null;
    this.releaseHold();
    if (this.collapseCache && !this.gpuPreview) this.dropCollapse();
    if (this.moveCache && !this.gestureStart) this.dropMove();
    this.requestRedraw();
  }

  /** Aplat refusé par le moteur pour le groupe optimisé (document, révision, groupe) : ses couches prennent le relais. */
  private flatRefused: string | null = null;

  /** Image du groupe optimisé à refaire (perdue, ou plus à la mesure de la vue) : au plus quatre fois par seconde. */
  private requestReoptimize(flatFailed = false): void {
    const o = this.optimized;
    // Pendant un geste, ou une transformation en cours (son image porte l'état d'avant) : refaite à la fin. Sauf
    // l'aplat refusé : sans le relais des couches, l'image resterait « en préparation » toute la transformation.
    if (!o || !this.onReoptimize || this.gestureStart) return;
    if (!flatFailed && (this.cachedTransform || this.transformStep)) return;
    if (flatFailed && this.doc) this.flatRefused = `${o.docId}:${this.doc.revision}:${o.group}`;
    const now = performance.now();
    if (!flatFailed && now - this.reoptimizeAt < 250) return;
    this.reoptimizeAt = now;
    setTimeout(() => this.onReoptimize?.(), 0);
  }

  /** Une transformation (Ctrl+T) du groupe optimisé est montrée par ses images : rien ne les retire avant sa fin. */
  private transformingOptimized(): boolean {
    return this.cachedTransform !== null && !!this.optimized;
  }

  /** Abandonne la préparation en couches (geste passé par une autre voie). */
  dropMoveCache(): void {
    this.dropMove();
  }

  /** Fin des images en cache gardées (ou abandon) : l'image du moteur reprend, recomposée une fois. */
  dropMove(): void {
    this.moveKept = false;
    this.moveCache?.dispose();
    this.moveCache = null;
    this.moveOrigin.clear();
    this.moveMoved.clear();
    this.moveLead = null;
    this.gestureStart = null;
    this.moveOffset = [0, 0];
    this.requestRedraw();
  }

  /** Calques sélectionnés dont les images de déplacement sont gardées après le relâchement, s'il y en a. */
  get keptMove(): readonly number[] | null {
    return this.moveKept ? (this.moveCache?.ids ?? null) : null;
  }

  /** Images gardées : les calques déplacés ont tous bougé d'autant que le repère, les autres pas du tout. */
  private moveIntact(doc: DocumentInfo): boolean {
    const delta = this.leadDelta(doc);
    if (!delta) return false;
    if (doc.layers.length !== this.moveOrigin.size) return false;
    return doc.layers.every((l) => {
      const at = this.moveOrigin.get(l.id);
      if (!at) return false;
      const [dx, dy] = this.moveMoved.has(l.id) ? delta : [0, 0];
      return l.offset[0] - at[0] === dx && l.offset[1] - at[1] === dy;
    });
  }

  /** Déplacement du calque repère depuis la préparation des images, d'après la description `doc`. */
  private leadDelta(doc: DocumentInfo): readonly [number, number] | null {
    const lead = doc.layers.find((l) => l.id === this.moveLead);
    const at = this.moveLead === null ? undefined : this.moveOrigin.get(this.moveLead);
    return lead && at ? [lead.offset[0] - at[0], lead.offset[1] - at[1]] : null;
  }

  /**
   * Décalage de la couche déplacée à l'écran : depuis la préparation des images, plus le geste en cours tant que le
   * moteur ne l'a pas appliqué (même règle que `moveShift` : pas de double décalage quand sa réponse arrive).
   */
  private cachedShift(doc: DocumentInfo): readonly [number, number] {
    const lead = doc.layers.find((l) => l.id === this.moveLead);
    const at = this.moveLead === null ? undefined : this.moveOrigin.get(this.moveLead);
    if (!lead || !at) return this.moveOffset;
    const start = this.gestureStart;
    if (start && lead.offset[0] === start[0] && lead.offset[1] === start[1]) return [start[0] - at[0] + this.moveOffset[0], start[1] - at[1] + this.moveOffset[1]];
    return [lead.offset[0] - at[0], lead.offset[1] - at[1]];
  }

  /** Images gardées suffisantes pour la vue : même niveau de détail, et la vue dans ce qu'elles couvrent. */
  private moveCovers(view: MoveView): boolean {
    const cache = this.moveCache;
    const doc = this.doc;
    if (!cache?.ready || !cache.slices || !doc || !(cache.layout?.exact || this.isOptimized(cache.ids))) return false;
    // Niveau de l'écran, ou celui d'au-dessus : l'image grossière arrive d'abord, la fine la remplace (voir `MoveCache`).
    const screen = Math.min(doc.maxLevel, levelForScale(view.scale, doc.maxLevel, doc.colorMode?.kind === "bitmap"));
    if (Math.abs((cache.level ?? 0) - screen) > 1) return false;
    const [vx0, vy0] = [Math.max(0, -view.originX / view.scale), Math.max(0, -view.originY / view.scale)];
    const [vx1, vy1] = [Math.min(doc.width, vx0 + view.viewW / view.scale), Math.min(doc.height, vy0 + view.viewH / view.scale)];
    const step = 2 ** (cache.level ?? 0);
    const [sx, sy] = this.cachedShift(doc);
    const bounds = cache.layout?.bounds ?? null;
    return cache.slices.every((slice) => {
      if (!slice.texture) return true;
      const [x, y, w, h] = slice.rect;
      if (!slice.moving) return x <= vx0 + step && y <= vy0 + step && x + w >= vx1 - step && y + h >= vy1 - step;
      if (!bounds) return false;
      // Partie visible du contenu déplacé, à sa place actuelle.
      const [nx0, ny0, nx1, ny1] = [Math.max(vx0, bounds[0]! + sx), Math.max(vy0, bounds[1]! + sy), Math.min(vx1, bounds[2]! + sx), Math.min(vy1, bounds[3]! + sy)];
      if (nx0 >= nx1 || ny0 >= ny1) return true;
      return x + sx <= nx0 + step && y + sy <= ny0 + step && x + sx + w >= nx1 - step && y + sy + h >= ny1 - step;
    });
  }

  /** Aplat du groupe déplacé prêt et aperçu GPU en cours pour ce groupe : il remplace son sous-programme. */
  private applyCollapse(): void {
    const cache = this.collapseCache;
    const preview = this.gpuPreview;
    const group = cache?.ids[0];
    const moving = !!preview && group !== undefined && preview.moving.has(group) && !preview.releasing;
    const ok = !!cache?.ready && group !== undefined && cache.docId === this.doc?.id && (moving || this.collapseKept);
    this.gpu.setCollapse(ok ? group! : null, ok ? (cache!.slices?.[0] ?? null) : null, this.collapseOrigin);
  }

  /** Aplat gardé : le groupe et ses calques sont toujours là, aux mêmes places les uns par rapport aux autres. */
  private collapseIntact(doc: DocumentInfo): boolean {
    const group = this.collapseCache?.ids[0];
    const layer = group === undefined ? undefined : doc.layers.find((l) => l.id === group);
    if (!layer?.isGroup || !this.collapseOrigin) return false;
    const now = withDescendantIds(doc, layer.id);
    if (now.length !== this.collapseLayout.size) return false;
    return now.every((id) => {
      const at = this.collapseLayout.get(id);
      const child = doc.layers.find((l) => l.id === id);
      return !!at && !!child && child.offset[0] - layer.offset[0] === at[0] && child.offset[1] - layer.offset[1] === at[1];
    });
  }

  /**
   * L'aplat gardé suffit à la vue `view` : préparé au niveau de détail de l'écran, et couvrant toute la partie
   * visible du groupe à sa position actuelle (il ne couvre que la vue du moment où il a été préparé, plus une marge).
   */
  private collapseCovers(view: MoveView): boolean {
    const cache = this.collapseCache;
    const doc = this.doc;
    const slice = cache?.slices?.[0];
    const layout = cache?.layout;
    const group = doc?.layers.find((l) => l.id === cache?.ids[0]);
    if (!cache?.ready || !doc || !slice || !layout?.bounds || !group || !this.collapseOrigin) return false;
    // Niveau de l'écran, ou celui d'au-dessus : l'image grossière arrive d'abord, la fine la remplace (voir `MoveCache`).
    const screen = Math.min(doc.maxLevel, levelForScale(view.scale, doc.maxLevel, doc.colorMode?.kind === "bitmap"));
    if (Math.abs((cache.level ?? 0) - screen) > 1) return false;
    const [dx, dy] = [group.offset[0] - this.collapseOrigin[0], group.offset[1] - this.collapseOrigin[1]];
    const [bx0, by0, bx1, by1] = [layout.bounds[0]! + dx, layout.bounds[1]! + dy, layout.bounds[2]! + dx, layout.bounds[3]! + dy];
    const [vx0, vy0] = [-view.originX / view.scale, -view.originY / view.scale];
    const [vx1, vy1] = [vx0 + view.viewW / view.scale, vy0 + view.viewH / view.scale];
    const [nx0, ny0, nx1, ny1] = [Math.max(bx0, vx0, 0), Math.max(by0, vy0, 0), Math.min(bx1, vx1, doc.width), Math.min(by1, vy1, doc.height)];
    if (nx0 >= nx1 || ny0 >= ny1) return true; // rien du groupe à l'écran
    const [x, y, w, h] = slice.rect;
    const step = 2 ** (cache.level ?? 0);
    return x + dx <= nx0 + step && y + dy <= ny0 + step && x + dx + w >= nx1 - step && y + dy + h >= ny1 - step;
  }

  /** Fin de l'aplat gardé (groupe désélectionné, contenu changé, vue qu'il ne couvre plus) : les calques reprennent. */
  dropCollapse(): void {
    const had = this.collapseCache !== null;
    this.collapseKept = false;
    this.collapseCache?.dispose();
    this.collapseCache = null;
    this.collapseOrigin = null;
    this.collapseLayout.clear();
    this.gpu.setCollapse(null, null);
    if (had) this.requestRedraw();
  }

  /** Calques déplacés dont l'aperçu et les chargements sont tenus (voir `releaseMovePreview`). */
  private held: { ids: number[]; all: Set<number> } | null = null;

  /** Sélection déplacée encore tenue, s'il y en a une. */
  get heldMove(): readonly number[] | null {
    return this.held?.ids ?? null;
  }

  /** Fin de la tenue (sélection changée, autre document, autre retouche) : les calques se rechargent, une fois. */
  releaseHold(): void {
    if (!this.held) return;
    this.held = null;
    if (this.collapseKept) this.dropCollapse();
    if (this.moveKept) this.dropMove();
    this.gpu.holdLoads(null);
    this.requestRedraw();
  }

  /**
   * Image en cache prête ou gardée pour la sélection `ids` (aplat d'un groupe, ou images en couches) : c'est elle qui
   * représente la sélection à l'écran pendant les déplacements et leur historique.
   */
  cacheShown(ids: readonly number[]): boolean {
    return this.optimizedState(ids) === "ready";
  }

  /** Groupe dont l'aplat est gardé après le relâchement, s'il y en a un. */
  get keptCollapse(): number | null {
    return this.collapseKept ? (this.collapseCache?.ids[0] ?? null) : null;
  }

  /**
   * Relâchement d'un déplacement : il part au moteur. L'aperçu reste affiché jusqu'à `endMovePreview` ; les textures
   * que le moteur replace d'ici là ne sont plus décalées (voir `GpuPreview.start`). L'aplat d'un groupe cède aussitôt
   * la place au programme complet (mêmes textures, même position).
   */
  releaseMovePreview(): void {
    if (this.gpuPreview) this.gpuPreview.releasing = true;
    // L'aplat du groupe reste affiché à sa nouvelle place tant que le groupe reste sélectionné (voir `collapseKept`).
    // Un remplissage couvre toujours le document : une image où il a bougé avec le reste serait fausse sur les bords.
    const doc = this.doc;
    this.collapseKept = this.gpu.collapsedGroup !== null && this.gpu.collapsedGroup === this.collapseCache?.ids[0];
    // Images en cache exactes (pas d'approximation pendant le geste) : gardées aussi, voir `moveKept`. Avec un
    // remplissage, l'aperçu reste celui du geste (bord du document approché) jusqu'à la désélection.
    this.moveKept = !!this.moveCache?.ready && (!!this.moveCache.layout?.exact || this.isOptimized(this.moveCache.ids));
    // Chargements des calques déplacés suspendus tant que la sélection reste la même (voir `holdLoads`).
    const moved = new Set<number>([...(this.gpuPreview?.moving ?? []), ...(this.moveCache?.ids ?? []), ...(this.collapseCache?.ids ?? [])]);
    if (doc && moved.size > 0) {
      const all = new Set<number>();
      for (const id of moved) for (const d of withDescendantIds(doc, id)) all.add(d);
      // Sélection déplacée : les calques de plus haut niveau (l'aperçu sur carte graphique compte aussi les enfants).
      const top = [...moved].filter((id) => { const parent = doc.layers.find((l) => l.id === id)?.parent; return parent == null || !moved.has(parent); });
      this.held = { ids: top, all };
      this.gpu.holdLoads(all);
    }
    this.applyCollapse();
    this.sceneStale = true;
  }

  /** Calque de réglage appliqué par la carte graphique : son aperçu passe par ses valeurs (voir adjust-preview.ts). */
  composesAdjustment(layer: number): boolean {
    return !this.preview && this.gpu.composes(layer);
  }

  /** Valeurs provisoires d'un réglage pendant qu'on tire un curseur (composition par la carte graphique). */
  setAdjustValues(layer: number, bytes: ArrayBuffer): void {
    this.gpu.setAdjustPreview(layer, bytes);
    this.gpuAdjustReleasing = false;
    this.requestRedraw();
  }

  /** Geste continu sans couches séparées : image un cran plus grossière, le temps du geste. */
  setDraftLevel(levels: number): void {
    const wanted = Math.max(0, Math.min(2, levels));
    if (this.draft === wanted) return;
    this.draft = wanted;
    this.requestRedraw();
  }

  /** Aperçu du curseur d'opacité : la couche du milieu, rendue pleine, est atténuée par la carte graphique. */
  setPreviewAlpha(alpha: number): void {
    this.sceneStale = true;
    if (this.preview) this.preview.alpha = Math.min(1, Math.max(0, alpha));
    if (this.gpuPreview) this.gpuPreview.alpha = Math.min(1, Math.max(0, alpha));
  }

  setMoveOffset(dx: number, dy: number): void {
    this.sceneStale = true;
    // Début d'un geste sur des images gardées : sa position de départ (voir `cachedShift`).
    if (!this.gestureStart && this.moveCache && this.doc) {
      const lead = this.doc.layers.find((l) => l.id === this.moveLead);
      if (lead) this.gestureStart = [lead.offset[0], lead.offset[1]];
    }
    this.moveOffset = [dx, dy];
    if (this.preview) Object.assign(this.preview, { dx, dy });
    if (this.gpuPreview) Object.assign(this.gpuPreview, { dx, dy });
  }

  // ---- Aperçu instantané d'un calque de réglage : l'image est affichée en deux couches (dessous, dessus) ; la
  // carte graphique applique le réglage à « dessous » par une table de correspondance 3D. Tirer un curseur ne
  // coûte plus qu'une nouvelle table (quelques dizaines de Ko), plus aucun recalcul de l'image.

  beginAdjustPreview(plan: { below: number[]; target: number[]; above: number[] }): void {
    this.endMovePreview();
    const part = (layers: number[]) =>
      new TileCache(this.gl, this.fetchPart(layers, null), (payload) => ({ texture: this.upload(payload, this.gl.RGBA8, this.gl.RGBA), uniform: 0 }), this.requestRedraw, 256);
    const target = plan.target.length > 0 ? part(plan.target) : null;
    this.preview = { below: part(plan.below), moving: target, above: part(plan.above), dx: 0, dy: 0, alpha: 1, lut: null, adjusting: true };
  }

  /** Nouvelle table du réglage (`size`³ couleurs RGBA8) et son opacité. */
  setAdjustLut(bytes: ArrayBuffer, size: number, mix: number): void {
    const preview = this.preview;
    if (!preview?.adjusting) return;
    const { gl } = this;
    const texture = preview.lut?.texture ?? gl.createTexture();
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_3D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage3D(gl.TEXTURE_3D, 0, gl.RGBA8, size, size, size, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(bytes));
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    for (const wrap of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T, gl.TEXTURE_WRAP_R]) gl.texParameteri(gl.TEXTURE_3D, wrap, gl.CLAMP_TO_EDGE);
    gl.activeTexture(gl.TEXTURE0);
    preview.lut = { texture, size, mix: Math.min(1, Math.max(0, mix)) };
    preview.releasing = false;
    this.requestRedraw();
  }

  /** Curseur relâché : l'aperçu reste affiché jusqu'à ce que l'image rendue par le moteur soit prête. */
  finishAdjustPreview(): void {
    if (this.preview?.adjusting) this.preview.releasing = true;
    // Composition par la carte graphique : les valeurs provisoires cèdent la place dès que celles du document
    // (identiques) sont arrivées.
    this.gpuAdjustReleasing = true;
    this.requestRedraw();
  }

  // ---- Aperçu des effets de calque : la carte graphique les calcule à partir des pixels seuls du calque (voir
  // gpu-effects.ts) tant qu'on règle ; le rendu du moteur prend la place au relâchement.

  /** Effets provisoires d'un calque pendant un réglage. Faux si la carte graphique ne peut pas les montrer. */
  previewEffects(layer: number, effects: readonly LayerEffect[]): boolean {
    if (this.preview) return false;
    const shown = this.gpu.previewEffects(layer, effects);
    if (shown) this.requestRedraw();
    return shown;
  }

  /** Le moteur a reçu les derniers effets : l'aperçu reste affiché jusqu'à ce que son rendu soit chargé. */
  finishEffectsPreview(layer: number): void {
    if (this.gpu.effectsPreviewLayer !== layer) return;
    this.gpu.finishEffectsPreview();
    this.requestRedraw();
  }

  endEffectsPreview(layer: number): void {
    if (this.gpu.effectsPreviewLayer !== layer) return;
    this.gpu.endEffectsPreview();
    this.requestRedraw();
  }

  /** `keepCollapse` : fin d'un déplacement de groupe, dont l'aplat est gardé tant que le groupe reste sélectionné. */
  endMovePreview(keepCollapse = false): void {
    if (this.preview?.lut) this.gl.deleteTexture(this.preview.lut.texture);
    for (const cache of this.preview ? [this.preview.below, this.preview.moving, this.preview.above] : []) cache?.reset();
    this.preview = null;
    this.gpuPreview = null;
    this.gpu.setLive(null);
    if (keepCollapse && this.collapseKept) this.applyCollapse();
    else this.dropCollapse();
    if (keepCollapse && this.moveKept) {
      // Geste fini : la couche déplacée est placée d'après la position des calques (voir `cachedShift`).
      this.gestureStart = null;
      this.moveOffset = [0, 0];
    } else this.dropMove();
    this.moveOffset = [0, 0];
    this.releaseFinishTarget();
  }

  /**
   * Aperçu immédiat d'un trait : les empreintes sont dessinées par la carte graphique, à la
   * fréquence de l'écran et quelle que soit la taille du pinceau. Pendant ce temps l'image de
   * départ reste figée ; le rendu du moteur prend sa place d'un seul coup à la fin du trait.
   */
  /** `accumulate` : pinceau flou, dont les empreintes se posent les unes sur les autres (comme dans le moteur et Photoshop). */
  beginStrokePreview(color: readonly [number, number, number], alpha: number, hardness: number, accumulate = false): void {
    // Trait enchaîné avant que le moteur ait rendu le précédent : son aperçu est conservé, sinon
    // le trait précédent disparaîtrait un instant de l'écran.
    const previous = this.stroke;
    const same = previous && previous.alpha === alpha && previous.hardness === hardness && previous.accumulate === accumulate && previous.color.every((c, i) => c === color[i]);
    this.stroke = same && !previous.tip ? { ...previous, releasing: false } : { color, alpha, hardness, accumulate, dabs: [], drawn: 0, signature: "", releasing: false, tip: null };
    this.image.hold = true;
  }

  /**
   * Pinceau en direct d'une pointe importée : ses vraies empreintes (pointe, taille, angle, rondeur, dispersion,
   * pression), le pinceau double et la texture, dessinés par la carte graphique à la fréquence de l'écran. Le moteur
   * rejoue ensuite les mêmes empreintes : son rendu, qui remplace l'aperçu au relâchement, est le même trait.
   */
  beginTipStrokePreview(color: readonly [number, number, number], alpha: number, spec: TipPreviewSpec): void {
    this.stroke = { color, alpha, hardness: 1, accumulate: false, dabs: [], drawn: 0, signature: "", releasing: false, tip: { ...spec, stamps: [], duals: [], dualDrawn: 0 } };
    this.image.hold = true;
    // Pointe, pointe double et motif : chargés s'il le faut (le trait est dessiné dès qu'ils sont là).
    void this.tipTexture(spec.tip);
    if (spec.dual) void this.tipTexture(spec.dual.tip);
    if (spec.texture) void this.patternTexture(spec.texture.pattern);
  }

  addTipStamps(stamps: readonly TipStamp[], duals: readonly DualTipStamp[]): void {
    const tip = this.stroke?.tip;
    if (!tip) return;
    this.sceneStale = true;
    tip.stamps.push(...stamps);
    tip.duals.push(...duals);
  }

  /** Texture d'une pointe (niveaux de détail compris), ou `null` tant qu'elle n'est pas chargée. */
  private tipTexture(id: number): WebGLTexture | null {
    const ready = this.tipTextures.get(id);
    if (ready) return ready;
    const image = tipImage(id);
    if (!image) {
      void loadTip(id).then(() => this.requestRedraw());
      return null;
    }
    const { gl } = this;
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texStorage2D(gl.TEXTURE_2D, Math.floor(Math.log2(Math.max(image.width, image.height))) + 1, gl.R8, image.width, image.height);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, image.width, image.height, gl.RED, gl.UNSIGNED_BYTE, image.mask);
    gl.generateMipmap(gl.TEXTURE_2D);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    this.tipTextures.set(id, texture);
    return texture;
  }

  /** Texture d'un motif (lu au pixel près, répété), ou `null` tant qu'il n'est pas chargé. */
  private patternTexture(id: number): { texture: WebGLTexture; width: number; height: number } | null {
    const ready = this.patternTextures.get(id);
    if (ready) return ready;
    const image = patternImage(id);
    if (!image) {
      void loadPattern(id).then(() => this.requestRedraw());
      return null;
    }
    const { gl } = this;
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, image.width, image.height, 0, gl.RED, gl.UNSIGNED_BYTE, image.gray);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.bindTexture(gl.TEXTURE_2D, null);
    const entry = { texture, width: image.width, height: image.height };
    this.patternTextures.set(id, entry);
    return entry;
  }

  addStrokeDabs(dabs: readonly StrokeDab[]): void {
    this.sceneStale = true;
    this.stroke?.dabs.push(...dabs);
  }

  // ---- Aperçu d'une transformation (cadre de transformation, Ctrl+T) : la carte graphique place les textures des
  // calques transformés par l'homographie du cadre, comme Photoshop, au lieu de demander au moteur un rééchantillonnage
  // complet (et un rechargement complet de la texture) à chaque mouvement de la souris. Le moteur ne reçoit qu'une
  // étape au relâchement de chaque poignée (en arrière-plan), puis le rendu final à la validation.

  /**
   * Montre les calques `ids` (groupes : tout leur contenu) du document `docId` transformés par `forward` (homographie
   * document → document, par lignes) ; `inside` : un point du cadre transformé (son centre). L'aperçu reste lié à ce
   * document (voir transform-previews.ts : un autre onglet a souvent les mêmes identifiants de calques). Faux si la
   * carte graphique ne compose pas tout
   * le document en ce moment (chargement, budget de mémoire vidéo dépassé, aperçu d'un autre genre) : le moteur doit
   * alors rendre chaque pas, comme avant.
   */
  showTransformPreview(gesture: string, docId: number, ids: readonly number[], forward: readonly number[], inside: { x: number; y: number }): boolean {
    // Groupe optimisé montré par ses images en couches : elles sont transformées par la carte graphique, avec ou sans
    // composition du document par elle (petite machine, budget dépassé).
    if (docId === this.doc?.id && this.isOptimized(ids) && this.moveKept && this.moveCache?.ready && !this.stroke && this.doc) {
      const shift = this.cachedTransform?.gesture === gesture ? this.cachedTransform.shift : this.cachedShift(this.doc);
      this.cachedTransform = { gesture, current: forward, inside, shift };
      this.requestRedraw();
      return true;
    }
    const canCompose = this.gpuComposes() && !this.gpuPreview && !this.stroke;
    if (!this.transforms.show(gesture, docId, ids, forward, inside, canCompose)) return false;
    if (docId === this.doc?.id) this.requestRedraw();
    return true;
  }

  /**
   * Une étape du moteur pour ce geste vient d'être appliquée (`forward`, `null` : retour à l'état d'avant le geste) :
   * les textures de ces calques posées désormais en viennent. Appelé dès la réponse du moteur, avant leur arrivée.
   */
  noteTransformStep(gesture: string, forward: readonly number[] | null): void {
    this.transforms.step(gesture, forward);
    if (this.optimized) this.transformStep = gesture;
  }

  /** Fin de l'aperçu de transformation (rendu final affiché, ou abandon). */
  endTransformPreview(gesture: string): void {
    if (this.transforms.end(gesture)) this.requestRedraw();
    if (this.cachedTransform?.gesture === gesture || this.transformStep === gesture) {
      // Rendu final du moteur affiché : les images d'avant (ou d'une étape) le groupe transformé cèdent la place, et
      // celles du groupe tel qu'il est maintenant sont refaites.
      this.cachedTransform = null;
      this.transformStep = null;
      if (this.moveKept && !this.gestureStart) this.dropMove();
      this.requestReoptimize(false);
      this.requestRedraw();
    }
  }

  // ---- Pinceau et gomme peints dans la texture du calque : ce qui s'affiche pendant le geste est le trait définitif.
  // Le moteur reçoit les mêmes empreintes en coulisse ; à sa réponse, rien n'est relu (voir `liveEngineDone`).

  /**
   * Commence un trait peint directement dans la texture du calque `layer`. Faux si ce n'est pas possible maintenant
   * (composition par le processeur, autre aperçu en cours, texture absente ou trop grande) : l'appelant passe alors par
   * l'esquisse et le rendu du moteur, comme avant.
   */
  beginLiveStroke(layer: number, spec: LiveSpec): boolean {
    if (this.liveLayer !== null || this.stroke || !this.gpuComposes() || this.gpuPreview || this.moveKept || this.collapseKept || this.optimized) return false;
    if (this.cachedTransform || this.transformStep || this.transforms.active || this.gpu.effectsPreviewLayer !== null) return false;
    const target = this.gpu.paintTarget(layer);
    if (!target) return false;
    if (!this.livePaint) {
      try {
        const paint = new LivePaint(this.gl, (fragment) => this.link(fragment), this.floatCoverage);
        this.livePaint = paint;
        this.gpu.refreshLevels = (texture, width, height, levels, rect) => paint.refreshTexture(texture, width, height, levels, rect);
      } catch (error) {
        console.error("Pinceau sur carte graphique indisponible", error);
        return false;
      }
    }
    this.livePaint.begin(target, spec);
    this.gpu.holdPaint(layer);
    this.liveLayer = layer;
    return true;
  }

  /** Calque du trait peint en direct en cours, ou `null`. */
  get liveStrokeLayer(): number | null {
    return this.liveLayer;
  }

  addLiveDabs(dabs: readonly StrokeDab[]): void {
    if (this.liveLayer === null || dabs.length === 0) return;
    this.livePaint?.addDabs(dabs);
    this.requestRedraw();
  }

  addLiveTipStamps(stamps: readonly TipStamp[], duals: readonly DualTipStamp[]): void {
    if (this.liveLayer === null || (stamps.length === 0 && duals.length === 0)) return;
    this.livePaint?.addTipStamps(stamps, duals);
    this.requestRedraw();
  }

  /**
   * Relâchement : les dernières empreintes sont posées, les textures de travail libérées. Les relectures du calque
   * restent suspendues jusqu'à `liveEngineDone` (le moteur n'a pas encore fini le même trait). Rend le calque peint.
   */
  endLiveStroke(): number | null {
    const layer = this.liveLayer;
    const paint = this.livePaint;
    if (layer === null || !paint) return null;
    if (this.gpu.paintHeld(layer) && paint.render()) this.gpu.painted();
    paint.end();
    this.liveLayer = null;
    this.requestRedraw();
    return layer;
  }

  /** Textures de travail libérées sans rien poser (texture du calque relâchée par `GpuLayers.reset`). */
  private dropLiveStroke(): void {
    this.livePaint?.end();
    this.liveLayer = null;
  }

  /** Abandon du trait en cours (le moteur ne l'a pas) : la texture reprend son état d'avant le trait. */
  cancelLiveStroke(): void {
    const layer = this.liveLayer;
    if (layer === null) return;
    if (this.gpu.paintHeld(layer)) this.livePaint?.cancel();
    else this.livePaint?.end();
    this.gpu.painted();
    this.liveLayer = null;
    this.gpu.releasePaint(layer);
  }

  /**
   * À appeler juste AVANT de signaler la modification du moteur pour ce trait (`invalidate`) : ses pixels sont déjà
   * dans la texture, ils ne sont pas relus. Puis `liveEngineDone`.
   */
  liveEngineAccepted(layer: number): void {
    this.gpu.markFresh(layer, "all");
  }

  /**
   * Le moteur a fini le trait peint sur `layer` (`ok`), ou l'a refusé : la texture n'est alors plus juste, elle est
   * relue. Les relectures suspendues de ce calque reprennent.
   */
  liveEngineDone(layer: number, ok: boolean): void {
    this.gpu.clearFresh();
    if (!ok) this.gpu.reloadPixels(layer);
    this.gpu.releasePaint(layer);
  }

  /**
   * Pixels envoyés par le moteur avec une annulation ou un rétablissement : posés tout de suite dans les textures ; la
   * modification signalée ensuite (`invalidate`) ne relit pas ces zones-là. `clearFresh` après ce signalement.
   */
  applyPatches(patches: readonly { layer: number; rect: readonly [number, number, number, number]; pixels: Uint8Array }[]): void {
    if (!this.gpuComposes() || this.liveLayer !== null) return;
    if (!this.livePaint) {
      try {
        const paint = new LivePaint(this.gl, (fragment) => this.link(fragment), this.floatCoverage);
        this.livePaint = paint;
        this.gpu.refreshLevels = (texture, width, height, levels, rect) => paint.refreshTexture(texture, width, height, levels, rect);
      } catch {
        // Sans programme de réduction : `generateMipmap` sur toute la texture (plus lent, même résultat).
      }
    }
    const applied = this.gpu.applyPatches(patches);
    for (const [layer, rects] of applied) this.gpu.markFresh(layer, rects);
    if (applied.size > 0) this.requestRedraw();
  }

  /** Après le signalement qui suit `liveEngineAccepted` ou `applyPatches` : les zones « déjà à jour » sont oubliées. */
  clearFresh(): void {
    this.gpu.clearFresh();
  }

  /** Fin du trait : les tuiles du moteur peuvent prendre place ; l'aperçu s'efface au même instant. */
  finishStrokePreview(): void {
    if (this.stroke) this.stroke.releasing = true;
    this.image.hold = false;
    this.requestRedraw();
  }

  /**
   * Une nouvelle sélection remplace l'ancienne : l'ancien contour est effacé tout de suite, au lieu de rester
   * affiché le temps que le nouveau arrive.
   */
  dropSelection(): void {
    this.selection.reset();
    this.requestRedraw();
  }

  /**
   * Résolue quand l'image affichée reflète le document (toutes les tuiles visibles à jour). `layers` : composition
   * sur carte graphique, n'attendre que les textures de ces calques (ceux qu'une modification vient de repeindre) ;
   * un geste commencé pendant le chargement d'un gros document n'attend plus, à chaque pas, toute la file.
   */
  whenSettled(layers?: readonly number[]): Promise<void> {
    return new Promise((resolve) => {
      this.settledWaiters.push({ resolve, layers });
      this.requestRedraw();
    });
  }

  /** Tient les attentes satisfaites : toutes (`ready` absent), ou celles dont les calques sont prêts. */
  private resolveSettled(ready?: (layers?: readonly number[]) => boolean): void {
    if (this.settledWaiters.length === 0) return;
    const waiting = this.settledWaiters;
    this.settledWaiters = [];
    for (const waiter of waiting) {
      if (!ready || ready(waiter.layers)) waiter.resolve();
      else this.settledWaiters.push(waiter);
    }
  }

  /** Attend que la carte graphique ait fini l'image en cours et rend ce temps d'attente (mesure uniquement). */
  waitForGpu(): number {
    const started = performance.now();
    this.gl.finish();
    return performance.now() - started;
  }

  /**
   * La zone de travail signale que la prochaine image ne diffère de la précédente que par l'angle de la vue (glisser de
   * l'outil Rotation de l'affichage) : l'image droite déjà composée est réutilisée si rien d'autre n'a changé.
   */
  reuseSceneOnce(): void {
    this.reuseNext = true;
  }

  /**
   * Dessine l'image. `camera` : placement du document dans le repère où l'image est composée — la zone de travail, ou
   * le cadre tourné (`rotated`). Vue tournée : composition droite dans la texture du cadre (le chemin habituel, tel
   * quel, y compris trait, sélection, résolution dynamique et épreuve CMJN), puis une seule passe qui la dessine
   * tournée autour du centre dans le canevas. Le canevas ne tourne pas et ne change pas de taille.
   */
  draw(camera: Camera, dpr: number, rotated: RotatedView | null = null): void {
    if (this.suspended) return;
    const { gl } = this;
    const reuse = this.reuseNext;
    this.reuseNext = false;
    if (!rotated || rotated.rotation === 0) {
      this.releaseRotated();
      this.target = null;
      this.gpu.screen = null;
      this.targetSize = [gl.canvas.width, gl.canvas.height];
      this.drawnScale = dpr;
      this.drawScene(camera, dpr);
      if (this.proofProfile && this.doc) this.drawProof(this.proofProfile, camera, dpr);
      return;
    }
    const { frame } = rotated;
    // Texture plus grande que ce que la carte accepte (écran immense) : l'image tournée est composée un peu moins fine.
    const limit = this.gpu.maxTexture;
    const ceiling = Math.min(1, limit / Math.max(1, frame.width * rotated.maxDpr), limit / Math.max(1, frame.height * rotated.maxDpr));
    const target = this.ensureRotated(Math.max(1, Math.min(limit, Math.ceil(frame.width * rotated.maxDpr * ceiling))), Math.max(1, Math.min(limit, Math.ceil(frame.height * rotated.maxDpr * ceiling))));
    const scale = Math.min(dpr, rotated.maxDpr * ceiling);
    const size: [number, number] = [Math.min(target.width, Math.max(1, Math.round(frame.width * scale))), Math.min(target.height, Math.max(1, Math.round(frame.height * scale)))];
    const key = [camera.zoom, camera.x, camera.y, scale, size[0], size[1], this.proofProfile && proofKey(this.proofProfile), this.quickMask, this.doc?.id, this.doc?.revision].join("/");
    if (!(reuse && !this.sceneStale && key === this.sceneKey && !this.previewing() && !this.transforms.active)) {
      // Avant la composition : ce qu'elle demande elle-même (tuile manquante…) rendra la suivante nécessaire.
      this.sceneStale = false;
      this.target = target.framebuffer;
      this.gpu.screen = target.framebuffer;
      this.targetSize = size;
      this.drawnScale = scale;
      try {
        this.drawScene(camera, scale);
        if (this.proofProfile && this.doc) this.drawProof(this.proofProfile, camera, scale);
      } finally {
        this.target = null;
        this.gpu.screen = null;
      }
      this.sceneKey = key;
    }
    this.present(rotated, target, size, dpr, scale);
  }

  /** Texture droite de la vue tournée : allouée une fois à cette taille, réallouée seulement si elle change (fenêtre). */
  private ensureRotated(width: number, height: number): NonNullable<ViewportRenderer["rotated"]> {
    const { gl } = this;
    if (this.rotated && this.rotated.width === width && this.rotated.height === height) return this.rotated;
    this.releaseRotated();
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, width, height);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.bindTexture(gl.TEXTURE_2D, null);
    const framebuffer = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    this.rotated = { texture, framebuffer, width, height };
    this.sceneStale = true;
    return this.rotated;
  }

  /** Retour à la vue droite (ou fin) : la texture de la vue tournée est rendue. */
  private releaseRotated(): void {
    if (!this.rotated) return;
    this.gl.deleteTexture(this.rotated.texture);
    this.gl.deleteFramebuffer(this.rotated.framebuffer);
    this.rotated = null;
    this.sceneKey = "";
  }

  /** Dessin final de la vue tournée : la texture droite, tournée autour du centre, couvre tout le canevas. */
  private present(rotated: RotatedView, target: NonNullable<ViewportRenderer["rotated"]>, size: readonly [number, number], dpr: number, scale: number): void {
    const { gl } = this;
    const p = this.presentProgram;
    const [canvasW, canvasH] = [gl.canvas.width, gl.canvas.height];
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, canvasW, canvasH);
    gl.disable(gl.BLEND);
    gl.disable(gl.SCISSOR_TEST);
    gl.useProgram(p.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, target.texture);
    gl.uniform1i(p.uniform("u_tex"), 0);
    gl.uniform2f(p.uniform("u_view"), canvasW, canvasH);
    gl.uniform4f(p.uniform("u_rect"), 0, 0, canvasW, canvasH);
    gl.uniform4f(p.uniform("u_uv"), 0, 0, 1, 1);
    gl.uniform1f(p.uniform("u_dpr"), dpr);
    gl.uniform1f(p.uniform("u_scale"), scale);
    gl.uniform2f(p.uniform("u_center"), rotated.viewW / 2, rotated.viewH / 2);
    gl.uniform2f(p.uniform("u_frame"), rotated.frame.x, rotated.frame.y);
    gl.uniform2f(p.uniform("u_rot"), Math.cos(rotated.rotation), Math.sin(rotated.rotation));
    gl.uniform2f(p.uniform("u_region"), size[0], size[1]);
    gl.uniform2f(p.uniform("u_texSize"), target.width, target.height);
    gl.uniform3f(p.uniform("u_bg"), this.canvasColor[0], this.canvasColor[1], this.canvasColor[2]);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.enable(gl.BLEND);
  }

  /**
   * Épreuve CMJN : l'image finie est recopiée de l'écran dans une texture, puis réécrite convertie, sur le seul
   * rectangle du document (le fond de la zone de travail et le damier gardent leurs gris). Une passe plein document,
   * quel que soit le nombre de calques : le coût est le même que celui du damier.
   */
  private drawProof(profile: CmykSpace, camera: Camera, dpr: number): void {
    const { gl } = this;
    const doc = this.doc!;
    // Sans table (premier affichage, ou profil introuvable), l'image reste en sRGB plutôt qu'une épreuve fausse.
    const key = proofKey(profile);
    if (!this.proofLuts.has(key)) this.loadProofLut(profile);
    const lut = this.proofLuts.get(key);
    if (typeof lut !== "object") return;
    const [viewW, viewH] = this.targetSize;
    const scale = camera.zoom * dpr;
    const [x, y, w, h] = docRect(camera.x * dpr, camera.y * dpr, doc.width, doc.height, scale);
    const [x0, y0] = [Math.max(0, x), Math.max(0, y)];
    const [x1, y1] = [Math.min(viewW, x + w), Math.min(viewH, y + h)];
    if (x1 <= x0 || y1 <= y0) return;
    if (!this.proofTexture || this.proofTexture.width !== viewW || this.proofTexture.height !== viewH) {
      if (this.proofTexture) gl.deleteTexture(this.proofTexture.texture);
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      // RGB8, pas RGBA8 : le canevas est créé sans alpha, et une copie d'écran vers une texture qui a une composante
      // de plus que l'écran est refusée (INVALID_OPERATION) — la texture resterait noire.
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGB8, viewW, viewH);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      this.proofTexture = { texture, width: viewW, height: viewH };
    }
    // Copie de la zone du document depuis l'écran (origine OpenGL en bas à gauche).
    const [gy0, gy1] = [viewH - y1, viewH - y0];
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, this.proofTexture.texture);
    gl.copyTexSubImage2D(gl.TEXTURE_2D, 0, x0, gy0, x0, gy0, x1 - x0, gy1 - gy0);
    gl.disable(gl.BLEND);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x0, gy0, x1 - x0, gy1 - gy0);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindSampler(1, null);
    gl.bindTexture(gl.TEXTURE_3D, lut.texture);
    gl.activeTexture(gl.TEXTURE0);
    gl.useProgram(this.proofProgram.program);
    gl.uniform2f(this.proofProgram.uniform("u_view"), viewW, viewH);
    gl.uniform4f(this.proofProgram.uniform("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(this.proofProgram.uniform("u_uv"), 0, 0, 1, 1);
    gl.uniform1i(this.proofProgram.uniform("u_tex"), 0);
    gl.uniform1i(this.proofProgram.uniform("u_lut"), 1);
    gl.uniform1f(this.proofProgram.uniform("u_lutSize"), lut.size);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.SCISSOR_TEST);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_3D, null);
    gl.activeTexture(gl.TEXTURE0);
  }

  /**
   * Charge la table de l'épreuve (N³ triplets RVB, rouge le plus rapide) dans une texture 3D à interpolation
   * trilinéaire. Un seul chargement par espace et par renderer ; un échec (profil introuvable sur cet ordinateur) est
   * noté et l'épreuve de cet espace reste sans effet, avec un message en console plutôt qu'une erreur à chaque image.
   */
  private loadProofLut(space: CmykSpace): void {
    const profile = proofKey(space);
    this.proofLuts.set(profile, "loading");
    (this.proofTables ? this.proofTables(space) : Promise.reject(new Error("aucune source de tables")))
      .then((buffer) => {
        const size = Math.round(Math.cbrt(buffer.byteLength / 3));
        if (size ** 3 * 3 !== buffer.byteLength) throw new Error(`taille inattendue : ${buffer.byteLength} octets`);
        const { gl } = this;
        const texture = gl.createTexture();
        gl.activeTexture(gl.TEXTURE1);
        gl.bindTexture(gl.TEXTURE_3D, texture);
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
        gl.texStorage3D(gl.TEXTURE_3D, 1, gl.RGB8, size, size, size);
        gl.texSubImage3D(gl.TEXTURE_3D, 0, 0, 0, 0, size, size, size, gl.RGB, gl.UNSIGNED_BYTE, new Uint8Array(buffer));
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
        gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_3D, gl.TEXTURE_WRAP_R, gl.CLAMP_TO_EDGE);
        gl.bindTexture(gl.TEXTURE_3D, null);
        gl.activeTexture(gl.TEXTURE0);
        this.proofLuts.set(profile, { texture, size });
        this.requestRedraw();
      })
      .catch((error: unknown) => {
        this.proofLuts.set(profile, "failed");
        console.error(`Table de l'épreuve CMJN ${profile} indisponible`, error);
      });
  }

  private drawScene(camera: Camera, dpr: number): void {
    const { gl, doc } = this;
    const [viewW, viewH] = this.targetSize;
    this.frame += 1;

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.viewport(0, 0, viewW, viewH);
    gl.clearColor(this.canvasColor[0], this.canvasColor[1], this.canvasColor[2], 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!doc) {
      this.resolveSettled();
      return;
    }

    const scale = camera.zoom * dpr;
    const originX = camera.x * dpr;
    const originY = camera.y * dpr;
    // Bitmap sous 100 % : un niveau réduit, montré en gris par le moteur (règle commune, voir `finishModeFor`).
    const level = Math.min(doc.maxLevel, levelForScale(scale, doc.maxLevel, doc.colorMode?.kind === "bitmap") + this.draft);
    const frame = this.frameAt(doc, level, scale, originX, originY, viewW, viewH);
    this.drawChecker(frame, viewW, viewH, dpr);

    // Déplacement par images en cache : les couches préparées au début du geste, assemblées par la carte graphique,
    // la couche déplacée décalée. Pendant ce temps, l'image du moteur (tuiles ou textures) se met à jour en coulisse :
    // au relâchement, l'aperçu reste jusqu'à ce qu'elle soit prête (voir Viewport.svelte, `finishLocalMove`).
    // Groupe optimisé (Ctrl+T) sans image utilisable (perdue, retouchée, autre vue) : elle est refaite.
    const optimized = this.optimizedGroup;
    if (optimized !== null && !this.gestureStart && this.optimizedMode([optimized]) === null) this.requestReoptimize();
    // Aplat du groupe optimisé qui ne couvre plus la vue (zoom, défilement) : refait pour la nouvelle vue.
    if (optimized !== null && this.collapseKept && this.collapseCache?.ready && !this.gpuPreview && !this.transformStep && !this.collapseCovers({ scale, originX, originY, viewW, viewH })) {
      this.dropCollapse();
      this.requestReoptimize();
    }
    const cached = this.moveCache;
    if (cached?.ready && cached.docId === doc.id && cached.slices) {
      gl.enable(gl.BLEND);
      gl.enable(gl.SCISSOR_TEST);
      const [x, y, w, h] = docRect(originX, originY, doc.width, doc.height, scale);
      gl.scissor(x, viewH - y - h, w, h);
      // Transformation (Ctrl+T) du groupe optimisé : ses couches suivent le cadre, l'état d'avant le geste transformé.
      const transform = this.cachedTransform;
      const matrix = transform ? relativeMatrix(transform.current, null, transform.inside) : null;
      const shift = transform ? transform.shift : this.cachedShift(doc);
      this.gpu.drawCached({ scale, originX, originY, viewW, viewH }, doc, cached.slices, shift, cached.level ?? 0, matrix);
      gl.disable(gl.SCISSOR_TEST);
      if (this.moveKept) {
        // Images gardées (dès le relâchement, et pendant les gestes suivants) : rien n'est demandé au moteur tant
        // qu'elles servent. Hors geste, si la vue sort de ce qu'elles couvrent (ou si le zoom change), elles cèdent la
        // place : l'image du moteur est recomposée une fois.
        this.resolveSettled();
        if (!this.gestureStart && !this.transformingOptimized() && !this.moveCovers({ scale, originX, originY, viewW, viewH })) {
          this.dropMove();
          this.requestReoptimize();
        }
      } else if (this.gpu.ready()) {
        const settled = this.gpu.settled;
        this.resolveSettled((layers) => settled || (layers !== undefined && !this.gpu.pending(layers)));
      } else {
        this.image.beginFrame();
        this.drawTiles(this.image, frame, viewW, viewH, false, true);
        if (this.image.promote()) this.requestRedraw();
        else if (this.image.settled) this.resolveSettled();
      }
      if (doc.hasSelection) this.drawSelection(frame, viewW, viewH, dpr);
      this.image.evict(this.frame);
      this.selection.evict(this.frame);
      return;
    }

    // Composition sur carte graphique : chaque calque est une texture déjà présente, rien n'est demandé au moteur.
    // (Un aperçu de déplacement commencé sur le processeur se termine sur le processeur.)
    if (this.gpu.ready() && !this.preview) {
      this.drawGpu(frame, viewW, viewH);
      if (doc.hasSelection) this.drawSelection(frame, viewW, viewH, dpr);
      this.image.evict(this.frame);
      this.selection.evict(this.frame);
      return;
    }

    // La carte graphique ne compose plus le document : un aperçu des effets n'y serait plus visible, il s'arrête (le
    // prochain réglage passera par le moteur).
    if (this.gpu.effectsPreviewLayer !== null) this.gpu.endEffectsPreview();
    gl.enable(gl.BLEND);
    this.image.beginFrame();
    // Les tuiles de l'image restent parcourues pendant un aperçu : elles se chargent en coulisse,
    // prêtes pour le retour à l'affichage normal.
    const preview = this.preview;
    const previewReady = preview !== null && this.drawPreview(preview, frame, viewW, viewH);
    this.drawTiles(this.image, frame, viewW, viewH, false, previewReady);
    // Aperçu d'un réglage relâché : dès que l'image du moteur est là (tuiles chargées), elle reprend la main.
    if (preview?.releasing && previewReady && this.image.settled) {
      this.endMovePreview();
      this.requestRedraw();
    }
    if (this.stroke) this.drawStroke(this.stroke, frame, viewW, viewH);
    if (this.image.promote()) {
      // Les vraies tuiles viennent de prendre place : l'aperçu du trait n'a plus lieu d'être.
      if (this.stroke?.releasing) this.stroke = null;
      this.requestRedraw();
    } else if (this.image.settled) {
      if (this.stroke?.releasing) {
        this.stroke = null;
        this.requestRedraw();
      }
      this.resolveSettled();
    }
    if (doc.hasSelection) this.drawSelection(frame, viewW, viewH, dpr);

    this.image.evict(this.frame);
    this.selection.evict(this.frame);
  }

  private drawGpu(frame: Frame, viewW: number, viewH: number): void {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    const preview = this.gpuPreview;
    // Aplat gardé après un déplacement : zoom changé, ou vue qui sort de ce qu'il couvre, il cède la place aux calques.
    if (this.collapseKept && !preview && !this.collapseCovers({ scale, originX, originY, viewW, viewH })) this.dropCollapse();
    // Rien ne déborde du document.
    const [x, y, w, h] = docRect(originX, originY, doc.width, doc.height, scale);
    // Trait peint en direct : les empreintes arrivées depuis l'image précédente sont posées dans la texture du calque.
    // (Textures du document relâchées entre-temps — autre document, carte graphique désactivée : le trait s'arrête là
    // côté écran, le moteur le termine et le calque sera relu.)
    if (this.liveLayer !== null && !this.gpu.paintHeld(this.liveLayer)) this.dropLiveStroke();
    if (this.liveLayer !== null && this.livePaint?.render()) this.gpu.painted();
    gl.enable(gl.BLEND);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    this.gpu.draw(
      { scale, originX, originY, viewW, viewH },
      {
        // Position de départ + décalage, quelle que soit la position où la texture est posée (voir `moveShift`).
        shift: (id, kind, texture) => moveShift(preview, id, kind, texture),
        opacity: (id, opacity, fill) => (preview && preview.opaque === id ? preview.alpha * fill : opacity),
        matrix: (id, _kind, texture) => this.transforms.matrix(id, texture),
      },
    );
    gl.disable(gl.SCISSOR_TEST);
    // Aperçu des effets relâché : le rendu du moteur est chargé, il prend la place.
    if (this.gpu.effectsPreviewDone()) {
      this.gpu.endEffectsPreview();
      this.requestRedraw();
    }
    if (this.gpuAdjustReleasing && this.gpu.settled) {
      this.gpuAdjustReleasing = false;
      this.gpu.endAdjustPreview();
      this.requestRedraw();
    }
    const stroke = this.stroke;
    if (stroke) this.drawStroke(stroke, frame, viewW, viewH);
    // Fin d'un trait : son aperçu s'efface dès que le calque repeint est arrivé sur la carte graphique.
    // `finishStrokePreview` n'est appelé qu'APRÈS que la réponse finale du moteur a été transmise : la mise à
    // jour du calque est alors déjà en cours, et « plus rien en attente » veut dire qu'elle est arrivée.
    // (Auparavant on attendait un signalement POSTÉRIEUR à la fin du trait, qui ne venait jamais : l'esquisse
    // restait affichée par-dessus le vrai trait jusqu'à la modification suivante.)
    const arrived = stroke?.releasing && this.gpu.settled;
    if (arrived) {
      this.stroke = null;
      this.requestRedraw();
    }
    const settled = this.gpu.settled;
    this.resolveSettled((layers) => settled || (layers !== undefined && !this.gpu.pending(layers)));
  }

  /**
   * Vrai si le point (pixels CSS du repère où l'image est composée : la zone de travail, ou le cadre tourné ; origine
   * en haut à gauche) est dans la sélection. Lit le masque du dernier rendu : réponse immédiate, sans interroger le moteur.
   */
  isSelectedAt(cssX: number, cssY: number): boolean {
    const { gl } = this;
    if (!this.doc?.hasSelection || this.maskSize[0] === 0) return false;
    const [x, y] = [cssX * this.drawnScale, cssY * this.drawnScale];
    const pixel = new Uint8Array(4);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.maskFramebuffer);
    gl.readPixels(Math.round(x), this.maskSize[1] - 1 - Math.round(y), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return (pixel[0] ?? 0) >= 128;
  }

  dispose(): void {
    withdrawEffectsPreview(this.effectsControls);
    this.livePaint?.dispose();
    this.livePaint = null;
    this.liveLayer = null;
    this.gpu.refreshLevels = null;
    this.gpu.reset();
    this.gpu.effectsCompute?.dispose();
    this.releaseRotated();
    this.releaseFinishTarget();
    this.image.reset();
    this.selection.reset();
  }

  /** Accumule les nouvelles empreintes dans la texture de couverture (maximum, pas addition), puis pose le trait sur l'image. */
  private drawStroke(stroke: StrokePreview, frame: Frame, viewW: number, viewH: number): void {
    if (stroke.tip) {
      this.drawTipStroke(stroke, stroke.tip, frame, viewW, viewH);
      return;
    }
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    gl.bindTexture(gl.TEXTURE_2D, this.coverageTarget);
    // Très gros pinceau : chaque empreinte couvre une grande part de l'écran. Sa couverture est
    // alors calculée à demi ou quart de résolution (4 à 16 fois moins de pixels à traiter) ; un
    // bord aussi large reste visuellement identique, et le rendu définitif du moteur est, lui,
    // toujours en pleine résolution.
    const largest = stroke.dabs.reduce((max, dab) => Math.max(max, dab.ar, dab.br), 0) * 2 * scale;
    const k = largest > 900 ? 0.25 : largest > 350 ? 0.5 : 1;
    const [coverW, coverH] = [Math.max(1, Math.ceil(viewW * k)), Math.max(1, Math.ceil(viewH * k))];
    const signature = `${coverW}x${coverH}@${scale}/${originX}/${originY}`;
    const resized = this.coverageSize[0] !== coverW || this.coverageSize[1] !== coverH;
    if (resized) {
      // Couverture en demi-flottant quand la carte le permet : l'accumulation de centaines d'empreintes légères
      // (bord d'un trait flou) garde alors tous ses niveaux ; sur 8 bits, les plus faibles seraient perdus (paliers).
      if (this.floatCoverage) gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, coverW, coverH, 0, gl.RED, gl.HALF_FLOAT, null);
      else gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, coverW, coverH, 0, gl.RED, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      this.coverageSize = [coverW, coverH];
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.coverageFramebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.coverageTarget, 0);
    if (resized || stroke.signature !== signature) {
      // Zoom ou déplacement de la vue en plein trait : la couverture est recalculée depuis le début.
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      stroke.drawn = 0;
      stroke.signature = signature;
    }

    gl.enable(gl.BLEND);
    if (stroke.accumulate) {
      // Chaque empreinte se pose par-dessus : couverture = a + b (1 - a), comme `round_dab` dans le moteur.
      gl.blendEquation(gl.FUNC_ADD);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_COLOR);
    } else {
      gl.blendEquation(gl.MAX);
      gl.blendFunc(gl.ONE, gl.ONE);
    }
    gl.useProgram(this.dabProgram.program);
    gl.viewport(0, 0, coverW, coverH);
    gl.uniform2f(this.dabProgram.uniform("u_view"), coverW, coverH);
    gl.uniform1f(this.dabProgram.uniform("u_scale"), scale * k);
    gl.uniform1f(this.dabProgram.uniform("u_hardness"), stroke.hardness);
    // Pinceau flou : empreintes à un tiers de force, qui s'additionnent (réglage du mélange plus haut).
    gl.uniform1f(this.dabProgram.uniform("u_deposit"), stroke.accumulate ? 1 : 0);
    for (const dab of stroke.dabs.slice(stroke.drawn)) {
      const [ax, ay] = [(originX + dab.ax * scale) * k, (originY + dab.ay * scale) * k];
      const [bx, by] = [(originX + dab.bx * scale) * k, (originY + dab.by * scale) * k];
      const reach = (Math.max(dab.ar, dab.br) + 1.5) * scale * k + 1;
      gl.uniform4f(this.dabProgram.uniform("u_rect"), ...snap(Math.min(ax, bx) - reach, Math.min(ay, by) - reach, Math.max(ax, bx) + reach, Math.max(ay, by) + reach));
      gl.uniform3f(this.dabProgram.uniform("u_a"), ax, ay, dab.ar);
      gl.uniform3f(this.dabProgram.uniform("u_b"), bx, by, dab.br);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    stroke.drawn = stroke.dabs.length;
    gl.blendEquation(gl.FUNC_ADD);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

    // Pose du trait sur l'image, limitée au document.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.viewport(0, 0, viewW, viewH);
    const [x, y, w, h] = docRect(originX, originY, doc.width, doc.height, scale);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    gl.useProgram(this.strokeProgram.program);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, this.coverageTarget);
    gl.uniform1i(this.strokeProgram.uniform("u_coverage"), 0);
    gl.uniform2f(this.strokeProgram.uniform("u_view"), viewW, viewH);
    gl.uniform4f(this.strokeProgram.uniform("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(this.strokeProgram.uniform("u_ink"), stroke.color[0] / 255, stroke.color[1] / 255, stroke.color[2] / 255, stroke.alpha);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.SCISSOR_TEST);
  }

  /**
   * Trait de pointe en direct : les nouvelles empreintes (et celles du pinceau double) s'ajoutent aux couvertures,
   * puis le trait est posé sur l'image. Rien n'est dessiné tant que la pointe n'est pas chargée (les empreintes
   * attendent, et sont toutes posées à son arrivée).
   */
  private drawTipStroke(stroke: StrokePreview, tip: TipPreview, frame: Frame, viewW: number, viewH: number): void {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    const tipTexture = this.tipTexture(tip.tip);
    const dualTexture = tip.dual ? this.tipTexture(tip.dual.tip) : null;
    const pattern = tip.texture ? this.patternTexture(tip.texture.pattern) : null;
    const tipSize = tipImage(tip.tip);
    const dualSize = tip.dual ? tipImage(tip.dual.tip) : null;
    if (!tipTexture || !tipSize || (tip.dual && (!dualTexture || !dualSize)) || (tip.texture && !pattern)) return;
    // Couverture à la résolution du document quand on est dézoomé (le moteur pose les empreintes au pixel du
    // document : à la résolution de l'écran, le grain d'une pointe perdait ses pointes, le trait paraissait plus
    // léger que le résultat), jusqu'au double de l'écran (la pose bilinéaire fait alors la moyenne exacte de 2 × 2
    // pixels, sans réduction à recalculer à chaque image), dans la limite de 4096 px de côté et de 16 millions de
    // pixels ; à l'écran sinon.
    const detail = Math.min(scale < 1 ? 1 / scale : 1, 2, 4096 / viewW, 4096 / viewH, Math.sqrt(16_777_216 / (viewW * viewH)));
    // Très gros pinceau : couverture à demi ou quart de résolution, comme le pinceau rond.
    const largest = Math.max(0, ...tip.stamps.slice(-64).map((s) => s.diameter), ...tip.duals.slice(-64).map((d) => d.diameter)) * scale * detail;
    const k = Math.max(0.25, detail) * (largest > 900 ? 0.25 : largest > 350 ? 0.5 : 1);
    const [coverW, coverH] = [Math.max(1, Math.ceil(viewW * k)), Math.max(1, Math.ceil(viewH * k))];
    const signature = `${coverW}x${coverH}@${scale}/${originX}/${originY}`;
    const allocate = (texture: WebGLTexture) => {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      if (this.floatCoverage) gl.texImage2D(gl.TEXTURE_2D, 0, gl.R16F, coverW, coverH, 0, gl.RED, gl.HALF_FLOAT, null);
      else gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, coverW, coverH, 0, gl.RED, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    };
    gl.activeTexture(gl.TEXTURE0);
    const resized = this.coverageSize[0] !== coverW || this.coverageSize[1] !== coverH;
    if (resized) {
      allocate(this.coverageTarget);
      this.coverageSize = [coverW, coverH];
    }
    if (tip.dual && (!this.dualTarget || this.dualSize[0] !== coverW || this.dualSize[1] !== coverH)) {
      this.dualTarget ??= gl.createTexture();
      allocate(this.dualTarget);
      this.dualSize = [coverW, coverH];
      stroke.signature = ""; // couverture du pinceau double à refaire avec le reste
    }
    const restart = resized || stroke.signature !== signature;
    if (restart) {
      stroke.drawn = 0;
      tip.dualDrawn = 0;
      stroke.signature = signature;
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.coverageFramebuffer);
    gl.viewport(0, 0, coverW, coverH);
    gl.useProgram(this.tipDabProgram.program);
    const u = this.tipDabProgram.uniform;
    gl.uniform2f(u("u_view"), coverW, coverH);
    gl.uniform1i(u("u_tip"), 0);
    gl.bindSampler(0, null); // filtrage propre à la texture de la pointe (niveaux de détail)
    gl.enable(gl.BLEND);
    /** Empreintes d'une pointe dans une couverture : `accumulate` (flux) ou maximum. */
    const stampInto = (target: WebGLTexture, texture: WebGLTexture, size: { width: number; height: number }, list: readonly { x: number; y: number; diameter: number; flipX: boolean; flipY: boolean; turn?: number; roundness?: number; strength?: number }[], from: number, flow: number) => {
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
      if (restart) {
        gl.clearColor(0, 0, 0, 0);
        gl.clear(gl.COLOR_BUFFER_BIT);
      }
      if (flow >= 1) {
        gl.blendEquation(gl.MAX);
        gl.blendFunc(gl.ONE, gl.ONE);
      } else {
        // Par-dessus : couverture = c + v (1 - c), comme `merge` dans le moteur.
        gl.blendEquation(gl.FUNC_ADD);
        gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_COLOR);
      }
      gl.bindTexture(gl.TEXTURE_2D, texture);
      const side = Math.max(size.width, size.height);
      for (let i = from; i < list.length; i++) {
        const s = list[i]!;
        const unit = s.diameter / side;
        const [w, h] = [size.width * unit, size.height * unit * (s.roundness ?? 1)];
        if (w < 0.5 || h < 0.5) continue;
        gl.uniform1f(u("u_lod"), tipLevel(size.width, size.height, Math.max(w, h)));
        const turn = s.turn ?? 0;
        const [cos, sin] = [Math.cos(turn), Math.sin(turn)];
        const [sw, sh] = [w * scale * k, h * scale * k];
        const [cx, cy] = [(originX + s.x * scale) * k, (originY + s.y * scale) * k];
        const [hw, hh] = [(sw * Math.abs(cos) + sh * Math.abs(sin)) / 2 + 1, (sw * Math.abs(sin) + sh * Math.abs(cos)) / 2 + 1];
        if (cx + hw < 0 || cy + hh < 0 || cx - hw > coverW || cy - hh > coverH) continue;
        gl.uniform4f(u("u_rect"), ...snap(cx - hw, cy - hh, cx + hw, cy + hh));
        gl.uniform2f(u("u_center"), cx, cy);
        gl.uniform2f(u("u_size"), sw, sh);
        gl.uniform2f(u("u_rot"), cos, sin);
        gl.uniform2f(u("u_flip"), s.flipX ? 1 : 0, s.flipY ? 1 : 0);
        gl.uniform1f(u("u_value"), Math.min(1, Math.max(0, s.strength ?? 1)) * Math.min(1, flow));
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      }
    };
    stampInto(this.coverageTarget, tipTexture, tipSize, tip.stamps, stroke.drawn, tip.flow);
    stroke.drawn = tip.stamps.length;
    if (tip.dual && this.dualTarget && dualTexture && dualSize) {
      // Les empreintes du pinceau double s'accumulent (flux 99,9 % dans le moteur).
      stampInto(this.dualTarget, dualTexture, dualSize, tip.duals, tip.dualDrawn, 0.999);
      tip.dualDrawn = tip.duals.length;
    }
    gl.blendEquation(gl.FUNC_ADD);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);

    // Pose du trait sur l'image, limitée au document.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.viewport(0, 0, viewW, viewH);
    const [x, y, w, h] = docRect(originX, originY, doc.width, doc.height, scale);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    const p = this.tipStrokeProgram.uniform;
    gl.useProgram(this.tipStrokeProgram.program);
    gl.bindSampler(0, null);
    gl.bindSampler(1, null);
    gl.bindSampler(2, null);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, this.coverageTarget);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, tip.dual ? this.dualTarget : this.coverageTarget);
    gl.activeTexture(gl.TEXTURE2);
    gl.bindTexture(gl.TEXTURE_2D, pattern?.texture ?? this.coverageTarget);
    gl.uniform1i(p("u_coverage"), 0);
    gl.uniform1i(p("u_dual"), 1);
    gl.uniform1i(p("u_pattern"), 2);
    gl.uniform2f(p("u_view"), viewW, viewH);
    gl.uniform4f(p("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(p("u_ink"), stroke.color[0] / 255, stroke.color[1] / 255, stroke.color[2] / 255, stroke.alpha);
    gl.uniform1i(p("u_dualMode"), tip.dual ? (TIP_BLEND_CODES[tip.dual.mode] ?? 0) : -1);
    const texture = tip.texture;
    gl.uniform1i(p("u_texMode"), texture ? (TIP_BLEND_CODES[texture.mode] ?? 0) : -1);
    gl.uniform4f(p("u_tex"), texture?.scale ?? 1, texture?.depth ?? 0, (texture?.brightness ?? 0) / 255, (texture?.contrast ?? 0) / 100);
    gl.uniform1f(p("u_texInvert"), texture?.invert ? 1 : 0);
    gl.uniform2f(p("u_patternSize"), pattern?.width ?? 1, pattern?.height ?? 1);
    gl.uniform3f(p("u_camera"), originX, originY, scale);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.SCISSOR_TEST);
    for (const unit of [2, 1, 0]) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, null);
    }
  }

  private drawChecker(frame: Frame, viewW: number, viewH: number, dpr: number): void {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    gl.disable(gl.BLEND);
    gl.useProgram(this.checkerProgram.program);
    gl.uniform2f(this.checkerProgram.uniform("u_view"), viewW, viewH);
    gl.uniform4f(this.checkerProgram.uniform("u_rect"), ...docRect(originX, originY, doc.width, doc.height, scale));
    gl.uniform1f(this.checkerProgram.uniform("u_cell"), Math.round(8 * dpr));
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  /** Dessine les trois couches de l'aperçu. Retourne faux tant qu'elles ne sont pas toutes chargées. */
  private drawPreview(preview: MovePreview, frame: Frame, viewW: number, viewH: number): boolean {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    const shifted = this.frameAt(doc, frame.level, scale, originX + preview.dx * scale, originY + preview.dy * scale, viewW, viewH);
    const layers: [TileCache, Frame][] = preview.moving ? [[preview.below, frame], [preview.moving, shifted], [preview.above, frame]] : [[preview.below, frame], [preview.above, frame]];

    // Premier passage « à blanc » : demande les tuiles et vérifie que tout est là avant d'afficher quoi que ce soit.
    for (const [cache, at] of layers) {
      cache.beginFrame();
      this.drawTiles(cache, at, viewW, viewH, false, true);
    }
    if (!layers.every(([cache]) => cache.settled)) return false;

    // Mode Gris ou Bitmap : les couches du moteur ne portent pas la règle de mode (elle s'applique à l'image composée
    // entière : un seuil par couche ne donnerait pas le même résultat). Elles sont assemblées hors écran, puis le
    // résultat est posé désaturé ou seuillé, comme le fait la composition sur carte graphique.
    const finish = finishModeFor(doc.colorMode?.kind, frame.level, scale);
    if (finish !== 0) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.ensureFinishTarget(viewW, viewH).framebuffer);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }
    // Le calque déplacé ne doit pas déborder du document.
    const [x, y, w, h] = docRect(originX, originY, doc.width, doc.height, scale);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    // Réglage : la table s'applique à la couche du milieu (base d'un réglage écrêté), sinon à « dessous ».
    const corrected = preview.adjusting ? (preview.moving ?? preview.below) : null;
    for (const [cache, at] of layers) this.drawTiles(cache, at, viewW, viewH, false, false, cache === preview.moving ? preview.alpha : 1, cache === corrected ? preview.lut : null);
    if (finish !== 0) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
      gl.useProgram(this.finishProgram.program);
      gl.uniform4f(this.finishProgram.uniform("u_rect"), 0, 0, viewW, viewH);
      gl.uniform4f(this.finishProgram.uniform("u_uv"), 0, 0, 1, 1);
      gl.uniform2f(this.finishProgram.uniform("u_view"), viewW, viewH);
      gl.uniform1i(this.finishProgram.uniform("u_tex"), 0);
      gl.uniform1i(this.finishProgram.uniform("u_colorMode"), finish);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindSampler(0, null);
      gl.bindTexture(gl.TEXTURE_2D, this.finishTarget!.texture);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      gl.bindTexture(gl.TEXTURE_2D, null);
    }
    gl.disable(gl.SCISSOR_TEST);
    return true;
  }

  /** Texture hors écran des aperçus en couches en mode Gris ou Bitmap, à la taille de la cible (réallouée si elle change). */
  private ensureFinishTarget(width: number, height: number): NonNullable<ViewportRenderer["finishTarget"]> {
    const { gl } = this;
    if (this.finishTarget && this.finishTarget.width === width && this.finishTarget.height === height) return this.finishTarget;
    this.releaseFinishTarget();
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, width, height);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.bindTexture(gl.TEXTURE_2D, null);
    const framebuffer = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    this.finishTarget = { texture, framebuffer, width, height };
    return this.finishTarget;
  }

  private releaseFinishTarget(): void {
    if (!this.finishTarget) return;
    this.gl.deleteTexture(this.finishTarget.texture);
    this.gl.deleteFramebuffer(this.finishTarget.framebuffer);
    this.finishTarget = null;
  }

  private frameAt(doc: DocumentInfo, level: number, scale: number, originX: number, originY: number, viewW: number, viewH: number): Frame {
    const span = doc.tileSize * 2 ** level; // côté d'une tuile de ce niveau, en pixels du document
    return {
      doc,
      level,
      span,
      scale,
      originX,
      originY,
      first: [Math.max(0, Math.floor(-originX / scale / span)), Math.max(0, Math.floor(-originY / scale / span))],
      last: [
        Math.min(Math.ceil(doc.width / span) - 1, Math.floor((viewW - originX) / scale / span)),
        Math.min(Math.ceil(doc.height / span) - 1, Math.floor((viewH - originY) / scale / span)),
      ],
    };
  }

  /**
   * Parcourt les tuiles visibles d'un cache (ce qui déclenche leur chargement) et les dessine.
   * `drawUniform` : les tuiles sans texture sont des aplats. `dryRun` : ne rien dessiner.
   */
  private drawTiles(cache: TileCache, frame: Frame, viewW: number, viewH: number, drawUniform: boolean, dryRun = false, alpha = 1, lut: MovePreview["lut"] = null): void {
    const { gl } = this;
    const { doc, level, span, scale } = frame;
    gl.useProgram(this.tileProgram.program);
    gl.uniform1f(this.tileProgram.uniform("u_alpha"), alpha);
    gl.uniform1i(this.tileProgram.uniform("u_lut"), 1);
    gl.uniform1f(this.tileProgram.uniform("u_lutMix"), lut ? lut.mix : 0);
    if (lut) {
      gl.uniform1f(this.tileProgram.uniform("u_lutSize"), lut.size);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_3D, lut.texture);
    }
    gl.uniform2f(this.tileProgram.uniform("u_view"), viewW, viewH);
    gl.uniform1i(this.tileProgram.uniform("u_tex"), 0);
    gl.activeTexture(gl.TEXTURE0);
    // Au-delà d'un pixel physique par texel, on veut voir les pixels nets.
    gl.bindSampler(0, scale * 2 ** level > 1 ? this.nearest : this.linear);

    // Le sommet de la pyramide sert de secours pendant les chargements : il passe en premier.
    cache.request(doc.maxLevel, 0, 0, this.frame);
    for (let ty = frame.first[1]; ty <= frame.last[1]; ty++) {
      for (let tx = frame.first[0]; tx <= frame.last[0]; tx++) {
        const source = cache.resolve(level, tx, ty, doc.maxLevel, this.frame);
        if (!dryRun && source && (source.texture || drawUniform)) this.drawTile(frame, tx, ty, span, source);
      }
    }
  }

  private drawTile(frame: Frame, tx: number, ty: number, span: number, source: Resolved): void {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    // Zone utile : les tuiles de bord dépassent du document.
    const docX0 = tx * span;
    const docY0 = ty * span;
    const docX1 = Math.min(doc.width, docX0 + span);
    const docY1 = Math.min(doc.height, docY0 + span);

    // `source` peut être un ancêtre : on n'en affiche que la portion correspondante.
    const sourceSpan = doc.tileSize * 2 ** source.level;
    const u0 = (docX0 - source.x * sourceSpan) / sourceSpan;
    const v0 = (docY0 - source.y * sourceSpan) / sourceSpan;
    const du = (docX1 - docX0) / sourceSpan;
    const dv = (docY1 - docY0) / sourceSpan;
    const half = 0.5 / doc.tileSize;

    gl.bindTexture(gl.TEXTURE_2D, source.texture);
    gl.uniform1f(this.tileProgram.uniform("u_solid"), source.texture ? -1 : source.uniform / 255);
    gl.uniform4f(
      this.tileProgram.uniform("u_rect"),
      ...snap(originX + docX0 * scale, originY + docY0 * scale, originX + docX1 * scale, originY + docY1 * scale),
    );
    gl.uniform4f(this.tileProgram.uniform("u_uv"), u0, v0, du, dv);
    gl.uniform4f(this.tileProgram.uniform("u_clamp"), u0 + half, v0 + half, u0 + du - half, v0 + dv - half);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  /** Masque de sélection rendu à la taille de l'écran, puis contour animé par-dessus l'image. */
  private drawSelection(frame: Frame, viewW: number, viewH: number, dpr: number): void {
    const { gl } = this;
    gl.bindTexture(gl.TEXTURE_2D, this.maskTarget);
    if (this.maskSize[0] !== viewW || this.maskSize[1] !== viewH) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, viewW, viewH, 0, gl.RED, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      this.maskSize = [viewW, viewH];
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.maskFramebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.maskTarget, 0);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.disable(gl.BLEND);
    this.selection.beginFrame();
    this.drawTiles(this.selection, frame, viewW, viewH, true);
    if (this.selection.promote()) this.requestRedraw();

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.target);
    gl.bindSampler(0, null);
    if (this.quickMask) {
      gl.enable(gl.BLEND);
      gl.useProgram(this.tintProgram.program);
      gl.bindTexture(gl.TEXTURE_2D, this.maskTarget);
      gl.uniform1i(this.tintProgram.uniform("u_mask"), 0);
      gl.uniform2f(this.tintProgram.uniform("u_view"), viewW, viewH);
      gl.uniform4f(this.tintProgram.uniform("u_rect"), 0, 0, viewW, viewH);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      return;
    }
    gl.useProgram(this.antsProgram.program);
    gl.bindTexture(gl.TEXTURE_2D, this.maskTarget);
    gl.uniform1i(this.antsProgram.uniform("u_mask"), 0);
    gl.uniform2f(this.antsProgram.uniform("u_view"), viewW, viewH);
    gl.uniform4f(this.antsProgram.uniform("u_rect"), 0, 0, viewW, viewH);
    gl.uniform1f(this.antsProgram.uniform("u_dash"), 4 * dpr);
    gl.uniform1f(this.antsProgram.uniform("u_time"), (performance.now() / 250) % 2);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.enable(gl.BLEND);
  }

  private upload(pixels: Uint8Array, internalFormat: number, format: number): WebGLTexture | null {
    if (pixels.byteLength === 0) return null;
    const { gl } = this;
    const size = this.doc?.tileSize ?? 256;
    const texture = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texImage2D(gl.TEXTURE_2D, 0, internalFormat, size, size, 0, format, gl.UNSIGNED_BYTE, pixels);
    return texture;
  }

  private sampler(filter: number): WebGLSampler {
    const { gl } = this;
    const sampler = gl.createSampler();
    gl.samplerParameteri(sampler, gl.TEXTURE_MIN_FILTER, filter);
    gl.samplerParameteri(sampler, gl.TEXTURE_MAG_FILTER, filter);
    gl.samplerParameteri(sampler, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.samplerParameteri(sampler, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return sampler;
  }

  private link(fragmentSource: string): Program {
    const { gl } = this;
    const program = gl.createProgram();
    const stages = [
      [gl.VERTEX_SHADER, VERTEX_SHADER],
      [gl.FRAGMENT_SHADER, fragmentSource],
    ] as const;
    for (const [type, source] of stages) {
      const shader = gl.createShader(type);
      if (!shader) throw new Error(t("Création de shader impossible."));
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        throw new Error(`Shader invalide : ${gl.getShaderInfoLog(shader) ?? t("erreur inconnue")}`);
      }
      gl.attachShader(program, shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(`Programme GPU invalide : ${gl.getProgramInfoLog(program) ?? t("erreur inconnue")}`);
    }
    const locations = new Map<string, WebGLUniformLocation | null>();
    const uniform = (name: string) => {
      if (!locations.has(name)) locations.set(name, gl.getUniformLocation(program, name));
      return locations.get(name) ?? null;
    };
    return { program, uniform };
  }
}

/** État d'un déplacement par images en cache, pour l'indicateur de diagnostic de la barre d'état. */
export interface MoveStatus {
  /** `cache` : aperçu en couches ; `collapse` : groupe aplati sur la carte graphique ; `engine` : impossible, le moteur. */
  mode: "cache" | "collapse" | "engine" | "pending" | "gpu";
  /** Délai jusqu'aux premières images prêtes. */
  ms: number | null;
  detail: string;
}

function moveStatus(cache: MoveCache): MoveStatus {
  // Groupe non aplatissable alors que la carte graphique compose : elle déplace quand même chaque calque (ce n'est pas
  // le moteur qui recalcule l'image, contrairement à ce qu'affichait la barre d'état).
  const mode = cache.state === "failed" ? (cache.mode === "collapse" ? "gpu" : "engine") : !cache.ready ? "pending" : cache.mode === "collapse" ? "collapse" : "cache";
  return { mode, ms: cache.readyMs, detail: cache.describe() };
}

/** Mêmes calques, dans le même ordre. */
function sameIds(a: readonly number[], b: readonly number[]): boolean {
  return a.length === b.length && a.every((id, i) => id === b[i]);
}

/** Bords alignés sur les pixels physiques : pas de jour entre deux tuiles voisines. */
function snap(x0: number, y0: number, x1: number, y1: number): [number, number, number, number] {
  const rx = Math.round(x0);
  const ry = Math.round(y0);
  return [rx, ry, Math.round(x1) - rx, Math.round(y1) - ry];
}

/**
 * Cadre du document à l'écran, arrondi VERS L'INTÉRIEUR. Le damier, les ciseaux et les aperçus s'arrêtent là ; les
 * calques, eux, sont posés à leur position exacte, fractionnaire. Arrondi au plus proche, le cadre dépassait d'une
 * fraction de pixel sous les calques dès que le bord tombait entre deux pixels, et un liseré de damier ou de fond
 * apparaissait en bas ou à gauche selon la position de la vue. Vers l'intérieur, au plus un pixel de bord est rogné.
 */
function docRect(originX: number, originY: number, width: number, height: number, scale: number): [number, number, number, number] {
  const x0 = Math.ceil(originX - 1e-6);
  const y0 = Math.ceil(originY - 1e-6);
  return [x0, y0, Math.max(0, Math.floor(originX + width * scale + 1e-6) - x0), Math.max(0, Math.floor(originY + height * scale + 1e-6) - y0)];
}

/** Clé d'une table d'épreuve (même forme que `spaceKey` de lib/color-settings.svelte.ts). */
function proofKey(space: CmykSpace): string {
  return `${space.profile}/${space.intent}/${space.bpc ? 1 : 0}`;
}
