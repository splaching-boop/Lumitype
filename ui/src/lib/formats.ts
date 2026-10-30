// Formats de document : la liste unique, partagée par l'accueil (formats rapides, page Formats) et la fenêtre
// Nouveau document. Les libellés et les notes sont écrits en français et passent par `t(...)` à l'affichage,
// comme toutes les chaînes de l'interface.
//
// Les tailles des réseaux sociaux sont celles recommandées par chaque plateforme (relevé d'octobre 2026).
// « HD » double la définition quand la plateforme l'accepte : plus de netteté à l'export, même proportion
// (à peu de chose près pour LinkedIn, qui plafonne certains formats).
//
// Tous les formats du catalogue sont à fond blanc (le choix « Transparent » reste dans la fenêtre Nouveau document).
//
// Les catégories personnelles de l'utilisateur (lib/custom-formats.ts) viennent après les plateformes, d'un filet :
// `formatCategories` est la liste complète et réactive, à lire avec `$formatCategories` dans un composant.
import { derived, get, writable } from "svelte/store";

export interface Size {
  width: number;
  height: number;
}

export interface Format extends Size {
  /** Identifiant stable : il est gardé dans l'historique des formats utilisés. */
  id: string;
  label: string;
  dpi: number;
  /** Taille « HD », quand la plateforme l'accepte. */
  hd?: Size;
  /** Précision affichée après la taille (« 4:5 · recommandé »). */
  note?: string;
  /** Fond transparent : seuls les formats personnels peuvent l'être (les autres sont à fond blanc). */
  transparent?: boolean;
  /** Format personnel : son nom est celui que l'utilisateur a tapé, il ne se traduit pas. */
  custom?: boolean;
}

export interface FormatCategory {
  id: string;
  label: string;
  formats: Format[];
}

/** Formats génériques, plateformes, puis catégories personnelles : chaque groupe est séparé d'un filet. */
export type CategoryGroup = "base" | "platform" | "custom";
export interface CatalogCategory extends FormatCategory {
  group: CategoryGroup;
}

const screen = (id: string, label: string, width: number, height: number, note?: string): Format => ({ id, label, width, height, dpi: 72, note });
const paper = (id: string, label: string, width: number, height: number): Format => ({ id, label, width, height, dpi: 300 });
const social = (id: string, label: string, width: number, height: number, hd: Size | null, note?: string): Format => ({
  id,
  label,
  width,
  height,
  dpi: 72,
  note,
  ...(hd ? { hd } : {}),
});
const hd = (width: number, height: number): Size => ({ width, height });

/** Papier ISO 216 à 300 ppp : la catégorie Impression. */
const PRINT: Format[] = [paper("a4", "A4 · 300 ppp", 2480, 3508), paper("a3", "A3 · 300 ppp", 3508, 4961), paper("a5", "A5 · 300 ppp", 1748, 2480)];

/** Les formats d'écran de toujours (et les cinq premiers sont les formats rapides par défaut, avec A4). */
const ESSENTIALS: Format[] = [
  screen("ecran-hd", "Écran HD", 1920, 1080),
  screen("4k", "4K", 3840, 2160),
  screen("carre", "Carré", 1080, 1080),
  screen("story", "Story", 1080, 1920),
  screen("hd", "HD", 1280, 720),
  screen("qhd", "QHD", 2560, 1440),
  screen("ultra-large", "Ultra large", 3440, 1440),
  screen("publication-4-5", "Publication 4:5", 1080, 1350),
  screen("banniere", "Bannière", 1500, 500),
  screen("miniature-video", "Miniature vidéo", 1280, 720),
];

const INSTAGRAM: Format[] = [
  social("instagram-portrait", "Publication portrait", 1080, 1350, hd(2160, 2700), "4:5 · recommandé"),
  social("instagram-3-4", "Publication 3:4", 1080, 1440, hd(2160, 2880), "grille du profil"),
  social("instagram-carre", "Publication carrée", 1080, 1080, hd(2160, 2160), "1:1"),
  social("instagram-paysage", "Publication paysage", 1080, 566, hd(2160, 1132), "1,91:1"),
  social("instagram-carrousel", "Carrousel", 1080, 1350, hd(2160, 2700), "4:5 · jusqu'à 20 images"),
  social("instagram-story", "Story", 1080, 1920, hd(2160, 3840), "9:16 · marges 250 px"),
  social("instagram-reel", "Couverture de Reel", 1080, 1920, null, "aperçu grille 3:4"),
  social("instagram-profil", "Photo de profil", 1080, 1080, null, "affichée en cercle"),
];

