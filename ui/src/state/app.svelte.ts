import { invoke } from "@tauri-apps/api/core";
// État de l'interface (runes Svelte 5). Aucune donnée de pixels ici, hormis les miniatures.
import { SvelteMap } from "svelte/reactivity";
import type { GradientState } from "../lib/gradient";
import type { MetadataSettings } from "../lib/metadata";
import type { PerformanceSettings } from "../lib/performance.svelte";
import type { ProofProfileId } from "../lib/proof-profiles";
import { type BrushDynamics, type BrushKind, type BrushTipInfo, NEUTRAL_DYNAMICS } from "../lib/ipc";
import type { DocumentInfo, Filter, FontFamily, LayerInfo, Rgb, ShapeData, StyleRun, SystemInfo, TextData } from "../lib/ipc";
import type { Camera } from "../viewport/camera";

export type ToolId = "move" | "crop" | "shape" | "heal" | "quickSelect" | "marquee" | "lasso" | "wand" | "eyedropper" | "brush" | "eraser" | "fill" | "pen" | "text" | "pathSelect" | "hand" | "zoom";
export type PaintTool = "brush" | "eraser";

export interface BrushSettings {
  size: number;
  hardness: number;
  opacity: number;
  pressureSize: boolean;
  /** La pression du stylet module l'opacité. */
  pressureOpacity: boolean;
}

export interface Point {
  x: number;
  y: number;
}

/** Transformation libre en cours (Ctrl+T). Tant qu'elle existe, les autres outils sont suspendus. */
export interface TransformSession {
  ids: number[];
  gesture: string;
  /** Coins du cadre d'origine : haut gauche, haut droit, bas droit, bas gauche. */
  source: Point[];
  /** Coins courants, même ordre. La matrice envoyée au moteur s'en déduit. */
  quad: Point[];
  /** Cadres précédents : Ctrl+Z revient d'une manipulation en arrière sans quitter la transformation. */
  steps: Point[][];
  /** Vrai dès qu'un aperçu a été envoyé : Échap devra alors le retirer de l'historique. */
  previewed: boolean;
  /** Cadre de l'outil Déplacement : non modal, validé automatiquement dès qu'on fait autre chose. */
  auto: boolean;
}

/** Saisie de texte en cours sur l'image. Toute la séance ne fait qu'un état d'historique. */
export interface TextEditSession {
  id: string;
  /** Document de la saisie : la validation ne touche qu'à lui (les ids de calque se répètent d'un document à l'autre). */
  docId: number;
  layerId: number;
  /** Texte tel que tapé, en avance éventuelle sur ce que le moteur a déjà confirmé. */
  draft: string;
  /** Calque créé par ce clic : Échap le retire. */
  isNew: boolean;
  /** Position initiale du curseur (clic dans un texte existant) ; `null` = fin du texte. */
  caretAt: number | null;
  /** Portions stylées, recalées à chaque frappe (en avance, comme `draft`, sur ce que le moteur a confirmé). */
  runs: StyleRun[];
  /** Sélection courante dans le texte, en unités UTF-16 : les réglages du panneau s'y appliquent. */
  selection: { start: number; end: number };
  /** Texte d'exemple d'un nouveau calque : entièrement sélectionné, la première frappe le remplace. */
  selectAll: boolean;
}

export interface OpenDocument {
  info: DocumentInfo;
  /**
   * Vrai tant que la vue est en « taille écran » : elle se réajuste alors toute seule quand la fenêtre
   * change de taille (agrandissement, plein écran…). Tout zoom ou déplacement manuel y met fin ; Ctrl+0 y revient.
   */
  autoFit: boolean;
  /** Révision du dernier enregistrement (ou de l'ouverture) : au-delà, le document a des modifications non enregistrées. */
  savedRevision: number;
  /** Nom de sa copie de secours (sauvegarde automatique), unique et stable tant que le document est ouvert. */
  recoveryKey: string;
  /** Révision et moment de la dernière sauvegarde automatique (ou de l'ouverture, ou du dernier enregistrement). */
  autosavedRevision: number;
  autosavedAt: number;
  /** `null` tant que la zone de travail n'a pas cadré le document une première fois. */
  camera: Camera | null;
  /** Calques sélectionnés ; le dernier est le calque actif. */
  selection: number[];
  /** Cible de la peinture sur le calque actif. */
  /** Ce que visent les outils : les pixels, le masque de fusion, ou les deux (Ctrl + clic sur l'autre vignette :
   *  l'outil Déplacement les déplace alors ensemble ; peindre vise les pixels). */
  target: "pixels" | "mask" | "both";
}

