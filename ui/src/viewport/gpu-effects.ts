// Effets de calque calculés par la carte graphique : l'aperçu pendant qu'on règle un effet (taille d'une ombre,
// épaisseur d'un contour…). Le moteur ne refait le rendu exact qu'au relâchement.
//
// Traduction de crates/engine/src/effects.rs (`styled_parts`), passe pour passe, à partir des pixels seuls du calque
// déjà en texture : même tampon de calcul (emprise du calque + marge), mêmes cartes, mêmes formules, même ordre de
// pose. Le résultat a la forme de celui du moteur : le calque stylé (`main`), et à part les effets qui se fondent
// dans un autre mode que Normal, dessous (`under`) et dessus (`over`).
//  - Flou : celui du moteur est trois moyennes glissantes de suite (`filters::gaussian`). Leur composée est un noyau
//    exact, calculé ici, appliqué en deux passes séparables (lignes puis colonnes), avec le même bord (zéro hors du
//    tampon) : la carte est celle du moteur aux arrondis près. Un très grand flou sur un très grand calque se calcule
//    sur une carte réduite (moyenne de blocs f × f, flou d'écart-type σ/f, relecture bilinéaire) : quelques pixels
//    d'écart sur un halo de plusieurs dizaines de pixels, le temps du geste.
//  - Contour : distance euclidienne exacte au bord (seuil d'alpha 0,5, comme le moteur), par deux passes séparables
//    bornées à l'épaisseur du contour (au-delà, le contour est nul : la distance exacte n'importe plus).
// Les poses en mode Normal se font par la carte (ONE, ONE_MINUS_SRC_ALPHA), comme `paint_over`.
import type { BlendMode, LayerEffect } from "../lib/ipc";

type Linker = (fragment: string) => { program: WebGLProgram; uniform: (name: string) => WebGLUniformLocation | null };

