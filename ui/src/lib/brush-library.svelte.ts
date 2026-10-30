// Bibliothèque des pinceaux : ce que montre le panneau des pinceaux (dossiers, pinceaux, aperçus de trait) et ce
// qu'il faut pour choisir un pinceau. Trois sortes de dossiers :
// - fournis : les ronds de base et le pack Krita, rangés par famille ;
// - importés : un dossier par fichier (.abr, .gbr, image), avec les préréglages Photoshop tels quels ;
// - personnels : créés par l'utilisateur (« Mes favoris » d'office), qui ne contiennent que des RÉFÉRENCES (clés) vers
//   les pinceaux des deux autres sortes, plus « Mes pinceaux », les réglages enregistrés à la main.
// Les dossiers personnels, l'état replié des dossiers, la taille des aperçus et la taille du panneau sont enregistrés
// dans les réglages (`brush-folders`).

import { t } from "./i18n.svelte";
import { type BrushDynamics, type BrushKind, type BrushParams, type BrushPresetInfo, type BrushTipInfo, type DualParam, type TextureParam, MIXER_DYNAMICS, NEUTRAL_DYNAMICS, api } from "./ipc";
import { app } from "../state/app.svelte";

/** Un pinceau tel que le panneau le présente et l'applique. */
export interface BrushEntry {
  key: string;
  name: string;
  kind: BrushKind;
  size: number;
  hardness: number;
  opacity: number;
  pressureSize: boolean;
  pressureOpacity: boolean;
  dynamics: BrushDynamics;
  dual: DualParam | null;
  texture: TextureParam | null;
}

export interface BrushFolder {
  id: string;
  name: string;
  /** `builtin` : fourni ; `pack` : fichier importé ; `user` : dossier personnel ; `mine` : réglages enregistrés. */
  source: "builtin" | "pack" | "user" | "mine";
  entries: BrushEntry[];
}

/** Réglages enregistrés à la main (« Mes pinceaux »), comme avant ; la pointe est désignée par son nom. */
interface SavedPreset {
  name: string;
  kind: "round" | "spray" | "mixer" | "tip";
  tip?: string;
  size: number;
  hardness: number;
  opacity: number;
  dynamics: BrushDynamics;
}

interface FolderSettings {
  /** Version des réglages : 2 = menu compact (les anciennes tailles de panneau sont oubliées). */
  v?: number;
  folders: { id: string; name: string; keys: string[] }[];
  collapsed: string[];
  thumb: number;
  view: "list" | "grid";
  panel: { width: number; height: number };
  /** Pinceaux récemment utilisés (clés), du plus récent au plus ancien : `RECENT_BRUSHES` au plus. */
  recent: string[];
  /** Couleur dynamique du pinceau : une nuance très proche, nouvelle à chaque trait (voir lib/color-jitter.ts). */
  dynamicColor: boolean;
}

/** Pinceaux récemment utilisés gardés (panneau Propriétés). */
export const RECENT_BRUSHES = 5;

/** Familles du pack fourni, dans l'ordre de `crates/engine/assets/brushes/brushes.tsv`. */
const BUILTIN_FAMILIES: { id: string; name: string; tips: string[] }[] = [
  { id: "general", name: "Généraux", tips: ["Marqueur rond", "Basique opacité", "Basique flux", "Basique taille", "Aérographe doux"] },
  { id: "ink", name: "Crayons et encres", tips: ["Crayon", "Crayon 4B large", "Crayon incliné", "Encre G-pen"] },
  { id: "wet", name: "Aquarelle et fusain", tips: ["Fusain roche", "Aquarelle ronde grunge", "Aquarelle plate"] },
  { id: "tones", name: "Trames et pixel art", tips: ["Pixel art", "Pixel art remplissage", "Trame moirée", "Trame pression"] },
  { id: "erasers", name: "Gommes", tips: ["Gomme ronde", "Gomme douce"] },
];

/** Réglages par défaut, en copie neuve (ils sont modifiés sur place). */
const defaults = (): FolderSettings => JSON.parse(JSON.stringify(DEFAULTS)) as FolderSettings;

const DEFAULTS: FolderSettings = { folders: [{ id: "favorites", name: "Mes favoris", keys: [] }], collapsed: ["builtin:wet", "builtin:tones", "builtin:erasers"], thumb: 34, view: "grid", panel: { width: 380, height: 460 }, recent: [], dynamicColor: false, v: 2 };