class AppState {
  documents = $state<OpenDocument[]>([]);
  activeId = $state<number | null>(null);
  tool = $state<ToolId>("move");
  rulersVisible = $state(true);
  /**
   * Affichage › Épreuve : profil d'impression simulé par la zone de travail (document et export restent RVB),
   * `null` sans épreuve. Voir lib/proof-profiles.ts.
   */
  proofProfile = $state<ProofProfileId | null>(null);
  /** Dernier profil d'épreuve choisi (gardé dans les réglages) : celui que rallume Épreuve CMJN (Ctrl+Y). */
  lastProofProfile = $state<ProofProfileId>("fogra39");
  /** Épreuve active, quel qu'en soit le profil (ancien état booléen, gardé pour ce qui le lit encore). */
  get proofCmyk(): boolean {
    return this.proofProfile !== null;
  }
  set proofCmyk(on: boolean) {
    this.proofProfile = on ? (this.proofProfile ?? this.lastProofProfile) : null;
  }
  /** Écran d'accueil : ouvert par l'écran de lancement quand il se retire, puis par Ctrl+N. */
  newDocumentDialogOpen = $state(false);
  shortcutsDialogOpen = $state(false);
  /** Libellé de l'opération longue en cours, affiché dans la barre d'état. */
  busy = $state<string | null>(null);
  /** Chargement des calques du document affiché (jauge discrète de la barre d'état) ; `null` au repos. */
  loading = $state<{ done: number; total: number } | null>(null);
  error = $state<string | null>(null);
  /** Taille de la zone de travail en pixels CSS, tenue à jour par le viewport. */
  viewport = $state({ width: 0, height: 0 });
  /** Position du pointeur en pixels du document. */
  pointer = $state<{ x: number; y: number } | null>(null);

  foreground = $state<Rgb>([0, 0, 0]);
  background = $state<Rgb>([255, 255, 255]);
  brushes = $state<Record<PaintTool, BrushSettings>>({
    brush: { size: 30, hardness: 1, opacity: 1, pressureSize: true, pressureOpacity: false },
    eraser: { size: 40, hardness: 1, opacity: 1, pressureSize: true, pressureOpacity: false },
  });

