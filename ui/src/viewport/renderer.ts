import { t } from "../lib/i18n.svelte";
// Affichage WebGL2 des tuiles composées par le moteur et du contour de sélection.
//
// Tout ce qui est purement visuel (zoom, déplacement, animation du contour) se fait
// ici, sans aucun aller-retour vers Rust : seules les tuiles absentes sont demandées.
import type { Adjustment, DirtyInfo, DocumentInfo, LayerEffect } from "../lib/ipc";
import { type Camera, levelForScale } from "./camera";
import type { Dab as StrokeDab } from "../lib/brush-path";
import { PROOF_SHADER, VERTEX_SHADER } from "./shaders";
// Tables de l'épreuve CMJN, une par profil d'impression : régénérer avec `python3 export/lut-cmjn.py --tous`.
import FOGRA39_LUT_URL from "../assets/proof/fogra39.bin?url";
import FOGRA51_LUT_URL from "../assets/proof/fogra51.bin?url";
import FOGRA52_LUT_URL from "../assets/proof/fogra52.bin?url";
import GRACOL2006_LUT_URL from "../assets/proof/gracol2006.bin?url";
import SWOP2006C3_LUT_URL from "../assets/proof/swop2006c3.bin?url";
import type { ProofProfileId } from "../lib/proof-profiles";
import { type EffectsFetcher, GpuLayers, type GpuStatus, type LayerFetcher, type TexturePart } from "./gpu-layers";
import { GpuEffects } from "./gpu-effects";
import { type EffectsPreviewControls, provideEffectsPreview, withdrawEffectsPreview } from "../lib/effects-preview";
import { type Resolved, TileCache, type TileFetcher } from "./tile-cache";

/** Fichier de la table 3D de chaque profil d'épreuve (voir lib/proof-profiles.ts). */
const PROOF_LUT_URLS: Record<ProofProfileId, string> = {
  fogra39: FOGRA39_LUT_URL,
  fogra51: FOGRA51_LUT_URL,
  fogra52: FOGRA52_LUT_URL,
  gracol2006: GRACOL2006_LUT_URL,
  swop2006c3: SWOP2006C3_LUT_URL,
};

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
}

/** Parties de l'image pendant un aperçu de déplacement : calques de pixels de chacune. */
export interface MovePlan {
  below: number[];
  moving: number[];
  above: number[];
}