/**
 * Réglages du menu relus d'un fichier, champ par champ : un champ absent, d'un autre type ou abîmé garde sa valeur par
 * défaut. Un fichier d'une version précédente (sans dossiers, sans `collapsed`…) remplaçait les valeurs par défaut par
 * `undefined`, et le menu des pinceaux plantait à l'ouverture (« Cannot read properties of undefined (reading 'map') »).
 * Réglages d'avant le menu compact (`v` ≠ 2) : dossiers et replis gardés, taille du menu et des aperçus remises à neuf.
 */
export function readFolderSettings(stored: unknown): FolderSettings {
  const out = defaults();
  if (!stored || typeof stored !== "object") return out;
  const raw = stored as Record<string, unknown>;
  const text = (v: unknown): v is string => typeof v === "string";
  if (Array.isArray(raw.folders)) {
    const folders = raw.folders
      .filter((f): f is Record<string, unknown> => !!f && typeof f === "object")
      .filter((f) => text(f.id) && text(f.name))
      .map((f) => ({ id: f.id as string, name: f.name as string, keys: Array.isArray(f.keys) ? f.keys.filter(text) : [] }));
    if (folders.length > 0) out.folders = folders;
  }
  if (Array.isArray(raw.collapsed)) out.collapsed = raw.collapsed.filter(text);
  if (Array.isArray(raw.recent)) out.recent = raw.recent.filter(text).slice(0, RECENT_BRUSHES);
  if (typeof raw.dynamicColor === "boolean") out.dynamicColor = raw.dynamicColor;
  if (raw.v === 2) {
    if (typeof raw.thumb === "number" && Number.isFinite(raw.thumb)) out.thumb = raw.thumb;
    if (raw.view === "list" || raw.view === "grid") out.view = raw.view;
    const panel = raw.panel as Record<string, unknown> | undefined;
    if (panel && typeof panel === "object") {
      if (typeof panel.width === "number" && Number.isFinite(panel.width)) out.panel.width = panel.width;
      if (typeof panel.height === "number" && Number.isFinite(panel.height)) out.panel.height = panel.height;
    }
  }
  return out;
}

function entryOf(base: Partial<BrushEntry> & { key: string; name: string; kind: BrushKind }): BrushEntry {
  return { size: 30, hardness: 1, opacity: 1, pressureSize: true, pressureOpacity: false, dynamics: { ...NEUTRAL_DYNAMICS }, dual: null, texture: null, ...base };
}

class BrushLibraryState {
  tips = $state<BrushTipInfo[]>([]);
  presets = $state<BrushPresetInfo[]>([]);
  saved = $state<SavedPreset[]>([]);
  settings = $state<FolderSettings>(defaults());
  loaded = $state(false);
  private loading: Promise<void> | null = null;

  /** Tous les dossiers, dans l'ordre du panneau. */
  readonly folders: BrushFolder[] = $derived.by(() => {
    const byKey = new Map(this.all.map((e) => [e.key, e]));
    const user: BrushFolder[] = this.settings.folders.map((f) => ({ id: `user:${f.id}`, name: f.name, source: "user", entries: f.keys.map((k) => byKey.get(k)).filter((e): e is BrushEntry => !!e) }));
    return [...user, ...this.builtinFolders, ...this.packFolders, ...(this.mine.length > 0 ? [{ id: "mine", name: t("Mes pinceaux"), source: "mine" as const, entries: this.mine }] : [])];
  });

  readonly builtinFolders: BrushFolder[] = $derived.by(() => {
    const builtin = this.tips.filter((tip) => tip.builtin);
    const tipEntry = (tip: BrushTipInfo) => entryOf({ key: `builtin:${tip.name}`, name: tip.name, kind: { type: "tip", id: tip.id } });
    const used = new Set<number>();
    const folders = BUILTIN_FAMILIES.map((family) => {
      const entries = family.tips.map((name) => builtin.find((tip) => tip.name === name)).filter((tip): tip is BrushTipInfo => !!tip);
      entries.forEach((tip) => used.add(tip.id));
      return { id: `builtin:${family.id}`, name: family.name, source: "builtin" as const, entries: entries.map(tipEntry) };
    });
    // Les deux ronds de base en tête des pinceaux généraux ; une pointe fournie hors des familles, à la suite.
    folders[0]!.entries.unshift(
      entryOf({ key: "basic:hard", name: "Net", kind: { type: "round" }, hardness: 1 }),
      entryOf({ key: "basic:soft", name: "Flou", kind: { type: "round" }, hardness: 0 }),
    );
    folders[0]!.entries.push(...builtin.filter((tip) => !used.has(tip.id)).map(tipEntry));
    return folders;
  });

