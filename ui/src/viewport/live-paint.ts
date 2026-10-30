// Pinceau et gomme en temps réel : le trait est peint DIRECTEMENT dans la texture du calque, sur la carte graphique,
// avec les mêmes empreintes et les mêmes formules que le moteur (crates/engine/src/paint.rs : `stamp_tip`, `merge`,
// `round_dab`, `Stroke::effective`, `Stroke::resolve`). Ce qui s'affiche pendant le geste est le trait définitif : à la
// fin, rien n'est relu ni remplacé (avant : une esquisse posée par-dessus l'image, jetée au relâchement et remplacée
// par les pixels relus au moteur — le trait changeait d'aspect, et il fallait attendre ce rechargement).
//
// Le moteur reçoit les mêmes empreintes en coulisse (enregistrement, export, historique). Repère des textures : celui
// du calque, décalé du coin de la texture (`dx`, `dy` : un pixel du document (X, Y) est le texel (X - dx, Y - dy)).
//
// Pendant le trait, deux textures de travail : la couverture du trait (comme `Stroke::coverage`) et une copie de la
// texture du calque au début du trait (comme `Snapshot::Pixels`). Le trait est recomposé à chaque image sur la zone
// touchée : copie ⊕ encre × couverture, puis les versions réduites de cette zone sont refaites. Entre deux traits qui
// s'enchaînent, les textures de travail sont gardées (les allouer, les remplir et les libérer à chaque trait coûtait
// plus qu'un trait court sur un grand document) ; elles sont libérées après quelques secondes sans peindre.
import type { Dab } from "../lib/brush-path";
import type { DualTipStamp, TipStamp } from "../lib/tip-stamps";

/** Texture du calque à peindre. */
export interface LiveTarget {
  texture: WebGLTexture;
  width: number;
  height: number;
  /** Nombre de niveaux de détail de la texture (versions réduites). */
  levels: number;
  /** Coin de la texture dans le document : texel = pixel du document − (dx, dy). */
  dx: number;
  dy: number;
  /** Taille du document : on ne peint pas au-delà (comme `Stroke::canvas`). */
  docWidth: number;
  docHeight: number;
}

/** Image d'une pointe (un octet de couverture par pixel). */
export interface LiveTipImage {
  width: number;
  height: number;
  mask: Uint8Array;
}

/** Modes de combinaison du pinceau double et de la texture (mêmes codes que le shader de l'esquisse). */
export const LIVE_BLEND_CODES: Record<string, number> = { multiply: 0, darken: 1, lighten: 2, overlay: 3, colorDodge: 4, colorBurn: 5, linearBurn: 6, hardMix: 7, subtract: 8, height: 9, linearHeight: 10, screen: 11 };

export type LiveSpec = {
  erase: boolean;
  /** Couleur du pinceau (0 à 255). */
  color: readonly [number, number, number];
  /** Opacité du pinceau (0 à 1) : celle du calque n'entre pas ici, elle s'applique à la composition. */
  opacity: number;
} & (
  | { kind: "round"; hardness: number; /** Pinceau flou : empreintes qui s'additionnent (un tiers chacune). */ accumulate: boolean }
  | {
      kind: "tip";
      tipId: number;
      tip: LiveTipImage;
      /** Flux : à 100 %, maximum ; en dessous, les empreintes s'additionnent (`merge`). */
      flow: number;
      dual: { tipId: number; tip: LiveTipImage; mode: string } | null;
      texture: { pattern: { width: number; height: number; gray: Uint8Array }; scale: number; depth: number; mode: string; invert: boolean; brightness: number; contrast: number } | null;
    }
);

type Program = { program: WebGLProgram; uniform: (name: string) => WebGLUniformLocation | null };
export type Linker = (fragment: string) => Program;

/** Textures de travail gardées après un trait, le temps qu'un autre suive ; libérées passé ce délai. */
const RELEASE_AFTER_MS = 3000;

// ------------------------------------------------------------------------------------------------- shaders
// Les rectangles passent par le shader de sommets commun (VERTEX_SHADER, origine en haut, retourné) : pour viser les
// lignes [y, y + h) d'une cible de hauteur H, on lui donne y' = H − y − h (voir `texRect`). Les fragments lisent
// gl_FragCoord tel quel : x = colonne, y = ligne de la texture (ligne 0 = haut de l'image, comme envoyé au moteur).

