// Outils empilés, comme dans Photoshop : chaque case de la barre d'outils réunit un groupe d'outils (Rectangle et
// Ellipse de sélection, Dégradé et Pot de peinture, Main et Rotation de l'affichage…). La case affiche le dernier
// outil utilisé du groupe ; un clic droit (ou un appui long, ou un clic sur le petit triangle) ouvre la liste des
// autres. La lettre du groupe reprend l'outil affiché ; Maj + la lettre passe à l'outil suivant du groupe.
//
// Les « variantes » qui ne sont, côté application, qu'un réglage d'un même outil (forme de la sélection, du lasso,
// du calque de forme, dégradé ou pot) deviennent ici des outils à part entière, comme dans Photoshop.
import type { ActionId } from "./actions";
import { api } from "./ipc";
import type { IconName } from "./icons";
import { app, type ToolId } from "../state/app.svelte";

export type ToolVariantId =
  | "move"
  | "marqueeRect"
  | "marqueeEllipse"
  | "lassoFree"
  | "lassoPolygon"
  | "quickSelect"
  | "wand"
  | "crop"
  | "eyedropper"
  | "heal"
  | "brush"
  | "eraser"
  | "gradient"
  | "bucket"
  | "pen"
  | "text"
  | "pathSelect"
  | "shapeRect"
  | "shapeEllipse"
  | "shapeTriangle"
  | "shapePolygon"
  | "hand"
  | "rotateView"
  | "zoom";

export interface ToolVariant {
  id: ToolVariantId;
  /** Outil de l'application qui est pris en main. */
  tool: ToolId;
  /** Libellé de la liste déroulante, comme dans Photoshop (« Outil Pot de peinture »). */
  label: string;
  /** Nom court, pour la bulle d'aide. */
  name: string;
  icon: IconName;
  /** Action dont le raccourci (personnalisable) sélectionne cet outil : sa lettre est affichée à droite de la liste. */
  action: ActionId;
  /** Règle la variante (forme, type de remplissage…) avant de prendre l'outil en main. */
  apply?: () => void;
  /** Cet outil est-il celui en main ? Sans fonction : il suffit que l'outil de l'application soit `tool`. */
  matches?: () => boolean;
  /** Double-clic sur la case, comme dans Photoshop : Main = taille écran, Zoom = 100 %, Rotation = 0°. */
  doubleClick?: ActionId;
}

export interface ToolGroup {
  id: string;
  variants: ToolVariant[];
  /** Outil affiché tant que l'utilisateur n'en a pas choisi d'autre. */
  initial: ToolVariantId;
  /** Action « outil suivant du groupe » (Maj + la lettre), gardée pour les raccourcis déjà personnalisés. */
  cycle?: ActionId;
}

const shapeIs = (kind: "rect" | "ellipse") => () => app.tool === "shape" && app.shapeKind === kind;