  /** Variante active des outils à plusieurs formes (Maj+M, Maj+L). */
  marqueeShape = $state<"rect" | "ellipse">("rect");
  lassoShape = $state<"free" | "polygon">("free");
  wand = $state({ tolerance: 32, contiguous: true, antiAlias: true, sample: 1, allLayers: false });
  /** Sélection rapide : diamètre du pinceau et sensibilité (tolérance de couleur). */
  quickSelect = $state({ size: 40, tolerance: 24 });
  /** Fenêtre « Modifier la sélection ». */
  modifySelectionOpen = $state(false);
  /** Forme du pinceau et de la gomme (une gomme « mélangeur » n'ayant pas de sens, elle redevient ronde). */
  brushKind = $state<BrushKind>({ type: "round" });
  /** Dynamique de forme du pinceau courant (espacement, dispersion, angle…). */
  brushDynamics = $state<BrushDynamics>({ ...NEUTRAL_DYNAMICS });
  /** Vignettes des pointes (adresses d'images en mémoire), par identifiant. */
  brushThumbnails = $state<Record<number, string>>({});
  /** Pointes connues : fournies avec le logiciel, puis importées. */
  brushTips = $state<BrushTipInfo[]>([]);
  /** Correcteur localisé : diamètre et dureté de la touche. */
  heal = $state({ size: 40, hardness: 0.6 });
  /** Lissage du tracé du pinceau et de la gomme, de 0 (brut) à 1 (très lissé, le trait suit avec un léger retard). */
  strokeSmoothing = $state(0.6);
  /** Outil G : dégradé ou pot de peinture (Maj+G pour alterner). */
  fill = $state({ kind: "gradient" as "gradient" | "bucket", radial: false, tolerance: 32, contiguous: true });
  /** Dégradé courant ; par défaut il suit les couleurs de premier plan et d'arrière-plan. */
  gradient = $state<GradientState>({ stops: [], dynamic: "foregroundToBackground" });
  /**
   * Masque rapide (Q) : le pinceau peint la sélection, affichée en rouge translucide.
   * Peindre ajoute à la sélection, Alt (ou la gomme) en retire.
   */
  quickMask = $state(false);
  /** Outil Forme (U) : rectangle ou ellipse (Maj+U pour alterner). */
  shapeKind = $state<ShapeData["kind"]>("rect");
  /** Nombre de côtés du polygone (3 = triangle). */
  shapeSides = $state(3);
  /** Fenêtre « Créer une forme » (double-clic avec l'outil Forme) : point cliqué, en pixels du document. */
  shapeDialog = $state<{ x: number; y: number } | null>(null);
  /** Dossier d'enregistrement par défaut (projets et exports) ; vide = celui que propose le système. */
  saveFolder = $state("");
  /** Arrondi, fond et contour de la prochaine forme. Un fond `undefined` = la couleur de premier plan du moment. */
  shapeDefaults = $state<Pick<ShapeData, "radius" | "stroke" | "strokeWidth"> & { fill?: number[] | null }>({ radius: 0, stroke: null, strokeWidth: 4 });
  /** Proportions imposées au cadre de recadrage (largeur / hauteur), ou libres. */
  cropRatio = $state<number | null>(null);
  /** Fluidité : fenêtre ouverte. */
  liquifyOpen = $state(false);
  /**
   * Vrai après un copier ou couper fait ici, jusqu'à ce que la fenêtre perde puis retrouve le focus
   * (on a pu copier autre chose ailleurs) : Ctrl+V recolle alors nos pixels en place plutôt que
   * d'importer leur copie PNG depuis le presse-papiers du système.
   */
  ownClipboard = false;
  /**
   * Empreinte du presse-papiers du système relevée juste après notre dernière copie (pixels ou calques). Au
   * collage, si elle n'a pas changé, c'est notre copie la plus récente ; sinon quelque chose a été copié ailleurs
   * depuis (capture d'écran…), et c'est ce contenu-là qu'on colle : le plus récent gagne.
   */
  clipboardMark: number | null = null;
  /** Sauvegarde automatique : intervalle en minutes (2 à 60), et avancement de celle en cours (`null` = aucune). */
  autosaveMinutes = $state(3);
  autosaveProgress = $state<number | null>(null);
  /** Copies de secours trouvées au démarrage (Lumitype s'était mal fermé) : la fenêtre de récupération les propose. */
  recoveries = $state<{ key: string; savedAt: number; size: number }[]>([]);
  /** Aperçu instantané d'un réglage en cours (curseur tiré) : la fenêtre native se retire pendant ce temps. */
  adjustPreviewing = $state(false);
  /** Écran d'accueil ouvert par-dessus les documents (Fichier > Accueil). Sans document, il est toujours là. */
  homeOpen = $state(false);
  get homeVisible(): boolean {
    return this.documents.length === 0 || this.homeOpen;
  }
  /** Calque survolé dans le panneau Calques : son étendue est montrée d'un cadre bleu sur l'image. */
  hoverLayer = $state<number | null>(null);
  /** Des fichiers sont glissés au-dessus de la fenêtre : la fenêtre native se retire pour laisser la webview les recevoir. */
  externalDrag = $state(false);
  /** Ce que le dernier Ctrl+C a copié : des pixels (sélection) ou des calques entiers (panneau Calques). */
  lastCopy: "pixels" | "layers" = "pixels";
  /** Calques en cours de glisser depuis le panneau Calques : document d'origine et calques. */
  layerDrag: { docId: number; ids: number[] } | null = null;
  /** Masque de fusion en cours de glisser : calque d'origine. */
  maskDrag: number | null = null;
  /** Menu du pinceau ouvert au clic droit sur l'image : position à l'écran. */
  brushMenu = $state<{ x: number; y: number } | null>(null);
  /** Avancement d'un export en cours (0 à 1) et étape ; `null` = pas d'export. */
  exportProgress = $state<{ progress: number; label: string } | null>(null);
  /** Fenêtre d'export. */
  exportOpen = $state(false);
  /** Fenêtre « À propos de Lumitype ». */
  aboutOpen = $state(false);
  /** Lignes d'alignement à afficher (pixels du document) quand ce qu'on déplace s'aimante. */
  smartGuides = $state<{ x: number | null; y: number | null }>({ x: null, y: null });
  /** Fenêtre « Disposition de repères ». */
  guideLayoutOpen = $state(false);
  /** Question « Enregistrer les modifications ? » en cours, et de quoi y répondre. */
  savePrompt = $state<{ name: string; resolve: (choice: "save" | "discard" | "cancel") => void } | null>(null);
  /** Calque dont la fenêtre « Effets de calque » est ouverte. */
  effectsLayer = $state<number | null>(null);
  /** Fenêtre des préférences. */
  preferencesOpen = $state(false);
  /** Fenêtre « Copyright et métadonnées » (menu Copyright). */
  copyrightOpen = $state(false);
  /** Métadonnées par défaut des exports (voir lib/metadata.ts). */
  metadata = $state<MetadataSettings>({ firstName: "", lastName: "", copyright: "", contact: "", keywords: "", description: "", embed: true, software: true, date: false });
  /** Réglages de performance (voir lib/performance.svelte.ts, qui importe ce module : ses défauts sont repris ici à la main). */
  performance = $state<PerformanceSettings>({ memoryPercent: 50, cores: 0, gpuPreviews: true, gpuMemoryPercent: 50, historyStates: 50 });
  /** Ce que la machine offre (cœurs, mémoire vive, mémoire vidéo), lu une fois au démarrage ; null tant que ce n'est pas lu. */
  systemInfo = $state<SystemInfo | null>(null);
  /** Compteur d'images de la webview : cadence et pire intervalle de la dernière demi-seconde (null = mesure arrêtée). */
  fps = $state<{ rate: number; worstMs: number; drawMs: number; gpuMs: number } | null>(null);
  /** Résolution dynamique : facteur du canevas (1 = écran ; 0,5 = quatre fois moins de pixels pendant un geste). */
  renderScale = $state(1);
  /** État de la composition sur carte graphique, pour les Préférences et la barre d'état. */
  gpuStatus = $state<{ renderer: string; maxTexture: number; active: boolean; reason: string | null; memoryMb: number } | null>(null);
  /** Onglet affiché dans le panneau de gauche. */
  leftTab = $state<"properties" | "adjustments" | "history">("properties");
  /** Filtre dont la boîte de dialogue est ouverte. */
  filterDialog = $state<Exclude<Filter["type"], "liquify"> | null>(null);
  /** Outil Déplacement : un clic sélectionne le calque visible sous le pointeur. */
  autoSelect = $state(true);
  guidesVisible = $state(true);
  /** Magnétisme : repères, bords et centre du document. */
  snapEnabled = $state(true);
  transform = $state<TransformSession | null>(null);
  /** Réglages du prochain calque de texte ; ils suivent les derniers choix de l'utilisateur. */
  /** La couleur du texte est la sienne : elle ne suit pas la couleur de premier plan. */
  textDefaults = $state<Omit<TextData, "content" | "runs">>({ fontFamily: "", size: 72, lineHeight: 1.2, tracking: 0, align: "left", color: [0, 0, 0], weight: 400, italic: false });
  textEdit = $state<TextEditSession | null>(null);
  recentColors = $state<Rgb[]>([]);
  /** Destinataire de la pipette quand un sélecteur de couleur est ouvert ; sinon elle règle le premier plan. */
  eyedropperTarget = $state<((rgb: Rgb) => void) | null>(null);
  /** Pipette armée : le prochain clic sur l'image prélève pour `eyedropperTarget`, sans avoir à tenir Alt. */
  eyedropperArmed = $state(false);
  /** Dernière couleur prélevée et position du pointeur (fenêtre), tant que le bouton est enfoncé. */
  sample = $state<{ x: number; y: number; rgb: Rgb } | null>(null);
  /** Polices installées ; `null` tant qu'elles n'ont pas été demandées au moteur. */
  fonts = $state<FontFamily[] | null>(null);
  /** Passe à vrai quand un calque de texte vient d'être créé : le champ de saisie prend alors le focus. */
  focusTextEditor = $state(false);
  featherDialogOpen = $state(false);