// Gélule ou empreinte ronde (même profil que `soft_falloff` et la gélule du moteur), repère de la texture.
const LIVE_DAB_SHADER = `#version 300 es
precision highp float;
uniform vec3 u_a;          // extrémités : centre (texels) et rayon
uniform vec3 u_b;
uniform float u_hardness;
uniform float u_deposit;   // 1 : pinceau flou, chaque empreinte dépose un tiers (SOFT_FLOW)
out vec4 o_color;
void main() {
  vec2 p = gl_FragCoord.xy;
  vec2 ab = u_b.xy - u_a.xy;
  float len2 = dot(ab, ab);
  float t = len2 > 1e-6 ? clamp(dot(p - u_a.xy, ab) / len2, 0.0, 1.0) : 0.0;
  float d = distance(p, u_a.xy + ab * t);
  float radius = mix(u_a.z, u_b.z, t);
  float hard = radius * u_hardness;
  float value = clamp(radius - d + 0.5, 0.0, 1.0);
  if (d > hard && radius - hard >= 1.0) {
    float s = clamp((d - hard) / (radius - hard), 0.0, 1.0) / 0.55;
    float tail = exp(-1.0 / (0.55 * 0.55));
    value = min(value, max(0.0, (exp(-(s * s)) - tail) / (1.0 - tail)));
  }
  if (u_deposit > 0.5) {
    if (d >= radius + 0.5) value = 0.0;
    value *= 0.33;
  }
  o_color = vec4(value);
}`;

// Empreinte d'une pointe : lecture bilinéaire EXACTE de la version réduite choisie par le moteur (`bilinear` de
// brushes.rs : texelFetch, demi-pixel, 0 hors de la pointe), force arrondie à l'octet comme dans `stamp_tip`, puis
// × flux (le mélange de la cible fait `merge` : maximum, ou c + v (1 − c)).
const LIVE_TIP_SHADER = `#version 300 es
precision highp float;
uniform sampler2D u_tip;    // une version de la pointe, R8, lue au texel près
uniform vec2 u_tipSize;     // sa taille
uniform vec2 u_center;      // centre de l'empreinte (texels)
uniform vec2 u_size;        // largeur, hauteur de l'empreinte (texels)
uniform vec2 u_rot;         // cosinus, sinus
uniform vec2 u_flip;
uniform float u_strength;
uniform float u_flow;       // < 1 : la valeur est multipliée par le flux (mélange additif)
out vec4 o_color;
float at(int x, int y) { return texelFetch(u_tip, ivec2(x, y), 0).r * 255.0; }
void main() {
  vec2 d = floor(gl_FragCoord.xy) + 0.5 - u_center;
  float u = (d.x * u_rot.x + d.y * u_rot.y) / u_size.x + 0.5;
  float v = (d.y * u_rot.x - d.x * u_rot.y) / u_size.y + 0.5;
  if (u_flip.x > 0.5) u = 1.0 - u;
  if (u_flip.y > 0.5) v = 1.0 - v;
  if (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) { o_color = vec4(0.0); return; }
  int w = int(u_tipSize.x), h = int(u_tipSize.y);
  float fx = max(u * u_tipSize.x - 0.5, 0.0), fy = max(v * u_tipSize.y - 0.5, 0.0);
  int x0 = min(int(fx), w - 1), y0 = min(int(fy), h - 1);
  int x1 = min(x0 + 1, w - 1), y1 = min(y0 + 1, h - 1);
  float tx = fx - float(x0), ty = fy - float(y0);
  float level = ((at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx) * (1.0 - ty) + (at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx) * ty) / 255.0;
  float value = floor(level * u_strength * 255.0 + 0.5) / 255.0;
  o_color = vec4(u_flow < 1.0 ? value * u_flow : value);
}`;