  readonly packFolders: BrushFolder[] = $derived.by(() => {
    const folders = new Map<string, BrushFolder>();
    for (const preset of this.presets) {
      const folder = folders.get(preset.folder) ?? { id: `pack:${preset.folder}`, name: preset.folder, source: "pack" as const, entries: [] };
      folder.entries.push(
        entryOf({
          key: `pack:${preset.key}`,
          name: preset.name,
          kind: preset.kind === "tip" && preset.tip !== null ? { type: "tip", id: preset.tip } : { type: "round" },
          size: preset.size,
          hardness: preset.hardness,
          opacity: preset.opacity,
          pressureSize: preset.pressureSize,
          pressureOpacity: preset.pressureOpacity,
          dynamics: { ...NEUTRAL_DYNAMICS, ...preset.dynamics },
          dual: preset.dual,
          texture: preset.texture,
        }),
      );
      folders.set(preset.folder, folder);
    }
    return [...folders.values()];
  });

  readonly mine: BrushEntry[] = $derived(
    this.saved.flatMap((preset): BrushEntry[] => {
      const tip = preset.kind === "tip" ? this.tips.find((t) => t.name === preset.tip) : undefined;
      if (preset.kind === "tip" && !tip) return [];
      const kind: BrushKind = tip ? { type: "tip", id: tip.id } : { type: preset.kind === "tip" ? "round" : preset.kind };
      return [entryOf({ key: `mine:${preset.name}`, name: preset.name, kind, size: preset.size, hardness: preset.hardness, opacity: preset.opacity, dynamics: { ...NEUTRAL_DYNAMICS, ...preset.dynamics } })];
    }),
  );

  readonly all: BrushEntry[] = $derived([...this.builtinFolders.flatMap((f) => f.entries), ...this.packFolders.flatMap((f) => f.entries), ...this.mine]);

