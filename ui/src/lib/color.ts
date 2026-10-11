import type { Rgb } from "./ipc";

/** Teinte en degrés `0..360`, saturation et luminosité `0..1` (modèle TSL/HSB de Photoshop). */
export interface Hsv {
  h: number;
  s: number;
  v: number;
}

export function hsvToRgb({ h, s, v }: Hsv): Rgb {
  const f = (n: number) => {
    const k = (n + h / 60) % 6;
    return Math.round(255 * (v - v * s * Math.max(0, Math.min(k, 4 - k, 1))));
  };
  return [f(5), f(3), f(1)];
}

/** `previousHue` est conservée quand la couleur est grise (teinte indéfinie). */
export function rgbToHsv([r, g, b]: Rgb, previousHue = 0): Hsv {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const delta = max - min;
  let h = previousHue;
  if (delta > 0) {
    if (max === r) h = 60 * (((g - b) / delta) % 6);
    else if (max === g) h = 60 * ((b - r) / delta + 2);
    else h = 60 * ((r - g) / delta + 4);
    if (h < 0) h += 360;
  }
  return { h, s: max === 0 ? 0 : delta / max, v: max / 255 };
}

export const rgbToHex = (rgb: Rgb) => `#${rgb.map((c) => c.toString(16).padStart(2, "0")).join("")}`;

export function hexToRgb(text: string): Rgb | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(text.trim());
  if (!match?.[1]) return null;
  const value = Number.parseInt(match[1], 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}