// Pose du trait dans la texture du calque : `Stroke::effective` (pinceau double, texture) puis `Stroke::resolve`
// (force = couverture × opacité, encre prémultipliée, gomme), en entiers comme le moteur (`mul_div_255`).
const LIVE_COMPOSE_SHADER = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D u_snapshot;   // la texture du calque au début du trait
uniform sampler2D u_coverage;
uniform sampler2D u_dual;
uniform sampler2D u_pattern;
uniform vec3 u_color;           // 0 à 255
uniform float u_opacity;        // 0 à 255 (déjà arrondie)
uniform int u_erase;
uniform int u_dualMode;         // -1 : pas de pinceau double
uniform int u_texMode;          // -1 : pas de texture
uniform vec4 u_tex;             // échelle, profondeur, luminosité (÷ 255), contraste (÷ 100)
uniform float u_texInvert;
uniform vec2 u_patternSize;
uniform vec2 u_origin;          // coin de la texture dans le document
uniform vec2 u_doc;             // taille du document
out vec4 o_color;
uint mul255(uint a, uint b) { uint t = a * b + 128u; return ((t >> 8u) + t) >> 8u; }
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
  ivec2 p = ivec2(gl_FragCoord.xy);
  vec4 s = texelFetch(u_snapshot, p, 0);
  uvec4 under = uvec4(floor(s * 255.0 + 0.5));
  vec2 doc = vec2(p) + u_origin;
  uint covered = uint(floor(clamp(texelFetch(u_coverage, p, 0).r, 0.0, 1.0) * 255.0 + 0.5));
  if (covered > 0u && (u_dualMode >= 0 || u_texMode >= 0)) {
    float c = float(covered) / 255.0;
    if (u_dualMode >= 0) c = tipBlend(u_dualMode, c, floor(texelFetch(u_dual, p, 0).r * 255.0 + 0.5) / 255.0);
    if (u_texMode >= 0) {
      // Le motif est fixé à la toile : repère du document.
      vec2 cell = floor(doc / max(u_tex.x, 0.01));
      float t = texelFetch(u_pattern, ivec2(mod(cell, u_patternSize)), 0).r;
      if (u_texInvert > 0.5) t = 1.0 - t;
      t = clamp((t + u_tex.z - 0.5) * (1.0 + u_tex.w) + 0.5, 0.0, 1.0);
      c += (tipBlend(u_texMode, c, t) - c) * clamp(u_tex.y, 0.0, 1.0);
    }
    covered = uint(clamp(c, 0.0, 1.0) * 255.0 + 0.5);
  }
  bool outside = doc.x < 0.0 || doc.y < 0.0 || doc.x >= u_doc.x || doc.y >= u_doc.y;
  uint t = outside ? 0u : mul255(covered, uint(u_opacity));
  if (t == 0u) { o_color = s; return; }
  uint keep = 255u - t;
  uvec4 o;
  if (u_erase == 1) {
    o = uvec4(mul255(under.r, keep), mul255(under.g, keep), mul255(under.b, keep), mul255(under.a, keep));
  } else {
    uvec3 ink = uvec3(u_color);
    o = uvec4(min(mul255(ink.r, t) + mul255(under.r, keep), 255u), min(mul255(ink.g, t) + mul255(under.g, keep), 255u), min(mul255(ink.b, t) + mul255(under.b, keep), 255u), min(mul255(255u, t) + mul255(under.a, keep), 255u));
  }
  o_color = vec4(o) / 255.0;
}`;

// Version réduite d'une zone : moyenne de 2 × 2 texels du niveau au-dessus (lu seul : niveaux de base et maximal
// réglés sur lui, ce qui permet d'écrire le niveau suivant de la même texture).
const LIVE_MIP_SHADER = `#version 300 es
precision highp float;
uniform sampler2D u_src;
out vec4 o_color;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy) * 2;
  o_color = (texelFetch(u_src, p, 0) + texelFetch(u_src, p + ivec2(1, 0), 0) + texelFetch(u_src, p + ivec2(0, 1), 0) + texelFetch(u_src, p + ivec2(1, 1), 0)) * 0.25;
}`;

// ------------------------------------------------------------------------------------------- versions de pointes

/**
 * Versions réduites d'une pointe, calculées EXACTEMENT comme le moteur (`BrushTip::with_levels` : moitiés arrondies
 * au-dessus, moyenne de 2 × 2 arrondie, bords répétés), niveau 0 compris.
 */
export function tipLevels(tip: LiveTipImage): LiveTipImage[] {
  const levels: LiveTipImage[] = [tip];
  let [w, h, mask] = [tip.width, tip.height, tip.mask];
  while (Math.max(w, h) > 2) {
    const [nw, nh] = [Math.ceil(w / 2), Math.ceil(h / 2)];
    const next = new Uint8Array(nw * nh);
    const at = (x: number, y: number) => mask[Math.min(y, h - 1) * w + Math.min(x, w - 1)]!;
    for (let y = 0; y < nh; y++) for (let x = 0; x < nw; x++) next[y * nw + x] = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1) + 2) >> 2;
    levels.push({ width: nw, height: nh, mask: next });
    [w, h, mask] = [nw, nh, next];
  }
  return levels;
}

/** Version de la pointe lue pour une empreinte de `side` pixels (plus grand côté) : `BrushTip::level_for`. */
export function tipLevelFor(levels: readonly LiveTipImage[], side: number): number {
  let level = 0;
  while (level + 1 < levels.length && Math.max(levels[level + 1]!.width, levels[level + 1]!.height) >= side) level += 1;
  return level;
}

/** Rectangle `[x, y, largeur, hauteur]` de texels, pour le shader de sommets commun (qui retourne l'axe vertical). */
function texRect(x: number, y: number, w: number, h: number, height: number): [number, number, number, number] {
  return [x, height - y - h, w, h];
}

interface Rect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** Une pointe sur la carte graphique : toutes ses versions, au texel près. */
interface TipTextures {
  levels: LiveTipImage[];
  textures: WebGLTexture[];
}

// ------------------------------------------------------------------------------------------------ peinture

export class LivePaint {
  private readonly dab: Program;
  private readonly tipProgram: Program;
  private readonly compose: Program;
  private readonly mip: Program;
  private readonly framebuffer: WebGLFramebuffer;
  private readonly readBuffer: WebGLFramebuffer;
  /** Pointes déjà envoyées à la carte (par identifiant) : un trait suivant de la même pointe ne les renvoie pas. */
  private readonly tips = new Map<number, TipTextures>();

  private target: LiveTarget | null = null;
  private spec: LiveSpec | null = null;
  private snapshot: WebGLTexture | null = null;
  private coverage: WebGLTexture | null = null;
  private dualCoverage: WebGLTexture | null = null;
  private pattern: WebGLTexture | null = null;
  /** Textures de travail gardées entre deux traits : leur taille, le format de la couverture, et ce qui y est sale. */
  private work: { width: number; height: number; float: boolean; coverageDirty: Rect | null; dualDirty: Rect | null } | null = null;
  private releaseTimer: ReturnType<typeof setTimeout> | null = null;
  private dabs: Dab[] = [];
  private stamps: TipStamp[] = [];
  private duals: DualTipStamp[] = [];
  /** Zone touchée depuis la dernière recomposition (texels). */
  private pending: Rect | null = null;
  /** Zone touchée par tout le trait (texels). */
  private touched: Rect | null = null;

  constructor(
    private readonly gl: WebGL2RenderingContext,
    link: Linker,
    /** Couverture du pinceau rond flou en demi-flottants quand la carte sait y dessiner (pas de paliers). */
    private readonly floatCoverage: boolean,
  ) {
    this.dab = link(LIVE_DAB_SHADER);
    this.tipProgram = link(LIVE_TIP_SHADER);
    this.compose = link(LIVE_COMPOSE_SHADER);
    this.mip = link(LIVE_MIP_SHADER);
    this.framebuffer = gl.createFramebuffer()!;
    this.readBuffer = gl.createFramebuffer()!;
    gl.useProgram(this.compose.program);
    for (const [name, unit] of [["u_snapshot", 0], ["u_coverage", 1], ["u_dual", 2], ["u_pattern", 3]] as const) gl.uniform1i(this.compose.uniform(name), unit);
    gl.useProgram(this.tipProgram.program);
    gl.uniform1i(this.tipProgram.uniform("u_tip"), 0);
    gl.useProgram(this.mip.program);
    gl.uniform1i(this.mip.uniform("u_src"), 0);
  }

  /** Un trait est en cours. */
  get active(): boolean {
    return this.target !== null;
  }

  /** Zone de la texture touchée par le trait en cours, ou `null`. `[x, y, largeur, hauteur]` en texels. */
  get area(): [number, number, number, number] | null {
    const r = this.touched;
    return r ? [r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0] : null;
  }

  /** Début d'un trait sur `target`. La texture du calque est copiée telle qu'elle est (état d'avant le trait). */
  begin(target: LiveTarget, spec: LiveSpec): void {
    this.end();
    if (this.releaseTimer !== null) {
      clearTimeout(this.releaseTimer);
      this.releaseTimer = null;
    }
    const { gl } = this;
    this.target = target;
    this.spec = spec;
    const float = spec.kind === "round" && spec.accumulate && this.floatCoverage;
    const saved = this.save();
    // Textures de travail : celles du trait précédent si elles conviennent (même taille, même format de couverture).
    const work = this.work;
    if (!work || work.width !== target.width || work.height !== target.height || work.float !== float) {
      this.releaseWork();
      this.snapshot = this.allocate(target.width, target.height, gl.RGBA8);
      this.coverage = this.allocate(target.width, target.height, float ? gl.R16F : gl.R8);
      // Neuves : entièrement à zéro (`texStorage2D`), rien à effacer.
      this.work = { width: target.width, height: target.height, float, coverageDirty: null, dualDirty: null };
    }
    if (spec.kind === "tip" && spec.dual && !this.dualCoverage) this.dualCoverage = this.allocate(target.width, target.height, gl.R8);
    // Copie de la texture du calque (niveau 0) : l'état d'avant le trait, comme `Snapshot::Pixels`.
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, this.readBuffer);
    gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target.texture, 0);
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, this.framebuffer);
    gl.framebufferTexture2D(gl.DRAW_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.snapshot, 0);
    gl.disable(gl.SCISSOR_TEST);
    gl.blitFramebuffer(0, 0, target.width, target.height, 0, 0, target.width, target.height, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    // Couvertures vides : seule la zone salie par le trait précédent est effacée.
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.clearColor(0, 0, 0, 0);
    for (const [texture, dirty] of [[this.coverage, this.work!.coverageDirty], [this.dualCoverage, this.work!.dualDirty]] as const) {
      if (!texture || !dirty) continue;
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      gl.enable(gl.SCISSOR_TEST);
      // Les ciseaux comptent les lignes comme gl_FragCoord : la ligne 0 de la texture est la ligne 0 des ciseaux (seul
      // le shader de sommets retourne les rectangles qu'on lui donne, voir `texRect`).
      gl.scissor(dirty.x0, dirty.y0, dirty.x1 - dirty.x0, dirty.y1 - dirty.y0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.disable(gl.SCISSOR_TEST);
    }
    this.work!.coverageDirty = null;
    this.work!.dualDirty = null;
    if (spec.kind === "tip") {
      this.tipTextures(spec.tipId, spec.tip);
      if (spec.dual) this.tipTextures(spec.dual.tipId, spec.dual.tip);
      if (spec.texture) {
        const { pattern } = spec.texture;
        this.pattern = gl.createTexture();
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, this.pattern);
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
        gl.texStorage2D(gl.TEXTURE_2D, 1, gl.R8, pattern.width, pattern.height);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, pattern.width, pattern.height, gl.RED, gl.UNSIGNED_BYTE, pattern.gray);
      }
    }
    this.restore(saved);
  }

  /** Gélules du pinceau rond (repère document), comme `DabPath` les calcule. */
  addDabs(dabs: readonly Dab[]): void {
    const t = this.target;
    if (!t || this.spec?.kind !== "round") return;
    for (const d of dabs) {
      this.dabs.push(d);
      const reach = Math.max(d.ar, d.br) + 2;
      this.grow(Math.min(d.ax, d.bx) - reach - t.dx, Math.min(d.ay, d.by) - reach - t.dy, Math.max(d.ax, d.bx) + reach - t.dx, Math.max(d.ay, d.by) + reach - t.dy);
    }
  }

  /** Empreintes d'une pointe (repère document), celles que le moteur recevra. */
  addTipStamps(stamps: readonly TipStamp[], duals: readonly DualTipStamp[]): void {
    const t = this.target;
    const spec = this.spec;
    if (!t || spec?.kind !== "tip") return;
    for (const s of stamps) {
      this.stamps.push(s);
      const reach = s.diameter + 2;
      this.grow(s.x - reach - t.dx, s.y - reach - t.dy, s.x + reach - t.dx, s.y + reach - t.dy);
    }
    if (spec.dual) {
      for (const d of duals) {
        this.duals.push(d);
        const reach = d.diameter + 2;
        this.grow(d.x - reach - t.dx, d.y - reach - t.dy, d.x + reach - t.dx, d.y + reach - t.dy);
      }
    }
  }

  /**
   * Pose ce qui est arrivé depuis la dernière fois : couverture, recomposition de la zone dans la texture du calque,
   * versions réduites de la zone. Vrai si la texture a changé. L'état de WebGL est rendu tel qu'il était.
   */
  render(): boolean {
    const t = this.target;
    const spec = this.spec;
    const zone = this.pending;
    if (!t || !spec || !zone) return false;
    this.pending = null;
    const { gl } = this;
    const saved = this.save();
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.disable(gl.SCISSOR_TEST);
    gl.enable(gl.BLEND);
    gl.viewport(0, 0, t.width, t.height);

    // 1. Couverture : les nouvelles empreintes, au maximum ou par-dessus (`merge`, `round_dab`).
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.coverage, 0);
    if (spec.kind === "round") {
      if (spec.accumulate) {
        gl.blendEquation(gl.FUNC_ADD);
        gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_COLOR);
      } else {
        gl.blendEquation(gl.MAX);
        gl.blendFunc(gl.ONE, gl.ONE);
      }
      const u = this.dab.uniform;
      gl.useProgram(this.dab.program);
      gl.uniform2f(u("u_view"), t.width, t.height);
      gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
      gl.uniform1f(u("u_hardness"), spec.hardness);
      gl.uniform1f(u("u_deposit"), spec.accumulate ? 1 : 0);
      for (const d of this.dabs) {
        const [ax, ay, bx, by] = [d.ax - t.dx, d.ay - t.dy, d.bx - t.dx, d.by - t.dy];
        const reach = Math.max(d.ar, d.br) + 2;
        const r = this.clip(Math.min(ax, bx) - reach, Math.min(ay, by) - reach, Math.max(ax, bx) + reach, Math.max(ay, by) + reach);
        if (!r) continue;
        gl.uniform4f(u("u_rect"), ...texRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0, t.height));
        gl.uniform3f(u("u_a"), ax, ay, d.ar);
        gl.uniform3f(u("u_b"), bx, by, d.br);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      }
      this.dabs = [];
    } else {
      this.stampTips(this.coverage!, spec.tipId, this.stamps, spec.flow);
      this.stamps = [];
      if (spec.dual && this.dualCoverage) {
        // Pinceau double : ses empreintes s'accumulent (flux 99,9 % dans le moteur).
        this.stampTips(this.dualCoverage, spec.dual.tipId, this.duals.map((d) => ({ ...d, turn: 0, roundness: 1, strength: 1 })), 0.999);
        this.duals = [];
      }
    }
    gl.blendEquation(gl.FUNC_ADD);
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.disable(gl.BLEND);

    // 2. Recomposition de la zone dans la texture du calque (niveau 0).
    const r = this.clip(zone.x0, zone.y0, zone.x1, zone.y1);
    if (r) {
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t.texture, 0);
      const p = this.compose.uniform;
      gl.useProgram(this.compose.program);
      gl.uniform2f(p("u_view"), t.width, t.height);
      gl.uniform4f(p("u_uv"), 0, 0, 1, 1);
      gl.uniform4f(p("u_rect"), ...texRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0, t.height));
      gl.uniform3f(p("u_color"), spec.color[0], spec.color[1], spec.color[2]);
      gl.uniform1f(p("u_opacity"), Math.round(Math.min(1, Math.max(0, spec.opacity)) * 255));
      gl.uniform1i(p("u_erase"), spec.erase ? 1 : 0);
      gl.uniform2f(p("u_origin"), t.dx, t.dy);
      gl.uniform2f(p("u_doc"), t.docWidth, t.docHeight);
      const dual = spec.kind === "tip" ? spec.dual : null;
      const texture = spec.kind === "tip" ? spec.texture : null;
      gl.uniform1i(p("u_dualMode"), dual ? (LIVE_BLEND_CODES[dual.mode] ?? 0) : -1);
      gl.uniform1i(p("u_texMode"), texture ? (LIVE_BLEND_CODES[texture.mode] ?? 0) : -1);
      if (texture) {
        gl.uniform4f(p("u_tex"), texture.scale, texture.depth, texture.brightness / 255, texture.contrast / 100);
        gl.uniform1f(p("u_texInvert"), texture.invert ? 1 : 0);
        gl.uniform2f(p("u_patternSize"), texture.pattern.width, texture.pattern.height);
      }
      const units: [number, WebGLTexture | null][] = [[0, this.snapshot], [1, this.coverage], [2, this.dualCoverage ?? this.coverage], [3, this.pattern ?? this.coverage]];
      for (const [unit, tex] of units) {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.bindSampler(unit, null);
      }
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

      // 3. Versions réduites de la zone, de niveau en niveau.
      this.refreshLevels(r);
    }
    this.restore(saved);
    return r !== null;
  }

  /**
   * Fin du trait (la texture du calque garde le trait). Les textures de travail restent prêtes pour un trait qui
   * suivrait aussitôt, et sont libérées après `RELEASE_AFTER_MS` sans peindre.
   */
  end(): void {
    const { gl } = this;
    if (this.pattern) gl.deleteTexture(this.pattern);
    this.pattern = null;
    if (this.work && this.touched) {
      const grow = (a: Rect | null, r: Rect): Rect => (a ? { x0: Math.min(a.x0, r.x0), y0: Math.min(a.y0, r.y0), x1: Math.max(a.x1, r.x1), y1: Math.max(a.y1, r.y1) } : { ...r });
      this.work.coverageDirty = grow(this.work.coverageDirty, this.touched);
      if (this.dualCoverage) this.work.dualDirty = grow(this.work.dualDirty, this.touched);
    }
    [this.target, this.spec, this.pending, this.touched] = [null, null, null, null];
    [this.dabs, this.stamps, this.duals] = [[], [], []];
    if (this.work && this.releaseTimer === null) {
      this.releaseTimer = setTimeout(() => {
        this.releaseTimer = null;
        if (!this.target) this.releaseWork();
      }, RELEASE_AFTER_MS);
    }
  }

  /** Libère les textures de travail (rien n'est gardé en mémoire vidéo). */
  private releaseWork(): void {
    const { gl } = this;
    for (const texture of [this.snapshot, this.coverage, this.dualCoverage]) if (texture) gl.deleteTexture(texture);
    [this.snapshot, this.coverage, this.dualCoverage] = [null, null, null];
    this.work = null;
  }

  /** Abandon : la texture du calque reprend son état d'avant le trait. */
  cancel(): void {
    const t = this.target;
    const area = this.touched;
    if (t && area && this.snapshot) {
      const { gl } = this;
      const saved = this.save();
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, this.readBuffer);
      gl.framebufferTexture2D(gl.READ_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.snapshot, 0);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, this.framebuffer);
      gl.framebufferTexture2D(gl.DRAW_FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t.texture, 0);
      gl.disable(gl.SCISSOR_TEST);
      gl.blitFramebuffer(area.x0, area.y0, area.x1, area.y1, area.x0, area.y0, area.x1, area.y1, gl.COLOR_BUFFER_BIT, gl.NEAREST);
      this.refreshLevels(area);
      this.restore(saved);
    }
    this.end();
  }

  /** Toutes les ressources (programmes compris restent ceux du renderer). */
  dispose(): void {
    this.end();
    if (this.releaseTimer !== null) clearTimeout(this.releaseTimer);
    this.releaseTimer = null;
    this.releaseWork();
    for (const { textures } of this.tips.values()) for (const texture of textures) this.gl.deleteTexture(texture);
    this.tips.clear();
    this.gl.deleteFramebuffer(this.framebuffer);
    this.gl.deleteFramebuffer(this.readBuffer);
  }

  // ------------------------------------------------------------------------------------------------ détails

  private allocate(width: number, height: number, format: number): WebGLTexture {
    const { gl } = this;
    const texture = gl.createTexture()!;
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, 1, format, width, height);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    return texture;
  }

  private tipTextures(id: number, image: LiveTipImage): TipTextures {
    const known = this.tips.get(id);
    if (known) return known;
    const { gl } = this;
    const levels = tipLevels(image);
    const textures = levels.map((level) => {
      const texture = gl.createTexture()!;
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.R8, level.width, level.height);
      gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, level.width, level.height, gl.RED, gl.UNSIGNED_BYTE, level.mask);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      return texture;
    });
    const entry = { levels, textures };
    this.tips.set(id, entry);
    return entry;
  }

  /** Empreintes d'une pointe dans une couverture (le mélange est réglé par l'appelant pour le flux plein). */
  private stampTips(target: WebGLTexture, tipId: number, list: readonly TipStamp[], flow: number): void {
    const t = this.target!;
    const tip = this.tips.get(tipId);
    if (!tip || list.length === 0) return;
    const { gl } = this;
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, target, 0);
    if (flow >= 1) {
      gl.blendEquation(gl.MAX);
      gl.blendFunc(gl.ONE, gl.ONE);
    } else {
      // c + v (1 − c), comme `merge`.
      gl.blendEquation(gl.FUNC_ADD);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_COLOR);
    }
    const u = this.tipProgram.uniform;
    gl.useProgram(this.tipProgram.program);
    gl.uniform2f(u("u_view"), t.width, t.height);
    gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
    gl.uniform1f(u("u_flow"), flow);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    const base = tip.levels[0]!;
    for (const s of list) {
      // Taille de l'empreinte : `stamp_tip` (diamètre = plus grand côté de la pointe).
      const unit = s.diameter / Math.max(base.width, base.height);
      const [w, h] = [base.width * unit, base.height * unit * s.roundness];
      if (w < 0.5 || h < 0.5) continue;
      const level = tipLevelFor(tip.levels, Math.max(w, h));
      const image = tip.levels[level]!;
      const [cos, sin] = [Math.cos(s.turn), Math.sin(s.turn)];
      const [hw, hh] = [(w * Math.abs(cos) + h * Math.abs(sin)) / 2, (w * Math.abs(sin) + h * Math.abs(cos)) / 2];
      const [cx, cy] = [s.x - t.dx, s.y - t.dy];
      const r = this.clip(Math.floor(cx - hw) - 1, Math.floor(cy - hh) - 1, Math.ceil(cx + hw) + 2, Math.ceil(cy + hh) + 2);
      if (!r) continue;
      gl.bindTexture(gl.TEXTURE_2D, tip.textures[level]!);
      gl.uniform2f(u("u_tipSize"), image.width, image.height);
      gl.uniform4f(u("u_rect"), ...texRect(r.x0, r.y0, r.x1 - r.x0, r.y1 - r.y0, t.height));
      gl.uniform2f(u("u_center"), cx, cy);
      gl.uniform2f(u("u_size"), w, h);
      gl.uniform2f(u("u_rot"), cos, sin);
      gl.uniform2f(u("u_flip"), s.flipX ? 1 : 0, s.flipY ? 1 : 0);
      gl.uniform1f(u("u_strength"), Math.min(1, Math.max(0, s.strength)));
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
  }

  /**
   * Refait les versions réduites d'une zone `[x, y, largeur, hauteur]` (texels du niveau 0) d'une texture à niveaux,
   * sans toucher au reste (pixels envoyés par le moteur avec une annulation, voir `GpuLayers.applyPatches`).
   * L'état de WebGL est rendu tel qu'il était.
   */
  refreshTexture(texture: WebGLTexture, width: number, height: number, levels: number, [x, y, w, h]: readonly [number, number, number, number]): void {
    if (w <= 0 || h <= 0) return;
    const saved = this.save();
    this.refreshLevelsOf({ texture, width, height, levels }, { x0: x, y0: y, x1: x + w, y1: y + h });
    this.restore(saved);
  }

  /** Refait les versions réduites de la texture du calque sur la zone `r` (niveau 0, texels). */
  private refreshLevels(r: Rect): void {
    this.refreshLevelsOf(this.target!, r);
  }

  private refreshLevelsOf(t: { texture: WebGLTexture; width: number; height: number; levels: number }, r: Rect): void {
    const { gl } = this;
    gl.disable(gl.BLEND);
    gl.useProgram(this.mip.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindSampler(0, null);
    gl.bindTexture(gl.TEXTURE_2D, t.texture);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    let [x0, y0, x1, y1] = [r.x0, r.y0, r.x1, r.y1];
    for (let level = 1; level < t.levels; level++) {
      const [w, h] = [Math.max(1, t.width >> level), Math.max(1, t.height >> level)];
      [x0, y0, x1, y1] = [Math.max(0, Math.floor(x0 / 2)), Math.max(0, Math.floor(y0 / 2)), Math.min(w, Math.ceil(x1 / 2)), Math.min(h, Math.ceil(y1 / 2))];
      if (x1 <= x0 || y1 <= y0) break;
      // Le niveau lu (seul visible) et le niveau écrit sont distincts : pas de boucle de rétroaction.
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_BASE_LEVEL, level - 1);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, level - 1);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t.texture, level);
      gl.viewport(0, 0, w, h);
      gl.uniform2f(this.mip.uniform("u_view"), w, h);
      gl.uniform4f(this.mip.uniform("u_uv"), 0, 0, 1, 1);
      gl.uniform4f(this.mip.uniform("u_rect"), ...texRect(x0, y0, x1 - x0, y1 - y0, h));
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_BASE_LEVEL, 0);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 1000);
    gl.viewport(0, 0, t.width, t.height);
  }

  /** Agrandit la zone à recomposer (et celle du trait) de ce rectangle (texels, bornes quelconques). */
  private grow(x0: number, y0: number, x1: number, y1: number): void {
    const r = this.clip(x0, y0, x1, y1);
    if (!r) return;
    const join = (a: Rect | null): Rect => (a ? { x0: Math.min(a.x0, r.x0), y0: Math.min(a.y0, r.y0), x1: Math.max(a.x1, r.x1), y1: Math.max(a.y1, r.y1) } : { ...r });
    this.pending = join(this.pending);
    this.touched = join(this.touched);
  }

  /** Rectangle entier, limité à la texture ; `null` s'il est vide. */
  private clip(x0: number, y0: number, x1: number, y1: number): Rect | null {
    const t = this.target!;
    const r = { x0: Math.max(0, Math.floor(x0)), y0: Math.max(0, Math.floor(y0)), x1: Math.min(t.width, Math.ceil(x1)), y1: Math.min(t.height, Math.ceil(y1)) };
    return r.x1 > r.x0 && r.y1 > r.y0 ? r : null;
  }

  /** État de WebGL touché ici, pour le rendre tel quel (la peinture se fait au milieu du dessin d'une image). */
  private save() {
    const { gl } = this;
    const units = [0, 1, 2, 3].map((unit) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      return { texture: gl.getParameter(gl.TEXTURE_BINDING_2D) as WebGLTexture | null, sampler: gl.getParameter(gl.SAMPLER_BINDING) as WebGLSampler | null };
    });
    return {
      active: gl.getParameter(gl.ACTIVE_TEXTURE) as number,
      units,
      framebuffer: gl.getParameter(gl.FRAMEBUFFER_BINDING) as WebGLFramebuffer | null,
      read: gl.getParameter(gl.READ_FRAMEBUFFER_BINDING) as WebGLFramebuffer | null,
      viewport: gl.getParameter(gl.VIEWPORT) as Int32Array,
      scissor: gl.isEnabled(gl.SCISSOR_TEST),
      blend: gl.isEnabled(gl.BLEND),
      program: gl.getParameter(gl.CURRENT_PROGRAM) as WebGLProgram | null,
      equation: gl.getParameter(gl.BLEND_EQUATION_RGB) as number,
      src: gl.getParameter(gl.BLEND_SRC_RGB) as number,
      dst: gl.getParameter(gl.BLEND_DST_RGB) as number,
    };
  }

  private restore(s: ReturnType<LivePaint["save"]>): void {
    const { gl } = this;
    s.units.forEach(({ texture, sampler }, unit) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.bindSampler(unit, sampler);
    });
    gl.activeTexture(s.active);
    gl.bindFramebuffer(gl.FRAMEBUFFER, s.framebuffer);
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, s.read);
    gl.viewport(s.viewport[0]!, s.viewport[1]!, s.viewport[2]!, s.viewport[3]!);
    if (s.scissor) gl.enable(gl.SCISSOR_TEST);
    else gl.disable(gl.SCISSOR_TEST);
    if (s.blend) gl.enable(gl.BLEND);
    else gl.disable(gl.BLEND);
    gl.useProgram(s.program);
    gl.blendEquation(s.equation);
    gl.blendFunc(s.src, s.dst);
  }
}
