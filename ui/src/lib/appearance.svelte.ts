// Apparence (Préférences › Apparence) : thème de couleurs et couleur d'accent, enregistrés dans les réglages et appliqués
// dès le démarrage. Le thème est l'attribut `data-theme` de <html> (variables de theme/tokens.css) ; l'accent est posé
// en ligne sur <html>, avec ses déclinaisons (plus vive, profonde, ombre) calculées ici.

import { api } from "./ipc";

export type ThemeId =
  | "legacy"
  | "oled"
  | "charcoal"
  | "gris"
  | "purple"
  | "burgundy"
  | "navy"
  | "pine"
  | "teal"
  | "brown"
  | "gold"
  | "mint"
  | "silver"
  | "lime"
  | "peach"
  | "beige"
  | "blanc";

/** Thèmes clairs : encre sombre, texte posé sur l'accent calculé en conséquence (voir tokens.css). */
export const LIGHT_THEMES: readonly ThemeId[] = ["mint", "silver", "lime", "peach", "beige", "blanc"];

/** Anciens identifiants de thème (réglage enregistré par une version précédente) → thème actuel le plus proche. */
const RENAMED_THEMES: Record<string, ThemeId> = { vanille: "beige" };

export const THEMES: { id: ThemeId; name: string; colors: { app: string; bar: string; panel: string; canvas: string; muted: string; border: string } }[] = [
  { id: "oled", name: "OLED", colors: { app: "#000000", bar: "#000000", panel: "#050505", canvas: "#0a0a0a", muted: "#7c7c82", border: "rgba(255,255,255,.1)" } },
  { id: "legacy", name: "Legacy", colors: { app: "#121111", bar: "#171616", panel: "#1a1919", canvas: "#1d1d1e", muted: "#8b8b93", border: "rgba(255,255,255,.08)" } },
  { id: "charcoal", name: "Charcoal", colors: { app: "#232528", bar: "#1c1e21", panel: "#2a2d31", canvas: "#26282b", muted: "#9196a0", border: "rgba(255,255,255,.1)" } },
  { id: "gris", name: "Gris", colors: { app: "#323232", bar: "#2a2a2a", panel: "#3a3a3a", canvas: "#262626", muted: "#9a9a9a", border: "rgba(255,255,255,.1)" } },
  { id: "purple", name: "Purple", colors: { app: "#1a1526", bar: "#150f1f", panel: "#221b31", canvas: "#1f1a2a", muted: "#9088a4", border: "rgba(220,200,255,.1)" } },
  { id: "burgundy", name: "Burgundy", colors: { app: "#231316", bar: "#1b0e11", panel: "#2b181c", canvas: "#221a1c", muted: "#9a868c", border: "rgba(255,220,225,.1)" } },
  { id: "navy", name: "Navy Blue", colors: { app: "#121a2b", bar: "#0d1322", panel: "#182236", canvas: "#161e2f", muted: "#8690a5", border: "rgba(200,215,255,.1)" } },
  { id: "pine", name: "Pine", colors: { app: "#121c17", bar: "#0d1511", panel: "#18241e", canvas: "#15201a", muted: "#86998d", border: "rgba(200,255,220,.1)" } },
  { id: "teal", name: "Teal", colors: { app: "#0f1f21", bar: "#0b181a", panel: "#152a2d", canvas: "#122426", muted: "#829a9d", border: "rgba(190,240,245,.1)" } },
  { id: "brown", name: "Brown", colors: { app: "#241c16", bar: "#1c1510", panel: "#2d241d", canvas: "#272019", muted: "#9a8c7d", border: "rgba(255,230,200,.1)" } },
  { id: "gold", name: "Gold", colors: { app: "#1f1a10", bar: "#17130b", panel: "#282215", canvas: "#221d12", muted: "#a09377", border: "rgba(255,225,150,.12)" } },
  { id: "mint", name: "Mint", colors: { app: "#dcebe3", bar: "#cfe2d8", panel: "#eaf4ee", canvas: "#cadfd3", muted: "#75877c", border: "rgba(20,70,45,.18)" } },
  { id: "silver", name: "Silver", colors: { app: "#dfe1e5", bar: "#d3d6db", panel: "#eceef1", canvas: "#cfd2d8", muted: "#777c86", border: "rgba(30,35,45,.18)" } },
  { id: "lime", name: "Lime", colors: { app: "#e4ecd2", bar: "#d8e2c2", panel: "#f0f5e4", canvas: "#d3dec0", muted: "#7a866a", border: "rgba(50,70,10,.18)" } },
  { id: "peach", name: "Peach", colors: { app: "#f3dfd2", bar: "#ebd1c0", panel: "#faede5", canvas: "#e8cfbf", muted: "#88766b", border: "rgba(90,40,20,.18)" } },
  { id: "beige", name: "Beige", colors: { app: "#e6dccb", bar: "#ddd2bf", panel: "#f1ebe0", canvas: "#d6ccb9", muted: "#85796b", border: "rgba(60,45,20,.18)" } },
  { id: "blanc", name: "Blanc", colors: { app: "#f4f4f6", bar: "#ffffff", panel: "#ffffff", canvas: "#e8e8ec", muted: "#85858c", border: "rgba(0,0,0,.1)" } },
];

