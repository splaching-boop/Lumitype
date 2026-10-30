// Composition sur carte graphique.
//
// Chaque calque vit sur la carte graphique en une texture, à pleine résolution, dans son propre repère, avec ses
// réductions (mipmaps) calculées par la carte elle-même ; de même pour chaque masque, et chaque calque de réglage y a
// ses valeurs. Zoomer, se déplacer, déplacer un calque ou tirer un curseur d'opacité ne demande alors plus
// rien au moteur : la carte graphique réassemble l'image à chaque rafraîchissement d'écran, en suivant le même
// programme que le moteur (voir lib/gpu-plan.ts) — modes de fusion, masques, écrêtage, groupes et réglages compris.
// Le moteur ne transmet un calque qu'une fois, puis seulement la partie repeinte à chaque retouche.
//
// Effets de calque : le moteur les dessine (une fois, puis à chaque changement du calque), la carte graphique les fond.
// Pendant qu'on règle un effet, elle les calcule elle-même à partir des pixels seuls du calque (voir gpu-effects.ts et
// `previewEffects`) : le rendu du moteur ne prend le relais qu'au relâchement.
//
// Tant qu'une texture manque, ou si un calque dépasse la taille de texture ou la mémoire réservée, l'affichage
// classique par tuiles composées sur le processeur prend le relais, sans trou.
import { type GpuOp, type GpuPlan, type GpuSource, gpuPlan, intersect, tilesRect } from "../lib/gpu-plan";
import type { Adjustment, BlendMode, DirtyInfo, DocumentInfo, LayerEffect, LayerInfo } from "../lib/ipc";
import type { GpuEffects, StyledPreview } from "./gpu-effects";
import type { CachedSlice } from "./move-cache";
import { UPLOAD_STRIP_BYTES, paced } from "./paced";
import { finishModeFor } from "./camera";
import type { LiveTarget } from "./live-paint";

/**
 * Une texture d'un calque : `rect` = `[x, y, largeur, hauteur]` dans son repère, ou tout son contenu. `part` : ses pixels
 * avec ses effets fondus (`pixels`), son masque (`mask`), ses pixels seuls (`raw`), ou l'un de ses effets fondus à part
 * (`under:<n>`, `over:<n>`). `limit` = `[côté maximal, nombre de pixels maximal]` d'une texture acceptable : au-delà, le
 * moteur ne renvoie que l'en-tête (largeur et hauteur réelles, aucun pixel), sans lire ni transférer le calque entier.
 */
export type LayerFetcher = (layer: number, rect: [number, number, number, number] | null, part: TexturePart, limit: [number, number] | null) => Promise<ArrayBuffer>;
/** Effets d'un calque fondus à part (`layer_effects`) : modes de ceux du dessous, puis du dessus. */
export type EffectsFetcher = (layer: number) => Promise<{ under: BlendMode[]; over: BlendMode[] }>;
/** Réglage mis à plat par le moteur (`adjustment_gpu`) : type (u32), puis ses valeurs (f32). */
export type AdjustFetcher = (adjustment: Adjustment) => Promise<ArrayBuffer>;

/** Ce qui bouge dans un aperçu de déplacement : les pixels (effets compris) ou le masque. */
export type TextureKind = "pixels" | "mask";
export type TexturePart = TextureKind | "raw" | `under:${number}` | `over:${number}`;

/**
 * Clés des textures : `p<id>` pixels (effets fondus compris), `m<id>` masque, `r<id>` pixels seuls, `u<id>:<n>` et
 * `o<id>:<n>` effets fondus à part, dessous et dessus.
 */
const keyId = (key: string): number => Number.parseInt(key.slice(1), 10);
const keyPart = (key: string): TexturePart => {
  const n = Number(key.split(":")[1] ?? 0);
  if (key[0] === "m") return "mask";
  if (key[0] === "r") return "raw";
  if (key[0] === "u") return `under:${n}`;
  if (key[0] === "o") return `over:${n}`;
  return "pixels";
};
/** Description des effets d'un calque : quand elle change, ses pixels stylés et ses effets à part sont à relire. */
const styleOf = (layer: LayerInfo): string => JSON.stringify(layer.effects);

/**
 * Remplissage « Couleur unie » sans effet : sa couleur (RVB), sinon `null`. La carte graphique le dessine d'une seule
 * couleur couvrant tout le document, sans demander ses pixels au moteur : le déplacer (seul ou avec son dossier) ne
 * recharge rien — auparavant, ses pixels recalculés pour couvrir le document à sa nouvelle place faisaient relire une
 * texture de la taille du document à chaque relâchement.
 */
export function solidFillColor(layer: LayerInfo | undefined): readonly [number, number, number] | null {
  const fill = layer?.fillLayer;
  if (!fill || fill.type !== "solidColor" || layer.effects.some((e) => e.enabled)) return null;
  return fill.color;
}

/** Rectangle (repère du calque) d'une couleur unie : trois fois le document, centré sur lui. */
function solidRect(doc: { width: number; height: number }, layer: LayerInfo): { x: number; y: number; w: number; h: number } {
  return { x: -layer.offset[0] - doc.width, y: -layer.offset[1] - doc.height, w: doc.width * 3, h: doc.height * 3 };
}

/** Largeur de la texture des valeurs d'un réglage (une valeur par texel, rangées de 1024). */
const VALUES_ROW = 1024;

interface Resident {
  texture: WebGLTexture | null;
  /** Rectangle couvert par la texture, dans le repère du calque. */
  x: number;
  y: number;
  w: number;
  h: number;
  bytes: number;
  /**
   * Position du calque dans le document pour laquelle la texture a été lue. Un simple déplacement la fait suivre
   * (les pixels, dans le repère du calque, restent les mêmes) ; une transformation, qui repeint le calque ET
   * change son repère, rend la texture entière périmée : elle reste affichée à cette position, là où ses pixels
   * étaient vrais, jusqu'à son remplacement.
   */
  offset: [number, number];
  /** Masque : valeur hors de la zone couverte par la texture (0 à 1). */
  background: number;
}

/** Texture refusée (calque trop grand, budget dépassé), et l'état qui a mené au refus. */
interface Rejection {
  reason: string;
  /** Plus grand qu'une texture possible : seul un changement du calque lui-même peut y remédier. */
  tooLarge: boolean;
  /** État du calque lors du refus (repeintes signalées, description de ses effets) : s'il change, on réessaie. */
  stamp: string;
  /** Budget en vigueur et mémoire prise par les autres textures lors du refus : plus de place, on réessaie. */
  budget: number;
  used: number;
}

/** Réglage prêt pour le shader : son type et la texture de ses valeurs. */
interface AdjustParams {
  kind: number;
  texture: WebGLTexture;
}

/** Aperçu des effets d'un calque, calculés par la carte graphique à partir de ses pixels seuls. */
interface EffectsPreview {
  layer: number;
  effects: LayerEffect[];
  key: string;
  /** Résultat du calcul ; `null` tant que rien n'est calculé, ou sans effet actif (les pixels seuls suffisent). */
  result: StyledPreview | null;
  /** Ce qui remplace les textures stylées du calque : `p` (calque stylé), effets à part dessous et dessus. */
  view: { main: Resident; under: { mode: BlendMode; entry: Resident }[]; over: { mode: BlendMode; entry: Resident }[] } | null;
  /** Effets et révision des pixels seuls du dernier calcul : s'ils changent, il est à refaire. */
  stamp: string;
  /** Impossible ici (mémoire vidéo, calque démesuré, pixels seuls refusés) : le moteur garde la main. */
  failed: boolean;
  /** Le rendu du moteur est demandé : l'aperçu cède la place dès que ses textures sont arrivées. */
  releasing: boolean;
}

export interface GpuStatus {
  /** Carte graphique annoncée par le pilote (« llvmpipe » = rendu logiciel). */
  renderer: string;
  maxTexture: number;
  /** Composition GPU en service pour le document affiché. */
  active: boolean;
  /** Pourquoi elle ne l'est pas, le cas échéant. */
  reason: string | null;
  /** Mémoire vidéo occupée par les calques, en Mio. */
  memoryMb: number;
}

/** Où et comment dessiner : pixels physiques par pixel du document, coin du document à l'écran, taille de l'écran. */
export interface GpuView {
  scale: number;
  originX: number;
  originY: number;
  viewW: number;
  viewH: number;
}

/** Aperçus : décalage d'un calque (déplacement), opacité effective (curseur d'opacité), transformation (cadre). */
export interface GpuOverrides {
  /**
   * Décalage de cette texture du calque. `texture` est la texture elle-même (position `offset` à laquelle elle est
   * posée, objet servant de jeton) : l'aperçu d'un déplacement vise une position du document, et une texture déjà
   * replacée ou remplacée par le moteur au relâchement ne doit pas être décalée une seconde fois (voir renderer.ts).
   */
  shift: (id: number, kind: TextureKind, texture?: { readonly offset: readonly [number, number] }) => readonly [number, number];
  /** `opacity` : opacité × fond du calque ; retourne l'opacité à appliquer. */
  opacity: (id: number, opacity: number, fill: number) => number;
  /**
   * Aperçu d'une transformation (Ctrl+T, poignées du cadre) : la matrice de cette texture du calque, ou `null` si elle
   * n'est pas transformée. `texture` est la texture elle-même, comme jeton : celle que le moteur aura rendue
   * transformée ne doit pas l'être une seconde fois (voir renderer.ts, `transformMatrix`).
   */
  matrix?: (id: number, kind: TextureKind, texture: object) => TransformMatrix | null;
}

/**
 * Transformation d'aperçu d'un calque : `forward` envoie le document d'avant la transformation sur celui d'après
 * (l'homographie du cadre, 3 × 3 par lignes), `inverse` fait le chemin inverse, signe choisi pour que le dénominateur
 * soit positif sur le calque transformé (voir `previewInverse`). Le shader n'utilise que l'inverse : pour chaque pixel
 * de l'écran, il lit le pixel du calque d'origine qui doit s'y trouver — comme le moteur (`transform_store`).
 */
export interface TransformMatrix {
  forward: readonly number[];
  inverse: readonly number[];
}

/** Matrice 3 × 3 par lignes → tableau par colonnes, l'ordre que demande `uniformMatrix3fv`. */
const IDENTITY_COLUMNS = new Float32Array([1, 0, 0, 0, 1, 0, 0, 0, 1]);
function columns(m: readonly number[] | null | undefined, out: Float32Array = new Float32Array(9), at = 0): Float32Array {
  if (!m) {
    out.set(IDENTITY_COLUMNS, at);
    return out;
  }
  out.set([m[0]!, m[3]!, m[6]!, m[1]!, m[4]!, m[7]!, m[2]!, m[5]!, m[8]!], at);
  return out;
}

/**
 * Inverse d'une homographie 3 × 3 (par lignes), au signe près choisi pour que le point `inside` (un point de l'image
 * transformée, par exemple le centre du cadre) ait un dénominateur positif : le shader écarte les points de
 * dénominateur négatif ou nul (derrière l'horizon d'une perspective). `null` si la matrice n'est pas inversible.
 */
export function previewInverse(m: readonly number[], inside: { x: number; y: number }): number[] | null {
  const [a, b, c, d, e, f, g, h, i] = m as [number, number, number, number, number, number, number, number, number];
  const adj = [e * i - f * h, c * h - b * i, b * f - c * e, f * g - d * i, a * i - c * g, c * d - a * f, d * h - e * g, b * g - a * h, a * e - b * d];
  const det = a * adj[0]! + b * adj[3]! + c * adj[6]!;
  if (!Number.isFinite(det) || Math.abs(det) < 1e-12) return null;
  const inv = adj.map((v) => v / det);
  const w = inv[6]! * inside.x + inv[7]! * inside.y + inv[8]!;
  // Normalisé (dernier terme à 1 quand c'est possible) : mêmes ordres de grandeur que l'homographie du cadre.
  const k = Math.abs(inv[8]!) > 1e-12 ? 1 / Math.abs(inv[8]!) : 1;
  return inv.map((v) => (w < 0 ? -v : v) * k);
}

/**
 * Aperçu d'une transformation sur une texture déjà transformée par le moteur : la texture porte l'état `base`
 * (homographie document d'avant le geste → document, `null` : état d'avant le geste), l'écran doit montrer l'état
 * `current`. La matrice à appliquer est `current · base⁻¹` ; `null` si les deux sont le même état (la texture s'affiche
 * telle quelle, au bit près). `inside` : un point de l'image transformée (voir `previewInverse`).
 */
export function relativeMatrix(current: readonly number[], base: readonly number[] | null, inside: { x: number; y: number }): TransformMatrix | null {
  const forward = base ? compose(current, previewInverse(base, inside) ?? base) : [...current];
  const scale = Math.abs(forward[8]!) > 1e-12 ? forward[8]! : 1;
  const normalized = forward.map((v) => v / scale);
  if (normalized.every((v, i) => Math.abs(v - (i % 4 === 0 ? 1 : 0)) < 1e-9)) return null;
  const inverse = previewInverse(normalized, inside);
  return inverse ? { forward: normalized, inverse } : null;
}

/** Produit de deux matrices 3 × 3 par lignes : `a · b` (b d'abord, puis a). */
function compose(a: readonly number[], b: readonly number[]): number[] {
  const out: number[] = [];
  for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) out.push(a[r * 3]! * b[c]! + a[r * 3 + 1]! * b[3 + c]! + a[r * 3 + 2]! * b[6 + c]!);
  return out;
}

/**
 * Boîte englobante `[x0, y0, x1, y1]` (pixels du document) du rectangle `[x, y, largeur, hauteur]` transformé par
 * l'homographie `forward` : la même étendue que le moteur donne au résultat (image des quatre coins, voir
 * `transform_store`). `null` si un coin part à l'infini (perspective extrême) : tout l'écran est alors à redessiner.
 */
export function transformedBounds(rect: readonly number[], forward: readonly number[]): [number, number, number, number] | null {
  const [x, y, w, h] = rect as [number, number, number, number];
  const m = forward;
  let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
  let sign = 0;
  for (const [px, py] of [[x, y], [x + w, y], [x + w, y + h], [x, y + h]] as const) {
    const q = m[6]! * px + m[7]! * py + m[8]!;
    if (!Number.isFinite(q) || Math.abs(q) < 1e-12 || (sign !== 0 && Math.sign(q) !== sign)) return null;
    sign = Math.sign(q);
    const [tx, ty] = [(m[0]! * px + m[1]! * py + m[2]!) / q, (m[3]! * px + m[4]! * py + m[5]!) / q];
    [x0, y0, x1, y1] = [Math.min(x0, tx), Math.min(y0, ty), Math.max(x1, tx), Math.max(y1, ty)];
  }
  return [x0, y0, x1, y1];
}

/** Calques qu'une passe de pile fond en une fois (une unité de texture chacun ; 8 sur toute carte WebGL2). */
export const STACK_SIZE = 8;

// Pile de calques en mode Normal, sans masque : jusqu'à STACK_SIZE calques fondus en UNE passe, du bas vers le haut
// (le « over » prémultiplié est associatif : la pile se compose à part, puis la carte la fond sur la cible avec
// ONE, ONE_MINUS_SRC_ALPHA). Même placement et même échantillonnage que `placed()` du shader d'opérations : l'image
// est la même, le trafic mémoire divisé par le nombre de calques de la pile (une lecture-écriture de la cible au lieu
// d'une par calque).
const STACK_SHADER = `#version 300 es
precision highp float;
uniform vec2 u_view;
uniform vec3 u_camera;
uniform int u_count;
uniform sampler2D u_tex0;
uniform sampler2D u_tex1;
uniform sampler2D u_tex2;
uniform sampler2D u_tex3;
uniform sampler2D u_tex4;
uniform sampler2D u_tex5;
uniform sampler2D u_tex6;
uniform sampler2D u_tex7;
uniform vec4 u_place[${STACK_SIZE}];
uniform float u_alpha[${STACK_SIZE}];
uniform mat3 u_inv[${STACK_SIZE}];
out vec4 o_color;

vec2 docPos() {
  vec2 p = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y);
  return (p - u_camera.xy) / u_camera.z;
}

// Aperçu d'une transformation (Ctrl+T, poignées du cadre) : \`inv\` ramène le point du document à celui d'avant la
// transformation (homographie inverse) ; l'identité pour tout calque qui n'est pas transformé (résultat inchangé, au
// bit près : x × 1 + y × 0 + 0). Échantillonnage trilinéaire (niveaux de détail) quand la transformation réduit.
vec2 unproject(mat3 inv, vec2 d) {
  vec3 h = inv * vec3(d, 1.0);
  return h.z > 0.0 ? h.xy / h.z : vec2(-1.0e9);
}

vec4 placed(sampler2D tex, vec4 place, mat3 inv, vec2 d) {
  vec2 uv = (unproject(inv, d) - place.xy) / max(place.zw, vec2(1.0));
  vec4 c = texture(tex, uv);
  bool inside = place.z > 0.0 && uv.x >= 0.0 && uv.y >= 0.0 && uv.x <= 1.0 && uv.y <= 1.0;
  return inside ? c : vec4(0.0);
}

void main() {
  vec2 d = docPos();
  vec4 acc = vec4(0.0);
  vec4 c;
  c = placed(u_tex0, u_place[0], u_inv[0], d) * u_alpha[0]; acc = c + acc * (1.0 - c.a);
  if (u_count > 1) { c = placed(u_tex1, u_place[1], u_inv[1], d) * u_alpha[1]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 2) { c = placed(u_tex2, u_place[2], u_inv[2], d) * u_alpha[2]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 3) { c = placed(u_tex3, u_place[3], u_inv[3], d) * u_alpha[3]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 4) { c = placed(u_tex4, u_place[4], u_inv[4], d) * u_alpha[4]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 5) { c = placed(u_tex5, u_place[5], u_inv[5], d) * u_alpha[5]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 6) { c = placed(u_tex6, u_place[6], u_inv[6], d) * u_alpha[6]; acc = c + acc * (1.0 - c.a); }
  if (u_count > 7) { c = placed(u_tex7, u_place[7], u_inv[7], d) * u_alpha[7]; acc = c + acc * (1.0 - c.a); }
  if (acc.a <= 0.0) discard;
  o_color = acc;
}`;

/** Numéros des modes de fusion dans le shader (même ordre de lecture que blend.rs). */
const MODES: Record<BlendMode, number> = {
  pass_through: 0,
  normal: 0,
  dissolve: 1,
  darken: 2,
  multiply: 3,
  color_burn: 4,
  linear_burn: 5,
  lighten: 6,
  screen: 7,
  color_dodge: 8,
  linear_dodge: 9,
  overlay: 10,
  soft_light: 11,
  hard_light: 12,
  difference: 13,
  exclusion: 14,
  subtract: 15,
  divide: 16,
  hue: 17,
  saturation: 18,
  color: 19,
  luminosity: 20,
};