export type PartFetcher = (layers: number[], opaque: number | null) => TileFetcher;

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
  private readonly dabProgram: Program;
  private readonly tintProgram: Program;
  private readonly proofProgram: Program;
  /** Copie de l'écran pour la passe d'épreuve CMJN (allouée à la taille du canevas, à la demande). */
  private proofTexture: { texture: WebGLTexture; width: number; height: number } | null = null;
  /**
   * Tables 3D sRGB → presse → sRGB de l'épreuve, par profil (voir `export/lut-cmjn.py`) ; chacune chargée à sa
   * première épreuve puis gardée : repasser d'un profil à l'autre ne recharge rien.
   */
  private readonly proofLuts = new Map<ProofProfileId, { texture: WebGLTexture; size: number } | "loading" | "failed">();
  /** Épreuve CMJN : l'image affichée simule une impression quadri selon ce profil (Affichage › Épreuve), `null` sans épreuve. */
  proofProfile: ProofProfileId | null = null;
  /** Masque rapide : la sélection s'affiche en rouge translucide au lieu du contour animé. */
  quickMask = false;
  private readonly strokeProgram: Program;
  private readonly coverageTarget: WebGLTexture;
  /** La carte sait dessiner (et mélanger) dans une texture en demi-flottant. */
  private readonly floatCoverage: boolean;
  private readonly coverageFramebuffer: WebGLFramebuffer;
  private coverageSize: [number, number] = [0, 0];
  /** Attentes de `whenSettled` ; `layers` : seulement les textures de ces calques (sinon tout ce qui se charge). */
  private settledWaiters: { resolve: () => void; layers?: readonly number[] }[] = [];
  private doc: DocumentInfo | null = null;
  private frame = 0;
  private readonly gpu: GpuLayers;
  private gpuPreview: GpuPreview | null = null;
  /** Aperçu d'un réglage relâché : à retirer dès que la carte graphique a les valeurs définitives. */
  private gpuAdjustReleasing = false;
  /** Aperçu des effets de calque, enregistré auprès de la boîte de dialogue des effets (lib/effects-preview.ts). */
  private readonly effectsControls: EffectsPreviewControls;

  constructor(
    canvas: HTMLCanvasElement,
    fetchImage: TileFetcher,
    fetchSelection: TileFetcher,
    private readonly fetchPart: PartFetcher,
    private readonly requestRedraw: () => void,
    private readonly canvasColor: readonly [number, number, number],
    fetchLayer: DocLayerFetcher,
    fetchAdjust: (adjustment: Adjustment) => Promise<ArrayBuffer>,
    fetchEffects: (docId: number, layer: number) => ReturnType<EffectsFetcher>,
  ) {
    const gl = canvas.getContext("webgl2", { alpha: false, antialias: false });
    if (!gl) {
      throw new Error(t("WebGL2 indisponible : vérifiez le pilote graphique."));
    }
    this.gl = gl;
    this.tileProgram = this.link(TILE_SHADER);
    this.checkerProgram = this.link(CHECKER_SHADER);
    this.antsProgram = this.link(ANTS_SHADER);
    this.dabProgram = this.link(DAB_SHADER);
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
    this.image = new TileCache(gl, fetchImage, (payload) => ({ texture: this.upload(payload, gl.RGBA8, gl.RGBA), uniform: 0 }), requestRedraw, 512);
    this.selection = new TileCache(
      gl,
      fetchSelection,
      // Premier octet : valeur de fond du masque, celle des tuiles sans données.
      (payload) => ({ texture: this.upload(payload.subarray(1), gl.R8, gl.RED), uniform: payload[0] ?? 0 }),
      requestRedraw,
      256,
    );

    const layerFetcher: LayerFetcher = (layer, rect, part, limit) => (this.doc ? fetchLayer(this.doc.id, layer, rect, part, limit) : Promise.reject(new Error(t("aucun document"))));
    const effectsFetcher: EffectsFetcher = (layer) => (this.doc ? fetchEffects(this.doc.id, layer) : Promise.reject(new Error(t("aucun document"))));
    this.gpu = new GpuLayers(gl, (fragment) => this.link(fragment), layerFetcher, fetchAdjust, effectsFetcher, requestRedraw);
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
      this.stroke = null;
      this.image.hold = false;
      this.image.reset();
      this.selection.reset();
      this.endMovePreview();
    }
    this.doc = doc;
    this.gpu.setDocument(doc);
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

  gpuStatus(): GpuStatus {
    return this.gpu.status();
  }

  /** Chargement des calques en cours (jauge de la barre d'état), ou `null` au repos. */
  loadProgress(): { done: number; total: number } | null {
    return this.gpu.progress();
  }

  /** Le moteur signale ce qui a changé : les tuiles concernées seront rechargées au prochain rendu. */
  invalidate(dirty: DirtyInfo, revision: number, immediate: boolean): void {
    const doc = this.doc;
    if (!doc) return;
    if (dirty.all) this.image.invalidateAll(revision);
    else this.image.invalidate(dirty.tiles, doc.maxLevel, revision, immediate);
    if (dirty.selection) this.selection.invalidateAll(revision);
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
      this.gpuPreview = { moving: new Set(plan.moving), parts: "both", opaque, dx: 0, dy: 0, alpha: 1 };
      this.gpu.setLive(opaque);
      return;
    }
    const part = (layers: number[], full: number | null) =>
      new TileCache(this.gl, this.fetchPart(layers, full), (payload) => ({ texture: this.upload(payload, this.gl.RGBA8, this.gl.RGBA), uniform: 0 }), this.requestRedraw, 256);
    this.preview = { below: part(plan.below, null), moving: part(plan.moving, opaque), above: part(plan.above, null), dx: 0, dy: 0, alpha: 1 };
  }

  /** Un aperçu est en cours (déplacement, opacité, réglage, trait, ébauche) : l'image change à chaque instant. */
  previewing(): boolean {
    return this.preview !== null || this.gpuPreview !== null || this.stroke !== null || this.draft > 0;
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
    this.endMovePreview();
    this.gpuPreview = { moving: new Set(ids), parts, opaque: null, dx: 0, dy: 0, alpha: 1 };
    return true;
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
    if (this.preview) this.preview.alpha = Math.min(1, Math.max(0, alpha));
    if (this.gpuPreview) this.gpuPreview.alpha = Math.min(1, Math.max(0, alpha));
  }

  setMoveOffset(dx: number, dy: number): void {
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

  endMovePreview(): void {
    if (this.preview?.lut) this.gl.deleteTexture(this.preview.lut.texture);
    for (const cache of this.preview ? [this.preview.below, this.preview.moving, this.preview.above] : []) cache?.reset();
    this.preview = null;
    this.gpuPreview = null;
    this.gpu.setLive(null);
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
    this.stroke = same ? { ...previous, releasing: false } : { color, alpha, hardness, accumulate, dabs: [], drawn: 0, signature: "", releasing: false };
    this.image.hold = true;
  }

  addStrokeDabs(dabs: readonly StrokeDab[]): void {
    this.stroke?.dabs.push(...dabs);
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

  draw(camera: Camera, dpr: number): void {
    if (this.suspended) return;
    this.drawScene(camera, dpr);
    if (this.proofProfile && this.doc) this.drawProof(this.proofProfile, camera, dpr);
  }

  /**
   * Épreuve CMJN : l'image finie est recopiée de l'écran dans une texture, puis réécrite convertie, sur le seul
   * rectangle du document (le fond de la zone de travail et le damier gardent leurs gris). Une passe plein document,
   * quel que soit le nombre de calques : le coût est le même que celui du damier.
   */
  private drawProof(profile: ProofProfileId, camera: Camera, dpr: number): void {
    const { gl } = this;
    const doc = this.doc!;
    // Sans table (premier affichage, ou fichier introuvable), l'image reste en sRGB plutôt qu'une épreuve fausse.
    if (!this.proofLuts.has(profile)) this.loadProofLut(profile);
    const lut = this.proofLuts.get(profile);
    if (typeof lut !== "object") return;
    const [viewW, viewH] = [gl.canvas.width, gl.canvas.height];
    const scale = camera.zoom * dpr;
    const [x, y, w, h] = snap(camera.x * dpr, camera.y * dpr, camera.x * dpr + doc.width * scale, camera.y * dpr + doc.height * scale);
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
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
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
   * trilinéaire. Un seul chargement par profil et par renderer ; un échec (fichier absent du paquet) est noté et
   * l'épreuve de ce profil reste sans effet, avec un message en console plutôt qu'une erreur à chaque image.
   */
  private loadProofLut(profile: ProofProfileId): void {
    this.proofLuts.set(profile, "loading");
    fetch(PROOF_LUT_URLS[profile])
      .then((response) => (response.ok ? response.arrayBuffer() : Promise.reject(new Error(`${response.status} ${response.statusText}`))))
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
    const viewW = gl.canvas.width;
    const viewH = gl.canvas.height;
    this.frame += 1;

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
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
    const level = Math.min(doc.maxLevel, levelForScale(scale, doc.maxLevel) + this.draft);
    const frame = this.frameAt(doc, level, scale, originX, originY, viewW, viewH);
    this.drawChecker(frame, viewW, viewH, dpr);

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
    // Rien ne déborde du document.
    const [x, y, w, h] = snap(originX, originY, originX + doc.width * scale, originY + doc.height * scale);
    gl.enable(gl.BLEND);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    this.gpu.draw(
      { scale, originX, originY, viewW, viewH },
      {
        shift: (id, kind) => (preview?.moving.has(id) && (preview.parts === "both" || preview.parts === kind) ? [preview.dx, preview.dy] : [0, 0]),
        opacity: (id, opacity, fill) => (preview && preview.opaque === id ? preview.alpha * fill : opacity),
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
   * Vrai si le pixel d'écran (en pixels physiques, origine en haut à gauche) est dans la
   * sélection. Lit le masque du dernier rendu : réponse immédiate, sans interroger le moteur.
   */
  isSelectedAt(x: number, y: number): boolean {
    const { gl } = this;
    if (!this.doc?.hasSelection || this.maskSize[0] === 0) return false;
    const pixel = new Uint8Array(4);
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.maskFramebuffer);
    gl.readPixels(Math.round(x), this.maskSize[1] - 1 - Math.round(y), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, pixel);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return (pixel[0] ?? 0) >= 128;
  }

  dispose(): void {
    withdrawEffectsPreview(this.effectsControls);
    this.gpu.reset();
    this.gpu.effectsCompute?.dispose();
    this.image.reset();
    this.selection.reset();
  }

  /** Accumule les nouvelles empreintes dans la texture de couverture (maximum, pas addition), puis pose le trait sur l'image. */
  private drawStroke(stroke: StrokePreview, frame: Frame, viewW: number, viewH: number): void {
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
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, viewW, viewH);
    const [x, y, w, h] = snap(originX, originY, originX + doc.width * scale, originY + doc.height * scale);
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

  private drawChecker(frame: Frame, viewW: number, viewH: number, dpr: number): void {
    const { gl } = this;
    const { doc, scale, originX, originY } = frame;
    gl.disable(gl.BLEND);
    gl.useProgram(this.checkerProgram.program);
    gl.uniform2f(this.checkerProgram.uniform("u_view"), viewW, viewH);
    gl.uniform4f(this.checkerProgram.uniform("u_rect"), ...snap(originX, originY, originX + doc.width * scale, originY + doc.height * scale));
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

    // Le calque déplacé ne doit pas déborder du document.
    const [x, y, w, h] = snap(originX, originY, originX + doc.width * scale, originY + doc.height * scale);
    gl.enable(gl.SCISSOR_TEST);
    gl.scissor(x, viewH - y - h, w, h);
    // Réglage : la table s'applique à la couche du milieu (base d'un réglage écrêté), sinon à « dessous ».
    const corrected = preview.adjusting ? (preview.moving ?? preview.below) : null;
    for (const [cache, at] of layers) this.drawTiles(cache, at, viewW, viewH, false, false, cache === preview.moving ? preview.alpha : 1, cache === corrected ? preview.lut : null);
    gl.disable(gl.SCISSOR_TEST);
    return true;
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

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
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

/** Bords alignés sur les pixels physiques : pas de jour entre deux tuiles voisines. */
function snap(x0: number, y0: number, x1: number, y1: number): [number, number, number, number] {
  const rx = Math.round(x0);
  const ry = Math.round(y0);
  return [rx, ry, Math.round(x1) - rx, Math.round(y1) - ry];
}