  /** Miniatures, par clé `document:calque:p|m`. */
  thumbnails = new SvelteMap<string, ImageData>();

  get active(): OpenDocument | null {
    return this.documents.find((d) => d.info.id === this.activeId) ?? null;
  }

  get activeLayer(): LayerInfo | null {
    const doc = this.active;
    const id = doc?.selection.at(-1);
    return doc?.info.layers.find((l) => l.id === id) ?? null;
  }

  /** Vrai si le document a été modifié depuis son ouverture ou son dernier enregistrement. */
  isModified(doc: OpenDocument): boolean {
    return doc.info.revision !== doc.savedRevision;
  }

  get paintTool(): PaintTool | null {
    return this.tool === "brush" || this.tool === "eraser" ? this.tool : null;
  }

  add(info: DocumentInfo): void {
    this.homeOpen = false; // un document s'ouvre : l'accueil laisse la place
    const first = info.layers[0];
    this.documents.push({
      info,
      autoFit: true,
      savedRevision: info.revision,
      recoveryKey: crypto.randomUUID(),
      autosavedRevision: info.revision,
      autosavedAt: Date.now(),
      camera: null,
      selection: first ? [first.id] : [],
      target: "pixels",
    });
    this.activeId = info.id;
  }

  /** Remplace la description d'un document et garde une sélection valide. */
  update(info: DocumentInfo): void {
    const entry = this.documents.find((d) => d.info.id === info.id);
    if (!entry) return;
    const previous = entry.info;
    const before = new Set(previous.layers.map((l) => l.id));
    const created = info.layers.filter((l) => !before.has(l.id));
    entry.info = info;

    if (created.length > 0) {
      // Nouveau calque, copie, groupe, import : l'élément créé le plus haut placé devient actif.
      const top = created.reduce((a, b) => (b.depth < a.depth ? b : a));
      entry.selection = [top.id];
      entry.target = "pixels";
    } else {
      const alive = new Set(info.layers.map((l) => l.id));
      const kept = entry.selection.filter((id) => alive.has(id));
      // Un calque sélectionné a disparu : on en sélectionne un autre. Une sélection vidée
      // exprès (clic dans le vide avec l'outil Déplacement) reste vide.
      const lostSome = kept.length < entry.selection.length;
      if (kept.length === 0 && lostSome) {
        const next = successorOf(previous.layers, entry.selection.at(-1), alive) ?? info.layers[0]?.id;
        entry.selection = next === undefined ? [] : [next];
      } else {
        entry.selection = kept;
      }
    }
    if (entry.target !== "pixels" && !this.activeLayer?.hasMask) entry.target = "pixels";
  }