/** Pixels seuls du calque (texture `r<id>`) : rectangle couvert, dans le repère du calque. */
export interface RawSource {
  texture: WebGLTexture;
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Calque stylé calculé par la carte graphique, dans le repère du calque. */
export interface StyledPreview {
  /** `[x, y, largeur, hauteur]` des textures (marge de recyclage comprise, transparente). */
  rect: [number, number, number, number];
  main: WebGLTexture;
  under: { mode: BlendMode; texture: WebGLTexture }[];
  over: { mode: BlendMode; texture: WebGLTexture }[];
  /** Mémoire vidéo prise (textures du résultat et tampons de calcul). */
  bytes: number;
  /** Plus forte réduction appliquée à un flou (1 : calcul à pleine résolution, identique au moteur). */
  reduction: number;
}

/** Comme le moteur : au-delà, le calque s'affiche sans effets (`MAX_AREA` d'effects.rs). */
const MAX_AREA = 80_000_000;
/** Rayon de noyau au-delà duquel le flou se calcule sur une carte réduite. */
const MAX_RADIUS = 64;
/**
 * Lectures de texture admises pour un flou (deux passes) avant de le calculer sur une carte réduite : de l'ordre de
 * quelques millisecondes sur une carte graphique intégrée. Un flou n'est jamais réduit au-delà de σ / 4 (mesuré, comparé au
 * moteur : écart max 3/255 sur une lueur, dont le gain × 1,8 amplifie l'écart ; jusqu'à 7/255 réduit à σ / 3).
 */
const BLUR_BUDGET = 400_000_000;
/** Les tampons sont alloués par pas de 256 pixels : tirer un curseur de taille ne réalloue presque jamais rien. */
const STEP = 256;

const f32 = Math.fround;
/** `f32::round` de Rust : moitiés arrondies en s'éloignant de zéro. */
const roundAway = (v: number): number => Math.sign(v) * Math.round(Math.abs(v));
const blendOf = (effect: LayerEffect): BlendMode => effect.blend ?? "normal";
const normal = (mode: BlendMode): boolean => mode === "normal" || mode === "pass_through";

/** Marge de calcul autour du contenu (`LayerEffect::reach`), en f32 comme le moteur. */
function reach(effect: LayerEffect): number {
  switch (effect.type) {
    case "dropShadow":
    case "innerShadow":
      return f32(f32(f32(Math.abs(f32(effect.distance))) + f32(f32(effect.size) * f32(1.6))) + 2);
    case "outerGlow":
    case "innerGlow":
      return f32(f32(f32(effect.size) * f32(1.6)) + 2);
    case "stroke":
      return effect.position !== "inside" ? f32(f32(effect.width) + 2) : 0;
    default:
      return 0;
  }
}

/** Marge du tampon de calcul autour du contenu du calque (comme `styled_into`). */
export function effectsMargin(effects: readonly LayerEffect[]): number {
  return Math.ceil(effects.filter((e) => e.enabled).reduce((m, e) => Math.max(m, reach(e)), 0)) + 1;
}

/** Décalage d'une ombre : à l'opposé de la lumière (l'axe vertical de l'écran pointe vers le bas). */
function offset(angle: number, distance: number): [number, number] {
  const radians = f32(f32(angle) * f32(Math.PI / 180));
  return [roundAway(f32(-f32(Math.cos(radians)) * f32(distance))), roundAway(f32(f32(Math.sin(radians)) * f32(distance)))];
}

/** Largeurs des trois moyennes glissantes équivalentes à un flou d'écart-type `sigma` (`filters::box_sizes`). */
export function boxSizes(sigma: number): [number, number, number] {
  const s = f32(sigma);
  const twelve = f32(f32(12 * s) * s);
  const ideal = f32(Math.sqrt(f32(f32(twelve / 3) + 1)));
  let lower = Math.floor(ideal);
  if (lower % 2 === 0) lower = Math.max(Math.max(lower - 1, 0), 1);
  const upper = lower + 2;
  const change = roundAway(f32(f32(twelve - (3 * lower * lower + 12 * lower + 9)) / f32(f32(-4 * lower) - 4)));
  return [0, 1, 2].map((i) => (i < change ? lower : upper)) as [number, number, number];
}

/** Noyau du flou : composée exacte des trois moyennes glissantes ; centré, de rayon `(longueur - 1) / 2`. */
export function blurKernel(sigma: number): Float32Array {
  let kernel = [1];
  for (const size of boxSizes(sigma)) {
    const next = new Array<number>(kernel.length + size - 1).fill(0);
    for (let i = 0; i < kernel.length; i++) for (let j = 0; j < size; j++) next[i + j]! += kernel[i]! / size;
    kernel = next;
  }
  return Float32Array.from(kernel);
}

// Commun à toutes les passes : les pixels seuls du calque placés dans le tampon de calcul, et la carte d'alpha
// décalée (`shifted`) et éventuellement inversée (vide = 1 - alpha) que floutent les ombres et les lueurs.
const PRELUDE = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D u_raw;
uniform ivec4 u_rawRect;   // coin des pixels du calque dans le tampon, et leur taille
uniform ivec2 u_size;      // taille du tampon de calcul (au-delà : hors tampon)
uniform ivec2 u_shift;     // décalage de la carte (ombres)
uniform float u_outside;   // valeur de ce qui entre par les bords en la décalant
uniform int u_invert;      // 1 : carte du vide (1 - alpha)
out vec4 o_color;

vec4 rawAt(ivec2 p) {
  ivec2 q = p - u_rawRect.xy;
  if (q.x < 0 || q.y < 0 || q.x >= u_rawRect.z || q.y >= u_rawRect.w) return vec4(0.0);
  return texelFetch(u_raw, q, 0);
}
bool inBuffer(ivec2 p) { return p.x >= 0 && p.y >= 0 && p.x < u_size.x && p.y < u_size.y; }
// Carte à flouter : zéro hors du tampon (bord des moyennes glissantes du moteur).
float sourceAt(ivec2 p) {
  if (!inBuffer(p)) return 0.0;
  ivec2 s = p - u_shift;
  if (!inBuffer(s)) return u_outside;
  float a = rawAt(s).a;
  return u_invert == 1 ? 1.0 - a : a;
}
`;

// Une passe du flou séparable : noyau (texture R32F) le long de `u_dir`, sur la carte calculée (`u_fromRaw`) ou sur
// la passe précédente (zéro au-delà de `u_extent`).
const BLUR_SHADER = `${PRELUDE}
uniform int u_fromRaw;
uniform highp sampler2D u_in;
uniform highp sampler2D u_kernel;
uniform int u_radius;
uniform ivec2 u_dir;
uniform ivec2 u_extent;
float inAt(ivec2 p) {
  if (u_fromRaw == 1) return sourceAt(p);
  if (p.x < 0 || p.y < 0 || p.x >= u_extent.x || p.y >= u_extent.y) return 0.0;
  return texelFetch(u_in, p, 0).r;
}
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  float sum = 0.0;
  for (int k = -u_radius; k <= u_radius; k++) sum += texelFetch(u_kernel, ivec2(k + u_radius, 0), 0).r * inAt(p + u_dir * k);
  o_color = vec4(sum, 0.0, 0.0, 1.0);
}`;

// Carte réduite : moyenne de blocs f × f (très grands flous sur de très grands calques).
const REDUCE_SHADER = `${PRELUDE}
uniform int u_factor;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy) * u_factor;
  float sum = 0.0;
  for (int y = 0; y < u_factor; y++) for (int x = 0; x < u_factor; x++) sum += sourceAt(p + ivec2(x, y));
  o_color = vec4(sum / float(u_factor * u_factor), 0.0, 0.0, 1.0);
}`;

// Distance au bord, première passe (colonnes) : distance verticale au plus proche pixel plein (alpha >= 0,5) et au
// plus proche pixel vide, bornée à `u_cap` (au-delà : u_cap + 1). On cherche du plus près au plus loin.
const EDT_COLUMNS_SHADER = `${PRELUDE}
uniform int u_cap;
uniform int u_needOut;   // un contour déborde : distance des pixels vides au plein
uniform int u_needIn;    // un contour rentre : distance des pixels pleins au vide
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  float far = float(u_cap + 1);
  float toSolid = far;
  float toEmpty = far;
  bool findSolid = u_needOut == 1;
  bool findEmpty = u_needIn == 1;
  for (int k = 0; k <= u_cap && (findSolid || findEmpty); k++) {
    for (int side = -1; side <= 1; side += 2) {
      ivec2 q = p + ivec2(0, k * side);
      if (!inBuffer(q)) continue;
      bool solid = rawAt(q).a >= 0.5;
      if (findSolid && solid) { toSolid = float(k); findSolid = false; }
      if (findEmpty && !solid) { toEmpty = float(k); findEmpty = false; }
    }
  }
  o_color = vec4(toSolid, toEmpty, 0.0, 1.0);
}`;

// Seconde passe (lignes) : distance euclidienne exacte (min de dx² + dy²), puis distance signée au bord comme le
// moteur : dedans 0,5 - distance au vide, dehors distance au plein - 0,5.
const EDT_ROWS_SHADER = `${PRELUDE}
uniform highp sampler2D u_in;
uniform int u_cap;
uniform int u_needOut;
uniform int u_needIn;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  if (!inBuffer(p)) { o_color = vec4(1e6, 0.0, 0.0, 1.0); return; }
  bool solid = rawAt(p).a >= 0.5;
  if (solid && u_needIn == 0) { o_color = vec4(-1e6, 0.0, 0.0, 1.0); return; }
  if (!solid && u_needOut == 0) { o_color = vec4(1e6, 0.0, 0.0, 1.0); return; }
  float cap = float(u_cap);
  float best = 1e12;
  for (int k = 0; k <= u_cap; k++) {
    float k2 = float(k * k);
    if (k2 >= best) break;
    for (int side = -1; side <= 1; side += 2) {
      ivec2 q = p + ivec2(k * side, 0);
      if (!inBuffer(q)) continue;
      vec2 v = texelFetch(u_in, q, 0).rg;
      float dy = solid ? v.g : v.r;
      if (dy <= cap) best = min(best, k2 + dy * dy);
    }
  }
  float distance = best >= 1e12 ? 1e6 : sqrt(best);
  o_color = vec4(solid ? 0.5 - distance : distance - 0.5, 0.0, 0.0, 1.0);
}`;

// Pose d'une couche (comme `paint_over`) : le calque lui-même, ou une couleur à travers sa couverture.
const PAINT_SHADER = `${PRELUDE}
uniform int u_kind;        // 0 calque, 1 carte (ombres, lueurs), 2 contour, 3 incrustation de couleur, 4 de dégradé,
                           // 5 recopie du calque stylé (demi-flottants) en RGBA8, bornée comme to_store
