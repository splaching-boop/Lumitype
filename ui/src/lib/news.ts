// Journal de l'écran d'accueil : nouveautés, tutoriels et astuces, livrés avec chaque version (aucune connexion
// nécessaire). Le plus récent en premier ; le premier article est mis en avant. Pour publier : ajouter une entrée
// en tête de liste.
//
// Le contenu d'un article est une suite de blocs (paragraphe, intertitre, étapes numérotées, liste, astuce,
// figure dessinée, illustration, animation). Dans un texte, `[Alt]` s'affiche comme une touche du clavier
// et `**gras**` en gras.

import { ADJUSTMENT_ARTICLES } from "./articles/adjustments";
import { MASK_ARTICLES } from "./articles/masks";
import { TOOL_ARTICLES } from "./articles/tools";
import type { IconName } from "./icons";
import { formatDate, t } from "./i18n.svelte";

export type ArticleKind = "update" | "tutorial" | "tip";

/**
 * Dessin d'une carte ou d'un bandeau : un des dessins intégrés (voir ArticleArt.svelte), l'icône d'un
 * outil, une illustration fabriquée par le moteur (`assets/help`) ou une animation (`assets/media`).
 */
export type Illustration =
  | "banner"
  | "clip"
  | "softBrush"
  | "multiSelect"
  | "icons"
  | "adjust"
  | { icon: IconName }
  | { image: string }
  | { gif: string }
  /**
   * Guide démontré en vidéo : le bandeau lit `assets/media/demos/<demo>.mp4`. `icon` est le repli tant
   * que le film n'a pas été tourné — un guide d'outil garde ainsi son icône plutôt qu'un rectangle vide.
   */
  | { demo: string; icon: IconName };
/** Figures détaillées, dans le corps d'un article. */
export type Figure = "clipLayers" | "softBrushClicks" | "multiSelectFrame" | "adjustLive" | "penCursors" | "autosave";

export type Block =
  | { type: "p"; text: string }
  | { type: "h"; text: string }
  | { type: "steps"; items: { title: string; text: string }[] }
  | { type: "list"; items: string[] }
  | { type: "tip"; text: string }
  | { type: "figure"; figure: Figure; caption: string }
  /** Illustration fixe produite par le moteur : `assets/help/<name>.jpg`. */
  | { type: "image"; name: string; caption: string }
  /** Emplacement d'animation : `assets/media/tutorials/<name>.gif`, ou l'animation de remplacement. */
  | { type: "gif"; name: string; caption: string };

export interface Article {
  id: string;
  kind: ArticleKind;
  /** Pour une mise à jour : la version. */
  version?: string;
  /** AAAA-MM-JJ. */
  date: string;
  minutes: number;
  level?: "Débutant" | "Intermédiaire" | "Avancé";
  title: string;
  /** Une phrase, sur les cartes de l'accueil. */
  summary: string;
  /** Chapeau de l'article, en tête du texte. */
  intro: string;
  blocks: Block[];
  /** Dessin de la carte. */
  illustration: Illustration;
  /** Bandeau de l'article : « avant / après » sur la photo de la bannière, ou le dessin en grand. */
  hero: "beforeAfter" | "banner" | Illustration;
}

export const KIND_LABEL: Record<ArticleKind, string> = { update: "Mise à jour", tutorial: "Tutoriel", tip: "Astuce" };

