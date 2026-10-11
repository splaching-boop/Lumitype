// Aide des outils : une phrase et une animation par outil, affichées après deux secondes de survol.
// L'animation vient de `assets/media/tools/<outil>.gif` dès que ce fichier existe (voir `media.ts`).
import type { ToolId } from "../state/app.svelte";

export interface ToolHelp {
  /** Nom de l'outil, sans le raccourci. */
  name: string;
  /** Raccourci clavier, affiché dans la pastille. */
  keys: string;
  /** Une phrase : la bulle réserve trois lignes, au-delà le texte serait coupé. */
  hint: string;
}

export const TOOL_HELP: Record<ToolId, ToolHelp> = {
  move: { name: "Déplacement", keys: "V", hint: "Déplace le calque ou la sélection, à la souris ou aux flèches du clavier." },
  marquee: { name: "Sélection", keys: "M", hint: "Sélectionne une zone rectangulaire, ou elliptique avec Maj+M." },
  lasso: { name: "Lasso", keys: "L", hint: "Trace une sélection à main levée, ou point par point avec Maj+L." },
  wand: { name: "Baguette magique", keys: "W", hint: "Sélectionne d'un clic les zones voisines de teinte proche." },
  quickSelect: { name: "Sélection rapide", keys: "Maj+W", hint: "Étend la sélection au passage du pinceau, en suivant les contours." },
  crop: { name: "Recadrage", keys: "C", hint: "Recadre l'image, la redresse ou change ses proportions." },
  eyedropper: { name: "Pipette", keys: "I", hint: "Prélève une couleur de l'image comme couleur de premier plan." },
  heal: { name: "Correcteur localisé", keys: "J", hint: "Efface une tache en reprenant la texture des pixels voisins." },
  brush: { name: "Pinceau", keys: "B", hint: "Peint à la couleur de premier plan, avec la taille et la dureté choisies." },
  eraser: { name: "Gomme", keys: "E", hint: "Efface les pixels du calque, ou creuse le masque quand il est visé." },
  fill: { name: "Dégradé / pot", keys: "G", hint: "Remplit d'une couleur unie, ou trace un dégradé avec Maj+G." },
  pen: { name: "Plume", keys: "P", hint: "Trace un chemin courbe, à convertir ensuite en sélection ou en forme." },
  text: { name: "Texte", keys: "T", hint: "Ajoute un calque de texte, avec police, taille et couleur réglables." },
  pathSelect: { name: "Sélection directe", keys: "A", hint: "Déplace les points et les poignées d'un chemin déjà tracé." },
  shape: { name: "Forme", keys: "U", hint: "Trace un rectangle, une ellipse ou un polygone vectoriel." },
  hand: { name: "Main", keys: "H", hint: "Fait glisser la vue ; un double-clic ajuste l'image à l'écran." },
  zoom: { name: "Zoom", keys: "Z", hint: "Agrandit ou réduit la vue ; un double-clic revient à 100 %." },
};