  /** Charge la bibliothèque (une fois) et les réglages du panneau. */
  load(): Promise<void> {
    this.loading ??= (async () => {
      const [library, folders, saved] = await Promise.all([api.brushLibrary(), api.loadSettings("brush-folders").catch(() => null), api.loadSettings("brush-presets").catch(() => null)]);
      this.adopt(library.tips);
      this.presets = library.presets;
      try {
        this.settings = readFolderSettings(folders ? JSON.parse(folders) : null);
      } catch {
        this.settings = defaults();
      }
      try {
        const parsed: unknown = saved ? JSON.parse(saved) : [];
        // Pinceaux enregistrés : chacun vérifié (nom, genre), les valeurs manquantes remises par défaut.
        const kinds = ["round", "spray", "mixer", "tip"];
        const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);
        if (Array.isArray(parsed))
          this.saved = parsed
            .filter((p): p is Record<string, unknown> => !!p && typeof p === "object" && typeof p.name === "string" && kinds.includes(p.kind as string))
            .map((p) => ({
              name: p.name as string,
              kind: p.kind as SavedPreset["kind"],
              tip: typeof p.tip === "string" ? p.tip : undefined,
              size: num(p.size, 30),
              hardness: num(p.hardness, 1),
              opacity: num(p.opacity, 1),
              dynamics: { ...NEUTRAL_DYNAMICS, ...(p.dynamics && typeof p.dynamics === "object" ? (p.dynamics as Partial<BrushDynamics>) : {}) },
            }));
      } catch {
        this.saved = [];
      }
      this.loaded = true;
    })().catch((error: unknown) => {
      this.loading = null;
      throw error;
    });
    return this.loading;
  }

  /** Recharge la liste après un import ou un retrait. */
  async refresh(): Promise<void> {
    const library = await api.brushLibrary();
    this.adopt(library.tips);
    this.presets = library.presets;
  }

  /** Pointes connues de l'application (et leurs vignettes, pour le panneau Propriétés). */
  adopt(tips: BrushTipInfo[]): void {
    const known = new Set(app.brushTips.map((tip) => tip.id));
    app.brushTips = [...app.brushTips, ...tips.filter((tip) => !known.has(tip.id))];
    this.tips = app.brushTips;
    for (const item of tips) {
      if (app.brushThumbnails[item.id]) continue;
      api.brushThumbnail(item.id).then(
        (png) => (app.brushThumbnails[item.id] = URL.createObjectURL(new Blob([png], { type: "image/png" }))),
        () => undefined,
      );
    }
  }

  saveSettings(): void {
    void api.saveSettings("brush-folders", JSON.stringify($state.snapshot(this.settings))).catch(() => undefined);
  }

  saveMine(): void {
    void api.saveSettings("brush-presets", JSON.stringify($state.snapshot(this.saved))).catch((error: unknown) => app.fail(error));
  }

  /** Choisit un pinceau : forme, taille, dureté, opacité, dynamiques, pinceau double et texture. */
  apply(entry: BrushEntry): void {
    const settings = app.brushes[app.paintTool ?? "brush"];
    app.brushKind = $state.snapshot(entry.kind) as BrushKind;
    app.brushDynamics = entry.kind.type === "mixer" ? { ...MIXER_DYNAMICS, ...entry.dynamics } : { ...NEUTRAL_DYNAMICS, ...entry.dynamics };
    app.brushDual = entry.dual ? { ...entry.dual } : null;
    app.brushTexture = entry.texture ? { ...entry.texture } : null;
    // Ronds de base et pack fourni : seule la forme change, la taille reste celle choisie.
    if (entry.key.startsWith("pack:") || entry.key.startsWith("mine:")) {
      Object.assign(settings, { size: entry.size, opacity: entry.opacity, pressureSize: entry.pressureSize, pressureOpacity: entry.pressureOpacity });
    }
    settings.hardness = entry.hardness;
    // Espacement « automatique » (0) d'un pinceau : 1 % par défaut, pour un trait continu.
    if (app.brushDynamics.spacing <= 0) app.brushDynamics.spacing = NEUTRAL_DYNAMICS.spacing;
    app.brushKey = entry.key;
    this.noteRecent(entry.key);
  }

  get dynamicColor(): boolean {
    return this.settings.dynamicColor;
  }

  set dynamicColor(on: boolean) {
    this.settings.dynamicColor = on;
    this.saveSettings();
  }

  /** Pinceau tout juste choisi : en tête des récents (sans doublon), `RECENT_BRUSHES` au plus, gardés. */
  noteRecent(key: string): void {
    const next = [key, ...this.settings.recent.filter((k) => k !== key)].slice(0, RECENT_BRUSHES);
    if (next.join("|") === this.settings.recent.join("|")) return;
    this.settings.recent = next;
    this.saveSettings();
  }

  /** Pinceaux récemment utilisés encore présents dans la bibliothèque, du plus récent au plus ancien. */
  readonly recent: BrushEntry[] = $derived.by(() => {
    const byKey = new Map(this.all.map((e) => [e.key, e]));
    return this.settings.recent.map((k) => byKey.get(k)).filter((e): e is BrushEntry => !!e);
  });

  isUserFolder(id: string): boolean {
    return id.startsWith("user:");
  }

  createFolder(name: string): string {
    const id = `f${Date.now().toString(36)}`;
    this.settings.folders = [...this.settings.folders, { id, name: name.trim() || t("Nouveau dossier"), keys: [] }];
    this.saveSettings();
    return `user:${id}`;
  }

  renameFolder(id: string, name: string): void {
    const folder = this.settings.folders.find((f) => `user:${f.id}` === id);
    if (!folder || !name.trim()) return;
    folder.name = name.trim();
    this.saveSettings();
  }

  async deleteFolder(id: string): Promise<void> {
    if (this.isUserFolder(id)) {
      this.settings.folders = this.settings.folders.filter((f) => `user:${f.id}` !== id);
      this.saveSettings();
    } else if (id.startsWith("pack:")) {
      await api.removeBrushPack(id.slice("pack:".length));
      await this.refresh();
    } else if (id === "mine") {
      this.saved = [];
      this.saveMine();
    }
  }

  /** Ajoute un pinceau à un dossier personnel (sans doublon). */
  addTo(folderId: string, key: string): void {
    const folder = this.settings.folders.find((f) => `user:${f.id}` === folderId);
    if (!folder || folder.keys.includes(key)) return;
    folder.keys = [...folder.keys, key];
    this.saveSettings();
  }

  removeFrom(folderId: string, key: string): void {
    const folder = this.settings.folders.find((f) => `user:${f.id}` === folderId);
    if (!folder) return;
    folder.keys = folder.keys.filter((k) => k !== key);
    this.saveSettings();
  }

  /** Enregistre le pinceau courant (ou `entry`) dans « Mes pinceaux » sous `name`. */
  saveCurrent(name: string): void {
    const settings = app.brushes[app.paintTool ?? "brush"];
    const kind = app.brushKind;
    const tip = kind.type === "tip" ? app.brushTips.find((t) => t.id === kind.id) : undefined;
    const preset: SavedPreset = { name, kind: kind.type, tip: tip?.name, size: settings.size, hardness: settings.hardness, opacity: settings.opacity, dynamics: { ...$state.snapshot(app.brushDynamics) } };
    this.saved = [...this.saved.filter((p) => p.name !== name), preset];
    this.saveMine();
  }

  removeMine(name: string): void {
    this.saved = this.saved.filter((p) => p.name !== name);
    this.saveMine();
  }

  toggle(folderId: string): void {
    const collapsed = new Set(this.settings.collapsed);
    if (collapsed.has(folderId)) collapsed.delete(folderId);
    else collapsed.add(folderId);
    this.settings.collapsed = [...collapsed];
    this.saveSettings();
  }

  isCollapsed(folderId: string): boolean {
    return this.settings.collapsed.includes(folderId);
  }
}