/** La dernière livraison : c'est elle qui est mise en avant sur l'accueil. */
const LATEST: Article[] = [
  {
    id: "0.4-langues-formats",
    kind: "update",
    version: "0.4",
    date: "2026-09-27",
    minutes: 3,
    title: "Quatre langues, les photos d'iPhone et les DNG, et le transfert de dégradé",
    summary: "L'interface et les guides en français, anglais, espagnol ou allemand ; HEIC, DNG, WebP et JPEG à l'ouverture ; le transfert de dégradé, et Inverser enfin distinct de Négatif.",
    intro: "Le drapeau en bas du menu de l'accueil change la langue de tout Lumitype, guides compris, sans redémarrer. Les photos d'iPhone (HEIC) et les négatifs numériques (DNG) s'ouvrent comme n'importe quelle image.",
    blocks: [
      { type: "h", text: "Langues" },
      { type: "p", text: "Français, anglais, espagnol, allemand : menus, panneaux, bulles d'aide, raccourcis et tutoriels. Le choix est retenu d'un lancement à l'autre." },
      { type: "h", text: "Formats d'image" },
      {
        type: "list",
        items: [
          "**HEIC / HEIF** : les photos d'iPhone, rotation comprise, décodées par Lumitype lui-même — rien à installer.",
          "**DNG** : développement complet (dématriçage, balance des blancs de la prise de vue, couleurs de l'appareil). Si le fichier résiste, son aperçu embarqué prend le relais.",
          "**JPEG, WebP, TIFF** : à l'ouverture comme à l'import en calque, en plus du PNG et de l'AVIF.",
        ],
      },
      { type: "h", text: "Transfert de dégradé" },
      { type: "p", text: "Un calque de réglage qui donne à chaque ton de l'image la couleur d'un dégradé : arrêts avec couleur, opacité et position, lissage, inversion, tramage, et les préréglages de l'outil Dégradé." },
      { type: "h", text: "Formes" },
      { type: "p", text: "Un rectangle, une ellipse ou un polygone se tourne, s'incline ou se met en perspective avec [Ctrl] + [T] **sans être pixellisé** : la forme est redessinée exactement, contour et arrondi compris, et reste modifiable dans Propriétés." },
      { type: "h", text: "Inverser et Négatif" },
      { type: "p", text: "Inverser ([Ctrl] + [I], et le calque de réglage du même nom) suit maintenant la courbe de Photoshop, aux tons moyens un peu plus denses. Il laisse le papier blanc ; le nouveau calque Négatif, juste dessous dans l'onglet Réglages, retourne tout, blanc compris." },
    ],
    illustration: { image: "gradientMap" },
    hero: { image: "gradientMap" },
  },
  {
    id: "0.3-sauvegarde",
    kind: "update",
    version: "0.3",
    date: "2026-09-22",
    minutes: 4,
    title: "Sauvegarde automatique avec l'historique, et un écran d'accueil",
    summary: "Tous les états de vos documents sont copiés en lieu sûr toutes les 3 minutes. Après une fermeture imprévue, Lumitype vous propose de tout reprendre, Ctrl+Z compris.",
    intro: "Lumitype copie maintenant chaque document modifié dans un dossier de secours, historique compris. Votre fichier .graf n'est jamais écrit à votre place : la copie vit à part.",
    blocks: [
      { type: "h", text: "Sauvegarde automatique" },
      {
        type: "list",
        items: [
          "**Intervalle** : 3 minutes par défaut, de 2 à 60 minutes dans Préférences > Fichiers.",
          "**Sans interruption** : pendant la copie, un filet avance en haut de la fenêtre ; vous pouvez continuer à travailler.",
          "**Récupération** : après une fermeture imprévue, une fenêtre propose de rouvrir chaque document avec tout son historique.",
        ],
      },
      { type: "figure", figure: "autosave", caption: "La copie de secours garde chaque état : [Ctrl] + [Z] fonctionne encore après une récupération." },
      { type: "h", text: "Un écran d'accueil" },
      { type: "p", text: "Déposez-y une image depuis vos dossiers ou une page web, collez une adresse ou une capture d'écran, choisissez un format rapide ou rouvrez un fichier récent. Fichier > Accueil le rappelle à tout moment." },
    ],
    illustration: "banner",
    hero: "banner",
  },
];