/** Genre de passe du shader d'opérations. */
const KIND = { over: 0, blend: 1, clip: 2, mix: 3, adjust: 4, load: 5, present: 6 } as const;

// Opérations du programme de composition, à la résolution de l'écran. Traduction fidèle de `plan.rs` et `blend.rs`
// (même formules, même ordre des calculs) : l'image est celle du moteur, aux arrondis près.
const OP_SHADER = `#version 300 es
precision highp float;
precision highp int;
uniform int u_kind;
uniform int u_colorMode;     // présentation : 0 telle quelle, 1 en gris, 2 seuillée (bitmap), voir finishColor
uniform int u_mode;
uniform float u_opacity;
uniform vec2 u_view;
uniform vec3 u_camera;      // coin du document à l'écran (pixels physiques), puis pixels physiques par pixel du document
uniform int u_srcLayer;     // 1 : pixels d'un calque (placés dans le document) ; 0 : un tampon (à l'écran)
uniform sampler2D u_src;
uniform vec4 u_srcPlace;    // rectangle de la texture, en pixels du document
uniform mat3 u_srcInv;      // aperçu de transformation (voir \`unproject\`) : de la source, du masque, de la forme
uniform mat3 u_maskInv;
uniform mat3 u_shapeInv;
uniform sampler2D u_dst;    // copie de la cible avant l'opération
uniform sampler2D u_before; // état mémorisé (interpolation d'un groupe en Transfert)
uniform int u_maskOn;
uniform sampler2D u_mask;
uniform vec4 u_maskPlace;
uniform float u_maskBg;
uniform int u_shapeKind;      // écrêtage : 0 alpha de la base, 1 pixels seuls de la base (u_shape), 2 rien
uniform sampler2D u_shape;
uniform vec4 u_shapePlace;
uniform int u_adjKind;
uniform int u_rangeOn;        // comparaison (options de fusion) : 0 sans, 1 avec
uniform int u_rangeChannel;   // 0 gris, 1 rouge, 2 vert, 3 bleu
uniform vec4 u_rangeThis;     // plages de 0 à 255 : noir début, noir fin, blanc début, blanc fin
uniform vec4 u_rangeUnder;
uniform highp sampler2D u_values; // valeurs du réglage (Prepared::to_gpu), une par texel
out vec4 o_color;

vec2 docPos() {
  vec2 p = vec2(gl_FragCoord.x, u_view.y - gl_FragCoord.y);
  return (p - u_camera.xy) / u_camera.z;
}

// Aperçu d'une transformation (Ctrl+T, poignées du cadre) : \`inv\` ramène le point du document à celui d'avant la
// transformation (homographie inverse) ; l'identité pour tout calque qui n'est pas transformé (résultat inchangé, au
// bit près : x × 1 + y × 0 + 0). Échantillonnage trilinéaire (niveaux de détail) quand la transformation réduit.
vec2 unproject(mat3 inv, vec2 d) {
  vec3 h = inv * vec3(d, 1.0);
  return h.z > 0.0 ? h.xy / h.z : vec2(-1.0e9);
}

vec4 placed(sampler2D tex, vec4 place, mat3 inv, vec2 d) {
  vec2 uv = (unproject(inv, d) - place.xy) / max(place.zw, vec2(1.0));
  vec4 c = texture(tex, uv);
  bool inside = place.z > 0.0 && uv.x >= 0.0 && uv.y >= 0.0 && uv.x <= 1.0 && uv.y <= 1.0;
  return inside ? c : vec4(0.0);
}

vec4 source(vec2 d) {
  vec4 fromLayer = placed(u_src, u_srcPlace, u_srcInv, d);
  vec4 fromBuffer = texelFetch(u_src, ivec2(gl_FragCoord.xy), 0);
  return u_srcLayer == 1 ? fromLayer : fromBuffer;
}

float coverage(vec2 d) {
  if (u_maskOn == 0) return 1.0;
  vec2 uv = (unproject(u_maskInv, d) - u_maskPlace.xy) / max(u_maskPlace.zw, vec2(1.0));
  float m = texture(u_mask, uv).r;
  bool inside = u_maskPlace.z > 0.0 && uv.x >= 0.0 && uv.y >= 0.0 && uv.x <= 1.0 && uv.y <= 1.0;
  return inside ? m : u_maskBg;
}

// ---- blend.rs
float hardLight(float b, float s) { return s <= 0.5 ? 2.0 * b * s : 1.0 - 2.0 * (1.0 - b) * (1.0 - s); }

float separable(int mode, float b, float s) {
  if (mode == 2) return min(b, s);
  if (mode == 3) return b * s;
  if (mode == 4) { if (b >= 1.0) return 1.0; if (s <= 0.0) return 0.0; return 1.0 - min((1.0 - b) / s, 1.0); }
  if (mode == 5) return max(b + s - 1.0, 0.0);
  if (mode == 6) return max(b, s);
  if (mode == 7) return b + s - b * s;
  if (mode == 8) { if (b <= 0.0) return 0.0; if (s >= 1.0) return 1.0; return min(b / (1.0 - s), 1.0); }
  if (mode == 9) return min(b + s, 1.0);
  if (mode == 10) return hardLight(s, b);
  if (mode == 11) return s <= 0.5 ? 2.0 * b * s + b * b * (1.0 - 2.0 * s) : 2.0 * b * (1.0 - s) + sqrt(b) * (2.0 * s - 1.0);
  if (mode == 12) return hardLight(b, s);
  if (mode == 13) return abs(b - s);
  if (mode == 14) return b + s - 2.0 * b * s;
  if (mode == 15) return max(b - s, 0.0);
  if (mode == 16) { if (b <= 0.0) return 0.0; if (s <= 0.0) return 1.0; return min(b / s, 1.0); }
  return s;
}

float lum(vec3 c) { return 0.30 * c.r + 0.59 * c.g + 0.11 * c.b; }
float sat(vec3 c) { return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b); }
vec3 clipColor(vec3 c) {
  float l = lum(c);
  float n = min(min(c.r, c.g), c.b);
  float x = max(max(c.r, c.g), c.b);
  vec3 o = c;
  if (n < 0.0) o = l + (o - l) * l / (l - n);
  if (x > 1.0) o = l + (o - l) * (1.0 - l) / (x - l);
  return o;
}
vec3 setLum(vec3 c, float l) { return clipColor(c + (l - lum(c))); }
vec3 setSat(vec3 c, float s) {
  float mx = max(max(c.r, c.g), c.b);
  float mn = min(min(c.r, c.g), c.b);
  if (mx <= mn) return vec3(0.0);
  return (c - mn) * s / (mx - mn);
}

vec3 blendFunction(int mode, vec3 b, vec3 s) {
  if (mode <= 1) return s;
  if (mode == 17) return setLum(setSat(s, sat(b)), lum(b));
  if (mode == 18) return setLum(setSat(b, sat(s)), lum(b));
  if (mode == 19) return setLum(s, lum(b));
  if (mode == 20) return setLum(b, lum(s));
  return vec3(separable(mode, b.r, s.r), separable(mode, b.g, s.g), separable(mode, b.b, s.b));
}

vec4 blendPremultiplied(int mode, vec4 src, vec4 dst) {
  float sa = src.a;
  float da = dst.a;
  if (sa <= 0.0) return dst;
  float invSa = 1.0 - sa;
  if (da <= 0.0 || mode <= 1) return vec4(src.rgb + dst.rgb * invSa, sa + da * invSa);
  vec3 mixed = blendFunction(mode, dst.rgb / da, src.rgb / sa);
  return vec4((1.0 - da) * src.rgb + sa * da * mixed + invSa * dst.rgb, sa + da * invSa);
}

// ---- plan.rs : bruit de Dissoudre, identique au moteur (entiers 32 bits qui débordent en boucle).
float dissolveNoise(uint x, uint y) {
  uint h = (x * 0x9E3779B1u) ^ (y * 0x85EBCA77u);
  h ^= h >> 15u;
  h *= 0x2C1B3C6Du;
  h ^= h >> 12u;
  h *= 0x297A2D39u;
  h ^= h >> 15u;
  return float(h >> 8u) / 16777216.0;
}

// ---- blend.rs (BlendIf) : comparaison, identique au moteur.
float rangeValue(vec4 p) {
  vec3 c = p.a > 0.0 ? p.rgb / p.a : vec3(0.0);
  float v = u_rangeChannel == 1 ? c.r : u_rangeChannel == 2 ? c.g : u_rangeChannel == 3 ? c.b : 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
  return v * 255.0;
}
float ramp(vec4 r, float v) {
  float low = r.y > r.x ? clamp((v - r.x) / (r.y - r.x), 0.0, 1.0) : (v >= r.x ? 1.0 : 0.0);
  float high = r.w > r.z ? clamp((r.w - v) / (r.w - r.z), 0.0, 1.0) : (v <= r.w ? 1.0 : 0.0);
  return min(low, high);
}
float compared(vec4 src, vec4 dst) {
  if (u_rangeOn == 0) return 1.0;
  return ramp(u_rangeThis, rangeValue(src)) * ramp(u_rangeUnder, rangeValue(dst));
}

// Source prête à fondre (alpha effectif, Dissoudre compris), ou alpha nul s'il n'y a rien.
vec4 prepareSource(vec4 px, float k, vec2 d) {
  if (px.a <= 0.0 || k <= 0.0) return vec4(0.0);
  if (u_mode != 1) return px * k;
  uvec2 at = uvec2(max(floor(d), vec2(0.0)));
  return dissolveNoise(at.x, at.y) < px.a * k ? vec4(px.rgb / px.a, 1.0) : vec4(0.0);
}

// ---- adjust.rs, via composite.wgsl : mêmes valeurs, mêmes calculs que le moteur.
float val(int i) { return texelFetch(u_values, ivec2(i % 1024, i / 1024), 0).r; }

float curveAt(float v, int offset) {
  float position = clamp(v, 0.0, 1.0) * 255.0;
  int index = min(int(position), 255);
  int next = min(index + 1, 255);
  float a = val(offset + index);
  return a + (val(offset + next) - a) * (position - float(index));
}

float remEuclid(float a, float m) { return a - m * floor(a / m); }

vec3 rgbToHsl(vec3 c) {
  float high = max(c.r, max(c.g, c.b));
  float low = min(c.r, min(c.g, c.b));
  float l = (high + low) / 2.0;
  float d = high - low;
  if (d < 1e-6) return vec3(0.0, 0.0, l);
  float s = d / max(1.0 - abs(2.0 * l - 1.0), 1e-6);
  float h;
  if (high == c.r) h = remEuclid((c.g - c.b) / d, 6.0);
  else if (high == c.g) h = (c.b - c.r) / d + 2.0;
  else h = (c.r - c.g) / d + 4.0;
  return vec3(h / 6.0, min(s, 1.0), l);
}

vec3 hslToRgb(vec3 hsl) {
  float c = (1.0 - abs(2.0 * hsl.z - 1.0)) * hsl.y;
  float h6 = hsl.x * 6.0;
  float x = c * (1.0 - abs(remEuclid(h6, 2.0) - 1.0));
  vec3 rgb;
  int sector = int(h6);
  if (sector == 0) rgb = vec3(c, x, 0.0);
  else if (sector == 1) rgb = vec3(x, c, 0.0);
  else if (sector == 2) rgb = vec3(0.0, c, x);
  else if (sector == 3) rgb = vec3(0.0, x, c);
  else if (sector == 4) rgb = vec3(x, 0.0, c);
  else rgb = vec3(c, 0.0, x);
  return rgb + vec3(hsl.z - c / 2.0);
}

vec3 hueSaturation(vec3 rgb) {
  float hue = val(0);
  float saturation = val(1);
  float lightness = val(2);
  bool hasRange = val(3) > 0.5;
  float start = val(4);
  float end = val(5);
  float featherIn = val(6);
  float featherOut = val(7);
  vec3 hsl = rgbToHsl(rgb);
  float weight = 1.0;
  if (hasRange) {
    float span = remEuclid(end - start, 1.0);
    float inside = remEuclid(hsl.x - start, 1.0);
    float part = 0.0;
    if (inside <= span) {
      part = 1.0;
    } else {
      float before = remEuclid(start - hsl.x, 1.0);
      float after = remEuclid(hsl.x - end, 1.0);
      if (before <= featherIn) {
        part = clamp(1.0 - before / max(featherIn, 1e-4), 0.0, 1.0);
        part = part * part * (3.0 - 2.0 * part);
      } else if (after <= featherOut) {
        part = clamp(1.0 - after / max(featherOut, 1e-4), 0.0, 1.0);
        part = part * part * (3.0 - 2.0 * part);
      }
    }
    weight = part * min(hsl.y * 8.0, 1.0);
  }
  if (weight <= 0.0) return rgb;
  float sAmount = saturation * weight;
  float lAmount = lightness * weight;
  float s = hsl.y;
  s = sAmount >= 0.0 ? s + (1.0 - s) * sAmount : s * (1.0 + sAmount);
  float l = hsl.z;
  l = lAmount >= 0.0 ? l + (1.0 - l) * lAmount : l * (1.0 + lAmount);
  return hslToRgb(vec3(remEuclid(hsl.x + hue * weight, 1.0), clamp(s, 0.0, 1.0), clamp(l, 0.0, 1.0)));
}

float safePow(float x, float y) { return x <= 0.0 ? 0.0 : pow(x, y); }
float luma(vec3 c) { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b; }
float toLinear(float v) { return v <= 0.04045 ? v / 12.92 : safePow((v + 0.055) / 1.055, 2.4); }
float toSrgb(float v) { return v <= 0.0031308 ? v * 12.92 : 1.055 * safePow(v, 1.0 / 2.4) - 0.055; }
float shoulder(float v) {
  float knee = 0.8;
  return v <= knee ? v : knee + (1.0 - knee) * (1.0 - exp(-(v - knee) / (1.0 - knee)));
}
float bell(float x, float center, float width) {
  float t = abs((x - center) / width);
  if (t >= 1.0) return 0.0;
  float u = 1.0 - t * t;
  return u * u;
}
float toneAt(float x, int offset) {
  float position = clamp(x, 0.0, 1.0) * 1024.0;
  int i = min(int(position), 1023);
  float a = val(offset + i);
  return a + (val(offset + i + 1) - a) * (position - float(i));
}

vec3 developLegacy(vec3 rgb, int offset) {
  vec3 m0 = vec3(val(offset), val(offset + 1), val(offset + 2));
  vec3 m1 = vec3(val(offset + 3), val(offset + 4), val(offset + 5));
  vec3 m2 = vec3(val(offset + 6), val(offset + 7), val(offset + 8));
  float exposure = val(offset + 9);
  float clarity = val(offset + 10);
  float dehaze = val(offset + 11);
  float vibranceAmount = val(offset + 12);
  float saturationAmount = val(offset + 13);
  int tone = offset + 14;

  vec3 clamped = clamp(rgb, 0.0, 1.0);
  vec3 lin = vec3(toLinear(clamped.r), toLinear(clamped.g), toLinear(clamped.b));
  float rolloff = clamp((exposure - 1.0) * 2.0, 0.0, 1.0);
  vec3 raw = vec3(max(dot(m0, lin), 0.0), max(dot(m1, lin), 0.0), max(dot(m2, lin), 0.0)) * exposure;
  vec3 c = vec3(
    toSrgb(min(raw.r + (shoulder(raw.r) - raw.r) * rolloff, 1.0)),
    toSrgb(min(raw.g + (shoulder(raw.g) - raw.g) * rolloff, 1.0)),
    toSrgb(min(raw.b + (shoulder(raw.b) - raw.b) * rolloff, 1.0)));

  float l0 = luma(c);
  if (l0 > 1e-5) {
    float toneOut = toneAt(l0, tone);
    toneOut = clamp((toneOut - dehaze * 0.10 * (1.0 - toneOut)) / max(1.0 - dehaze * 0.10, 0.3), 0.0, 1.0);
    float mid = 1.0 - abs(2.0 * toneOut - 1.0);
    float pushed = toneOut < 0.5
      ? 0.5 * safePow(2.0 * toneOut, 1.0 + abs(clarity) * 0.9)
      : 1.0 - 0.5 * safePow(2.0 * (1.0 - toneOut), 1.0 + abs(clarity) * 0.9);
    toneOut = toneOut + (pushed - toneOut) * mid * sign(clarity);
    c = c * (max(toneOut, 0.0) / l0);
  }

  float l = luma(c);
  vec3 chroma = c - vec3(l);
  float high = max(c.r, max(c.g, c.b));
  float low = min(c.r, min(c.g, c.b));
  float current = high > 1e-5 ? (high - low) / high : 0.0;
  float skin = 1.0;
  if (c.r > c.g && c.g > c.b) skin = 1.0 - 0.5 * bell((c.g - c.b) / max(c.r - c.b, 1e-5), 0.55, 0.45);
  float vibrance;
  if (vibranceAmount >= 0.0) {
    float k = 1.0 - current;
    vibrance = vibranceAmount * k * k * skin * 1.4;
  } else {
    vibrance = vibranceAmount * (1.0 - current * 0.5);
  }
  float factor = max((1.0 + saturationAmount) * (1.0 + vibrance) * (1.0 + max(dehaze, 0.0) * 0.2), 0.0);
  return clamp(vec3(l) + chroma * factor, 0.0, 1.0);
}

// Calage mesuré sur Lightroom : même modèle que engine::calibration, mêmes tailles de tables.
const int CAL_LUT = 129;
const int CAL_SECTORS = 96;
const int CAL_LUMA = 65;
const int CAL_BLOCK = 889;
const float CAL_CHROMA_FULL = 0.25;

float calCurve(int base, float position) {
  int k = int(clamp(floor(position), 0.0, float(CAL_LUT - 2)));
  float f = clamp(position - float(k), 0.0, 1.0);
  return clamp(mix(val(base + k), val(base + k + 1), f), 0.0, 1.0);
}

float truncMod(float a, float m) { return a - m * trunc(a / m); }

vec3 calModel(vec3 rgb, int base) {
  int flags = int(val(base));
  int terms = flags / 10;
  bool srgbSpace = flags % 10 == 1;
  vec3 clamped = clamp(rgb, 0.0, 1.0);
  vec3 b = srgbSpace ? clamped : vec3(toLinear(clamped.r), toLinear(clamped.g), toLinear(clamped.b));
  float f[6] = float[6](b.r, b.g, b.b, 0.0, 0.0, 0.0);
  if (terms == 6) {
    f[3] = sqrt(max(b.r * b.g, 0.0));
    f[4] = sqrt(max(b.g * b.b, 0.0));
    f[5] = sqrt(max(b.r * b.b, 0.0));
  }
  float o[3] = float[3](0.0, 0.0, 0.0);
  for (int c = 0; c < 3; c++) {
    float t = 0.0;
    for (int k = 0; k < 6; k++) {
      if (k < terms) t += val(base + 1 + c * 6 + k) * f[k];
    }
    float u = clamp(t, 0.0, 1.0);
    if (!srgbSpace) u = toSrgb(u);
    o[c] = calCurve(base + 19 + c * CAL_LUT, u * float(CAL_LUT - 1));
  }
  float high = max(clamped.r, max(clamped.g, clamped.b));
  float low = min(clamped.r, min(clamped.g, clamped.b));
  float chroma = high - low;
  float neutral = max(1.0 - chroma / CAL_CHROMA_FULL, 0.0);
  if (neutral > 0.0) {
    int lumBase = base + 19 + 3 * CAL_LUT + CAL_SECTORS * 3;
    float position = clamp(luma(clamped), 0.0, 1.0) * float(CAL_LUMA - 1);
    int k = min(int(floor(position)), CAL_LUMA - 2);
    float f2 = position - float(k);
    for (int c = 0; c < 3; c++) {
      float shift = mix(val(lumBase + k * 3 + c), val(lumBase + (k + 1) * 3 + c), f2);
      o[c] = clamp(o[c] + shift * neutral, 0.0, 1.0);
    }
  }
  if (chroma > 1e-6) {
    float raw;
    if (high == clamped.r) {
      raw = truncMod((clamped.g - clamped.b) / chroma, 6.0);
      if (raw < 0.0) raw += 6.0;
    } else if (high == clamped.g) {
      raw = (clamped.b - clamped.r) / chroma + 2.0;
    } else {
      raw = (clamped.r - clamped.g) / chroma + 4.0;
    }
    float hue = truncMod(raw / 6.0, 1.0);
    if (hue < 0.0) hue += 1.0;
    int hueBase = base + 19 + 3 * CAL_LUT;
    float position = hue * float(CAL_SECTORS);
    int k = int(floor(position)) % CAL_SECTORS;
    float f3 = position - floor(position);
    int next = (k + 1) % CAL_SECTORS;
    for (int c = 0; c < 3; c++) {
      float shift = mix(val(hueBase + k * 3 + c), val(hueBase + next * 3 + c), f3);
      o[c] = clamp(o[c] + shift * chroma, 0.0, 1.0);
    }
  }
  return vec3(o[0], o[1], o[2]);
}

vec3 develop(vec3 rgb) {
  int stepCount = int(val(0));
  bool hasLegacy = int(val(1)) == 1;
  int legacyAt = int(val(2));
  vec3 c = rgb;
  int cursor = 3;
  for (int i = 0; i < stepCount; i++) {
    bool hasBelow = int(val(cursor)) == 1;
    float amount = val(cursor + 1);
    cursor += 2;
    vec3 below = c;
    if (hasBelow) {
      below = calModel(c, cursor);
      cursor += CAL_BLOCK;
    }
    vec3 above = calModel(c, cursor);
    cursor += CAL_BLOCK;
    c = mix(below, above, amount);
  }
  if (hasLegacy) c = developLegacy(c, legacyAt);
  return c;
}

// Transfert de dégradé : la luminosité vidéo, décalée du bruit de tramage, choisit une couleur de la table ; l'alpha
// de l'arrêt dose le mélange. Le bruit dépend de la position dans la tuile du moteur (tuiles de 256 pixels).
vec3 gradientMap(vec3 rgb, vec2 d) {
  float noise = 0.0;
  if (val(0) > 0.5) {
    vec2 xy = mod(max(floor(d), vec2(0.0)), 256.0);
    float v = sin(xy.x * 12.9898 + xy.y * 78.233) * 43758.547;
    noise = (fract(v) - 0.5) * 2.0 / 255.0;
  }
  float l = 0.299 * rgb.r + 0.587 * rgb.g + 0.114 * rgb.b;
  float position = clamp(l + noise, 0.0, 1.0) * 255.0;
  int index = min(int(position), 255);
  int next = min(index + 1, 255);
  float k = position - float(index);
  vec4 a = vec4(val(1 + index * 4), val(2 + index * 4), val(3 + index * 4), val(4 + index * 4));
  vec4 b = vec4(val(1 + next * 4), val(2 + next * 4), val(3 + next * 4), val(4 + next * 4));
  vec4 mapped = a + (b - a) * k;
  return rgb + (mapped.rgb - rgb) * mapped.a;
}

// Inverser / Négatif : 1 - v, comme Photoshop (voir \`invert\` dans adjust.rs).
vec3 invertColor(vec3 rgb) {
  return vec3(1.0) - rgb;
}

// Couches rangées de la plus forte à la plus faible, l'ordre rouge, vert, bleu départageant les égalités (tri stable,
// comme \`ranked\` dans adjust.rs) : indices et valeurs.
void ranked(vec3 c, out ivec3 index, out vec3 value) {
  index = ivec3(0, 1, 2);
  value = c;
  if (value.y > value.x) { value.xy = value.yx; index.xy = index.yx; }
  if (value.z > value.y) { value.yz = value.zy; index.yz = index.zy; }
  if (value.y > value.x) { value.xy = value.yx; index.xy = index.yx; }
}

// Exposition, en lumière linéaire : gain, décalage, puis gamma.
vec3 exposureColor(vec3 rgb) {
  float gain = val(0);
  float offset = val(1);
  float inverseGamma = val(2);
  vec3 o;
  for (int c = 0; c < 3; c++) {
    float lin = max(toLinear(clamp(rgb[c], 0.0, 1.0)) * gain + offset, 0.0);
    o[c] = clamp(toSrgb(min(safePow(lin, inverseGamma), 1.0)), 0.0, 1.0);
  }
  return o;
}

// Balance des couleurs : chaque correction n'agit que dans sa plage de luminosité (trapèzes de somme 1).
float balanceMap(float value, float lightness, float shadows, float midtones, float highlights) {
  float a = 0.25;
  float b = 0.333;
  float scale = 0.7;
  float s = shadows * clamp((lightness - b) / -a + 0.5, 0.0, 1.0) * scale;
  float m = midtones * clamp((lightness - b) / a + 0.5, 0.0, 1.0) * clamp((lightness + b - 1.0) / -a + 0.5, 0.0, 1.0) * scale;
  float h = highlights * clamp((lightness + b - 1.0) / a + 0.5, 0.0, 1.0) * scale;
  return clamp(value + s + m + h, 0.0, 1.0);
}

vec3 colorBalance(vec3 rgb) {
  float lightness = rgbToHsl(rgb).z;
  vec3 o;
  for (int c = 0; c < 3; c++) o[c] = balanceMap(rgb[c], lightness, val(c), val(3 + c), val(6 + c));
  if (val(9) < 0.5) return o;
  vec3 hsl = rgbToHsl(o);
  return hslToRgb(vec3(hsl.x, hsl.y, lightness));
}

// Noir et blanc : la couche la plus faible, plus les écarts pondérés par la couleur secondaire puis primaire.
vec3 blackWhite(vec3 rgb) {
  ivec3 index;
  vec3 v;
  ranked(clamp(rgb, 0.0, 1.0), index, v);
  int primary = index.x * 2;
  int lo = min(index.x, index.y);
  int hi = max(index.x, index.y);
  int secondary = (lo == 0 && hi == 1) ? 1 : ((lo == 1 && hi == 2) ? 3 : 5);
  float gray = clamp(v.z + (v.y - v.z) * val(secondary) + (v.x - v.y) * val(primary), 0.0, 1.0);
  if (val(6) < 0.5) return vec3(gray);
  return hslToRgb(vec3(val(7), val(8), gray));
}

// Mélangeur de couches : une ligne (rouge, vert, bleu, constante) par couche de sortie.
vec3 channelMixer(vec3 rgb) {
  vec3 o;
  for (int c = 0; c < 3; c++) o[c] = clamp(val(c * 4) * rgb.r + val(c * 4 + 1) * rgb.g + val(c * 4 + 2) * rgb.b + val(c * 4 + 3), 0.0, 1.0);
  return o;
}

// Correction sélective : poids des neuf familles, puis cyan / magenta / jaune (et noir) de chacune, dosés selon la
// méthode (Relative : à proportion de l'encre déjà présente).
vec3 selectiveColor(vec3 rgb) {
  vec3 c = clamp(rgb, 0.0, 1.0);
  ivec3 index;
  vec3 v;
  ranked(c, index, v);
  float w[9];
  w[0] = c.r >= v.x ? v.x - v.y : 0.0;
  w[1] = c.b <= v.z ? v.y - v.z : 0.0;
  w[2] = c.g >= v.x ? v.x - v.y : 0.0;
  w[3] = c.r <= v.z ? v.y - v.z : 0.0;
  w[4] = c.b >= v.x ? v.x - v.y : 0.0;
  w[5] = c.g <= v.z ? v.y - v.z : 0.0;
  w[6] = v.z > 0.5 ? (v.z - 0.5) * 2.0 : 0.0;
  w[7] = max(1.0 - (abs(v.x - 0.5) + abs(v.z - 0.5)), 0.0);
  w[8] = v.x < 0.5 ? (0.5 - v.x) * 2.0 : 0.0;
  bool absolute = val(0) > 0.5;
  vec3 o;
  for (int ch = 0; ch < 3; ch++) {
    float scale = absolute ? 1.0 : 1.0 - c[ch];
    float delta = 0.0;
    for (int g = 0; g < 9; g++) {
      float a = val(1 + g * 4 + ch);
      float k = val(1 + g * 4 + 3);
      delta += ((-1.0 - a) * k - a) * scale * w[g];
    }
    o[ch] = clamp(c[ch] + delta, 0.0, 1.0);
  }
  return o;
}

// Isohélie : les 256 niveaux d'entrée en \`levels\` paliers égaux.
vec3 posterize(vec3 rgb) {
  float levels = val(0);
  vec3 bytes = floor(clamp(rgb, 0.0, 1.0) * 255.0 + 0.5);
  return min(floor(bytes * levels / 256.0), vec3(levels - 1.0)) / (levels - 1.0);
}

// Seuil sur la luminosité (pondération vidéo, comme le transfert de dégradé), de 0 à 255.
vec3 thresholdColor(vec3 rgb) {
  vec3 c = clamp(rgb, 0.0, 1.0);
  float l = (0.299 * c.r + 0.587 * c.g + 0.114 * c.b) * 255.0;
  return vec3(l + 1e-3 >= val(0) ? 1.0 : 0.0);
}

vec3 adjustColor(vec3 rgb, vec2 d) {
  if (u_adjKind == 1) return vec3(curveAt(rgb.r, 0), curveAt(rgb.g, 0), curveAt(rgb.b, 0));
  if (u_adjKind == 2) return hueSaturation(rgb);
  if (u_adjKind == 3) return develop(rgb);
  if (u_adjKind == 4) return gradientMap(rgb, d);
  if (u_adjKind == 5) return invertColor(rgb);
  if (u_adjKind == 6) return vec3(curveAt(rgb.r, 0), curveAt(rgb.g, 256), curveAt(rgb.b, 512));
  if (u_adjKind == 7) return exposureColor(rgb);
  if (u_adjKind == 8) return colorBalance(rgb);
  if (u_adjKind == 9) return blackWhite(rgb);
  if (u_adjKind == 10) return channelMixer(rgb);
  if (u_adjKind == 11) return selectiveColor(rgb);
  if (u_adjKind == 12) return posterize(rgb);
  if (u_adjKind == 13) return thresholdColor(rgb);
  return rgb;
}

// Rendu final d'un document en gris ou en bitmap, exactement celui du moteur (color_mode::finish_tile) : le pixel est
// d'abord arrondi en 8 bits (comme la tuile du moteur), puis la luminance Rec. 601 en entiers, sur les valeurs
// prémultipliées ; le seuil compare sans division (Y × 255 ≥ 128 × alpha), l'alpha est seuillé à 50 %.
vec4 finishColor(vec4 c) {
  ivec4 q = ivec4(floor(clamp(c, 0.0, 1.0) * 255.0 + 0.5));
  int y = (77 * q.r + 150 * q.g + 29 * q.b + 128) >> 8;
  if (u_colorMode == 1) return vec4(vec3(float(y)), float(q.a)) / 255.0;
  if (q.a < 128) return vec4(0.0);
  return y * 255 >= 128 * q.a ? vec4(1.0) : vec4(0.0, 0.0, 0.0, 1.0);
}

void main() {
  ivec2 here = ivec2(gl_FragCoord.xy);
  if (u_kind == 6) {
    vec4 c = texelFetch(u_src, here, 0);
    o_color = u_colorMode == 0 ? c : finishColor(c);
    return;
  }
  vec2 d = docPos();
  float k = u_opacity * coverage(d);
  if (u_kind == 5) { o_color = source(d); return; }
  if (u_kind == 0) {
    // Normal et Dissoudre : la carte graphique fond elle-même (ONE, ONE_MINUS_SRC_ALPHA), comme blend_premultiplied.
    vec4 src = prepareSource(source(d), k, d);
    if (src.a <= 0.0) discard;
    o_color = src;
    return;
  }
  vec4 dst = texelFetch(u_dst, here, 0);
  if (u_kind == 1) {
    vec4 px = source(d);
    o_color = blendPremultiplied(u_mode, prepareSource(px, k * compared(px, dst), d), dst);
    return;
  }
  if (u_kind == 2) {
    // Calque écrêté : fondu dans la couleur de sa base, l'alpha de la base ne change pas. Base à effets : seule la
    // part du pixel qui appartient à sa forme (pixels seuls) est couverte, pas son ombre ni son halo.
    vec4 src = prepareSource(source(d), k, d);
    float rawAlpha = placed(u_shape, u_shapePlace, u_shapeInv, d).a;
    if (dst.a <= 0.0 || src.a <= 0.0) { o_color = dst; return; }
    float within = u_shapeKind == 0 ? 1.0 : u_shapeKind == 2 ? 0.0 : clamp(rawAlpha / dst.a, 0.0, 1.0);
    if (within <= 0.0) { o_color = dst; return; }
    float sa = src.a * within;
    vec3 s = src.rgb / src.a;
    vec3 b = dst.rgb / dst.a;
    vec3 mixed = blendFunction(u_mode, b, s);
    o_color = vec4((b + (mixed - b) * sa) * dst.a, dst.a);
    return;
  }
  if (u_kind == 3) {
    vec4 before = texelFetch(u_before, here, 0);
    o_color = before + (dst - before) * k;
    return;
  }
  // Réglage : la couleur corrigée remplace l'originale à proportion de l'opacité et du masque, à travers le mode.
  if (dst.a <= 0.0 || k <= 0.0) { o_color = dst; return; }
  vec3 base = dst.rgb / dst.a;
  vec3 corrected = adjustColor(base, d);
  vec3 mixed = u_mode <= 1 ? corrected : blendFunction(u_mode, base, corrected);
  o_color = vec4((base + (mixed - base) * k) * dst.a, dst.a);
}`;

