// Guides des deux masques. Le masque de fusion cache une partie d'un calque ; le masque d'écrêtage
// limite un calque à la forme de celui du dessous. Les illustrations viennent du moteur
// (`assets/help/mask-layer.jpg`, `mask-clip.jpg`).
import type { Article } from "../news";

const DATE = "2026-09-23";

export const MASK_ARTICLES: Article[] = [
  {
    id: "masque-de-fusion",
    kind: "tutorial",
    date: DATE,
    minutes: 5,
    level: "Débutant",
    title: "Masque de fusion : cacher sans effacer",
    summary: "Peindre en noir cache, peindre en blanc fait revenir. Rien n'est jamais perdu.",
    intro: "Un masque de fusion est une image en niveaux de gris attachée au calque : blanc = visible, noir = caché, gris = à moitié transparent. Les pixels du calque ne sont jamais touchés, on peut donc revenir en arrière indéfiniment.",
    blocks: [
      { type: "image", name: "mask-layer", caption: "À gauche, un dégradé peint dans le masque : le calque disparaît en fondu, sans qu'un seul pixel ait été effacé." },
      { type: "h", text: "Ajouter un masque" },
      {
        type: "list",
        items: [
          "**Le bouton masque**, au pied du panneau Calques : ajoute un masque blanc, tout reste visible.",
          "**Avec une sélection active**, le même bouton reprend la sélection comme masque : ce qui était sélectionné reste visible, le reste est caché — et la sélection est relâchée.",
          "**Clic droit sur le calque > Ajouter un masque noir** : tout est caché d'emblée, à faire revenir au pinceau blanc.",
          "Un calque n'a qu'un seul masque : le bouton est inactif si le calque en a déjà un.",
        ],
      },
      { type: "h", text: "Peindre dedans" },
      {
        type: "steps",
        items: [
          { title: "Cliquez la vignette du masque", text: "dans le panneau Calques : elle devient la cible. Pinceau, gomme, dégradé, pot de peinture et déplacement agissent désormais sur le masque." },
          { title: "Peignez en noir pour cacher,", text: "en blanc pour faire revenir. Seule la **luminosité** de la couleur compte : un rouge vif vaut un gris moyen." },
          { title: "Nuancez.", text: "Un gris à 50 % laisse passer la moitié du calque ; un pinceau doux à faible opacité fait des transitions invisibles." },
        ],
      },
      { type: "h", text: "Les gestes du panneau Calques" },
      {
        type: "list",
        items: [
          "**Clic** sur la vignette : viser le masque.",
          "**[Ctrl] + clic** : viser le masque **et** les pixels, pour les déplacer ensemble.",
          "**[Maj] + clic** : activer ou désactiver le masque — la vignette se grise, le calque réapparaît en entier.",
          "**Glisser la vignette sur un autre calque** : y déplacer le masque ; avec **[Alt]**, le copier.",
          "**[Ctrl] + [I]**, masque visé : inverser le masque, donc échanger le caché et le visible.",
          "**Clic droit > Supprimer le masque de fusion** : s'en défaire.",
        ],
      },
      { type: "p", text: "Le masque agit aussi sur les **effets** du calque, et sur un **groupe** : un masque posé sur un groupe cache d'un coup tout ce qu'il contient." },
      { type: "tip", text: "La gomme, sur un masque, peint du blanc : elle fait donc revenir ce que le pinceau noir avait caché. Un seul outil suffit pour aller dans les deux sens." },
    ],
    illustration: { image: "mask-layer" },
    hero: { image: "mask-layer" },
  },
  {
    id: "masque-ecretage-guide",
    kind: "tutorial",
    date: DATE,
    minutes: 4,
    level: "Débutant",
    title: "Masque d'écrêtage : prendre la forme du calque du dessous",
    summary: "Le calque du dessus n'apparaît que là où celui du dessous a des pixels.",
    intro: "Écrêter un calque, c'est le découper à la forme de celui juste en dessous. Une photo écrêtée sur un texte remplit les lettres ; un réglage écrêté sur une image ne corrige que cette image.",
    blocks: [
      { type: "image", name: "mask-clip", caption: "À gauche, la photo écrêtée sur un disque : elle n'apparaît qu'à l'intérieur, bord adouci compris." },
      { type: "h", text: "Écrêter, détacher" },
      {
        type: "list",
        items: [
          "**[Alt] + clic sur la limite** entre deux calques, dans le panneau Calques : le calque du dessus s'écrête sur celui du dessous. Refaire le geste le détache.",
          "**[Ctrl] + [Alt] + [G]** fait la même bascule sur le calque sélectionné.",
          "**Clic droit sur le calque > Créer un masque d'écrêtage** : même chose au menu.",
          "Une petite flèche sur la ligne du calque signale qu'il est écrêté.",
          "Le calque le plus bas d'une liste n'a rien sous quoi s'écrêter : la commande y est inactive.",
        ],
      },
      { type: "h", text: "Ce qui se passe vraiment" },
      {
        type: "list",
        items: [
          "Le calque du dessous devient la **base** ; tous les calques écrêtés empilés juste au-dessus forment un groupe.",
          "Les calques écrêtés **prennent la couleur, pas la transparence** : c'est la base qui décide de la forme.",
          "Le **mode de fusion, l'opacité et le masque de la base** s'appliquent ensuite à tout le lot. Une base invisible fait disparaître tout le groupe.",
          "Un **réglage écrêté** ne corrige que le contenu de sa base, pas ce qui se trouve encore en dessous.",
          "Si la base porte des **effets** (ombre portée, lueur), les calques écrêtés s'arrêtent à sa forme, pas à son ombre.",
        ],
      },
      { type: "p", text: "Un nouveau calque inséré au milieu d'un groupe d'écrêtage le rejoint automatiquement : empiler plusieurs réglages sur la même photo ne demande pas de refaire le geste à chaque fois." },
      { type: "tip", text: "Les calques créés depuis l'onglet **Réglages** sont déjà écrêtés sur le calque sélectionné : c'est le comportement voulu neuf fois sur dix, et [Ctrl] + [Alt] + [G] libère le dixième." },
    ],
    illustration: { image: "mask-clip" },
    hero: { image: "mask-clip" },
  },
];