/** Articles plus anciens, à la suite des guides. */
const EARLIER: Article[] = [
  {
    id: "tuto-ecretage",
    kind: "tutorial",
    date: "2026-09-21",
    minutes: 6,
    level: "Débutant",
    title: "Masques d'écrêtage : ne corriger qu'un seul calque",
    summary: "Alt + clic entre deux calques, et le réglage ne touche plus que celui du dessous.",
    intro: "Un calque écrêté ne s'affiche qu'à travers la forme du calque juste en dessous. C'est la façon la plus simple de corriger une seule image d'une composition, sans toucher au reste.",
    blocks: [
      { type: "p", text: "Vous voulez réchauffer la photo du fond sans que le texte ni le logo ne changent de couleur ? Un calque de réglage écrêté sur la photo fait exactement ça, et il reste modifiable à tout moment." },
      { type: "figure", figure: "clipLayers", caption: "Le réglage Développement, écrêté, ne réchauffe que la photo." },
      { type: "h", text: "En trois gestes" },
      {
        type: "steps",
        items: [
          { title: "Placez le réglage juste au-dessus de la photo.", text: "Dans le panneau Calques, glissez le calque Développement (ou Niveaux, Teinte/Saturation…) pour qu'il soit collé à l'image à corriger." },
          { title: "Maintenez [Alt] et survolez la limite entre les deux calques.", text: "Un trait bleu apparaît : cliquez." },
          { title: "Réglez en direct.", text: "Une petite flèche bleue indique l'écrêtage. Les curseurs du réglage n'agissent plus que sur la photo. Refaites [Alt] + clic pour l'en détacher." },
        ],
      },
      { type: "tip", text: "Un nouveau calque créé dans un groupe d'écrêtage le rejoint automatiquement : empilez plusieurs réglages sur la même photo sans refaire le geste." },
    ],
    illustration: "clip",
    hero: "beforeAfter",
  },
  {
    id: "tuto-pinceau-flou",
    kind: "tutorial",
    date: "2026-09-21",
    minutes: 4,
    level: "Débutant",
    title: "Le pinceau flou, clic après clic",
    summary: "Chaque clic dépose un tiers : construisez ombres et lumières en douceur.",
    intro: "Avec une dureté sous 100 %, le pinceau rond devient flou : chaque empreinte dépose environ un tiers de la couleur, et les passages s'additionnent.",
    blocks: [
      { type: "figure", figure: "softBrushClicks", caption: "Un, deux, quatre, huit clics au même endroit : la tache se densifie en douceur." },
      { type: "h", text: "Bien s'en servir" },
      {
        type: "steps",
        items: [
          { title: "Baissez la dureté.", text: "Clic droit sur l'image : le menu rapide règle la taille et la dureté sans quitter le dessin. À 0 %, le fondu occupe tout le cercle." },
          { title: "Construisez par passages.", text: "Un clic donne une tache légère ; huit clics au même endroit, presque opaque. Repassez sur un trait pour le densifier." },
          { title: "Pour un dégradé très doux,", text: "peignez à 30 % d'opacité (touche [3]) en plusieurs passages." },
        ],
      },
      { type: "tip", text: "[Alt] + clic droit glissé règle la taille (horizontalement) et la dureté (verticalement) directement sur l'image." },
    ],
    illustration: "softBrush",
    hero: "softBrush",
  },
  {
    id: "astuce-maj-clic",
    kind: "tip",
    date: "2026-09-21",
    minutes: 1,
    title: "Maj + clic : transformer plusieurs images d'un coup",
    summary: "Un seul cadre englobe toute la sélection, sur l'image comme dans les calques.",
    intro: "Avec l'outil Déplacement, cliquez une image puis [Maj] + clic sur les autres : un cadre commun les entoure toutes.",
    blocks: [
      { type: "figure", figure: "multiSelectFrame", caption: "Trois images, un seul cadre : déplacer, agrandir ou tourner agit sur toutes." },
      {
        type: "list",
        items: [
          "**Agrandir ensemble** : tirez un angle du cadre.",
          "**Tourner ensemble** : tirez juste à l'extérieur d'un angle ; [Maj] tourne par pas de 15°.",
          "**Dans le panneau Calques**, [Maj] + clic ajoute ou retire un calque de la sélection ; [Ctrl] + [Maj] + clic sélectionne toute une plage.",
        ],
      },
    ],
    illustration: "multiSelect",
    hero: "multiSelect",
  },
  {
    id: "astuce-reglages-direct",
    kind: "tip",
    date: "2026-09-21",
    minutes: 2,
    title: "Des réglages instantanés",
    summary: "Exposition, température, niveaux : l'image suit le curseur sans le moindre délai.",
    intro: "Les curseurs des calques de réglage sont appliqués par la carte graphique pendant que vous les tirez. Le rendu définitif prend le relais dès que vous relâchez, sans saut de couleur.",
    blocks: [
      { type: "figure", figure: "adjustLive", caption: "La carte graphique applique le réglage à chaque image affichée ; le moteur calcule la version définitive en coulisse." },
      { type: "p", text: "Ça vaut pour un réglage libre, qui corrige tout ce qui est dessous, comme pour un réglage écrêté sur un seul calque." },
      { type: "tip", text: "Double-clic sur un curseur pour le remettre à sa valeur neutre." },
    ],
    illustration: "adjust",
    hero: "adjust",
  },
  {
    id: "0.2.8-icones",
    kind: "update",
    version: "0.2.8",
    date: "2026-09-21",
    minutes: 2,
    title: "Un nouveau jeu d'icônes",
    summary: "38 icônes géométriques pleines et des curseurs de plume qui annoncent chaque geste.",
    intro: "Toute l'interface adopte un jeu d'icônes dessiné pour Lumitype : formes pleines, évidements, traits épais aux bouts ronds.",
    blocks: [
      { type: "figure", figure: "penCursors", caption: "La plume annonce ce que le clic va faire : tracer, ajouter un point, en supprimer un, convertir un angle." },
      {
        type: "list",
        items: [
          "**Sur le tracé** : plume +, le clic ajoute un point sans changer la forme.",
          "**Sur un point** : plume −, le clic le supprime.",
          "**[Alt] sur un point** : l'angle, le clic le convertit entre angle et point lisse.",
        ],
      },
    ],
    illustration: "icons",
    hero: "icons",
  },
];