uniform int u_mapMode;     // 0 carte pleine résolution, 1 carte réduite (bilinéaire), 2 carte calculée ici (sans flou)
uniform highp sampler2D u_map;
uniform float u_factor;
uniform vec2 u_mapSize;
uniform float u_gain;      // lueurs : min(carte × 1,8 ; 1) ; 0 : aucun
uniform int u_inner;       // effets internes : limités à l'alpha du calque
uniform vec3 u_color;
uniform float u_opacity;
uniform vec2 u_band;       // contour : épaisseur dehors, dedans
uniform vec3 u_from;
uniform vec3 u_to;
uniform vec4 u_content;    // dégradé : centre et taille du contenu, dans le tampon
uniform vec2 u_axis;
uniform float u_half;
uniform int u_radial;
void main() {
  ivec2 p = ivec2(gl_FragCoord.xy);
  if (!inBuffer(p)) { o_color = vec4(0.0); return; }
  if (u_kind == 0) { o_color = rawAt(p); return; }
  if (u_kind == 5) {
    vec4 v = texelFetch(u_map, p, 0);
    float a = clamp(v.a, 0.0, 1.0);
    o_color = vec4(clamp(v.rgb, vec3(0.0), vec3(a)), a);
    return;
  }
  float alpha = rawAt(p).a;
  vec3 color = u_color;
  float c = alpha;
  if (u_kind == 1) {
    float m = u_mapMode == 0 ? texelFetch(u_map, p, 0).r
      : u_mapMode == 1 ? texture(u_map, (vec2(p) + 0.5) / u_factor / u_mapSize).r
      : sourceAt(p);
    if (u_gain > 0.0) m = min(m * u_gain, 1.0);
    c = u_inner == 1 ? m * alpha : m;
  } else if (u_kind == 2) {
    float d = texelFetch(u_map, p, 0).r;
    c = min(clamp(u_band.x - d + 0.5, 0.0, 1.0), clamp(d + u_band.y + 0.5, 0.0, 1.0));
  } else if (u_kind == 4) {
    vec2 at = vec2(p) + 0.5 - u_content.xy;
    float t = u_radial == 1 ? length(at / (u_content.zw / 2.0)) : dot(at, u_axis) / max(u_half, 1.0) * 0.5 + 0.5;
    color = mix(u_from, u_to, clamp(t, 0.0, 1.0));
  }
  float a = clamp(c * u_opacity, 0.0, 1.0);
  o_color = vec4(color * a, a);
}`;

interface Scratch {
  texture: WebGLTexture;
  w: number;
  h: number;
  bytes: number;
}

/** Couverture d'un effet à poser : où lire sa carte. */
type Coverage = { mode: 0 | 1 | 2; map: WebGLTexture | null; factor: number; mapSize: [number, number] };

const rgb = (color: readonly number[]): [number, number, number] => [color[0]! / 255, color[1]! / 255, color[2]! / 255];

export class GpuEffects {
  private readonly blurProgram: ReturnType<Linker>;
  private readonly reduceProgram: ReturnType<Linker>;
  private readonly columnsProgram: ReturnType<Linker>;
  private readonly rowsProgram: ReturnType<Linker>;
  private readonly paintProgram: ReturnType<Linker>;
  private readonly kernel: WebGLTexture;
  private readonly framebuffer: WebGLFramebuffer;
  /** Tampons de calcul, recyclés d'un réglage à l'autre (par format). */
  private readonly scratch = new Map<string, Scratch>();
  /** Carte graphique capable de dessiner dans des textures flottantes : sans elles, pas d'aperçu ici. */
  readonly supported: boolean;

  constructor(
    private readonly gl: WebGL2RenderingContext,
    link: Linker,
  ) {
    this.supported = gl.getExtension("EXT_color_buffer_float") !== null;
    this.blurProgram = link(BLUR_SHADER);
    this.reduceProgram = link(REDUCE_SHADER);
    this.columnsProgram = link(EDT_COLUMNS_SHADER);
    this.rowsProgram = link(EDT_ROWS_SHADER);
    this.paintProgram = link(PAINT_SHADER);
    // Unités : 0 pixels du calque, 1 carte lue, 2 noyau.
    for (const program of [this.blurProgram, this.reduceProgram, this.columnsProgram, this.rowsProgram, this.paintProgram]) {
      gl.useProgram(program.program);
      gl.uniform1i(program.uniform("u_raw"), 0);
      gl.uniform1i(program.uniform("u_in"), 1);
      gl.uniform1i(program.uniform("u_map"), 1);
      gl.uniform1i(program.uniform("u_kernel"), 2);
    }
    this.kernel = gl.createTexture();
    this.framebuffer = gl.createFramebuffer();
  }

  /**
   * Mémoire vidéo que demanderait ce calcul (textures du résultat et tampons), ou `null` si le calque est trop grand
   * pour des effets (le moteur l'affiche alors sans eux).
   */
  estimate(raw: { w: number; h: number }, effects: readonly LayerEffect[]): number | null {
    const margin = effectsMargin(effects);
    const [w, h] = [raw.w + 2 * margin, raw.h + 2 * margin];
    if (w * h > MAX_AREA) return null;
    const [tw, th] = [padded(w), padded(h)];
    // Le calque stylé et chaque effet fondu à part (RGBA8 et leurs niveaux de détail), plus trois tampons de calcul.
    const parts = 1 + effects.filter((e) => e.enabled && !normal(blendOf(e))).length;
    return Math.round(tw * th * (parts * 4 * (4 / 3) + 4 + 4 + 8));
  }

  /**
   * Calque stylé : comme `styled_parts` du moteur, à partir des pixels seuls du calque. `previous` : résultat
   * précédent dont les textures sont reprises si elles ont la bonne taille. `null` : calque vide ou démesuré.
   */
  render(raw: RawSource, effects: readonly LayerEffect[], previous: StyledPreview | null): StyledPreview | null {
    const { gl } = this;
    const active = effects.filter((e) => e.enabled);
    const margin = effectsMargin(effects);
    const [left, top] = [raw.x - margin, raw.y - margin];
    const [w, h] = [raw.w + 2 * margin, raw.h + 2 * margin];
    if (raw.w <= 0 || raw.h <= 0 || w * h > MAX_AREA) return null;
    const [tw, th] = [padded(w), padded(h)];

    // État partagé : aucun échantillonneur ne doit remplacer le filtrage des textures, pas de ciseaux, pas de fusion.
    for (let unit = 0; unit < 3; unit++) gl.bindSampler(unit, null);
    gl.disable(gl.SCISSOR_TEST);
    gl.disable(gl.BLEND);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);

    // Textures du résultat : reprises si possible.
    const pool = previous && previous.rect[2] === tw && previous.rect[3] === th ? [previous.main, ...previous.under.map((p) => p.texture), ...previous.over.map((p) => p.texture)] : [];
    if (previous && pool.length === 0) this.release(previous);
    const take = (): WebGLTexture => pool.shift() ?? this.colorTexture(tw, th);
    const main = take();
    // Le calque stylé s'accumule en demi-flottants (le moteur calcule en f32 et n'arrondit qu'à la fin) : en RGBA8,
    // chaque pose arrondissait, et l'ombre sous un calque semi-transparent fondu en Lumière crue s'écartait de 3/255.
    const sum = this.scratchTexture("sum", gl.RGBA16F, tw, th);
    const under: StyledPreview["under"] = [];
    const over: StyledPreview["over"] = [];
    this.clearTexture(sum.texture, tw, th);

    let reduction = 1;
    const setCommon = (program: ReturnType<Linker>, shift: [number, number] = [0, 0], outside = 0, invert = false) => {
      gl.useProgram(program.program);
      // Les allocations passent par l'unité 0 : les pixels du calque y sont remis avant chaque passe.
      bind(0, raw.texture);
      const u = program.uniform;
      gl.uniform2f(u("u_view"), tw, th);
      gl.uniform4f(u("u_rect"), 0, 0, tw, th);
      gl.uniform4f(u("u_uv"), 0, 0, 1, 1);
      gl.uniform4i(u("u_rawRect"), margin, margin, raw.w, raw.h);
      gl.uniform2i(u("u_size"), w, h);
      gl.uniform2i(u("u_shift"), shift[0], shift[1]);
      gl.uniform1f(u("u_outside"), outside);
      gl.uniform1i(u("u_invert"), invert ? 1 : 0);
    };
    const bind = (unit: number, texture: WebGLTexture | null) => {
      gl.activeTexture(gl.TEXTURE0 + unit);
      gl.bindTexture(gl.TEXTURE_2D, texture);
    };
    const target = (texture: WebGLTexture, width: number, height: number) => {
      gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      gl.viewport(0, 0, width, height);
    };
    bind(0, raw.texture);

    /** Carte floutée d'une ombre ou d'une lueur (`blur(shifted(...), size / 2)`). */
    const blurred = (size: number, shift: [number, number], outside: number, invert: boolean): Coverage => {
      const sigma = f32(f32(size) / 2);
      if (sigma < 0.3) return { mode: 2, map: null, factor: 1, mapSize: [w, h] };
      const full = blurKernel(sigma);
      const radius = (full.length - 1) / 2;
      // Réduction : la plus petite qui tient le rayon et le budget, sans jamais dépasser σ / 4.
      let factor = 1;
      while (factor * 2 <= sigma / 4 && (radius / factor > MAX_RADIUS || 2 * (2 * radius / factor + 1) * ((w * h) / (factor * factor)) > BLUR_BUDGET)) factor *= 2;
      reduction = Math.max(reduction, factor);
      const [mw, mh] = [Math.ceil(w / factor), Math.ceil(h / factor)];
      // Carte réduite : flou d'écart-type σ/f, diminué de ce qu'ajoutent la moyenne de blocs et la relecture bilinéaire.
      const kernel = factor === 1 ? full : blurKernel(Math.max(0.3, Math.sqrt(Math.max(0, (sigma / factor) ** 2 - 0.25))));
      const kRadius = (kernel.length - 1) / 2;
      bind(2, this.kernel);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R32F, kernel.length, 1, 0, gl.RED, gl.FLOAT, kernel);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      const first = this.scratchTexture(factor === 1 ? "rows" : "small-a", factor === 1 ? gl.RG16F : gl.R16F, factor === 1 ? tw : padded(mw), factor === 1 ? th : padded(mh));
      const second = this.scratchTexture(factor === 1 ? "map" : "small-b", factor === 1 ? gl.R32F : gl.R16F, factor === 1 ? tw : padded(mw), factor === 1 ? th : padded(mh));
      let input: Scratch | null = null;
      if (factor > 1) {
        // Carte réduite dans `second`, floutée en lignes dans `first`, puis en colonnes de nouveau dans `second`.
        setCommon(this.reduceProgram, shift, outside, invert);
        gl.uniform1i(this.reduceProgram.uniform("u_factor"), factor);
        target(second.texture, second.w, second.h);
        bind(1, null);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        input = second;
      }
      const pass = (from: Scratch | null, to: Scratch, dir: [number, number]) => {
        setCommon(this.blurProgram, shift, outside, invert);
        const u = this.blurProgram.uniform;
        gl.uniform2f(u("u_view"), to.w, to.h);
        gl.uniform4f(u("u_rect"), 0, 0, to.w, to.h);
        gl.uniform1i(u("u_fromRaw"), from ? 0 : 1);
        gl.uniform1i(u("u_radius"), kRadius);
        gl.uniform2i(u("u_dir"), dir[0], dir[1]);
        gl.uniform2i(u("u_extent"), mw, mh);
        bind(1, from?.texture ?? null);
        target(to.texture, to.w, to.h);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        bind(1, null);
      };
      pass(input, first, [1, 0]);
      pass(first, second, [0, 1]);
      return { mode: factor === 1 ? 0 : 1, map: second.texture, factor, mapSize: [second.w, second.h] };
    };

    /** Pose une couche dans `into` (fondue par-dessus ce qu'il contient). */
    const paint = (into: WebGLTexture, kind: number, setup: (u: ReturnType<Linker>["uniform"]) => void, cover?: Coverage, shift: [number, number] = [0, 0], outside = 0, invert = false) => {
      setCommon(this.paintProgram, shift, outside, invert);
      const u = this.paintProgram.uniform;
      gl.uniform1i(u("u_kind"), kind);
      gl.uniform1i(u("u_mapMode"), cover?.mode ?? 0);
      gl.uniform1f(u("u_factor"), cover?.factor ?? 1);
      gl.uniform2f(u("u_mapSize"), cover?.mapSize[0] ?? 1, cover?.mapSize[1] ?? 1);
      gl.uniform1f(u("u_gain"), 0);
      gl.uniform1i(u("u_inner"), 0);
      setup(u);
      bind(1, cover?.map ?? null);
      if (cover?.mode === 1 && cover.map) {
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      }
      target(into, tw, th);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
      gl.disable(gl.BLEND);
      bind(1, null);
    };
    const tint = (color: readonly number[], opacity: number, gain = 0, inner = false) => (u: ReturnType<Linker>["uniform"]) => {
      gl.uniform3f(u("u_color"), ...rgb(color));
      gl.uniform1f(u("u_opacity"), opacity);
      gl.uniform1f(u("u_gain"), gain);
      gl.uniform1i(u("u_inner"), inner ? 1 : 0);
    };
    /** Couche du dessus : dans le calque en mode Normal, seule dans sa texture dans un autre mode. */
    const place = (mode: BlendMode, draw: (into: WebGLTexture) => void) => {
      if (normal(mode)) return draw(sum.texture);
      const alone = take();
      this.clearTexture(alone, tw, th);
      draw(alone);
      over.push({ mode, texture: alone });
    };

    // Dessous : ombres portées et lueurs externes, dans l'ordre de la liste.
    for (const effect of active) {
      if (effect.type !== "dropShadow" && effect.type !== "outerGlow") continue;
      const shift = effect.type === "dropShadow" ? offset(effect.angle, effect.distance) : ([0, 0] as [number, number]);
      const cover = blurred(effect.size, shift, 0, false);
      const setup = tint(effect.color, effect.opacity, effect.type === "outerGlow" ? 1.8 : 0);
      const mode = blendOf(effect);
      if (normal(mode)) {
        paint(sum.texture, 1, setup, cover, shift);
      } else {
        const alone = take();
        this.clearTexture(alone, tw, th);
        paint(alone, 1, setup, cover, shift);
        under.push({ mode, texture: alone });
      }
    }
    // Le calque lui-même.
    paint(sum.texture, 0, () => undefined);
    // Dessus, dans l'ordre de Photoshop : incrustations, lueur interne, ombre interne, puis les contours.
    for (const effect of active) {
      if (effect.type === "colorOverlay") place(blendOf(effect), (into) => paint(into, 3, tint(effect.color, effect.opacity)));
      if (effect.type === "gradientOverlay") {
        const [cw, ch] = [raw.w, raw.h];
        const radians = f32(f32(effect.angle) * f32(Math.PI / 180));
        const [dx, dy] = [Math.cos(radians), -Math.sin(radians)];
        place(blendOf(effect), (into) =>
          paint(into, 4, (u) => {
            gl.uniform1f(u("u_opacity"), effect.opacity);
            gl.uniform3f(u("u_from"), ...rgb(effect.from));
            gl.uniform3f(u("u_to"), ...rgb(effect.to));
            gl.uniform4f(u("u_content"), margin + cw / 2, margin + ch / 2, cw, ch);
            gl.uniform2f(u("u_axis"), dx, dy);
            gl.uniform1f(u("u_half"), (Math.abs(dx) * cw + Math.abs(dy) * ch) / 2);
            gl.uniform1i(u("u_radial"), effect.radial ? 1 : 0);
          }),
        );
      }
    }
    for (const effect of active) {
      if (effect.type !== "innerGlow") continue;
      const cover = blurred(effect.size, [0, 0], 0, true);
      place(blendOf(effect), (into) => paint(into, 1, tint(effect.color, effect.opacity, 1.8, true), cover, [0, 0], 0, true));
    }
    for (const effect of active) {
      if (effect.type !== "innerShadow") continue;
      const shift = offset(effect.angle, effect.distance);
      const cover = blurred(effect.size, shift, 1, true);
      place(blendOf(effect), (into) => paint(into, 1, tint(effect.color, effect.opacity, 0, true), cover, shift, 1, true));
    }
    // Contours : le premier de la liste est posé en dernier (au-dessus). Une seule carte de distances pour tous.
    const strokes = active.filter((e): e is Extract<LayerEffect, { type: "stroke" }> => e.type === "stroke").reverse();
    if (strokes.length > 0) {
      const widths = strokes.map((s) => (s.position === "outside" ? [s.width, 0] : s.position === "inside" ? [0, s.width] : [s.width / 2, s.width / 2]) as [number, number]);
      const needOut = widths.some(([o]) => o > 0);
      const needIn = widths.some(([, i]) => i > 0);
      const cap = Math.ceil(Math.max(...widths.flat())) + 2;
      const columns = this.scratchTexture("rows", gl.RG16F, tw, th);
      const distances = this.scratchTexture("map", gl.R32F, tw, th);
      for (const [program, to, from] of [[this.columnsProgram, columns, null], [this.rowsProgram, distances, columns]] as const) {
        setCommon(program);
        gl.uniform1i(program.uniform("u_cap"), cap);
        gl.uniform1i(program.uniform("u_needOut"), needOut ? 1 : 0);
        gl.uniform1i(program.uniform("u_needIn"), needIn ? 1 : 0);
        bind(1, from?.texture ?? null);
        target(to.texture, to.w, to.h);
        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
        bind(1, null);
      }
      const cover: Coverage = { mode: 0, map: distances.texture, factor: 1, mapSize: [tw, th] };
      strokes.forEach((stroke, n) => {
        place(blendOf(stroke), (into) =>
          paint(
            into,
            2,
            (u) => {
              tint(stroke.color, stroke.opacity)(u);
              gl.uniform2f(u("u_band"), widths[n]![0], widths[n]![1]);
            },
            cover,
          ),
        );
      });
    }

    // Textures non reprises : libérées. Niveaux de détail des textures posées.
    // Calque stylé en RGBA8, comme le moteur le livre.
    this.clearTexture(main, tw, th);
    paint(main, 5, () => undefined, { mode: 0, map: sum.texture, factor: 1, mapSize: [tw, th] });
    for (const left of pool) gl.deleteTexture(left);
    for (const texture of [main, ...under.map((p) => p.texture), ...over.map((p) => p.texture)]) {
      bind(0, texture);
      gl.generateMipmap(gl.TEXTURE_2D);
    }
    for (let unit = 2; unit >= 0; unit--) bind(unit, null);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    let bytes = (1 + under.length + over.length) * Math.round(tw * th * 4 * (4 / 3));
    for (const s of this.scratch.values()) bytes += s.bytes;
    return { rect: [left, top, tw, th], main, under, over, bytes, reduction };
  }

  /** Textures d'un résultat qui ne sert plus. */
  release(result: StyledPreview): void {
    for (const texture of [result.main, ...result.under.map((p) => p.texture), ...result.over.map((p) => p.texture)]) this.gl.deleteTexture(texture);
  }

  /** Tampons de calcul libérés (fin de l'aperçu). */
  trim(): void {
    for (const s of this.scratch.values()) this.gl.deleteTexture(s.texture);
    this.scratch.clear();
  }

  private colorTexture(w: number, h: number): WebGLTexture {
    const { gl } = this;
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, Math.floor(Math.log2(Math.max(w, h))) + 1, gl.RGBA8, w, h);
    return texture;
  }

  private clearTexture(texture: WebGLTexture, w: number, h: number): void {
    const { gl } = this;
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
    gl.viewport(0, 0, w, h);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
  }

  /** Tampon de calcul `name`, au moins de `w` × `h`, recyclé tant qu'il a le bon format et assez de place. */
  private scratchTexture(name: string, format: number, w: number, h: number): Scratch {
    const { gl } = this;
    const known = this.scratch.get(name);
    if (known && known.w === w && known.h === h) return known;
    if (known) gl.deleteTexture(known.texture);
    const texture = gl.createTexture();
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, texture);
    gl.texStorage2D(gl.TEXTURE_2D, 1, format, w, h);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    const texel = format === gl.RGBA16F ? 8 : format === gl.RG16F || format === gl.R32F ? 4 : 2;
    const entry = { texture, w, h, bytes: w * h * texel };
    this.scratch.set(name, entry);
    return entry;
  }

  dispose(): void {
    this.trim();
    this.gl.deleteTexture(this.kernel);
    this.gl.deleteFramebuffer(this.framebuffer);
  }
}

const padded = (n: number): number => Math.max(STEP, Math.ceil(n / STEP) * STEP);