const LINKEDIN: Format[] = [
  social("linkedin-paysage", "Post paysage", 1200, 627, hd(2400, 1254), "1,91:1"),
  social("linkedin-carre", "Post carré", 1200, 1200, hd(2160, 2160), "1:1"),
  social("linkedin-portrait", "Post portrait", 1200, 1500, hd(2160, 2700), "4:5"),
  social("linkedin-carrousel", "Carrousel (PDF)", 1080, 1350, hd(2160, 2700), "4:5 · exporter en PDF"),
  social("linkedin-banniere", "Bannière de profil", 1584, 396, hd(3168, 792), "4:1"),
  social("linkedin-entreprise", "Couverture entreprise", 1512, 256, null, "≈ 5,9:1"),
  social("linkedin-article", "Couverture d'article", 1200, 627, hd(1920, 1080), "1,91:1"),
  social("linkedin-profil", "Photo de profil", 400, 400, hd(800, 800), "cercle"),
];

const YOUTUBE: Format[] = [
  social("youtube-miniature", "Miniature", 1280, 720, hd(3840, 2160), "16:9"),
  social("youtube-banniere", "Bannière de chaîne", 2560, 1440, null, "zone sûre 1546 × 423"),
  social("youtube-shorts", "Shorts", 1080, 1920, hd(2160, 3840), "9:16"),
  social("youtube-communaute", "Post communauté", 1080, 1080, null, "1:1"),
  social("youtube-profil", "Photo de profil", 800, 800, null, "cercle"),
];

const TIKTOK: Format[] = [
  social("tiktok-video", "Vidéo / couverture", 1080, 1920, hd(2160, 3840), "9:16"),
  social("tiktok-carrousel", "Carrousel photo", 1080, 1920, null, "9:16 · jusqu'à 35 photos"),
  social("tiktok-profil", "Photo de profil", 1080, 1080, null, "cercle"),
];

const X: Format[] = [
  social("x-paysage", "Post paysage", 1200, 675, hd(1920, 1080), "16:9"),
  social("x-carre", "Post carré", 1200, 1200, hd(2048, 2048), "1:1"),
  social("x-portrait", "Post portrait", 1080, 1350, hd(1440, 1800), "4:5"),
  social("x-lien", "Image de lien", 1200, 628, null, "1,91:1 · aperçu des liens"),
  social("x-entete", "En-tête de profil", 1500, 500, hd(3000, 1000), "3:1 · avatar en bas à gauche"),
  social("x-profil", "Photo de profil", 400, 400, hd(800, 800), "cercle"),
];

const REDDIT: Format[] = [
  social("reddit-post", "Post paysage", 1200, 960, null, "5:4 · ordinateur"),
  social("reddit-post-mobile", "Post carré", 1080, 1080, null, "1:1 · application"),
  social("reddit-post-portrait", "Post portrait", 1080, 1350, null, "4:5 · application"),
  social("reddit-banniere", "Bannière de communauté", 1920, 384, null, "5:1"),
  social("reddit-icone", "Icône de communauté", 256, 256, null, "cercle"),
  social("reddit-avatar", "Avatar de profil", 256, 256, null, "cercle"),
  social("reddit-banniere-profil", "Bannière de profil", 1000, 300, null, "10:3"),
];

const DISCORD: Format[] = [
  social("discord-avatar", "Avatar de profil", 512, 512, hd(1024, 1024), "cercle"),
  social("discord-banniere-profil", "Bannière de profil", 600, 240, null, "5:2 · Nitro"),
  social("discord-icone", "Icône de serveur", 512, 512, hd(1024, 1024), "cercle"),
  social("discord-banniere", "Bannière de serveur", 960, 540, hd(1920, 1080), "16:9 · boosts"),
  social("discord-invitation", "Image d'invitation", 1920, 1080, null, "16:9 · boosts"),
  social("discord-emoji", "Emoji", 128, 128, null, "256 Ko max"),
  social("discord-sticker", "Sticker", 320, 320, null, "512 Ko max"),
  social("discord-lien", "Image de lien", 1200, 630, null, "1,91:1 · aperçu des liens"),
];

/** Twitch : tailles du tableau de bord créateur (relevé d'octobre 2026, guides vérifiés sur l'aide Twitch). */
const TWITCH: Format[] = [
  social("twitch-profil", "Photo de profil", 256, 256, hd(800, 800), "cercle · 10 Mo"),
  social("twitch-banniere", "Bannière de profil", 1200, 480, null, "5:2 · contenu à gauche"),
  social("twitch-hors-ligne", "Écran hors ligne", 1920, 1080, null, "16:9 · 10 Mo"),
  social("twitch-panneau", "Panneau d'infos", 320, 160, null, "320 px de large · hauteur libre"),
  social("twitch-emote", "Emote", 112, 112, hd(4096, 4096), "PNG transparent · 1 Mo"),
  social("twitch-badge", "Badge d'abonné", 72, 72, null, "réduit à 36 et 18 px · 25 Ko"),
  social("twitch-points", "Icône de points de chaîne", 112, 112, null, "réduite à 56 et 28 px"),
  social("twitch-miniature", "Miniature de vidéo", 1280, 720, hd(1920, 1080), "16:9"),
  social("twitch-overlay", "Overlay", 1920, 1080, hd(3840, 2160), "16:9 · PNG transparent"),
];

