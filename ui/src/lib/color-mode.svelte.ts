// Image › Mode : mode colorimétrique du document actif (voir crates/engine/src/color_mode.rs).
//
// Le moteur garde ses pixels en RVB ; le mode est une règle tenue à la conversion, au rendu final (gris, seuil du
// bitmap) et à l'export. Ici, la part de l'interface :
//  - les fenêtres Bitmap et CMJN (`modeDialogs`), et la conversion elle-même (`convertColorMode`) ;
//  - les couleurs choisies : en gris, premier plan et arrière-plan sont EMPLOYÉS ramenés au gris de même luminance ; en
//    bitmap, au noir ou au blanc (`foregroundInMode`, `backgroundInMode` : outils, remplissages, formes, dégradés,
//    pastilles du sélecteur). Les couleurs choisies, elles, restent intactes : revenir sur un document RVB les retrouve
//    telles quelles (auparavant, un simple passage sur un document gris les grisait pour de bon) ;
//  - le libellé de l'onglet (« Gris/8 », « Bitmap », « CMJN/8 ») ;
//  - l'épreuve écran : en mode CMJN, elle suit le profil du document et ne s'éteint pas.
import { app } from "../state/app.svelte";
import { edit } from "./engine";
import { t } from "./i18n.svelte";
import type { BitmapOptions, CmykSpace, ColorMode, DocumentInfo, Rgb } from "./ipc";
import { colorSettings } from "./color-settings.svelte";

/** Fenêtres ouvertes du sous-menu Mode. */
export const modeDialogs = $state({ bitmap: false, cmyk: false });

const RGB: ColorMode = { kind: "rgb" };

/** Mode d'un document (RVB pour une description sans mode). */
export function colorModeOf(info: DocumentInfo | null | undefined): ColorMode {
  return info?.colorMode ?? RGB;
}

/** Mode du document actif. */
export function activeColorMode(): ColorMode {
  return colorModeOf(app.active?.info);
}

/** Espace CMJN du document s'il est en mode Couleurs CMJN (celui de sa conversion). */
export function workflowSpace(info: DocumentInfo | null | undefined = app.active?.info): CmykSpace | null {
  const mode = colorModeOf(info);
  return mode.kind === "cmykWorkflow" ? { profile: mode.profile, intent: mode.intent ?? "relative", bpc: mode.bpc ?? true } : null;
}

/**
 * Épreuve écran à montrer : celle du document en mode CMJN (toujours allumée), sinon, Épreuve CMJN allumée, celle des
 * Paramètres de couleur.
 */
export function effectiveProof(info: DocumentInfo | null | undefined = app.active?.info): CmykSpace | null {
  return workflowSpace(info) ?? (app.proofCmyk ? colorSettings.cmyk : null);
}

/** Espace de l'export CMJN : celui du document en mode CMJN, sinon celui des Paramètres de couleur. */
export function exportSpace(info: DocumentInfo | null | undefined = app.active?.info): CmykSpace {
  return workflowSpace(info) ?? colorSettings.cmyk;
}

/** Libellé du mode dans l'onglet du document, comme Photoshop : « RVB/8 », « Gris/8 », « Bitmap », « CMJN/8 ». */
export function modeTabLabel(info: DocumentInfo | null | undefined): string {
  const mode = colorModeOf(info);
  switch (mode.kind) {
    case "grayscale":
      return `${t("Gris")}/8`;
    case "bitmap":
      return t("Bitmap");
    case "cmykWorkflow":
      return `${t("CMJN")}/8`;
    default:
      // Épreuve d'Affichage › Épreuve sur un document RVB : l'onglet le dit aussi, comme avant.
      return app.proofCmyk ? `${t("CMJN")}/8` : `${t("RVB")}/8`;
  }
}

/** Luminance Rec. 601 en entiers exacts, celle du moteur (`color_mode::luma`). */
export function luma([r, g, b]: Rgb): number {
  return (77 * r + 150 * g + 29 * b + 128) >> 8;
}

/** Couleur ramenée à ce que le mode peut montrer : gris de même luminance, ou noir / blanc ; inchangée sinon. */
export function coerceColor(color: Rgb, mode: ColorMode): Rgb {
  if (mode.kind === "grayscale") {
    const y = luma(color);
    return [y, y, y];
  }
  if (mode.kind === "bitmap") return luma(color) >= 128 ? [255, 255, 255] : [0, 0, 0];
  return color;
}

/**
 * Couleur de premier plan à EMPLOYER sur le document actif (outil, remplissage, forme, pastille affichée) : ramenée à
 * ce que son mode peut montrer. La couleur choisie (`app.foreground`) n'est pas modifiée. Réactive (lit `app`).
 */
export function foregroundInMode(): Rgb {
  return coerceColor(app.foreground, activeColorMode());
}

/** Couleur d'arrière-plan à employer sur le document actif (voir `foregroundInMode`). */
export function backgroundInMode(): Rgb {
  return coerceColor(app.background, activeColorMode());
}

/** Réglages de la fenêtre Bitmap gardés d'une ouverture à l'autre (comme Photoshop). */
export const bitmapDefaults = $state<{ method: "threshold" | "diffusion" | "halftone"; frequency: number; angle: number; outputDpi: number | null }>({
  method: "diffusion",
  frequency: 53,
  angle: 45,
  outputDpi: null,
});

export function bitmapOptions(dpi: number): BitmapOptions {
  const d = bitmapDefaults;
  const method = d.method === "halftone" ? { kind: "halftone" as const, frequency: d.frequency, angle: d.angle, shape: "round" as const } : { kind: d.method };
  return { method, outputDpi: d.outputDpi ?? dpi };
}

/** Image › Mode › … : une conversion, un seul état d'historique (Ctrl+Z rend le mode et les pixels d'avant). */
export async function convertColorMode(mode: ColorMode, bitmap?: BitmapOptions): Promise<boolean> {
  return edit(bitmap ? { op: "setColorMode", mode, bitmap } : { op: "setColorMode", mode });
}

/**
 * Entrée du sous-menu : RVB et Niveaux de gris convertissent tout de suite, Bitmap et CMJN ouvrent leur fenêtre.
 * Depuis un document en couleur, le bitmap passe d'abord par le gris, comme dans Photoshop — ici, la fenêtre Bitmap
 * s'en charge (l'image est aplatie et convertie d'un seul tenant).
 */
export function chooseColorMode(kind: ColorMode["kind"]): void {
  const current = activeColorMode();
  if (current.kind === kind && kind !== "cmykWorkflow") return; // déjà dans ce mode (l'espace CMJN, lui, se change)
  if (kind === "bitmap") {
    modeDialogs.bitmap = true;
    return;
  }
  if (kind === "cmykWorkflow") {
    modeDialogs.cmyk = true;
    return;
  }
  void convertColorMode(kind === "grayscale" ? { kind: "grayscale" } : RGB);
}

/** Le document doit-il être aplati avant le bitmap ? (plusieurs calques, ou un calque qui n'est pas de pixels) */
export function needsFlatten(info: DocumentInfo | null | undefined): boolean {
  const layers = info?.layers ?? [];
  return layers.length > 1 || layers.some((layer) => layer.isGroup || layer.adjustment || layer.text || layer.shape || layer.fillLayer || layer.smart);
}