interface Target {
  texture: WebGLTexture;
  framebuffer: WebGLFramebuffer;
}

type Linker = (fragment: string) => { program: WebGLProgram; uniform: (name: string) => WebGLUniformLocation | null };

export class GpuLayers {
  /** Textures des calques (`p<id>`) et des masques (`m<id>`). */
  private readonly resident = new Map<string, Resident>();
  /** Tampons des uniformes de la pile (`drawStack`, dessin direct), réutilisés d'une image à l'autre plutôt qu'alloués à chacune. */
  private readonly stackPlaces = new Float32Array(STACK_SIZE * 4);
  private readonly stackAlphas = new Float32Array(STACK_SIZE);
  private readonly stackInverses = new Float32Array(STACK_SIZE * 9);
  private readonly stackOnes = new Float32Array(STACK_SIZE).fill(1);
  /** Chargements complets en cours ou à faire (un à la fois : pas de pointe de mémoire). */
  private readonly queue: string[] = [];
  /**
   * Textures chargées mais pas encore posées : les pixels d'un calque et son masque relus ensemble (transformation,
   * texte redessiné…) ne sont échangés qu'ENSEMBLE, dans la même image. Sinon l'écran montrait un instant les
   * nouveaux pixels sous l'ancien masque, ou l'inverse : un décalage entre le calque et son masque.
   */
  private readonly staged = new Map<string, { texture: WebGLTexture | null; commit: () => void }>();
  /** Mises à jour partielles en attente (rectangles dans le repère du calque). */
  private readonly partial = new Map<string, [number, number, number, number][]>();
  /** Nombre de repeintes signalées par calque : un chargement pendant lequel le calque a été repeint est périmé. */
  private readonly repaints = new Map<number, number>();
  /**
   * Textures refusées, avec l'état du refus : tant que ni le budget, ni le calque, ni la place prise par les autres
   * textures n'ont changé, `replan` ne les remet pas en file et le document reste au moteur. (Auparavant chaque
   * nouvelle description du document — chaque pas d'un déplacement — remettait la texture manquante en file, la
   * relisait en entier, puis la refusait à nouveau : jauge « Chargement » en boucle et gestes retardés.)
   */
  private readonly rejected = new Map<string, Rejection>();
  /** Réglages des calques de réglage (clé : leur description), et ceux en cours de préparation par le moteur. */
  private readonly adjusts = new Map<number, AdjustParams & { key: string }>();
  private readonly adjustsPending = new Map<number, string>();
  /** Effets fondus à part de chaque calque à effets (modes dessous et dessus), pour la description d'effets `key`. */
  private readonly effectParts = new Map<number, { key: string; under: BlendMode[]; over: BlendMode[] }>();
  private readonly effectsPending = new Map<number, string>();
  /** Description des effets de chaque calque lors du dernier programme : un changement relit ses textures stylées. */
  private readonly styles = new Map<number, string>();
  /** Aperçu d'un réglage : valeurs provisoires d'un calque, le temps qu'on tire un curseur. */
  private adjustPreview: (AdjustParams & { layer: number }) | null = null;
  /**
   * Calcul des effets de calque par la carte graphique (gpu-effects.ts), fourni par l'affichage : sans lui, régler un
   * effet passe par le moteur à chaque pas.
   */
  effectsCompute: GpuEffects | null = null;
  /** Aperçu des effets d'un calque pendant un réglage (voir `previewEffects`). */
  private effectsPreview: EffectsPreview | null = null;
  /** Programme de composition avec les effets de l'aperçu (calculé à la demande). */
  private previewPlan: GpuPlan | null = null;
  /**
   * Groupe déplacé dont l'aplat (calculé par le moteur au début du geste, voir move-cache.ts) remplace tout le
   * sous-programme pendant le geste : une seule fusion décalée au lieu de chacun de ses calques.
   */
  private collapse: { group: number; entry: Resident } | null = null;
  private collapsePlan: GpuPlan | null = null;
  /** Calque dont les pixels seuls restent chargés pour l'aperçu des effets (boîte de dialogue ouverte). */
  private effectsHeld: number | null = null;
  /**
   * Calque dont l'aperçu des effets a échoué ici (mémoire vidéo, calque démesuré) : ses réglages passent par le moteur
   * jusqu'à la fermeture de la boîte de dialogue, plutôt que d'alterner entre les deux chemins à chaque pas.
   */
  private effectsRefused: number | null = null;
  /** Mises à jour des pixels seuls de chaque calque : l'aperçu de ses effets est à refaire. */
  private readonly rawRevisions = new Map<number, number>();
  private busy = false;
  /** Texture en cours de chargement (voir `pending`). */
  private loading: string | null = null;
  /** Textures demandées depuis le dernier repos de la file : le total de la jauge de chargement. */
  private batchTotal = 0;
  private plan: GpuPlan = { ok: false, reason: "aucun document" };
  private doc: DocumentInfo | null = null;
  private layersById = new Map<number, LayerInfo>();
  /** Calque dont l'opacité suit un curseur (voir `GpuPlanOptions.live`). */
  private live: number | null = null;
  private readonly ops: ReturnType<Linker>;
  private readonly stack: ReturnType<Linker>;
  private readonly mipNearest: WebGLSampler;
  private readonly mipLinear: WebGLSampler;
  private readonly rendererName: string;
  /** Tampons de composition à la taille de l'écran (demi-flottants si possible), et la copie de travail. */
  private targets: Target[] = [];
  private scratch: Target | null = null;
  private targetSize: [number, number] = [0, 0];
  /** Jeux de tampons d'une autre taille, gardés pour la résolution dynamique (voir `ensureTargets`). */
  private readonly spareTargets = new Map<string, { targets: Target[]; scratch: Target | null; size: [number, number] }>();
  private readonly floatTargets: boolean;
  /** Tampons de composition en demi-flottants plutôt qu'en RGBA8 (voir le constructeur). */
  static floatBuffers = true;
  /** Change à chaque fois qu'une texture (calque, masque, effet, réglage) ou le plan change. */
  private version = 0;
  readonly maxTexture: number;
  /**
   * Tampon de dessin final : l'écran (`null`), ou la texture droite de la vue tournée (outil R, voir renderer.ts).
   * Tout ce qui est « posé à l'écran » va là.
   */
  screen: WebGLFramebuffer | null = null;
  /**
   * Appelé à chaque nouvelle texture posée (ou retouchée : nouvel objet) pour un calque : l'aperçu d'une transformation
   * note ainsi de quel état du moteur elle vient (voir renderer.ts, `TransformPreview`).
   */
  onTextureCommitted: ((id: number, texture: object) => void) | null = null;
  enabled = true;
  budgetBytes = 1024 * 1024 * 1024;