/** Kick : tailles relevées en octobre 2026 (guides spécialisés ; Kick ne publie pas toutes ses tailles). */
const KICK: Format[] = [
  social("kick-profil", "Photo de profil", 800, 800, null, "cercle · 4 Mo"),
  social("kick-banniere", "Bannière de chaîne", 1280, 700, hd(2560, 1400), "≈ 1,83:1 · 4 Mo"),
  social("kick-hors-ligne", "Écran hors ligne", 1920, 1080, null, "16:9 · 4 Mo"),
  social("kick-emote", "Emote", 500, 500, null, "PNG ou GIF · 1 Mo"),
  social("kick-badge", "Badge d'abonné", 72, 72, null, "PNG transparent"),
  social("kick-panneau", "Panneau « À propos »", 320, 160, null, "320 px de large · hauteur libre"),
  social("kick-overlay", "Overlay", 1920, 1080, hd(3840, 2160), "16:9"),
];

/** Signature e-mail : 600 px de large au plus à l'affichage ; « HD » = le double, pour les écrans Retina. */
const EMAIL: Format[] = [
  social("signature-banniere", "Bannière de signature", 600, 200, hd(1200, 400), "affichée à 600 px de large"),
  social("signature-logo", "Logo de signature", 300, 100, hd(600, 200), "affiché à 300 px de large"),
];

/** Cartes de visite à 300 ppp : format fini, et avec fond perdu (à rogner) pour l'imprimeur. */
const CARDS: Format[] = [
  paper("carte-europe", "Carte européenne · 85 × 55 mm", 1004, 650),
  paper("carte-europe-fp", "Carte européenne + fond perdu 3 mm", 1075, 720),
  paper("carte-usa", "Carte américaine · 3,5 × 2 po", 1050, 600),
  paper("carte-usa-fp", "Carte américaine + fond perdu ⅛ po", 1125, 675),
  paper("carte-japon", "Carte japonaise · 91 × 55 mm", 1075, 650),
  paper("carte-japon-fp", "Carte japonaise + fond perdu 3 mm", 1146, 720),
];

/**
 * Catégories, dans l'ordre de la colonne de gauche (après « Dernier utilisé », qui est l'historique).
 * `group` sépare les formats génériques des plateformes, d'un filet.
 */
export const FORMAT_CATEGORIES: (FormatCategory & { group: "base" | "platform" })[] = [
  { id: "essentiels", label: "Essentiels", formats: ESSENTIALS, group: "base" },
  { id: "impression", label: "Impression", formats: PRINT, group: "base" },
  { id: "cartes", label: "Cartes de visite", formats: CARDS, group: "base" },
  { id: "instagram", label: "Instagram", formats: INSTAGRAM, group: "platform" },
  { id: "linkedin", label: "LinkedIn", formats: LINKEDIN, group: "platform" },
  { id: "youtube", label: "YouTube", formats: YOUTUBE, group: "platform" },
  { id: "tiktok", label: "TikTok", formats: TIKTOK, group: "platform" },
  { id: "x", label: "X", formats: X, group: "platform" },
  { id: "reddit", label: "Reddit", formats: REDDIT, group: "platform" },
  { id: "discord", label: "Discord", formats: DISCORD, group: "platform" },
  { id: "twitch", label: "Twitch", formats: TWITCH, group: "platform" },
  { id: "kick", label: "Kick", formats: KICK, group: "platform" },
  { id: "signature", label: "Signature e-mail", formats: EMAIL, group: "platform" },
];

/** Tous les formats du catalogue livré, toutes catégories confondues (sans les formats personnels). */
export const ALL_FORMATS: Format[] = FORMAT_CATEGORIES.flatMap((c) => c.formats);

/**
 * Catégories personnelles, déjà validées (`group: "custom"`, formats marqués `custom`). Seul lib/custom-formats.ts
 * les écrit : il les lit dans les réglages, les modifie et les enregistre.
 */
export const customCategories = writable<CatalogCategory[]>([]);