export const TOOL_GROUPS: ToolGroup[] = [
  { id: "move", initial: "move", variants: [{ id: "move", tool: "move", label: "Outil Déplacement", name: "Déplacement", icon: "move", action: "tool.move" }] },
  {
    id: "marquee",
    initial: "marqueeRect",
    cycle: "tool.marqueeCycle",
    variants: [
      { id: "marqueeRect", tool: "marquee", label: "Outil Rectangle de sélection", name: "Rectangle de sélection", icon: "marquee", action: "tool.marquee", apply: () => (app.marqueeShape = "rect"), matches: () => app.tool === "marquee" && app.marqueeShape === "rect" },
      { id: "marqueeEllipse", tool: "marquee", label: "Outil Ellipse de sélection", name: "Ellipse de sélection", icon: "marqueeEllipse", action: "tool.marquee", apply: () => (app.marqueeShape = "ellipse"), matches: () => app.tool === "marquee" && app.marqueeShape === "ellipse" },
    ],
  },
  {
    id: "lasso",
    initial: "lassoFree",
    cycle: "tool.lassoCycle",
    variants: [
      { id: "lassoFree", tool: "lasso", label: "Outil Lasso", name: "Lasso", icon: "lasso", action: "tool.lasso", apply: () => (app.lassoShape = "free"), matches: () => app.tool === "lasso" && app.lassoShape === "free" },
      { id: "lassoPolygon", tool: "lasso", label: "Outil Lasso polygonal", name: "Lasso polygonal", icon: "lassoPolygon", action: "tool.lasso", apply: () => (app.lassoShape = "polygon"), matches: () => app.tool === "lasso" && app.lassoShape === "polygon" },
    ],
  },
  {
    id: "wand",
    // W reprend la baguette magique, comme avant les outils empilés.
    initial: "wand",
    cycle: "tool.wandCycle",
    variants: [
      { id: "quickSelect", tool: "quickSelect", label: "Outil Sélection rapide", name: "Sélection rapide", icon: "quickSelect", action: "tool.wand" },
      { id: "wand", tool: "wand", label: "Outil Baguette magique", name: "Baguette magique", icon: "wand", action: "tool.wand" },
    ],
  },
  { id: "crop", initial: "crop", variants: [{ id: "crop", tool: "crop", label: "Outil Recadrage", name: "Recadrage", icon: "crop", action: "tool.crop" }] },
  { id: "eyedropper", initial: "eyedropper", variants: [{ id: "eyedropper", tool: "eyedropper", label: "Outil Pipette", name: "Pipette", icon: "pipette", action: "tool.eyedropper" }] },
  { id: "heal", initial: "heal", variants: [{ id: "heal", tool: "heal", label: "Outil Correcteur localisé", name: "Correcteur localisé", icon: "heal", action: "tool.heal" }] },
  { id: "brush", initial: "brush", variants: [{ id: "brush", tool: "brush", label: "Outil Pinceau", name: "Pinceau", icon: "brush", action: "tool.brush" }] },
  { id: "eraser", initial: "eraser", variants: [{ id: "eraser", tool: "eraser", label: "Outil Gomme", name: "Gomme", icon: "eraser", action: "tool.eraser" }] },
  {
    id: "fill",
    // G prend le pot de peinture tant que l'utilisateur n'a pas pris le dégradé.
    initial: "bucket",
    cycle: "tool.fillCycle",
    variants: [
      // Pot de peinture en premier, comme la touche G par défaut ; le dégradé ensuite.
      { id: "bucket", tool: "fill", label: "Outil Pot de peinture", name: "Pot de peinture", icon: "bucket", action: "tool.fill", apply: () => (app.fill.kind = "bucket"), matches: () => app.tool === "fill" && app.fill.kind === "bucket" },
      { id: "gradient", tool: "fill", label: "Outil Dégradé", name: "Dégradé", icon: "gradient", action: "tool.fill", apply: () => (app.fill.kind = "gradient"), matches: () => app.tool === "fill" && app.fill.kind === "gradient" },
    ],
  },
  { id: "pen", initial: "pen", variants: [{ id: "pen", tool: "pen", label: "Outil Plume", name: "Plume", icon: "pen", action: "tool.pen" }] },
  { id: "text", initial: "text", variants: [{ id: "text", tool: "text", label: "Outil Texte", name: "Texte", icon: "type", action: "tool.text" }] },
  { id: "pathSelect", initial: "pathSelect", variants: [{ id: "pathSelect", tool: "pathSelect", label: "Outil Sélection directe", name: "Sélection directe", icon: "pathSelect", action: "tool.pathSelect" }] },
  {
    id: "shape",
    initial: "shapeRect",
    cycle: "tool.shapeCycle",
    // Seules les formes que le moteur sait tracer : rectangle, ellipse et polygone (dont le triangle, à 3 côtés).
    variants: [
      { id: "shapeRect", tool: "shape", label: "Outil Rectangle", name: "Rectangle", icon: "shape", action: "tool.shape", apply: () => (app.shapeKind = "rect"), matches: shapeIs("rect") },
      { id: "shapeEllipse", tool: "shape", label: "Outil Ellipse", name: "Ellipse", icon: "shapeEllipse", action: "tool.shape", apply: () => (app.shapeKind = "ellipse"), matches: shapeIs("ellipse") },
      {
        id: "shapeTriangle",
        tool: "shape",
        label: "Outil Triangle",
        name: "Triangle",
        icon: "shapePolygon",
        action: "tool.shape",
        apply: () => ([app.shapeKind, app.shapeSides] = ["polygon", 3]),
        matches: () => app.tool === "shape" && app.shapeKind === "polygon" && app.shapeSides === 3,
      },
      {
        id: "shapePolygon",
        tool: "shape",
        label: "Outil Polygone",
        name: "Polygone",
        icon: "shapeHexagon",
        action: "tool.shape",
        // Comme le bouton « Polygone » de la barre d'options : un triangle devient un pentagone, sinon le nombre de côtés reste.
        apply: () => ([app.shapeKind, app.shapeSides] = ["polygon", app.shapeSides === 3 ? 5 : app.shapeSides]),
        matches: () => app.tool === "shape" && app.shapeKind === "polygon" && app.shapeSides !== 3,
      },
    ],
  },
  {
    id: "hand",
    initial: "hand",
    variants: [
      { id: "hand", tool: "hand", label: "Outil Main", name: "Main", icon: "hand", action: "tool.hand", doubleClick: "view.fit" },
      { id: "rotateView", tool: "rotateView", label: "Outil Rotation de l'affichage", name: "Rotation de l'affichage", icon: "rotateView", action: "tool.rotateView", doubleClick: "view.resetRotation" },
    ],
  },
  { id: "zoom", initial: "zoom", variants: [{ id: "zoom", tool: "zoom", label: "Outil Zoom", name: "Zoom", icon: "zoom", action: "tool.zoom", doubleClick: "view.actualSize" }] },
];