  constructor(
    private readonly gl: WebGL2RenderingContext,
    link: Linker,
    private readonly fetch: LayerFetcher,
    private readonly fetchAdjust: AdjustFetcher,
    private readonly fetchEffects: EffectsFetcher,
    private readonly requestRedraw: () => void,
  ) {
    this.ops = link(OP_SHADER);
    this.stack = link(STACK_SHADER);
    gl.useProgram(this.stack.program);
    for (let unit = 0; unit < STACK_SIZE; unit++) gl.uniform1i(this.stack.uniform(`u_tex${unit}`), unit);
    // Chaque échantillonneur sur son unité : source, copie de la cible, masque, état mémorisé, valeurs du réglage.
    gl.useProgram(this.ops.program);
    for (const [name, unit] of [["u_src", 0], ["u_dst", 1], ["u_mask", 2], ["u_before", 3], ["u_values", 4], ["u_shape", 5]] as const) gl.uniform1i(this.ops.uniform(name), unit);
    this.maxTexture = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number;
    const debug = gl.getExtension("WEBGL_debug_renderer_info");
    this.rendererName = String(gl.getParameter(debug ? debug.UNMASKED_RENDERER_WEBGL : gl.RENDERER) ?? "inconnue");
    // Tampons intermédiaires en demi-flottants quand la carte les accepte. Essayé en RGBA8 (deux fois moins de trafic
    // mémoire) : la parité avec le moteur tient partout sauf en mode Saturation (écart 38/255 : le dé-prémultiplié
    // d'une valeur arrondie à 8 bits sous faible alpha s'écarte trop). `GpuLayers.floatBuffers = false` le refait
    // pour mesurer (PARITY_FLOAT=0 dans le test de parité).
    this.floatTargets = GpuLayers.floatBuffers && gl.getExtension("EXT_color_buffer_float") !== null;
    const sampler = (mag: number) => {
      const s = gl.createSampler();
      gl.samplerParameteri(s, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.samplerParameteri(s, gl.TEXTURE_MAG_FILTER, mag);
      gl.samplerParameteri(s, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.samplerParameteri(s, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      return s;
    };
    this.mipNearest = sampler(gl.NEAREST);
    this.mipLinear = sampler(gl.LINEAR);
  }

  status(): GpuStatus {
    let bytes = 0;
    for (const entry of this.resident.values()) bytes += entry.bytes;
    const reason = !this.enabled ? "désactivée dans les préférences" : this.plan.ok ? (this.ready() ? null : "chargement des calques…") : this.plan.reason;
    return { renderer: this.rendererName, maxTexture: this.maxTexture, active: reason === null, reason, memoryMb: Math.round(bytes / 1048576) };
  }

  /** Nouveau document, ou nouvelle description du même (calques ajoutés, supprimés, propriétés changées). */
  setDocument(doc: DocumentInfo | null): void {
    if (doc?.id !== this.doc?.id || doc?.width !== this.doc?.width || doc?.height !== this.doc?.height) this.reset();
    this.doc = doc;
    this.layersById = new Map(doc?.layers.map((l) => [l.id, l]) ?? []);
    this.replan();
  }

  /** Aperçu du curseur d'opacité de ce calque (`null` : fin). */
  setLive(id: number | null): void {
    if (this.live === id) return;
    this.live = id;
    this.replan();
  }

  /** Une texture ou le plan vient de changer : la prochaine image ne pourra pas s'appuyer sur la précédente. */
  private touch(): void {
    this.version += 1;
  }

  private replan(): void {
    this.touch();
    this.previewPlan = null;
    this.collapsePlan = null;
    // Aplat d'un groupe : posé à la position actuelle du groupe (il le suit quand il est déplacé), retiré s'il a disparu.
    if (this.collapse) {
      const group = this.layersById.get(this.collapse.group);
      if (group?.isGroup) this.collapse.entry.offset = [group.offset[0], group.offset[1]];
      else this.collapse = null;
    }
    const doc = this.doc;
    // Calque de l'aperçu des effets disparu : l'aperçu aussi.
    if (this.effectsPreview && !this.layersById.has(this.effectsPreview.layer)) this.endEffectsPreview();
    if (!doc || !this.enabled) {
      this.plan = { ok: false, reason: doc ? "désactivée dans les préférences" : "aucun document" };
      return;
    }
    this.plan = gpuPlan(doc, { live: this.live });
    // Calques disparus (ou masque retiré) : leur texture est libérée.
    for (const [key, entry] of this.resident) {
      const layer = this.layersById.get(keyId(key));
      if (!layer || (key[0] === "m" && !layer.hasMask)) {
        if (entry.texture) this.gl.deleteTexture(entry.texture);
        this.resident.delete(key);
      }
    }
    for (const id of [...this.repaints.keys()]) if (!this.layersById.has(id)) this.repaints.delete(id);
    for (const key of [...this.rejected.keys()]) if (!this.layersById.has(keyId(key))) this.rejected.delete(key);
    for (const [id, params] of this.adjusts) {
      if (!this.layersById.get(id)?.adjustment) {
        this.gl.deleteTexture(params.texture);
        this.adjusts.delete(id);
      }
    }
    if (!this.plan.ok) return;
    // Une texture refusée dont rien n'a changé depuis (voir `rejected`) : le document reste au moteur, sans la relire.
    const refused = this.plannedKeys().find((key) => this.stillRejected(key));
    if (refused !== undefined) {
      this.plan = { ok: false, reason: this.rejected.get(refused)?.reason ?? "mémoire vidéo réservée dépassée" };
      return;
    }
    // Textures manquantes : chargées.
    // Effets modifiés (ajoutés, retirés, réglés) : les pixels stylés du calque et ses effets à part sont à relire.
    for (const layer of doc.layers) {
      const style = styleOf(layer);
      const known = this.styles.get(layer.id);
      this.styles.set(layer.id, style);
      if (known === undefined || known === style) continue;
      // Les anciennes textures restent affichées jusqu'à l'arrivée des nouvelles (aucun trou à l'écran).
      if (this.resident.has(`p${layer.id}`)) this.enqueue(`p${layer.id}`, true);
    }
    // Calques qui n'ont plus d'effet fondu à part : leurs effets sont oubliés.
    for (const id of [...this.effectParts.keys()]) {
      if (this.plan.effects.includes(id)) continue;
      this.effectParts.delete(id);
      for (const key of [...this.resident.keys()]) if ((key[0] === "u" || key[0] === "o") && keyId(key) === id) this.dropTexture(key);
    }
    for (const id of [...this.styles.keys()]) if (!this.layersById.has(id)) this.styles.delete(id);
    // Couleurs unies : posées à la position actuelle (elles couvrent toujours le document) ; une autre couleur, ou un
    // remplissage devenu dégradé ou motif, se repose.
    for (const [key, entry] of this.resident) this.refitSolid(key, entry);
    for (const [id, color] of [...this.solids]) {
      const now = solidFillColor(this.layersById.get(id));
      if (!now || now.join(",") !== color) {
        this.solids.delete(id);
        if (this.resident.has(`p${id}`)) this.enqueue(`p${id}`, true);
      }
    }
    for (const layer of this.plan.layers) this.enqueue(`p${layer.id}`, false);
    for (const id of this.plan.masks) this.enqueue(`m${id}`, false);
    for (const id of this.plan.raws) this.enqueue(`r${id}`, false);
    // Effets fondus à part : le moteur dit lesquels (et dans quels modes), puis leurs pixels sont chargés.
    for (const id of this.plan.effects) {
      const layer = this.layersById.get(id);
      if (!layer) continue;
      const key = styleOf(layer);
      const known = this.effectParts.get(id);
      if (known?.key === key) {
        known.under.forEach((_, n) => this.enqueue(`u${id}:${n}`, false));
        known.over.forEach((_, n) => this.enqueue(`o${id}:${n}`, false));
        continue;
      }
      if (this.effectsPending.get(id) === key) continue;
      this.effectsPending.set(id, key);
      const docId = doc.id;
      this.fetchEffects(id)
        .then((parts) => {
          if (this.doc?.id !== docId || this.effectsPending.get(id) !== key) return;
          this.effectParts.set(id, { key, under: parts.under, over: parts.over });
          this.touch();
          // Effets en trop (il y en a moins qu'avant) : retirés ; les autres sont relus.
          for (const key of [...this.resident.keys()]) {
            const side = key[0] === "u" ? parts.under : key[0] === "o" ? parts.over : null;
            if (side && keyId(key) === id && Number(key.split(":")[1]) >= side.length) this.dropTexture(key);
          }
          parts.under.forEach((_, n) => this.enqueue(`u${id}:${n}`, true));
          parts.over.forEach((_, n) => this.enqueue(`o${id}:${n}`, true));
        })
        .catch(() => undefined) // effets illisibles : le moteur garde l'affichage de ce document
        .finally(() => {
          if (this.effectsPending.get(id) === key) this.effectsPending.delete(id);
          this.pump();
          this.requestRedraw();
        });
    }
    // Réglages : mis à plat par le moteur pour chaque réglage nouveau ou modifié.
    for (const { id, adjustment } of this.plan.adjustments) {
      const key = JSON.stringify(adjustment);
      if (this.adjusts.get(id)?.key === key || this.adjustsPending.get(id) === key) continue;
      this.adjustsPending.set(id, key);
      const docId = doc.id;
      this.fetchAdjust(adjustment)
        .then((bytes) => {
          if (this.doc?.id !== docId || this.adjustsPending.get(id) !== key) return;
          const previous = this.adjusts.get(id);
          this.adjusts.set(id, { ...this.uploadParams(bytes, previous?.texture), key });
          this.touch();
        })
        .catch(() => undefined) // réglage illisible : le moteur garde l'affichage de ce document
        .finally(() => {
          if (this.adjustsPending.get(id) === key) this.adjustsPending.delete(id);
          this.requestRedraw();
        });
    }
    this.pump();
  }

  private dropTexture(key: string): void {
    this.touch();
    const entry = this.resident.get(key);
    if (entry?.texture) this.gl.deleteTexture(entry.texture);
    this.resident.delete(key);
    this.partial.delete(key);
  }

  private enqueue(key: string, reload: boolean): void {
    // Texture absente mais déjà en cours de chargement : ne pas la redemander (à l'ouverture d'un projet, le plan est
    // recalculé plusieurs fois pendant qu'un grand calque se charge ; il était relu une seconde fois en entier).
    const missing = !this.resident.has(key) && this.loading !== key;
    if ((reload || missing) && !this.queue.includes(key)) {
      if (this.queue.length === 0 && !this.busy) this.batchTotal = 0; // nouvelle fournée
      this.queue.push(key);
      this.batchTotal += 1;
    }
  }

  /** Textures que le plan courant exige (effets à part compris, quand le moteur les a décrits). */
  private plannedKeys(): string[] {
    const plan = this.plan;
    if (!plan.ok) return [];
    const keys = [...plan.layers.map((l) => `p${l.id}`), ...plan.masks.map((id) => `m${id}`), ...plan.raws.map((id) => `r${id}`)];
    for (const id of plan.effects) {
      const parts = this.effectParts.get(id);
      if (!parts) continue;
      parts.under.forEach((_, n) => keys.push(`u${id}:${n}`));
      parts.over.forEach((_, n) => keys.push(`o${id}:${n}`));
    }
    return keys;
  }

  /** Mémoire prise par les textures posées, sauf celle de `key` (aperçu des effets compris). */
  private usedBy(key: string): number {
    let used = this.effectsPreview?.result?.bytes ?? 0;
    for (const [other, entry] of this.resident) if (other !== key) used += entry.bytes;
    return used;
  }

  /** État d'un calque dont dépend l'acceptation de ses textures : ses repeintes signalées et ses effets. */
  private stampOf(id: number): string {
    const layer = this.layersById.get(id);
    // Un remplissage couvre toujours le document : le repeindre (il l'est à chaque déplacement) ne change pas sa
    // taille, un refus faute de place reste valable — sinon chaque déplacement relançait un chargement voué à l'échec.
    const repaints = layer?.fillLayer ? 0 : (this.repaints.get(id) ?? 0);
    return `${repaints}|${layer ? styleOf(layer) : ""}`;
  }

  /**
   * Calques dont les chargements sont suspendus (déplacement d'un groupe tant qu'il reste sélectionné, voir
   * `Renderer.releaseMovePreview`) : leurs textures actuelles restent affichées, décalées avec eux ; ce qui est à
   * relire ne l'est qu'une fois la suspension levée, sans jauge « Chargement » entre-temps.
   */
  private held: ReadonlySet<number> | null = null;

  holdLoads(ids: ReadonlySet<number> | null): void {
    this.held = ids;
    if (!ids) {
      this.pump();
      this.requestRedraw();
    }
  }

  private isHeld(key: string): boolean {
    return !!this.held?.has(keyId(key)) || (key[0] === "p" && (this.painting.get(keyId(key)) ?? 0) > 0);
  }

  // ------------------------------------------------------------------ peinture en direct (pinceau, gomme)
  // Le trait est peint dans la texture du calque (voir live-paint.ts). Tant que le moteur n'a pas fini ce trait, les
  // relectures de ce calque attendent (elles liraient l'état d'avant le trait et l'effaceraient de l'écran) ; à la
  // fin, la zone que le moteur signale repeinte n'est PAS relue : la texture la porte déjà.

  /** Traits en cours sur chaque calque (peints ici, pas encore finis par le moteur). */
  private readonly painting = new Map<number, number>();
  /**
   * Zones déjà à jour sur la carte graphique pour la prochaine modification signalée : `"all"` (trait peint ici, tout
   * ce qu'il a touché) ou des rectangles du repère du calque (zones envoyées par le moteur avec une annulation).
   */
  private readonly fresh = new Map<number, "all" | [number, number, number, number][]>();
  /** Refait les versions réduites d'une zone d'une texture (fourni par le renderer, voir `LivePaint.refreshTexture`). */
  refreshLevels: ((texture: WebGLTexture, width: number, height: number, levels: number, rect: readonly [number, number, number, number]) => void) | null = null;

  /**
   * Texture du calque `id` prête à être peinte, agrandie s'il le faut pour couvrir tout le document (un calque vide,
   * ou dont le contenu ne couvre qu'une partie du document, n'a qu'une texture à la taille de ce contenu) ; `null`
   * si la carte graphique ne peut pas s'en charger maintenant (texture absente ou en cours de chargement, calque
   * trop grand, mémoire réservée dépassée) : le trait passe alors par le moteur, comme avant.
   */
  paintTarget(id: number): LiveTarget | null {
    const doc = this.doc;
    const layer = this.layersById.get(id);
    const key = `p${id}`;
    const entry = this.resident.get(key);
    if (!doc || !layer || !entry || !this.enabled || !this.plan.ok || this.solids.has(id) || this.pending([id]) || this.staged.has(key)) return null;
    if (entry.offset[0] !== layer.offset[0] || entry.offset[1] !== layer.offset[1]) return null;
    // Le document dans le repère du calque.
    const page: [number, number, number, number] = [-layer.offset[0], -layer.offset[1], doc.width, doc.height];
    const covers = !!entry.texture && entry.x <= page[0] && entry.y <= page[1] && entry.x + entry.w >= page[0] + page[2] && entry.y + entry.h >= page[1] + page[3];
    let current = entry;
    if (!covers) {
      const [x, y, w, h] = entry.texture && entry.w > 0 && entry.h > 0 ? unionRect([entry.x, entry.y, entry.w, entry.h], page) : page;
      const bytes = Math.round(w * h * 4 * (4 / 3));
      if (w > this.maxTexture || h > this.maxTexture || this.usedBy(key) + bytes > this.budgetBytes) return null;
      const grown = this.growTexture(entry, [x, y, w, h]);
      if (!grown) return null;
      current = { ...entry, texture: grown, x, y, w, h, bytes };
      this.resident.set(key, current);
      this.touch();
      this.onTextureCommitted?.(id, current);
    }
    return {
      texture: current.texture!,
      width: current.w,
      height: current.h,
      levels: Math.floor(Math.log2(Math.max(current.w, current.h))) + 1,
      dx: layer.offset[0] + current.x,
      dy: layer.offset[1] + current.y,
      docWidth: doc.width,
      docHeight: doc.height,
    };
  }

  /** Nouvelle texture couvrant `rect` (repère du calque), transparente, avec le contenu de l'ancienne recopié. */
  private growTexture(entry: Resident, [x, y, w, h]: readonly [number, number, number, number]): WebGLTexture | null {
    const { gl } = this;
    const levels = Math.floor(Math.log2(Math.max(w, h))) + 1;
    const texture = gl.createTexture();
    if (!texture) return null;
    const previous = { framebuffer: gl.getParameter(gl.FRAMEBUFFER_BINDING) as WebGLFramebuffer | null, read: gl.getParameter(gl.READ_FRAMEBUFFER_BINDING) as WebGLFramebuffer | null, active: gl.getParameter(gl.ACTIVE_TEXTURE) as number, scissor: gl.isEnabled(gl.SCISSOR_TEST) };
    gl.activeTexture(gl.TEXTURE0);
    const bound = gl.getParameter(gl.TEXTURE_BINDING_2D) as WebGLTexture | null;
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, levels, gl.RGBA8, w, h);
    const draw = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, draw);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.disable(gl.SCISSOR_TEST);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (entry.texture && entry.w > 0 && entry.h > 0) {
      const read = gl.createFramebuffer();
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, read);
      gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, entry.texture, 0);
      const [ox, oy] = [entry.x - x, entry.y - y];
      gl.blitFramebuffer(0, 0, entry.w, entry.h, ox, oy, ox + entry.w, oy + entry.h, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      gl.deleteFramebuffer(read);
      gl.deleteTexture(entry.texture);
    }
    gl.generateMipmap(gl.TEXTURE_2D);
    gl.deleteFramebuffer(draw);
    gl.bindFramebuffer(gl.FRAMEBUFFER, previous.framebuffer);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, previous.read);
    gl.bindTexture(gl.TEXTURE_2D, bound);
    gl.activeTexture(previous.active);
    if (previous.scissor) gl.enable(gl.SCISSOR_TEST);
    return texture;
  }

  /** Un trait commence (ou finit, côté moteur) sur ce calque : ses relectures attendent jusqu'à la fin. */
  holdPaint(id: number): void {
    this.painting.set(id, (this.painting.get(id) ?? 0) + 1);
  }

  /** Vrai si un trait est en cours sur ce calque (faux après `reset` : sa texture a été relâchée). */
  paintHeld(id: number): boolean {
    return (this.painting.get(id) ?? 0) > 0;
  }

  releasePaint(id: number): void {
    const count = (this.painting.get(id) ?? 0) - 1;
    if (count > 0) this.painting.set(id, count);
    else this.painting.delete(id);
    this.pump();
    this.requestRedraw();
  }

  /** La texture de ce calque vient d'être peinte ici : l'image suivante ne peut pas réutiliser la précédente. */
  painted(): void {
    this.touch();
  }

  /** Zones de ce calque déjà justes sur la carte graphique pour la PROCHAINE modification signalée (voir `fresh`). */
  markFresh(id: number, rects: "all" | [number, number, number, number][]): void {
    this.fresh.set(id, rects);
  }

  clearFresh(): void {
    this.fresh.clear();
  }

  /** Relit entièrement les pixels de ce calque (trait refusé par le moteur : la texture peinte ici n'est plus juste). */
  reloadPixels(id: number): void {
    if (this.resident.has(`p${id}`)) this.enqueue(`p${id}`, true);
    this.pump();
  }

  /**
   * Pixels envoyés par le moteur avec une annulation (repère du calque, RGBA prémultiplié) : posés directement dans les
   * textures, sans relecture. Rend les calques et zones posés (à marquer `fresh` avant la modification signalée).
   */
  applyPatches(patches: readonly { layer: number; rect: readonly [number, number, number, number]; pixels: Uint8Array }[]): Map<number, [number, number, number, number][]> {
    const applied = new Map<number, [number, number, number, number][]>();
    const { gl } = this;
    for (const patch of patches) {
      const key = `p${patch.layer}`;
      const entry = this.resident.get(key);
      const [x, y, w, h] = patch.rect;
      if (!entry?.texture || this.solids.has(patch.layer) || (this.painting.get(patch.layer) ?? 0) > 0 || this.queue.includes(key) || this.loading === key) continue;
      if (w <= 0 || h <= 0 || x < entry.x || y < entry.y || x + w > entry.x + entry.w || y + h > entry.y + entry.h) continue;
      if (patch.pixels.byteLength < w * h * 4) continue;
      const active = gl.getParameter(gl.ACTIVE_TEXTURE) as number;
      gl.activeTexture(gl.TEXTURE0);
      const bound = gl.getParameter(gl.TEXTURE_BINDING_2D) as WebGLTexture | null;
      gl.bindTexture(gl.TEXTURE_2D, entry.texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, x - entry.x, y - entry.y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, patch.pixels);
      const levels = Math.floor(Math.log2(Math.max(entry.w, entry.h))) + 1;
      if (this.refreshLevels) this.refreshLevels(entry.texture, entry.w, entry.h, levels, [x - entry.x, y - entry.y, w, h]);
      else gl.generateMipmap(gl.TEXTURE_2D);
      gl.bindTexture(gl.TEXTURE_2D, bound);
      gl.activeTexture(active);
      applied.set(patch.layer, [...(applied.get(patch.layer) ?? []), [x, y, w, h]]);
      // Une relecture déjà demandée pour cette zone n'apporterait rien de plus récent.
      this.partial.delete(key);
    }
    if (applied.size > 0) this.touch();
    return applied;
  }

  /**
   * Vrai si `key` a été refusée et que rien n'a changé depuis : même calque (ni repeint, ni autres effets), même
   * budget, pas plus de place libre. Sinon le refus est oublié : on réessaiera.
   */
  private stillRejected(key: string): boolean {
    const refusal = this.rejected.get(key);
    if (!refusal) return false;
    const id = keyId(key);
    const changed = !this.layersById.has(id) || this.stampOf(id) !== refusal.stamp;
    const roomier = !refusal.tooLarge && (this.budgetBytes > refusal.budget || this.usedBy(key) < refusal.used);
    if (changed || roomier) this.rejected.delete(key);
    return !(changed || roomier);
  }

  /**
   * Chargement des textures de calques en cours : textures posées / attendues depuis le dernier repos, ou `null`
   * au repos. Nourrit la jauge de la barre d'état à l'ouverture d'un projet.
   */
  progress(): { done: number; total: number } | null {
    const waiting = this.queue.filter((key) => !this.isHeld(key)).length;
    const remaining = waiting + (this.busy ? 1 : 0);
    if (remaining === 0 || this.batchTotal === 0) return null;
    return { done: Math.max(0, this.batchTotal - remaining), total: this.batchTotal };
  }

  /**
   * Le moteur signale des changements : les calques repeints sont rechargés, en partie si possible.
   * `setDocument` doit avoir reçu, avant, la description du document APRÈS la modification : les zones
   * repeintes sont ramenées dans le repère du calque avec sa nouvelle position.
   */
  invalidate(dirty: DirtyInfo): void {
    const doc = this.doc;
    if (!doc || !this.enabled) return;
    const repainted = new Set(dirty.repainted ?? []);
    for (const id of repainted) this.repaints.set(id, (this.repaints.get(id) ?? 0) + 1);
    // Calques seulement déplacés : leurs pixels n'ont pas bougé dans leur repère, la texture suit la position.
    for (const [key, entry] of this.resident) {
      const id = keyId(key);
      if (this.refitSolid(key, entry)) {
        this.touch();
        continue;
      }
      // Suspendu : la texture actuelle suit le calque, même repeint (elle sera relue à la levée de la suspension) — sauf
      // un remplissage dégradé ou motif, qui couvre le document : décalé, il laisserait une bande vide au bord ; il
      // reste en place jusqu'à sa relecture.
      if (repainted.has(id) && !this.held?.has(id)) continue;
      if (this.held?.has(id) && this.layersById.get(id)?.fillLayer) continue;
      const layer = this.layersById.get(id);
      if (layer) entry.offset = [layer.offset[0], layer.offset[1]];
      this.touch();
    }
    // Zone touchée : connue par ses tuiles, sauf changement de fond (dirty.all) où il faut tout recharger.
    // Sans tuiles ni `all` (trait en cours, rendu différé à sa fin), il n'y a rien à recharger maintenant.
    const changed = dirty.all ? null : tilesRect(dirty.tiles, doc.tileSize);
    if (!dirty.all && !changed) return;
    for (const id of repainted) {
      const layer = this.layersById.get(id);
      if (!layer) continue;
      // Couleur unie déjà posée dans sa couleur : rien à relire (la nouvelle couleur, elle, se repose tout de suite).
      const solid = this.solids.get(id);
      const color = solidFillColor(layer);
      const sameSolid = solid !== undefined && !!color && solid === color.join(",") && this.resident.has(`p${id}`);
      if (sameSolid) {
        // Seul le masque, s'il a été repeint, est encore à recharger plus bas.
        if (!(dirty.areas ?? []).some((a) => a.layer === id && a.mask)) continue;
      }
      // Partie repeinte, quand le moteur la précise (zones) : un calque déplacé SANS son masque (ou l'inverse) n'a
      // qu'une des deux parties réécrite. L'autre garde sa texture et suit seulement la position du calque —
      // auparavant les deux étaient rechargées, et pendant le chargement le calque restait à l'ancienne place face
      // à son masque (décalage visible un instant après le relâchement).
      const areas = dirty.all ? [] : (dirty.areas ?? []).filter((a) => a.layer === id);
      const repaintedMask = areas.some((a) => a.mask);
      const repaintedPixels = areas.some((a) => !a.mask);
      const known = areas.length > 0;
      // Pixels, masque, effets : sans précision du moteur, toutes ses textures sont mises à jour.
      // Déjà à jour ici : trait peint par la carte graphique, ou zones envoyées par le moteur avec une annulation.
      const fresh = this.fresh.get(id);
      const pixelAreas = (dirty.areas ?? []).filter((a) => a.layer === id && !a.mask).map((a) => a.rect);
      const freshPixels = !dirty.all && fresh !== undefined && layer.effects.every((e) => !e.enabled) && (fresh === "all" || (pixelAreas.length > 0 && pixelAreas.every((r) => fresh.some((f) => contains(f, r)))));
      for (const key of [...this.resident.keys()].filter((k) => keyId(k) === id)) {
        const entry = this.resident.get(key);
        if (!entry) continue;
        if (key[0] === "p" && sameSolid) continue; // couleur unie : déjà juste, partout
        if (key[0] === "p" && freshPixels) {
          entry.offset = [layer.offset[0], layer.offset[1]];
          this.touch();
          continue;
        }
        // `p` porte aussi les effets fondus : avec des effets, un masque changé peut les changer, on recharge.
        const untouched = known && ((key[0] === "p" && !repaintedPixels && layer.effects.length === 0) || (key[0] === "m" && !repaintedMask));
        if (untouched) {
          entry.offset = [layer.offset[0], layer.offset[1]];
          this.touch();
          continue;
        }
        // Repeint ET repositionné (transformation, rotation, texte ou forme redessinés) : le repère de la texture
        // n'est plus celui du calque, une mise à jour partielle écrirait les nouveaux pixels au mauvais endroit.
        const rebased = entry.offset[0] !== layer.offset[0] || entry.offset[1] !== layer.offset[1];
        if (rebased || !changed || this.held?.has(id)) {
          this.enqueue(key, true); // tout recharger ; l'ancienne texture reste affichée d'ici là
          continue;
        }
        // Zone repeinte dans le repère du calque : celle que donne le moteur, sans limite (un calque peut déborder du
        // document, et les tuiles du document s'arrêtent à ses bords) ; à défaut, les tuiles du document ramenées dans
        // le repère du calque. Si elle déborde de la texture, il faut l'agrandir.
        const areas = (dirty.areas ?? []).filter((a) => a.layer === id && a.mask === (key[0] === "m"));
        const local: [number, number, number, number] =
          areas.length > 0 ? union(areas.map((a) => a.rect)) : [changed[0] - layer.offset[0], changed[1] - layer.offset[1], changed[2], changed[3]];
        const inside = intersect(local, [entry.x, entry.y, entry.w, entry.h]);
        // Masque : hors de sa texture, il vaut son fond ; une retouche là (hors d'un masque vide) l'agrandit aussi.
        const grows = !inside || inside[2] < local[2] || inside[3] < local[3];
        if (grows) this.enqueue(key, true);
        else this.partial.set(key, [...(this.partial.get(key) ?? []), inside]);
      }
    }
    this.pump();
  }

  /** Vrai si le document affiché peut être entièrement composé par la carte graphique, maintenant. */
  ready(): boolean {
    const plan = this.plan;
    if (!this.enabled || !plan.ok) return false;
    // Calque dont l'aperçu des effets est affiché : ses effets relus du moteur peuvent manquer un instant.
    const previewed = this.effectsPreview?.view ? this.effectsPreview.layer : null;
    return (
      plan.layers.every((layer) => this.resident.has(`p${layer.id}`)) &&
      plan.masks.every((id) => this.resident.has(`m${id}`)) &&
      plan.raws.every((id) => this.resident.has(`r${id}`)) &&
      plan.effects.every((id) => {
        if (id === previewed) return true;
        const parts = this.effectParts.get(id);
        return !!parts && parts.under.every((_, n) => this.resident.has(`u${id}:${n}`)) && parts.over.every((_, n) => this.resident.has(`o${id}:${n}`));
      }) &&
      plan.adjustments.every(({ id }) => this.adjusts.has(id) || this.adjustPreview?.layer === id)
    );
  }

  /**
   * Vrai si une texture de l'un de ces calques (pixels, masque, effets à part, réglage) est encore en file ou en
   * cours de chargement. Plus étroit que `settled` : un geste n'attend que ce qu'il a repeint.
   */
  pending(ids: Iterable<number>): boolean {
    const wanted = new Set(ids);
    if (wanted.size === 0) return false;
    if (this.loading !== null && wanted.has(keyId(this.loading))) return true;
    if (this.queue.some((key) => wanted.has(keyId(key)))) return true;
    for (const key of this.partial.keys()) if (wanted.has(keyId(key))) return true;
    for (const id of this.adjustsPending.keys()) if (wanted.has(id)) return true;
    for (const id of this.effectsPending.keys()) if (wanted.has(id)) return true;
    return false;
  }

  /** Plus rien en cours de chargement. */
  get settled(): boolean {
    return !this.busy && this.queue.length === 0 && this.partial.size === 0 && this.adjustsPending.size === 0 && this.effectsPending.size === 0;
  }

  /** Aperçu d'un réglage : ces valeurs (`adjustment_gpu`) remplacent celles du calque jusqu'à `endAdjustPreview`. */
  setAdjustPreview(layer: number, bytes: ArrayBuffer): void {
    this.adjustPreview = { ...this.uploadParams(bytes, this.adjustPreview?.texture), layer };
    this.touch();
  }

  endAdjustPreview(): void {
    if (this.adjustPreview) this.gl.deleteTexture(this.adjustPreview.texture);
    this.adjustPreview = null;
    this.touch();
  }

  /** Vrai si le calque de réglage est appliqué par la carte graphique (son aperçu peut alors passer par elle). */
  composes(layer: number): boolean {
    return this.ready() && this.plan.ok && this.plan.adjustments.some((a) => a.id === layer);
  }

  // ------------------------------------------------------------------ aperçu des effets de calque

  /**
   * Garde chargés les pixels seuls de ce calque (`null` : fin), le temps que la boîte de dialogue des effets est
   * ouverte : le premier mouvement d'un curseur n'attend pas leur chargement.
   */
  holdEffectsSource(id: number | null): void {
    const previous = this.effectsHeld;
    this.effectsHeld = id;
    if (id !== previous) this.effectsRefused = null;
    if (previous !== null && previous !== id && this.effectsPreview?.layer !== previous) this.dropPreviewSource(previous);
    if (id !== null && this.canPreviewEffects(id)) {
      this.enqueue(`r${id}`, false);
      this.pump();
    }
  }

  /**
   * Rendu logiciel (llvmpipe, SwiftShader…) : le processeur y émule la carte graphique, et un flou y coûte plusieurs
   * fois le calcul du moteur (mesuré : 6 à 50 s par pas sur un calque de 2000 px, contre 1 à 5 s au moteur). L'aperçu
   * des effets y reste donc au moteur (les tests de parité le forcent).
   */
  static effectsOnSoftware = false;

  /** Vrai si les effets de ce calque peuvent être calculés ici pendant un réglage. */
  canPreviewEffects(id: number): boolean {
    const plan = this.plan;
    if (!this.effectsCompute?.supported || !plan.ok || !this.ready() || !plan.layers.some((l) => l.id === id)) return false;
    if (!GpuLayers.effectsOnSoftware && /llvmpipe|softpipe|swiftshader|software|basic render/i.test(this.rendererName)) return false;
    if (this.stillRejected(`r${id}`) || this.effectsRefused === id) return false;
    return !(this.effectsPreview?.layer === id && this.effectsPreview.failed);
  }

  /**
   * Aperçu des effets d'un calque : ces effets, calculés par la carte graphique, remplacent ceux que le moteur a
   * dessinés jusqu'à `finishEffectsPreview` (ou `endEffectsPreview`). Faux si c'est impossible ici : le moteur doit
   * alors rendre chaque pas, comme avant.
   */
  previewEffects(id: number, effects: readonly LayerEffect[]): boolean {
    // Biseautage, incrustation de motif : pas calculés ici (voir gpu-effects.ts), le moteur rend chaque pas.
    if (!this.canPreviewEffects(id) || !this.effectsCompute?.handles(effects)) return false;
    if (this.effectsPreview && this.effectsPreview.layer !== id) this.endEffectsPreview();
    const key = JSON.stringify(effects);
    const current = this.effectsPreview;
    if (current?.key === key) {
      current.releasing = false;
      return true;
    }
    this.effectsPreview = {
      layer: id,
      effects: JSON.parse(key) as LayerEffect[],
      key,
      result: current?.result ?? null,
      view: current?.view ?? null,
      stamp: current?.stamp ?? "",
      failed: false,
      releasing: false,
    };
    this.previewPlan = null;
    this.enqueue(`r${id}`, false);
    this.pump();
    this.touch();
    return true;
  }

  /** Calque dont les effets sont en aperçu, s'il y en a un. */
  get effectsPreviewLayer(): number | null {
    return this.effectsPreview?.layer ?? null;
  }

  /** Le moteur a reçu les derniers effets : l'aperçu cédera la place dès que ses textures seront arrivées. */
  finishEffectsPreview(): void {
    if (this.effectsPreview) this.effectsPreview.releasing = true;
  }

  /** Vrai si l'aperçu des effets, relâché, peut s'effacer : le rendu du moteur est entièrement chargé. */
  effectsPreviewDone(): boolean {
    const preview = this.effectsPreview;
    return !!preview && preview.releasing && this.ready() && !this.pending([preview.layer]);
  }

  /** Fin immédiate de l'aperçu des effets (annulation, ou rendu du moteur arrivé). */
  endEffectsPreview(): void {
    const preview = this.effectsPreview;
    if (!preview) return;
    if (preview.result) this.effectsCompute?.release(preview.result);
    this.effectsCompute?.trim();
    this.effectsPreview = null;
    this.previewPlan = null;
    if (this.effectsHeld !== preview.layer) this.dropPreviewSource(preview.layer);
    this.touch();
  }

  /** Pixels seuls chargés pour l'aperçu des effets, libérés s'ils ne servent pas au programme (base d'écrêtage). */
  private dropPreviewSource(id: number): void {
    if (this.plan.ok && this.plan.raws.includes(id)) return;
    const at = this.queue.indexOf(`r${id}`);
    if (at >= 0) this.queue.splice(at, 1);
    if (this.resident.has(`r${id}`)) this.dropTexture(`r${id}`);
  }

  /**
   * Calcule l'aperçu des effets s'il a changé (effets réglés, pixels seuls arrivés ou retouchés). Appelé avant de
   * composer une image ; laisse l'écran comme cible, sur toute la vue.
   */
  private refreshEffectsPreview(view: GpuView): void {
    const preview = this.effectsPreview;
    const compute = this.effectsCompute;
    if (!preview || !compute || preview.failed) return;
    const raw = this.resident.get(`r${preview.layer}`);
    if (!raw) return; // pixels seuls en chemin : l'image d'avant reste affichée
    const stamp = `${preview.key}|${this.rawRevisions.get(preview.layer) ?? 0}|${raw.x},${raw.y},${raw.w},${raw.h}`;
    if (preview.stamp === stamp && preview.view) {
      preview.view.main.offset = raw.offset;
      for (const part of [...preview.view.under, ...preview.view.over]) part.entry.offset = raw.offset;
      return;
    }
    preview.stamp = stamp;
    this.previewPlan = null;
    this.touch();
    const active = preview.effects.some((e) => e.enabled);
    if (!active || !raw.texture) {
      // Aucun effet actif (ou calque vide) : le calque se compose tel quel, pixels seuls.
      if (preview.result) compute.release(preview.result);
      preview.result = null;
      preview.view = { main: raw, under: [], over: [] };
      return;
    }
    const need = compute.estimate(raw, preview.effects);
    const used = this.usedBy("") - (preview.result?.bytes ?? 0);
    if (need === null || used + need > this.budgetBytes) {
      // Calque démesuré ou mémoire vidéo réservée dépassée : le moteur garde la main.
      preview.failed = true;
      this.effectsRefused = preview.layer;
      if (preview.result) compute.release(preview.result);
      preview.result = null;
      preview.view = null;
      return;
    }
    const result = compute.render({ texture: raw.texture, x: raw.x, y: raw.y, w: raw.w, h: raw.h }, preview.effects, preview.result);
    if (!result && preview.result) compute.release(preview.result); // calque vidé : rien n'a été repris
    preview.result = result;
    if (!result) {
      preview.view = { main: { ...raw, texture: null, w: 0, h: 0 }, under: [], over: [] };
    } else {
      const [x, y, w, h] = result.rect;
      const entry = (texture: WebGLTexture): Resident => ({ texture, x, y, w, h, bytes: 0, offset: raw.offset, background: 0 });
      preview.view = {
        main: entry(result.main),
        under: result.under.map((part) => ({ mode: part.mode, entry: entry(part.texture) })),
        over: result.over.map((part) => ({ mode: part.mode, entry: entry(part.texture) })),
      };
    }
    const { gl } = this;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.screen);
    gl.viewport(0, 0, view.viewW, view.viewH);
    gl.activeTexture(gl.TEXTURE0);
  }

  /** Texture d'un calque pour la composition : celles de l'aperçu des effets remplacent les textures stylées. */
  private entryOf(key: string): Resident | undefined {
    if (this.collapse && key === `p${this.collapse.group}`) return this.collapse.entry;
    const view = this.effectsPreview?.view;
    if (view && keyId(key) === this.effectsPreview!.layer && key[0] !== "m" && key[0] !== "r") {
      if (key[0] === "p") return view.main;
      const n = Number(key.split(":")[1] ?? 0);
      return (key[0] === "u" ? view.under : view.over)[n]?.entry;
    }
    return this.resident.get(key);
  }

  /** Modes des effets fondus à part d'un calque (ceux de l'aperçu, s'il est affiché). */
  private partsOf(id: number): { under: BlendMode[]; over: BlendMode[] } | undefined {
    const view = this.effectsPreview?.view;
    if (view && id === this.effectsPreview!.layer) return { under: view.under.map((p) => p.mode), over: view.over.map((p) => p.mode) };
    return this.effectParts.get(id);
  }

  /** Programme à dessiner : celui du document, ou, pendant l'aperçu des effets, celui avec les effets de l'aperçu. */
  private drawingPlan(): GpuPlan {
    const preview = this.effectsPreview;
    const doc = this.doc;
    if (!preview?.view && this.collapse && doc && this.plan.ok) {
      this.collapsePlan ??= gpuPlan(doc, { live: this.live, collapsed: this.collapse.group });
      return this.collapsePlan;
    }
    if (!preview?.view || !doc || !this.plan.ok) return this.plan;
    if (!this.previewPlan) {
      const layers = doc.layers.map((l) => (l.id === preview.layer ? { ...l, effects: preview.effects } : l));
      const plan = gpuPlan({ ...doc, layers }, { live: this.live });
      // Base d'écrêtage qui prend des effets : ses pixels seuls sont déjà là (ils servent au calcul).
      const complete = plan.ok && plan.raws.every((id) => this.resident.has(`r${id}`));
      this.previewPlan = complete ? plan : this.plan;
    }
    return this.previewPlan;
  }

  /** Valeurs d'un réglage (type u32, puis f32) dans une texture R32F, rangées de `VALUES_ROW` valeurs. */
  private uploadParams(bytes: ArrayBuffer, reuse?: WebGLTexture): AdjustParams {
    const { gl } = this;
    const kind = new DataView(bytes, 0, 4).getUint32(0, true);
    const values = new Float32Array(bytes.slice(4));
    const rows = Math.max(1, Math.ceil(values.length / VALUES_ROW));
    const padded = new Float32Array(rows * VALUES_ROW);
    padded.set(values);
    const texture = reuse ?? gl.createTexture();
    gl.activeTexture(gl.TEXTURE4);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R32F, VALUES_ROW, rows, 0, gl.RED, gl.FLOAT, padded);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.bindTexture(gl.TEXTURE_2D, null);
    gl.activeTexture(gl.TEXTURE0);
    return { kind, texture };
  }

  // ------------------------------------------------------------------ déplacement par images en cache

  /**
   * Pendant le déplacement du groupe `group` : son aplat `slice` (sans son opacité ni son masque, voir move-cache.ts)
   * remplace son sous-programme, une seule fusion avec l'opacité et le masque du groupe — même image, une fraction du
   * travail par image pour un groupe de dizaines de calques. `null` : fin, le programme complet reprend.
   */
  setCollapse(group: number | null, slice: CachedSlice | null, origin: readonly [number, number] | null = null): void {
    const layer = group === null ? undefined : this.layersById.get(group);
    if (!layer?.isGroup || !slice?.texture) {
      if (this.collapse) this.touch();
      this.collapse = null;
      this.collapsePlan = null;
      return;
    }
    // L'aplat est rangé relativement à la position du groupe quand il a été préparé (`origin`) ; posé à la position
    // actuelle, il suit le groupe déplacé (voir `replan`).
    const [ox, oy] = origin ?? layer.offset;
    const [x, y, w, h] = slice.rect;
    if (this.collapse?.group === layer.id && this.collapse.entry.texture === slice.texture) return;
    this.collapse = { group: layer.id, entry: { texture: slice.texture, x: x - ox, y: y - oy, w, h, bytes: 0, offset: [layer.offset[0], layer.offset[1]], background: 0 } };
    this.collapsePlan = null;
    this.touch();
  }

  /** Groupe dont l'aplat est affiché en ce moment, s'il y en a un. */
  get collapsedGroup(): number | null {
    return this.collapse?.group ?? null;
  }

  /**
   * Aperçu d'un déplacement par images en cache (move-cache.ts) : les couches sont assemblées du bas vers le haut,
   * chacune dans son mode, les réglages appliqués à ce qui est dessous, les couches qui suivent les calques déplacés
   * (`moving`) décalées de `shift`, puis transformées par `matrix` (cadre de transformation, Ctrl+T : homographie du
   * document d'avant le geste vers le document) — mêmes calculs que le programme de composition. Rien n'est demandé
   * au moteur ni chargé.
   */
  drawCached(view: GpuView, size: { width: number; height: number }, slices: readonly CachedSlice[], shift: readonly [number, number], level = 0, matrix: TransformMatrix | null = null): void {
    const { gl } = this;
    const { scale, originX, originY, viewW, viewH } = view;
    const work = intersect(snapRect(originX, originY, originX + size.width * scale, originY + size.height * scale), [0, 0, viewW, viewH]);
    if (!work) return;
    // Cas courant (groupe de calques normaux, rien d'autre qu'en mode Normal au-dessus) : une seule passe de pile,
    // directement à l'écran, comme une superposition de calques (voir STACK_SHADER).
    const drawn = slices.filter((s) => s.texture);
    const plain = slices.every((s) => s.kind !== "adjust" && (s.mode === "normal" || s.mode === "pass_through"));
    const movingInverse = columns(matrix?.inverse);
    /** Emprise à l'écran d'un rectangle du document (transformé s'il suit les calques déplacés), limitée à `work`. */
    const screenRect = (place: readonly number[], moving: boolean): [number, number, number, number] | null => {
      const [px, py, pw, ph] = place as [number, number, number, number];
      let corners: [number, number][] = [[px, py], [px + pw, py], [px + pw, py + ph], [px, py + ph]];
      if (moving && matrix) {
        const f = matrix.forward;
        corners = corners.map(([x, y]) => {
          const w = f[6]! * x + f[7]! * y + f[8]!;
          return w > 1e-9 ? [(f[0]! * x + f[1]! * y + f[2]!) / w, (f[3]! * x + f[4]! * y + f[5]!) / w] : [Number.NaN, Number.NaN];
        });
        // Point derrière l'horizon d'une perspective : toute la zone de travail.
        if (corners.some(([x]) => !Number.isFinite(x))) return work;
      }
      const xs = corners.map(([x]) => originX + x * scale);
      const ys = corners.map(([, y]) => originY + y * scale);
      return intersect(snapRect(Math.floor(Math.min(...xs)) - 1, Math.floor(Math.min(...ys)) - 1, Math.ceil(Math.max(...xs)) + 1, Math.ceil(Math.max(...ys)) + 1), work);
    };
    // Gris ou bitmap : pas de dessin direct, le rendu final se fait à la présentation (voir `finishMode`).
    if (plain && drawn.length <= STACK_SIZE && this.finishMode(scale, level) === 0) {
      if (drawn.length === 0) return;
      const sampler = scale > 1 ? this.mipNearest : this.mipLinear;
      const places = this.stackPlaces;
      const inverses = this.stackInverses;
      drawn.forEach((slice, unit) => {
        const [dx, dy] = slice.moving ? shift : [0, 0];
        places[unit * 4] = slice.rect[0] + dx;
        places[unit * 4 + 1] = slice.rect[1] + dy;
        places[unit * 4 + 2] = slice.rect[2];
        places[unit * 4 + 3] = slice.rect[3];
        inverses.set(slice.moving ? movingInverse : IDENTITY_COLUMNS, unit * 9);
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, slice.texture);
        gl.bindSampler(unit, sampler);
      });
      gl.activeTexture(gl.TEXTURE0);
      const u = this.stack.uniform;
      gl.useProgram(this.stack.program);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.screen);
      gl.viewport(0, 0, viewW, viewH);
      gl.uniform2f(u("u_view"), viewW, viewH);
      gl.uniform3f(u("u_camera"), originX, originY, scale);
      gl.uniform4f(u("u_rect"), 0, 0, viewW, viewH);
      gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
      gl.uniform1i(u("u_count"), drawn.length);
      gl.uniform4fv(u("u_place"), places);
      gl.uniform1fv(u("u_alpha"), this.stackOnes);
      gl.uniformMatrix3fv(u("u_inv"), false, inverses);
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(work[0], viewH - work[1] - work[3], work[2], work[3]);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      gl.disable(gl.SCISSOR_TEST);
      this.leaveStack(null);
      return;
    }
    this.ensureTargets(1, viewW, viewH);
    const scratch = this.scratch!;
    const target = this.targets[0]!;
    const u = this.ops.uniform;
    const sampler = scale > 1 ? this.mipNearest : this.mipLinear;
    gl.useProgram(this.ops.program);
    gl.uniform2f(u("u_view"), viewW, viewH);
    gl.uniform3f(u("u_camera"), originX, originY, scale);
    gl.viewport(0, 0, viewW, viewH);
    gl.uniform4f(u("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
    for (const name of ["u_srcInv", "u_maskInv", "u_shapeInv"]) gl.uniformMatrix3fv(u(name), false, IDENTITY_COLUMNS);
    gl.uniform1i(u("u_srcLayer"), 1);
    gl.uniform1f(u("u_opacity"), 1);
    gl.enable(gl.SCISSOR_TEST);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    const scissor = (rect: readonly number[]) => gl.scissor(rect[0]!, viewH - rect[1]! - rect[3]!, rect[2]!, rect[3]!);
    gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
    scissor(work);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    /** Copie de la cible (unité 1) pour les fusions qui lisent ce qui est dessous. */
    const snapshot = (rect: readonly number[]) => {
      const [x, y, w, h] = rect as [number, number, number, number];
      const [y0, y1] = [viewH - y - h, viewH - y];
      scissor(rect);
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, target.framebuffer);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, scratch.framebuffer);
      gl.blitFramebuffer(x, y0, x + w, y1, x, y0, x + w, y1, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, scratch.texture);
      gl.activeTexture(gl.TEXTURE0);
    };
    const draw = (rect: readonly number[], kind: number, blend: boolean) => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, target.framebuffer);
      scissor(rect);
      if (blend) gl.enable(gl.BLEND);
      else gl.disable(gl.BLEND);
      gl.uniform1i(u("u_kind"), kind);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    };
    for (const slice of slices) {
      gl.uniform1i(u("u_mode"), MODES[slice.mode]);
      if (slice.kind === "adjust") {
        if (!slice.adjust) continue;
        gl.uniform1f(u("u_opacity"), slice.opacity);
        gl.uniform1i(u("u_maskOn"), slice.texture ? 1 : 0);
        if (slice.texture) {
          // Réglage du groupe déplacé : son masque suit le groupe (décalé, transformé).
          const [mx, my] = slice.moving ? shift : [0, 0];
          gl.activeTexture(gl.TEXTURE2);
          gl.bindSampler(2, sampler);
          gl.bindTexture(gl.TEXTURE_2D, slice.texture);
          gl.uniform4f(u("u_maskPlace"), slice.rect[0] + mx, slice.rect[1] + my, slice.rect[2], slice.rect[3]);
          gl.uniformMatrix3fv(u("u_maskInv"), false, slice.moving ? movingInverse : IDENTITY_COLUMNS);
          gl.uniform1f(u("u_maskBg"), slice.maskBackground);
        }
        gl.uniform1i(u("u_adjKind"), slice.adjust.kind);
        gl.activeTexture(gl.TEXTURE4);
        gl.bindTexture(gl.TEXTURE_2D, slice.adjust.texture);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, null);
        snapshot(work);
        draw(work, KIND.adjust, false);
        gl.uniform1f(u("u_opacity"), 1);
        gl.uniformMatrix3fv(u("u_maskInv"), false, IDENTITY_COLUMNS);
        continue;
      }
      if (!slice.texture) continue;
      const [dx, dy] = slice.moving ? shift : [0, 0];
      const place: [number, number, number, number] = [slice.rect[0] + dx, slice.rect[1] + dy, slice.rect[2], slice.rect[3]];
      const rect = screenRect(place, slice.moving);
      if (!rect) continue;
      gl.uniform1i(u("u_maskOn"), 0);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindSampler(0, sampler);
      gl.bindTexture(gl.TEXTURE_2D, slice.texture);
      gl.uniform4f(u("u_srcPlace"), ...place);
      gl.uniformMatrix3fv(u("u_srcInv"), false, slice.moving ? movingInverse : IDENTITY_COLUMNS);
      if (slice.mode === "normal" || slice.mode === "pass_through" || slice.mode === "dissolve") {
        draw(rect, KIND.over, true);
        continue;
      }
      snapshot(rect);
      draw(rect, KIND.blend, false);
    }
    // Résultat posé sur le damier, à l'écran.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.screen);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, target.texture);
    gl.uniform1i(u("u_kind"), KIND.present);
    gl.uniform1i(u("u_colorMode"), this.finishMode(view.scale, level));
    gl.enable(gl.BLEND);
    scissor(work);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.SCISSOR_TEST);
    gl.bindSampler(2, null);
    for (const unit of [1, 2, 4]) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, null);
    }
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, null);
  }

  // ------------------------------------------------------------------ dessin

  /** Dessine le document à l'écran (framebuffer courant : l'écran), par-dessus le damier. */
  draw(view: GpuView, overrides: GpuOverrides): void {
    if (!this.plan.ok) return;
    this.refreshEffectsPreview(view);
    const plan = this.drawingPlan();
    if (!plan.ok) return;
    if (plan.simple) {
      this.drawSimple(plan.layers, view, overrides);
      return;
    }
    // Tout le document visible est recomposé à chaque image. (Une recomposition partielle — seule la zone des calques
    // déplacés — a existé en beta89h-90 : juste au pixel près, mais c'est une source de décalages si une emprise est
    // mal estimée, pour un gain sans intérêt sur une carte qui compose l'écran entier en une fraction de milliseconde.)
    this.drawProgram(plan.program.ops, plan.program.buffers, view, overrides);
  }

  /**
   * Rendu final selon le mode du document (Image › Mode), comme le moteur : gris (1) ; bitmap seuillé (2) à partir de
   * 100 %, où chaque pixel d'écran montre un pixel du document — en zoom arrière, seulement en gris, comme les niveaux
   * réduits du moteur (un seuil sur une image réduite n'aurait aucun sens). Règle commune : `finishModeFor` (camera.ts).
   * `level` : niveau des textures dessinées (images d'un déplacement préparées à un niveau réduit : en gris).
   */
  private finishMode(scale: number, level = 0): number {
    return finishModeFor(this.doc?.colorMode?.kind, level, scale);
  }

  /** Opacité effective d'un calque pour une opération, aperçu compris. */
  private opacityOf(id: number, overrides: GpuOverrides): number {
    const layer = this.layersById.get(id);
    if (!layer) return 0;
    return overrides.opacity(id, layer.opacity * layer.fill, layer.fill);
  }

  /**
   * Superposition de calques en mode Normal, sans masque : dessinée directement à l'écran, par piles de STACK_SIZE
   * calques (une passe chacune) — le cas le plus courant, et le plus rapide.
   */
  private drawSimple(layers: readonly { id: number }[], view: GpuView, overrides: GpuOverrides): void {
    const { gl } = this;
    const { scale, originX, originY, viewW, viewH } = view;
    const doc = this.doc;
    if (!doc) return;
    const docRect = intersect(snapRect(originX, originY, originX + doc.width * scale, originY + doc.height * scale), [0, 0, viewW, viewH]);
    if (!docRect) return;
    gl.enable(gl.SCISSOR_TEST);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.screen);
    const ids = layers.map((layer) => layer.id);
    for (let first = 0; first < ids.length; first += STACK_SIZE) this.drawStack(ids.slice(first, first + STACK_SIZE), view, docRect, overrides);
    gl.disable(gl.SCISSOR_TEST);
    this.leaveStack(null);
  }

  /**
   * Emprise à l'écran d'une texture de calque, limitée à `within` ; `null` si elle est vide. Transformée (aperçu du
   * cadre) : la boîte englobante du quadrilatère transformé — tout `within` si un coin part à l'infini.
   */
  private placedRect(entry: Resident, id: number, view: GpuView, within: readonly number[], overrides: GpuOverrides, kind: TextureKind = "pixels"): readonly number[] | null {
    const { scale, originX, originY } = view;
    const [dx, dy] = overrides.shift(id, kind, entry);
    const rect = [entry.offset[0] + dx + entry.x, entry.offset[1] + dy + entry.y, entry.w, entry.h];
    const matrix = overrides.matrix?.(id, kind, entry) ?? null;
    if (matrix) {
      if (entry.w <= 0 || entry.h <= 0) return null;
      const box = transformedBounds(rect, matrix.forward);
      if (!box) return within;
      const [x0, y0, x1, y1] = [originX + box[0] * scale, originY + box[1] * scale, originX + box[2] * scale, originY + box[3] * scale];
      return intersect(snapRect(Math.floor(x0) - 1, Math.floor(y0) - 1, Math.ceil(x1) + 1, Math.ceil(y1) + 1), within);
    }
    const x0 = originX + rect[0]! * scale;
    const y0 = originY + rect[1]! * scale;
    return intersect(snapRect(Math.floor(x0) - 1, Math.floor(y0) - 1, Math.ceil(x0 + entry.w * scale) + 1, Math.ceil(y0 + entry.h * scale) + 1), within);
  }

  /** Matrice d'aperçu de cette texture pour le shader (par colonnes), l'identité si elle n'est pas transformée. */
  private inverseOf(entry: Resident, id: number, kind: TextureKind, overrides: GpuOverrides, out?: Float32Array, at = 0): Float32Array {
    return columns(overrides.matrix?.(id, kind, entry)?.inverse, out, at);
  }

  /**
   * Une pile de calques (mode Normal, sans masque, du bas vers le haut) fondue en une passe sur le tampon de dessin
   * courant, limitée (ciseaux) à l'union de leurs emprises dans `within`. Les calques vides ou invisibles sont sautés.
   * Laisse le programme de pile actif et des échantillonneurs sur les unités 0..STACK_SIZE : voir `leaveStack`.
   */
  private drawStack(ids: readonly number[], view: GpuView, within: readonly number[], overrides: GpuOverrides): void {
    const { gl } = this;
    const { scale, originX, originY, viewW, viewH } = view;
    const sampler = scale > 1 ? this.mipNearest : this.mipLinear;
    const places = this.stackPlaces;
    const alphas = this.stackAlphas;
    const inverses = this.stackInverses;
    let count = 0;
    let union: readonly number[] | null = null;
    for (const id of ids) {
      const entry = this.entryOf(`p${id}`);
      const alpha = this.opacityOf(id, overrides);
      if (!entry?.texture || alpha <= 0) continue;
      const rect = this.placedRect(entry, id, view, within, overrides);
      if (!rect) continue;
      union = union ? unionRect(union, rect) : rect;
      const [dx, dy] = overrides.shift(id, "pixels", entry);
      places[count * 4] = entry.offset[0] + dx + entry.x;
      places[count * 4 + 1] = entry.offset[1] + dy + entry.y;
      places[count * 4 + 2] = entry.w;
      places[count * 4 + 3] = entry.h;
      alphas[count] = alpha;
      this.inverseOf(entry, id, "pixels", overrides, inverses, count * 9);
      gl.activeTexture(gl.TEXTURE0 + count);
      gl.bindTexture(gl.TEXTURE_2D, entry.texture);
      gl.bindSampler(count, sampler);
      count += 1;
    }
    gl.activeTexture(gl.TEXTURE0);
    if (count === 0 || !union) return;
    const u = this.stack.uniform;
    gl.useProgram(this.stack.program);
    gl.uniform2f(u("u_view"), viewW, viewH);
    gl.uniform3f(u("u_camera"), originX, originY, scale);
    gl.uniform4f(u("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
    gl.uniform1i(u("u_count"), count);
    gl.uniform4fv(u("u_place"), places);
    gl.uniform1fv(u("u_alpha"), alphas);
    gl.uniformMatrix3fv(u("u_inv"), false, inverses);
    gl.scissor(union[0]!, viewH - union[1]! - union[3]!, union[2]!, union[3]!);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
  }

  /** Après une ou plusieurs piles : plus de calque lié sur les unités 1.., échantillonneurs rendus (`keep` : unités à garder). */
  private leaveStack(keep: WebGLSampler | null, units: readonly number[] = []): void {
    const { gl } = this;
    for (let unit = 0; unit < STACK_SIZE; unit++) {
      gl.bindSampler(unit, units.includes(unit) ? keep : null);
      if (unit > 0) {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, null);
      }
    }
    gl.activeTexture(gl.TEXTURE0);
  }

  /**
   * Tampons de composition à la taille de l'écran : au moins `count`, plus la copie de travail. Les jeux des deux
   * dernières tailles sont gardés : la résolution dynamique alterne entre la taille de l'écran et une taille
   * réduite à chaque geste, et réallouer trois tampons 4K en demi-flottants à chaque bascule était un à-coup.
   */
  private ensureTargets(count: number, viewW: number, viewH: number): void {
    const { gl } = this;
    const resized = this.targetSize[0] !== viewW || this.targetSize[1] !== viewH;
    if (resized) {
      if (this.targetSize[0] > 0) this.spareTargets.set(`${this.targetSize[0]}×${this.targetSize[1]}`, { targets: this.targets, scratch: this.scratch, size: this.targetSize });
      const key = `${viewW}×${viewH}`;
      const spare = this.spareTargets.get(key);
      this.spareTargets.delete(key);
      this.targets = spare?.targets ?? [];
      this.scratch = spare?.scratch ?? null;
      this.targetSize = [viewW, viewH];
      // Au plus un jeu de côté (l'autre taille du geste en cours) : les plus anciens sont libérés.
      while (this.spareTargets.size > 1) {
        const [oldest, set] = this.spareTargets.entries().next().value!;
        this.spareTargets.delete(oldest);
        for (const target of [...set.targets, ...(set.scratch ? [set.scratch] : [])]) {
          gl.deleteTexture(target.texture);
          gl.deleteFramebuffer(target.framebuffer);
        }
      }
    }
    const make = (): Target => {
      const texture = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, texture);
      if (this.floatTargets) gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA16F, viewW, viewH);
      else gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, viewW, viewH);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      const framebuffer = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      return { texture, framebuffer };
    };
    while (this.targets.length < count) this.targets.push(make());
    this.scratch ??= make();
  }

  /**
   * Exécute le programme de composition à la résolution de l'écran, puis pose le résultat sur le damier. Chaque
   * opération est limitée (ciseaux) au document visible, et à l'emprise du calque quand elle en dépend.
   */
  private drawProgram(ops: readonly GpuOp[], buffers: number, view: GpuView, overrides: GpuOverrides): void {
    const { gl } = this;
    const { scale, originX, originY, viewW, viewH } = view;
    const doc = this.doc;
    if (!doc) return;
    // Document visible, en pixels de l'écran (origine en haut à gauche).
    const docRect = intersect(snapRect(originX, originY, originX + doc.width * scale, originY + doc.height * scale), [0, 0, viewW, viewH]);
    if (!docRect) return;
    const area: readonly number[] = docRect;
    this.ensureTargets(buffers, viewW, viewH);
    const scratch = this.scratch!;
    const u = this.ops.uniform;
    const work = area;

    gl.useProgram(this.ops.program);
    gl.uniform2f(u("u_view"), viewW, viewH);
    gl.uniform3f(u("u_camera"), originX, originY, scale);
    gl.viewport(0, 0, viewW, viewH);
    gl.uniform4f(u("u_rect"), 0, 0, viewW, viewH);
    gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
    gl.enable(gl.SCISSOR_TEST);
    const layerSampler = scale > 1 ? this.mipNearest : this.mipLinear;
    gl.bindSampler(2, layerSampler);
    gl.bindSampler(5, layerSampler);

    /** Rectangle d'écran (origine en haut à gauche) -> ciseaux OpenGL (origine en bas à gauche). */
    const scissor = (rect: readonly number[]) => gl.scissor(rect[0]!, viewH - rect[1]! - rect[3]!, rect[2]!, rect[3]!);
    /** Emprise d'une texture de calque à l'écran, limitée au document visible. */
    const placedRect = (entry: Resident, id: number) => this.placedRect(entry, id, view, work, overrides);
    /** Matrice d'aperçu de chaque rectangle de texture du shader. */
    const inverseName: Record<string, string> = { u_srcPlace: "u_srcInv", u_maskPlace: "u_maskInv", u_shapePlace: "u_shapeInv" };
    const place = (name: string, entry: Resident, id: number, kind: TextureKind) => {
      const [dx, dy] = overrides.shift(id, kind, entry);
      gl.uniform4f(u(name), entry.offset[0] + dx + entry.x, entry.offset[1] + dy + entry.y, entry.w, entry.h);
      gl.uniformMatrix3fv(u(inverseName[name]!), false, this.inverseOf(entry, id, kind, overrides));
    };
    /** Source d'une opération : sa texture est liée à l'unité 0 ; retourne son emprise, ou `null` si elle est vide. */
    const bindSource = (source: GpuSource): readonly number[] | null => {
      gl.activeTexture(gl.TEXTURE0);
      if ("buffer" in source) {
        gl.uniform1i(u("u_srcLayer"), 0);
        gl.bindSampler(0, null);
        gl.bindTexture(gl.TEXTURE_2D, this.targets[source.buffer]!.texture);
        return work;
      }
      return bindTexture(`p${source.layer}`);
    };
    /** Texture d'un calque (pixels ou effet) comme source : retourne son emprise, ou `null` si elle est vide. */
    const bindTexture = (key: string): readonly number[] | null => {
      const entry = this.entryOf(key);
      if (!entry?.texture) return null;
      const id = keyId(key);
      gl.activeTexture(gl.TEXTURE0);
      gl.uniform1i(u("u_srcLayer"), 1);
      gl.bindSampler(0, layerSampler);
      gl.bindTexture(gl.TEXTURE_2D, entry.texture);
      place("u_srcPlace", entry, id, "pixels");
      return placedRect(entry, id);
    };
    /** Opacité et masque de l'opération, lus sur le calque (`opacity` : autre opacité que opacité × fond). */
    const bindLayer = (id: number, opacity = this.opacityOf(id, overrides)) => {
      gl.uniform1f(u("u_opacity"), opacity);
      const layer = this.layersById.get(id);
      const mask = layer?.hasMask && layer.maskEnabled ? this.resident.get(`m${id}`) : undefined;
      gl.uniform1i(u("u_maskOn"), mask ? 1 : 0);
      if (!mask) return;
      gl.activeTexture(gl.TEXTURE2);
      gl.bindTexture(gl.TEXTURE_2D, mask.texture);
      place("u_maskPlace", mask, id, "mask");
      gl.uniform1f(u("u_maskBg"), mask.background);
      gl.activeTexture(gl.TEXTURE0);
    };
    /** Copie de la cible dans la copie de travail (unité 1), pour les opérations qui lisent ce qui est dessous. */
    const snapshot = (target: number, rect: readonly number[]) => {
      const [x, y, w, h] = rect as [number, number, number, number];
      const [y0, y1] = [viewH - y - h, viewH - y];
      scissor(rect); // les ciseaux limitent aussi les copies : ceux de l'opération précédente couperaient celle-ci
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, this.targets[target]!.framebuffer);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, scratch.framebuffer);
      gl.blitFramebuffer(x, y0, x + w, y1, x, y0, x + w, y1, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets[target]!.framebuffer);
      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, scratch.texture);
      gl.activeTexture(gl.TEXTURE0);
    };
    const drawInto = (target: number, rect: readonly number[], kind: number, blend: boolean) => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets[target]!.framebuffer);
      scissor(rect);
      if (blend) gl.enable(gl.BLEND);
      else gl.disable(gl.BLEND);
      gl.uniform1i(u("u_kind"), kind);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    };
    const clear = (target: number) => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets[target]!.framebuffer);
      scissor(work);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
    };

    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    if (area) clear(0);
    const hasMaskOn = (id: number) => {
      const layer = this.layersById.get(id);
      return !!layer?.hasMask && layer.maskEnabled;
    };
    for (const op of area ? stackRuns(ops, hasMaskOn) : []) {
      if (op.op === "stack") {
        // Suite de calques en mode Normal sans masque : une passe pour la pile entière.
        gl.bindFramebuffer(gl.FRAMEBUFFER, this.targets[op.target]!.framebuffer);
        this.drawStack(op.layers, view, work, overrides);
        this.leaveStack(layerSampler, [2, 5]);
        gl.useProgram(this.ops.program);
        continue;
      }
      // Aucune texture de tampon ne reste liée d'une opération à l'autre : la cible de celle-ci pourrait être l'une
      // d'elles (lire et écrire la même texture est interdit, même dans une branche du shader non suivie).
      for (const unit of [0, 3]) {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, null);
      }
      gl.activeTexture(gl.TEXTURE0);
      switch (op.op) {
        case "clear":
          clear(op.target);
          break;
        case "copy": {
          const [x, y, w, h] = work as [number, number, number, number];
          const [y0, y1] = [viewH - y - h, viewH - y];
          scissor(work);
          gl.bindFramebuffer(gl.READ_FRAMEBUFFER, this.targets[op.source]!.framebuffer);
          gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, this.targets[op.target]!.framebuffer);
          gl.blitFramebuffer(x, y0, x + w, y1, x, y0, x + w, y1, gl.COLOR_BUFFER_BIT, gl.NEAREST);
          break;
        }
        case "load": {
          const rect = bindSource({ layer: op.layer });
          if (!rect) {
            clear(op.target);
            break;
          }
          gl.uniform1i(u("u_maskOn"), 0);
          drawInto(op.target, work, KIND.load, false);
          break;
        }
        case "blend":
        case "clip": {
          const rect = bindSource(op.source);
          if (!rect) break; // calque vide : rien à fondre
          bindLayer(op.layer);
          gl.uniform1i(u("u_mode"), MODES[op.mode]);
          if (op.op === "clip") {
            // Base à effets : les calques écrêtés s'arrêtent à ses pixels seuls (texture `r`), pas à son ombre.
            const raw = typeof op.shape === "number" ? this.resident.get(`r${op.shape}`) : undefined;
            gl.uniform1i(u("u_shapeKind"), op.shape === undefined ? 0 : raw?.texture ? 1 : 2);
            if (typeof op.shape === "number" && raw?.texture) {
              gl.activeTexture(gl.TEXTURE5);
              gl.bindTexture(gl.TEXTURE_2D, raw.texture);
              place("u_shapePlace", raw, op.shape, "pixels");
              gl.activeTexture(gl.TEXTURE0);
            }
          }
          const range = op.op === "blend" ? op.range : undefined;
          const direct = op.op === "blend" && !range && (op.mode === "normal" || op.mode === "dissolve" || op.mode === "pass_through");
          if (direct) {
            drawInto(op.target, rect, KIND.over, true);
            break;
          }
          if (range) {
            // Comparaison : la part visible dépend aussi de l'image dessous (copie de travail).
            gl.uniform1i(u("u_rangeOn"), 1);
            gl.uniform1i(u("u_rangeChannel"), ["gray", "red", "green", "blue"].indexOf(range.channel));
            gl.uniform4f(u("u_rangeThis"), ...range.this);
            gl.uniform4f(u("u_rangeUnder"), ...range.under);
          }
          snapshot(op.target, rect);
          drawInto(op.target, rect, op.op === "blend" ? KIND.blend : KIND.clip, false);
          if (range) gl.uniform1i(u("u_rangeOn"), 0);
          break;
        }
        case "effects": {
          // Effets fondus à part (dessous, ou dessus), chacun dans son mode, avec l'opacité du calque (sans son
          // fond : le fond n'atténue que le calque lui-même) et son masque.
          const parts = this.partsOf(op.layer);
          const layer = this.layersById.get(op.layer);
          if (!parts || !layer) break;
          const opacity = overrides.opacity(op.layer, layer.opacity, 1);
          (op.side === "under" ? parts.under : parts.over).forEach((mode, n) => {
            const rect = bindTexture(`${op.side === "under" ? "u" : "o"}${op.layer}:${n}`);
            if (!rect) return;
            bindLayer(op.layer, opacity);
            gl.uniform1i(u("u_mode"), MODES[mode]);
            if (mode === "normal" || mode === "dissolve" || mode === "pass_through") {
              drawInto(op.target, rect, KIND.over, true);
              return;
            }
            snapshot(op.target, rect);
            drawInto(op.target, rect, KIND.blend, false);
          });
          break;
        }
        case "mix": {
          bindLayer(op.layer);
          gl.activeTexture(gl.TEXTURE3);
          gl.bindTexture(gl.TEXTURE_2D, this.targets[op.before]!.texture);
          gl.activeTexture(gl.TEXTURE0);
          snapshot(op.target, work);
          drawInto(op.target, work, KIND.mix, false);
          break;
        }
        case "adjust": {
          const params = this.adjustPreview?.layer === op.layer ? this.adjustPreview : this.adjusts.get(op.layer);
          if (!params) break;
          bindLayer(op.layer);
          gl.uniform1i(u("u_mode"), MODES[op.mode]);
          gl.uniform1i(u("u_adjKind"), params.kind);
          gl.activeTexture(gl.TEXTURE4);
          gl.bindTexture(gl.TEXTURE_2D, params.texture);
          gl.activeTexture(gl.TEXTURE0);
          snapshot(op.target, work);
          drawInto(op.target, work, KIND.adjust, false);
          break;
        }
      }
    }

    // Résultat posé sur le damier, à l'écran.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.screen);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, this.targets[0]!.texture);
    gl.uniform1i(u("u_kind"), KIND.present);
    gl.uniform1i(u("u_colorMode"), this.finishMode(scale));
    gl.enable(gl.BLEND);
    scissor(docRect);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    gl.disable(gl.SCISSOR_TEST);
    gl.bindSampler(2, null);
    gl.bindSampler(5, null);
    // Aucune texture de tampon ne reste liée : la prochaine passe pourrait écrire dedans.
    for (const unit of [1, 3]) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, null);
    }
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, null);
  }

  reset(): void {
    this.touch();
    this.painting.clear();
    this.fresh.clear();
    this.collapse = null;
    this.collapsePlan = null;
    this.endEffectsPreview();
    this.rawRevisions.clear();
    this.effectsRefused = null;
    for (const entry of this.resident.values()) if (entry.texture) this.gl.deleteTexture(entry.texture);
    this.resident.clear();
    this.repaints.clear();
    this.rejected.clear();
    this.queue.length = 0;
    this.batchTotal = 0;
    for (const held of this.staged.values()) if (held.texture) this.gl.deleteTexture(held.texture);
    this.staged.clear();
    this.partial.clear();
    for (const params of this.adjusts.values()) this.gl.deleteTexture(params.texture);
    this.adjusts.clear();
    this.adjustsPending.clear();
    this.effectParts.clear();
    this.effectsPending.clear();
    this.styles.clear();
    this.endAdjustPreview();
  }

  // ------------------------------------------------------------------ chargements

  private pump(): void {
    if (this.busy) return;
    const doc = this.doc;
    if (!doc) return;
    // Rien ne reste à charger pour le compagnon d'une texture en attente : elle est posée (sécurité : un chargement
    // du compagnon abandonné, par exemple).
    for (const [key, held] of [...this.staged]) {
      if (!this.queue.includes(this.sibling(key))) {
        this.staged.delete(key);
        held.commit();
      }
    }
    // Chargements suspendus (voir `held`) : on passe au suivant, ils restent en file.
    const at = this.queue.findIndex((key) => !this.isHeld(key));
    const full = at < 0 ? undefined : this.queue.splice(at, 1)[0];
    const partialEntry = full === undefined ? [...this.partial.entries()].find(([key]) => !this.isHeld(key)) : undefined;
    if (full === undefined && !partialEntry) return;
    this.busy = true;
    this.loading = full ?? partialEntry![0];
    const docId = doc.id;
    const job = full !== undefined ? this.loadFull(full) : this.loadPartial(partialEntry![0], partialEntry![1]);
    job
      .catch(() => undefined) // un calque illisible reste simplement affiché par le moteur
      .finally(() => {
        this.busy = false;
        this.loading = null;
        if (this.queue.length === 0 && this.partial.size === 0) this.batchTotal = 0; // fournée terminée
        if (this.doc?.id === docId) this.requestRedraw();
        this.pump();
      });
  }

  private async loadFull(key: string): Promise<void> {
    const doc = this.doc;
    if (!doc) return;
    const id = keyId(key);
    if (key[0] === "p" && this.placeSolid(key)) return;
    const part = keyPart(key);
    const kind: TextureKind = part === "mask" ? "mask" : "pixels";
    this.partial.delete(key); // le chargement complet couvre aussi les retouches en attente
    const generation = this.repaints.get(id) ?? 0;
    // Texture acceptable : au plus `maxTexture` de côté, et pas plus de pixels que le budget restant n'en loge (les
    // niveaux de détail prennent un tiers de plus). Le moteur vérifie AVANT de lire les pixels : au-delà, il ne
    // renvoie que l'en-tête, et le calque entier n'est ni copié ni transféré pour être refusé.
    const texel = kind === "mask" ? 1 : 4;
    const perPixel = texel * (4 / 3);
    const maxPixels = Math.min(0xffffffff, Math.max(0, Math.floor((this.budgetBytes - this.usedBy(key)) / perPixel)));
    const buffer = await this.fetch(id, null, part, [this.maxTexture, maxPixels]);
    if (this.doc?.id !== doc.id) return;
    // Position à laquelle ces pixels sont vrais. Repeint ET déplacé pendant la lecture (transformation) : la texture
    // est posée à la position du départ et relue aussitôt. Seulement repeint : les retouches en attente la
    // compléteront. Seulement déplacé : la position actuelle.
    const startLayer = doc.layers.find((l) => l.id === id);
    const nowLayer = this.layersById.get(id);
    const moved = !!startLayer && !!nowLayer && (startLayer.offset[0] !== nowLayer.offset[0] || startLayer.offset[1] !== nowLayer.offset[1]);
    const stale = moved && (this.repaints.get(id) ?? 0) !== generation;
    const source = stale ? startLayer : (nowLayer ?? startLayer);
    const offset: [number, number] = source ? [source.offset[0], source.offset[1]] : [0, 0];
    const header = kind === "mask" ? 20 : 16;
    const view = new DataView(buffer, 0, header);
    const [x, y, w, h] = [view.getInt32(0, true), view.getInt32(4, true), view.getUint32(8, true), view.getUint32(12, true)];
    const background = kind === "mask" ? view.getUint32(16, true) / 255 : 0;
    const { gl } = this;
    if (w === 0 || h === 0) {
      // Calque vide (ou masque uniforme) : rien à dessiner, mais il est « chargé ».
      if (stale) this.enqueue(key, true);
      this.place(key, null, { texture: null, x: 0, y: 0, w: 0, h: 0, bytes: 0, offset, background });
      return;
    }
    // Trop grand pour une texture, ou au-delà du budget (le moteur n'a alors envoyé que l'en-tête ; ou bien la place a
    // manqué entre-temps) : ce document reste au moteur. Le refus est retenu (voir `rejected`).
    const used = this.usedBy(key);
    const tooLarge = w > this.maxTexture || h > this.maxTexture;
    if (tooLarge || used + w * h * perPixel > this.budgetBytes || buffer.byteLength < header + w * h * texel) {
      const reason = tooLarge ? `calque plus grand que ${this.maxTexture} px` : "mémoire vidéo réservée dépassée";
      // État du calque tel qu'il a été lu (repeint pendant la lecture : le prochain plan réessaiera une fois).
      const stamp = `${nowLayer?.fillLayer ? 0 : generation}|${nowLayer ? styleOf(nowLayer) : ""}`;
      this.rejected.set(key, { reason, tooLarge, stamp, budget: this.budgetBytes, used });
      // Pixels seuls demandés pour l'aperçu des effets seulement : le document reste composé ici, l'aperçu non.
      if (this.plannedKeys().includes(key)) this.plan = { ok: false, reason };
      else if (this.effectsPreview?.layer === id && key[0] === "r") [this.effectsPreview.failed, this.effectsRefused] = [true, id];
      return;
    }
    if (stale) this.enqueue(key, true);
    this.rejected.delete(key);
    const levels = Math.floor(Math.log2(Math.max(w, h))) + 1;
    const texture = gl.createTexture();
    const bind = () => {
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    };
    bind();
    gl.texStorage2D(gl.TEXTURE_2D, levels, kind === "mask" ? gl.R8 : gl.RGBA8, w, h);
    // Envoi par bandes, quelques millisecondes par image affichée (voir `paced`) : un calque 4K (45 Mio) envoyé d'un
    // coup figeait l'affichage, surtout sur une carte intégrée. Les bandes font au plus 4 Mio.
    const rowsPerStrip = Math.max(1, Math.floor(UPLOAD_STRIP_BYTES / (w * texel)));
    for (let row = 0; row < h; row += rowsPerStrip) {
      const rows = Math.min(rowsPerStrip, h - row);
      await paced(() => {
        bind();
        const pixels = new Uint8Array(buffer, header + row * w * texel, rows * w * texel);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, row, w, rows, kind === "mask" ? gl.RED : gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      });
      if (this.doc?.id !== doc.id) {
        gl.deleteTexture(texture);
        return;
      }
    }
    await paced(() => {
      bind();
      gl.generateMipmap(gl.TEXTURE_2D);
    });
    if (this.doc?.id !== doc.id) {
      gl.deleteTexture(texture);
      return;
    }
    this.place(key, texture, { texture, x, y, w, h, bytes: Math.round(w * h * texel * (4 / 3)), offset, background });
  }

  /** Couleurs unies posées (voir `solidFillColor`), par calque : leur couleur « r,g,b ». */
  private solids = new Map<number, string>();

  /**
   * Remplissage « Couleur unie » : une texture d'un seul pixel, étirée sur bien plus que le document (trois fois sa
   * taille, centrée sur lui) — décalée par un aperçu de déplacement, elle le couvre encore ; le reste est coupé au
   * bord du document. Vrai si `key` est une couleur unie ainsi posée.
   */
  private placeSolid(key: string): boolean {
    const id = keyId(key);
    const layer = this.layersById.get(id);
    const color = solidFillColor(layer);
    const doc = this.doc;
    if (!color || !layer || !doc) return false;
    const { gl } = this;
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, 1, 1);
    gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([color[0], color[1], color[2], 255]));
    gl.bindTexture(gl.TEXTURE_2D, null);
    this.solids.set(id, color.join(","));
    this.partial.delete(key);
    this.place(key, texture, { texture, ...solidRect(doc, layer), bytes: 4, offset: [layer.offset[0], layer.offset[1]], background: 0 });
    return true;
  }

  /** Couleur unie posée : remise sur tout le document à la position actuelle du calque. Vrai si `key` en est une. */
  private refitSolid(key: string, entry: Resident): boolean {
    if (key[0] !== "p") return false;
    const id = keyId(key);
    const layer = this.layersById.get(id);
    const doc = this.doc;
    if (!this.solids.has(id) || !layer || !doc || !solidFillColor(layer)) return false;
    Object.assign(entry, solidRect(doc, layer), { offset: [layer.offset[0], layer.offset[1]] });
    return true;
  }

  /** L'autre texture du même calque, à échanger en même temps : masque pour les pixels, pixels pour le masque. */
  private sibling(key: string): string {
    const id = keyId(key);
    return key[0] === "m" ? `p${id}` : `m${id}`;
  }

  /**
   * Pose une texture chargée. L'ancienne n'est remplacée qu'une fois la nouvelle prête : aucun trou à l'écran. Si le
   * compagnon (pixels ↔ masque) est lui aussi en cours de rechargement, elle attend qu'il arrive, et les deux sont
   * posés d'un coup.
   */
  private place(key: string, texture: WebGLTexture | null, entry: Resident): void {
    const commit = () => {
      const previous = this.resident.get(key);
      if (previous?.texture) this.gl.deleteTexture(previous.texture);
      this.resident.set(key, entry);
      if (key[0] === "r") this.rawRevisions.set(keyId(key), (this.rawRevisions.get(keyId(key)) ?? 0) + 1);
      this.touch();
      this.onTextureCommitted?.(keyId(key), entry);
    };
    const sibling = this.sibling(key);
    if (key[0] !== "r" && key[0] !== "u" && key[0] !== "o" && this.queue.includes(sibling)) {
      const held = this.staged.get(key);
      if (held?.texture) this.gl.deleteTexture(held.texture);
      this.staged.set(key, { texture, commit });
      return;
    }
    commit();
    const waiting = this.staged.get(sibling);
    if (waiting) {
      this.staged.delete(sibling);
      waiting.commit();
    }
  }

  private async loadPartial(key: string, rects: [number, number, number, number][]): Promise<void> {
    this.partial.delete(key);
    if (key[0] === "p" && this.solids.has(keyId(key))) return; // couleur unie : rien à relire
    const entry = this.resident.get(key);
    if (!entry?.texture) return;
    const id = keyId(key);
    const part = keyPart(key);
    const kind: TextureKind = part === "mask" ? "mask" : "pixels";
    // Une seule demande couvrant toutes les zones repeintes depuis la dernière mise à jour.
    const [x0, y0] = [Math.min(...rects.map((r) => r[0])), Math.min(...rects.map((r) => r[1]))];
    const [x1, y1] = [Math.max(...rects.map((r) => r[0] + r[2])), Math.max(...rects.map((r) => r[1] + r[3]))];
    const buffer = await this.fetch(id, [x0, y0, x1 - x0, y1 - y0], part, null);
    const current = this.resident.get(key);
    if (current !== entry || !entry.texture) return; // remplacé entre-temps par un chargement complet
    const header = kind === "mask" ? 20 : 16;
    const view = new DataView(buffer, 0, header);
    const [w, h] = [view.getUint32(8, true), view.getUint32(12, true)];
    const { gl } = this;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, entry.texture);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    if (kind === "mask") gl.texSubImage2D(gl.TEXTURE_2D, 0, x0 - entry.x, y0 - entry.y, w, h, gl.RED, gl.UNSIGNED_BYTE, new Uint8Array(buffer, header, w * h));
    else gl.texSubImage2D(gl.TEXTURE_2D, 0, x0 - entry.x, y0 - entry.y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(buffer, header, w * h * 4));
    if (key[0] === "r") this.rawRevisions.set(id, (this.rawRevisions.get(id) ?? 0) + 1);
    this.touch();
    gl.generateMipmap(gl.TEXTURE_2D);
    // Contenu changé : nouvel objet (même texture). L'aperçu d'une transformation reconnaît une texture à cet objet
    // (`GpuOverrides.matrix`) : une texture retouchée par une étape du moteur porte l'état de cette étape.
    const updated = { ...entry };
    this.resident.set(key, updated);
    this.onTextureCommitted?.(id, updated);
  }
}