/** Toutes les catégories, dans l'ordre de la colonne : le catalogue, puis les catégories personnelles. */
export const formatCategories = derived(customCategories, (custom): CatalogCategory[] => [...FORMAT_CATEGORIES, ...custom]);

/** Un format du catalogue ou un format personnel, par son identifiant. */
export const formatById = (id: string): Format | undefined =>
  ALL_FORMATS.find((f) => f.id === id) ?? get(customCategories).flatMap((c) => c.formats).find((f) => f.id === id);

/** Libellé affiché : traduit pour le catalogue, tel quel pour ce que l'utilisateur a nommé lui-même. */
export const labelOf = (item: { label: string; custom?: boolean; group?: CategoryGroup }, translate: (text: string) => string): string =>
  item.custom || item.group === "custom" ? item.label : translate(item.label);

/** Formats rapides de l'accueil sans historique : l'ordre de toujours (Personnalisé… vient en sixième case). */
export const DEFAULT_QUICK_IDS = ["ecran-hd", "4k", "a4", "carre", "story"];

/** Libellé de la catégorie d'un format (« Instagram »), pour distinguer deux « Story » ou deux « Photo de profil ». */
export const categoryOf = (id: string): CatalogCategory | undefined => get(formatCategories).find((c) => c.formats.some((f) => f.id === id));

/**
 * Recherche par nom, note, catégorie ou taille : « story », « linkedin », « 1080 », « 1080x1350 »,
 * « 1080 × 1350 ». `label` traduit un libellé (la recherche se fait aussi dans la langue affichée).
 */
export function matchesFormat(format: Format, category: FormatCategory, query: string, label: (text: string) => string = (s) => s): boolean {
  const q = query.trim().toLowerCase().replace(/\s*[x×*]\s*/g, "×");
  if (!q) return true;
  const sizes = [`${format.width}×${format.height}`, ...(format.hd ? [`${format.hd.width}×${format.hd.height}`] : [])];
  const words = [format.label, label(format.label), format.note ?? "", format.note ? label(format.note) : "", category.label, label(category.label), ...sizes];
  return words.some((word) => word.toLowerCase().includes(q));
}

/** Les séries de tailles de la fenêtre Nouveau document (écrans par proportion, papier ISO), avec leur orientation. */
export interface SeriesSize {
  name: string;
  /** Grand et petit côté en pixels ; l'orientation décide lequel est la largeur. */
  long: number;
  short: number;
  detail?: string;
}
export interface Series {
  id: string;
  label: string;
  title: string;
  dpi: number;
  portrait: boolean;
  /** Proportions de la petite icône de la liste. */
  icon: [number, number];
  sizes: SeriesSize[];
}
const ss = (name: string, long: number, short: number, detail?: string): SeriesSize => ({ name, long, short, ...(detail ? { detail } : {}) });
export const SIZE_SERIES: Series[] = [
  { id: "16:9", label: "16:9", title: "Écran 16:9", dpi: 72, portrait: false, icon: [24, 14], sizes: [ss("SD", 854, 480), ss("HD", 1280, 720), ss("Full HD", 1920, 1080), ss("QHD", 2560, 1440), ss("UHD 4K", 3840, 2160)] },
  { id: "16:10", label: "16:10", title: "Écran 16:10", dpi: 72, portrait: false, icon: [22, 14], sizes: [ss("WXGA", 1280, 800), ss("WXGA+", 1440, 900), ss("WSXGA+", 1680, 1050), ss("WUXGA", 1920, 1200), ss("WQXGA", 2560, 1600)] },
  { id: "21:9", label: "21:9", title: "Écran 21:9", dpi: 72, portrait: false, icon: [26, 11], sizes: [ss("UW Full HD", 2560, 1080), ss("UW QHD", 3440, 1440), ss("UW 5K", 5120, 2160)] },
  // Formats ISO 216 à 300 ppp.
  {
    id: "papier",
    label: "Papier",
    title: "Papier · 300 ppp",
    dpi: 300,
    portrait: true,
    icon: [13, 18],
    sizes: [ss("A2", 7016, 4961, "420 × 594 mm"), ss("A3", 4961, 3508, "297 × 420 mm"), ss("A4", 3508, 2480, "210 × 297 mm"), ss("A5", 2480, 1748, "148 × 210 mm"), ss("A6", 1748, 1240, "105 × 148 mm")],
  },
];

/** Petit rectangle aux proportions du format, côté long `side` px. */
export const shapeOf = (width: number, height: number, side = 26): { width: string; height: string } => ({
  width: `${Math.max(3, Math.round((side * width) / Math.max(width, height)))}px`,
  height: `${Math.max(3, Math.round((side * height) / Math.max(width, height)))}px`,
});