const VARIANTS = new Map(TOOL_GROUPS.flatMap((group) => group.variants.map((variant) => [variant.id, { variant, group }] as const)));

export const variantById = (id: ToolVariantId): ToolVariant | undefined => VARIANTS.get(id)?.variant;
export const groupOfVariant = (id: ToolVariantId): ToolGroup | undefined => VARIANTS.get(id)?.group;

const isCurrent = (variant: ToolVariant): boolean => (variant.matches ? variant.matches() : app.tool === variant.tool);

/** Outil empilé actuellement en main (d'après l'outil de l'application et ses réglages), ou `null`. */
export function currentVariant(): ToolVariant | null {
  for (const group of TOOL_GROUPS) {
    const found = group.variants.find(isCurrent);
    if (found) return found;
  }
  return null;
}

const SETTINGS_NAME = "tool-groups";

class ToolGroupsState {
  /** Outil affiché par chaque case, par identifiant de groupe. */
  shown = $state<Record<string, ToolVariantId>>(Object.fromEntries(TOOL_GROUPS.map((group) => [group.id, group.initial])));
}

export const toolGroups = new ToolGroupsState();

/** Outil affiché par la case d'un groupe (le dernier utilisé), toujours un membre du groupe. */
export function shownVariant(group: ToolGroup): ToolVariant {
  const id = toolGroups.shown[group.id];
  return group.variants.find((v) => v.id === id) ?? group.variants.find((v) => v.id === group.initial) ?? (group.variants[0] as ToolVariant);
}

let saveTimer: ReturnType<typeof setTimeout> | undefined;
/** Enregistrement différé : changer plusieurs fois d'outil d'affilée n'écrit qu'une fois. */
function save(): void {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    void api.saveSettings(SETTINGS_NAME, JSON.stringify({ shown: $state.snapshot(toolGroups.shown) })).catch(() => undefined);
  }, 400);
}

/** La case du groupe affiche désormais cet outil (et s'en souviendra au prochain lancement). */
export function rememberVariant(id: ToolVariantId): void {
  const group = groupOfVariant(id);
  if (!group || toolGroups.shown[group.id] === id) return;
  toolGroups.shown[group.id] = id;
  save();
}

/** Prend un outil empilé en main : sa variante est réglée, et il devient l'outil affiché de sa case. */
export function activateVariant(id: ToolVariantId): void {
  const variant = variantById(id);
  if (!variant) return;
  variant.apply?.();
  app.tool = variant.tool;
  rememberVariant(id);
}

/** Groupe dont un outil répond à cette action (sa lettre). */
const groupOfAction = (action: ActionId): ToolGroup | undefined => TOOL_GROUPS.find((group) => group.variants.some((v) => v.action === action));

/**
 * Lettre d'un outil (M, G, U…) : l'outil affiché de sa case s'il répond à cette lettre, sinon le premier du groupe
 * qui y répond (Main et Rotation de l'affichage partagent une case mais pas leur lettre, H et R).
 */
export function selectGroupTool(action: ActionId): void {
  const group = groupOfAction(action);
  if (!group) return;
  const shown = shownVariant(group);
  const target = shown.action === action ? shown : group.variants.find((v) => v.action === action);
  if (target) activateVariant(target.id);
}

/**
 * Maj + la lettre : outil suivant de la case parmi ceux qui partagent cette lettre, à partir de celui en main (ou,
 * si la case n'est pas en main, de celui qu'elle affiche), comme dans Photoshop.
 */
export function cycleGroupTool(action: ActionId): void {
  const group = groupOfAction(action);
  if (!group) return;
  const candidates = group.variants.filter((v) => v.action === action);
  if (candidates.length === 0) return;
  const base = candidates.find(isCurrent) ?? (candidates.includes(shownVariant(group)) ? shownVariant(group) : null);
  const next = base ? candidates[(candidates.indexOf(base) + 1) % candidates.length] : candidates[0];
  if (next) activateVariant(next.id);
}

/** Action « outil suivant » d'un groupe à partir de son action Maj + lettre (raccourcis gardés : Maj+M, Maj+G…). */
export function cycleByCycleAction(cycle: ActionId): void {
  const group = TOOL_GROUPS.find((g) => g.cycle === cycle);
  const letter = group?.variants[0]?.action;
  if (letter) cycleGroupTool(letter);
}

/** Au démarrage : relit l'outil affiché par chaque case. Un réglage illisible ou inconnu est ignoré. */
export async function loadToolGroups(): Promise<void> {
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored = (text ? JSON.parse(text) : null) as { shown?: Record<string, unknown> } | null;
    for (const group of TOOL_GROUPS) {
      const id = stored?.shown?.[group.id];
      if (typeof id === "string" && group.variants.some((v) => v.id === id)) toolGroups.shown[group.id] = id as ToolVariantId;
    }
  } catch {
    // Réglage absent ou illisible : chaque case affiche son outil par défaut.
  }
}