/**
 * Tout le journal : la dernière livraison, puis les guides (réglages, masques, outils), puis les
 * articles plus anciens. L'ordre du tableau est celui de l'affichage ; la date n'est qu'une étiquette.
 */
export const ARTICLES: Article[] = [...LATEST, ...ADJUSTMENT_ARTICLES, ...MASK_ARTICLES, ...TOOL_ARTICLES, ...EARLIER];

/** « 22 sept. 2026 », dans la langue de l'interface. */
export const articleDate = (date: string): string => formatDate(new Date(`${date}T12:00:00`), { day: "numeric", month: "short", year: "numeric" });

/**
 * L'article dans la langue de l'interface : chaque texte passe par `t`, la structure (identifiant,
 * dates, illustrations) reste la même. Les guides sont écrits en français une seule fois ; leurs
 * phrases sont les clés des dictionnaires, comme n'importe quelle chaîne de l'interface.
 */
export function localizeArticle(article: Article): Article {
  const block = (b: Block): Block => {
    switch (b.type) {
      case "steps":
        return { ...b, items: b.items.map((item) => ({ title: t(item.title), text: t(item.text) })) };
      case "list":
        return { ...b, items: b.items.map((item) => t(item)) };
      case "figure":
      case "image":
      case "gif":
        return { ...b, caption: t(b.caption) };
      default:
        return { ...b, text: t(b.text) };
    }
  };
  return { ...article, title: t(article.title), summary: t(article.summary), intro: t(article.intro), blocks: article.blocks.map(block) };
}

/** Tout le journal, dans la langue de l'interface. À lire dans un `$derived` : il suit la langue. */
export const localizedArticles = (): Article[] => ARTICLES.map(localizeArticle);

/** Tout le texte d'un article (recherche). */
export const articleText = (article: Article): string =>
  [
    article.title,
    article.summary,
    article.intro,
    ...article.blocks.flatMap((block) =>
      block.type === "steps" ? block.items.flatMap((item) => [item.title, item.text]) : block.type === "list" ? block.items : "caption" in block ? [block.caption] : [block.text],
    ),
  ].join(" ");

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