export const brushLibrary = new BrushLibraryState();

// ------------------------------------------------------------------ aperçus de trait

const previews = new Map<string, Promise<string>>();
let queue: Promise<unknown> = Promise.resolve();
/** Aperçus gardés au plus (les moins récemment demandés sont oubliés, leur adresse d'image révoquée). */
const PREVIEW_CACHE_MAX = 100;

/** Paramètres de trait d'une entrée, pour l'aperçu. */
export function previewParams(entry: BrushEntry): BrushParams {
  return {
    size: entry.size,
    hardness: entry.hardness,
    opacity: 1,
    color: [255, 255, 255],
    erase: false,
    pressureSize: entry.pressureSize,
    pressureOpacity: entry.pressureOpacity,
    kind: entry.kind,
    dynamics: entry.dynamics,
    dual: entry.dual,
    texture: entry.texture,
  };
}

/**
 * Trait d'essai tracé par le moteur, en adresse d'image. Les aperçus sont calculés un par un (le moteur reste libre
 * pour la peinture) et gardés pour la séance (rouvrir le panneau est immédiat), dans la limite d'une centaine : au-delà,
 * les moins récemment demandés sont oubliés et leur adresse d'image révoquée.
 */
export function strokePreview(params: BrushParams, width: number, height: number, ramp = false): Promise<string> {
  const scale = Math.min(2, window.devicePixelRatio || 1);
  const [w, h] = [Math.round(width * scale), Math.round(height * scale)];
  // La taille affichée est ramenée à la hauteur de l'aperçu : un pinceau de 2600 px y tient aussi.
  const shown = { ...params, size: Math.max(2, Math.min(params.size, h * 0.55)) };
  const key = `${JSON.stringify(shown)}|${w}x${h}|${ramp}`;
  let found = previews.get(key);
  if (found) {
    // Réinséré en fin de table : l'ordre d'insertion sert d'ordre d'usage (le premier est le plus ancien).
    previews.delete(key);
    previews.set(key, found);
    return found;
  }
  found = queue.then(() => api.brushPreview(shown, w, h, ramp)).then((png) => URL.createObjectURL(new Blob([png], { type: "image/png" })));
  queue = found.catch(() => undefined);
  previews.set(key, found);
  found.catch(() => previews.delete(key));
  while (previews.size > PREVIEW_CACHE_MAX) {
    const oldest = previews.keys().next().value;
    if (oldest === undefined) break;
    const evicted = previews.get(oldest);
    previews.delete(oldest);
    // Révoquée une fois résolue : une adresse encore affichée est simplement redemandée au prochain besoin.
    void evicted?.then((url) => URL.revokeObjectURL(url), () => undefined);
  }
  return found;
}