/** Vrai si le rectangle `outer` contient entièrement `inner` (`[x, y, largeur, hauteur]`). */
function contains(outer: readonly number[], inner: readonly number[]): boolean {
  return inner[0]! >= outer[0]! && inner[1]! >= outer[1]! && inner[0]! + inner[2]! <= outer[0]! + outer[2]! && inner[1]! + inner[3]! <= outer[1]! + outer[3]!;
}

/** Plus petit rectangle `[x, y, largeur, hauteur]` qui contient tous ceux-ci. */
function union(rects: readonly (readonly number[])[]): [number, number, number, number] {
  const x0 = Math.min(...rects.map((r) => r[0]!));
  const y0 = Math.min(...rects.map((r) => r[1]!));
  const x1 = Math.max(...rects.map((r) => r[0]! + r[2]!));
  const y1 = Math.max(...rects.map((r) => r[1]! + r[3]!));
  return [x0, y0, x1 - x0, y1 - y0];
}

/** Bords alignés sur les pixels physiques, en `[x, y, largeur, hauteur]`. */
/** Union (englobant) de deux rectangles `[x, y, largeur, hauteur]`. */
function unionRect(a: readonly number[], b: readonly number[]): [number, number, number, number] {
  const x0 = Math.min(a[0]!, b[0]!);
  const y0 = Math.min(a[1]!, b[1]!);
  const x1 = Math.max(a[0]! + a[2]!, b[0]! + b[2]!);
  const y1 = Math.max(a[1]! + a[3]!, b[1]! + b[3]!);
  return [x0, y0, x1 - x0, y1 - y0];
}