/** Couleurs d'accent proposées ; le rouge de Lumitype par défaut. */
export const ACCENTS: { name: string; color: string }[] = [
  { name: "Rouge", color: "#ec2a3c" },
  { name: "Orange", color: "#f2711c" },
  { name: "Ambre", color: "#e3a008" },
  { name: "Vert", color: "#2fa84f" },
  { name: "Turquoise", color: "#13a39b" },
  { name: "Bleu", color: "#2f6bff" },
  { name: "Violet", color: "#8b5cf6" },
  { name: "Rose", color: "#e0407a" },
];

export const DEFAULT_APPEARANCE: { theme: ThemeId; accent: string } = { theme: "legacy", accent: "#ec2a3c" };
const DEFAULT = DEFAULT_APPEARANCE;
const SETTINGS = "appearance";
const CACHE = "lumitype-appearance";

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = Number.parseInt(m[1]!, 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

const toHex = (rgb: number[]) => `#${rgb.map((c) => Math.round(Math.max(0, Math.min(255, c))).toString(16).padStart(2, "0")).join("")}`;
const mix = (a: number[], b: number[], t: number) => a.map((v, i) => v + (b[i]! - v) * t);

/** Luminance relative (WCAG) : décide si le texte posé sur l'accent est blanc ou sombre. */
function luminance([r, g, b]: number[]): number {
  const lin = (c: number) => ((c /= 255) <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r!) + 0.7152 * lin(g!) + 0.0722 * lin(b!);
}

class AppearanceState {
  theme = $state<ThemeId>(DEFAULT.theme);
  accent = $state(DEFAULT.accent);

  /** Applique ce qui est connu tout de suite (copie locale), puis les réglages enregistrés. */
  async init(): Promise<void> {
    try {
      const cached = JSON.parse(localStorage.getItem(CACHE) ?? "null") as { theme?: string; accent?: string } | null;
      if (cached) this.set(cached.theme ?? DEFAULT.theme, cached.accent ?? DEFAULT.accent, false);
    } catch {
      // stockage local indisponible : les réglages suffisent
    }
    try {
      const saved = await api.loadSettings(SETTINGS);
      const parsed = saved ? (JSON.parse(saved) as { theme?: string; accent?: string }) : null;
      if (parsed) this.set(parsed.theme ?? DEFAULT.theme, parsed.accent ?? DEFAULT.accent, false);
      // Thème renommé : le réglage est réécrit sous son nouveau nom, pour ne plus avoir à le traduire.
      if (parsed?.theme && parsed.theme in RENAMED_THEMES) this.set(this.theme, this.accent);
    } catch {
      // réglage illisible : apparence par défaut
    }
  }

  set(theme: ThemeId | string, accent: string, persist = true): void {
    // Un thème disparu (« vanille ») prend le nom de son remplaçant ; un identifiant inconnu, le thème par défaut.
    const renamed = RENAMED_THEMES[theme] ?? theme;
    this.theme = THEMES.some((t) => t.id === renamed) ? (renamed as ThemeId) : DEFAULT.theme;
    this.accent = parseHex(accent) ? accent.toLowerCase() : DEFAULT.accent;
    this.apply();
    if (!persist) return;
    const value = JSON.stringify({ theme: this.theme, accent: this.accent });
    void api.saveSettings(SETTINGS, value).catch(() => undefined);
    try {
      localStorage.setItem(CACHE, value);
    } catch {
      // sans stockage local, l'apparence s'applique un instant plus tard au démarrage
    }
  }

  private apply(): void {
    const root = document.documentElement;
    if (this.theme === "legacy") delete root.dataset.theme;
    else root.dataset.theme = this.theme;
    const rgb = parseHex(this.accent) ?? parseHex(DEFAULT.accent)!;
    const light = LIGHT_THEMES.includes(this.theme);
    const background = parseHex(THEMES.find((t) => t.id === this.theme)!.colors.panel)!;
    const vars: Record<string, string> = {
      "--accent": toHex(rgb),
      "--accent-strong": toHex(mix(rgb, light ? [0, 0, 0] : [255, 255, 255], 0.1)),
      "--accent-deep": toHex(mix(rgb, light ? [255, 255, 255] : [0, 0, 0], 0.42)),
      "--accent-shadow": toHex(mix(background, rgb, light ? 0.16 : 0.2)),
      "--pill-bg": toHex(rgb),
      "--on-accent": luminance(rgb) > 0.45 ? "#14110e" : "#ffffff",
      "--pill-text": luminance(rgb) > 0.45 ? "#14110e" : "#ffffff",
    };
    for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value);
  }
}

export const appearance = new AppearanceState();
