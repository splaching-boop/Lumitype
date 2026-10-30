// Texte enrichi léger, partagé par la notice et l'accueil : dans une phrase, `[Alt]` s'affiche comme une touche du
// clavier et `**gras**` en gras. Le reste est du texte simple.

/** Morceaux d'un texte : `[Touche]` → touche, `**gras**` → gras, le reste tel quel. */
export type Run = { kind: "text" | "key" | "bold"; text: string };

export function runs(text: string): Run[] {
  const out: Run[] = [];
  for (const part of text.split(/(\[[^\]]{1,12}\]|\*\*[^*]+\*\*)/)) {
    if (!part) continue;
    if (/^\[[^\]]+\]$/.test(part)) out.push({ kind: "key", text: part.slice(1, -1) });
    else if (/^\*\*[^*]+\*\*$/.test(part)) out.push({ kind: "bold", text: part.slice(2, -2) });
    else out.push({ kind: "text", text: part });
  }
  return out;
}

/** Le texte sans balisage, pour la recherche : `[Ctrl]` → `Ctrl`, `**gras**` → `gras`. */
export const plainText = (text: string): string => runs(text).map((run) => run.text).join("");