  remove(id: number): void {
    const index = this.documents.findIndex((d) => d.info.id === id);
    if (index < 0) return;
    // Document fermé (enregistré ou abandonné en connaissance de cause) : sa copie de secours ne sert plus.
    const closing = this.documents[index];
    if (closing) void invoke("recovery_delete", { key: closing.recoveryKey }).catch(() => undefined);
    this.documents.splice(index, 1);
    for (const key of [...this.thumbnails.keys()]) {
      if (key.startsWith(`${id}:`)) this.thumbnails.delete(key);
    }
    if (this.activeId === id) {
      this.activeId = this.documents[Math.min(index, this.documents.length - 1)]?.info.id ?? null;
    }
  }

  /** `fit` : ce placement est un ajustement à la fenêtre ; sinon c'est un choix de l'utilisateur, qui met fin à l'ajustement automatique. */
  setCamera(camera: Camera, fit = false): void {
    const active = this.active;
    if (active) [active.camera, active.autoFit] = [camera, fit];
  }

  /** Fournie par le viewport : amène la caméra à destination par une transition douce. */
  cameraAnimator: ((target: Camera, fit: boolean) => void) | null = null;
  /** Fournie par le viewport : remet la vue en « taille écran » (après un recadrage, par exemple). */
  fitToWindow: (() => void) | null = null;

  /** Déplacement « de confort » (paliers de zoom, taille écran, 100 %) : animé si possible. */
  moveCamera(target: Camera, fit = false): void {
    if (this.cameraAnimator) this.cameraAnimator(target, fit);
    else this.setCamera(target, fit);
  }

  fail(error: unknown): void {
    this.error = typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
  }
}

/**
 * Calque à sélectionner après la disparition du calque actif `lostId`, comme dans Photoshop : son voisin
 * du dessous dans le même parent, sinon celui du dessus, sinon le parent (puis les ancêtres suivants).
 * `layers` est la description AVANT la suppression : elle seule connaît encore la place du calque
 * disparu. `alive` contient les ids qui existent encore ; `undefined` si aucun candidat ne convient.
 */
function successorOf(layers: readonly LayerInfo[], lostId: number | undefined, alive: ReadonlySet<number>): number | undefined {
  const byId = new Map(layers.map((l) => [l.id, l]));
  const seen = new Set<number>(); // garde contre une parenté incohérente (cycle)
  let lost = lostId === undefined ? undefined : byId.get(lostId);
  while (lost && !seen.has(lost.id)) {
    seen.add(lost.id);
    const parent = lost.parent;
    const index = lost.index;
    // Indices du moteur : du bas vers le haut. Les voisins du dessous d'abord (du plus proche au plus
    // lointain), puis ceux du dessus.
    const siblings = layers.filter((l) => l.parent === parent && l.id !== lost?.id && alive.has(l.id));
    const below = siblings.filter((l) => l.index < index).sort((a, b) => b.index - a.index)[0];
    const above = siblings.filter((l) => l.index > index).sort((a, b) => a.index - b.index)[0];
    const found = below ?? above;
    if (found) return found.id;
    if (parent === null) return undefined;
    if (alive.has(parent)) return parent;
    // Le parent a disparu lui aussi (groupe supprimé avec son contenu) : on cherche autour de lui.
    lost = byId.get(parent);
  }
  return undefined;
}

export const app = new AppState();