/** Opération de pile : des `blend` consécutifs en mode Normal, sans masque, sur la même cible, fondus en une passe. */
type StackOp = { op: "stack"; target: number; layers: number[] };

/**
 * Regroupe les suites de `blend` Normal sans masque (sur la même cible, source = calque) en opérations de pile de
 * STACK_SIZE calques au plus. Une suite d'un seul calque reste une opération ordinaire. Module pur : testé à part.
 */
export function stackRuns(ops: readonly GpuOp[], hasMaskOn: (id: number) => boolean): (GpuOp | StackOp)[] {
  const out: (GpuOp | StackOp)[] = [];
  const stackable = (op: GpuOp): op is Extract<GpuOp, { op: "blend" }> & { source: { layer: number } } =>
    op.op === "blend" && (op.mode === "normal" || op.mode === "pass_through") && !op.range && "layer" in op.source && !hasMaskOn(op.layer);
  let i = 0;
  while (i < ops.length) {
    const op = ops[i]!;
    if (!stackable(op)) {
      out.push(op);
      i += 1;
      continue;
    }
    let end = i + 1;
    while (end < ops.length && end - i < STACK_SIZE) {
      const next = ops[end]!;
      if (!stackable(next) || next.target !== op.target) break;
      end += 1;
    }
    if (end - i === 1) out.push(op);
    else out.push({ op: "stack", target: op.target, layers: ops.slice(i, end).map((o) => (o as { source: { layer: number } }).source.layer) });
    i = end;
  }
  return out;
}

function snapRect(x0: number, y0: number, x1: number, y1: number): [number, number, number, number] {
  const rx = Math.round(x0);
  const ry = Math.round(y0);
  return [rx, ry, Math.round(x1) - rx, Math.round(y1) - ry];
}
