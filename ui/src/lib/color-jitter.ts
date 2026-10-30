// Couleur dynamique du pinceau (comme « Aléatoire à chaque trait » de Clip Studio Paint) : chaque nouveau trait prend
// une nuance très proche de la couleur choisie, jamais tout à fait la même. Profil fixe : teinte ±2 % du cercle,
// saturation et luminosité ±4 % (relatifs). La couleur reste constante pendant le trait. Module pur : testé à part.

export type Rgb = [number, number, number];

export const COLOR_JITTER = { hue: 0.02, saturation: 0.04, lightness: 0.04 } as const;

function toHsl([r, g, b]: Rgb): [number, number, number] {
  const [rr, gg, bb] = [r / 255, g / 255, b / 255];
  const max = Math.max(rr, gg, bb);
  const min = Math.min(rr, gg, bb);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === rr ? (gg - bb) / d + (gg < bb ? 6 : 0) : max === gg ? (bb - rr) / d + 2 : (rr - gg) / d + 4;
  return [h / 6, s, l];
}

function toRgb([h, s, l]: [number, number, number]): Rgb {
  if (s === 0) {
    const v = Math.round(l * 255);
    return [v, v, v];
  }
  const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  const p = 2 * l - q;
  const channel = (t: number) => {
    const u = ((t % 1) + 1) % 1;
    const v = u < 1 / 6 ? p + (q - p) * 6 * u : u < 1 / 2 ? q : u < 2 / 3 ? p + (q - p) * (2 / 3 - u) * 6 : p;
    return Math.round(Math.min(1, Math.max(0, v)) * 255);
  };
  return [channel(h + 1 / 3), channel(h), channel(h - 1 / 3)];
}

/** Nuance d'un trait : `random` rend des nombres dans [0, 1) (Math.random par défaut). */
export function jitterColor(color: Rgb, random: () => number = Math.random): Rgb {
  const [h, s, l] = toHsl(color);
  const spread = (amount: number) => (random() * 2 - 1) * amount;
  const hue = h + spread(COLOR_JITTER.hue);
  const saturation = Math.min(1, Math.max(0, s * (1 + spread(COLOR_JITTER.saturation))));
  const lightness = Math.min(1, Math.max(0, l * (1 + spread(COLOR_JITTER.lightness))));
  return toRgb([hue, saturation, lightness]);
}
