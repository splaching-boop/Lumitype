// La notice de Lumitype : toutes les fonctionnalités, section par section, affichée dans l'onglet « Notice » de
// l'écran d'accueil (components/home/ManualView.svelte). Elle remplace l'ancien journal (notes de version,
// tutoriels, astuces).
//
// Le texte est écrit en français une seule fois : chaque phrase passe par `t()` au moment de l'affichage, et c'est
// elle la clé des dictionnaires (`locales/*.json`), comme n'importe quelle chaîne de l'interface. Après une
// modification : `python3 tools/i18n-extract.py`, puis traduire les nouvelles phrases dans en, es et de.
//
// Balisage des phrases (voir lib/rich-text.ts) : `**gras**`, et `[Touche]` pour une touche du clavier.
//
// Un même mot français peut avoir deux sens (« Annuler » : Undo dans Édition, Cancel sur un bouton) : on l'écrit
// alors avec son contexte, `Édition|Annuler`, qui a sa propre traduction ; le contexte n'est jamais affiché (`tm`).
//
// Raccourcis (`keys`) : l'identifiant d'une action (`file.new`) affiche le raccourci RÉEL de l'utilisateur, tel
// qu'il l'a réglé (keymap.label) ; une touche fixe s'écrit `=Ctrl+V` (noms anglais : Ctrl, Alt, Shift, Del, Enter,
// Esc, Space, Tab), et s'affiche avec les noms de touches de la langue.
//
// Images (`image`) : `assets/manual/<langue>/<nom>.png` (ou .jpg, .webp), sinon la version anglaise
// `assets/manual/en/<nom>.png`, sinon `assets/manual/<nom>.*` ; sans fichier, le bloc n'est pas affiché.
import { CATEGORIES, specOf } from "./filters";
import { i18n, t } from "./i18n.svelte";
import { plainText } from "./rich-text";

export { manualOpen } from "./manual-nav.svelte";

export type ManualBlock =
  /** Paragraphe, en texte enrichi. */
  | { type: "p"; text: string }
  /** Intertitre, avec son ancre. */
  | { type: "h"; id: string; text: string }
  /**
   * Ligne « fonction » : le nom en gras, le raccourci à droite, une phrase grise dessous. `items` : une liste de
   * libellés (déjà traduits par ailleurs) affichée à la place de la phrase, séparés par des virgules.
   */
  | { type: "fn"; name: string; keys?: string[]; text?: string; items?: string[] }
  /** Encadré « À savoir ». */
  | { type: "note"; text: string }
  | { type: "list"; items: string[] }
  /** Capture ou photo : `src` est le nom du fichier, sans extension ni dossier de langue. */
  | { type: "image"; src: string; alt: string; caption?: string }
  /** Cartes d'outils : `tool` est l'outil de l'application, dont le film vient de `TOOL_DEMO` (lib/media.ts). */
  | { type: "tools"; tools: { tool: string; name: string; keys?: string[] }[] }
  /** Tableau des raccourcis. */
  | { type: "table"; rows: { label: string; keys: string[] }[] };

export interface ManualSubsection {
  id: string;
  title: string;
  blocks: ManualBlock[];
}

export interface ManualSection {
  id: string;
  title: string;
  /** Titre court pour le sommaire, quand le titre est long. */
  short?: string;
  blocks: ManualBlock[];
  subsections?: ManualSubsection[];
}

/** Menu Filtre › catégories : tirées de la galerie elle-même (lib/filters.ts), elles restent toujours à jour. */
const FILTER_CATEGORIES: ManualBlock[] = CATEGORIES.map((category) => ({
  type: "fn",
  name: category.label,
  items: category.types.map((type) => specOf(type)?.label ?? type),
}));

export const MANUAL_SECTIONS: ManualSection[] = [
  {
    id: "histoire",
    title: "L'histoire",
    blocks: [
      { type: "p", text: "**Lumitype** est un nom inspiré de la machine française créée en **1944**, à **Lyon**. Elle permettait de faire de la photocomposition, c'est donc dans les faits, l'ancêtre physique de Photoshop, Gimp, ou encore… ce logiciel que vous utilisez maintenant." },
      { type: "p", text: "En concevant **Lumitype**, je voulais faire sortir mon esprit créatif et je l'ai conçu comme si c'était un outil que j'allais utiliser tous les jours. J'essaie de le penser tant pour les novices que pour les professionnels. Ceux-ci seront plus durs à satisfaire, car malgré mon expérience de graphiste depuis 2017, certains me dépassent de loin." },
      { type: "image", src: "lumitype-1944", alt: "La Lumitype, machine de photocomposition de 1944" },
      { type: "h", id: "direction-artistique", text: "Mais pourquoi cette direction artistique sombre, rouge et… carrée ?" },
      { type: "p", text: "La réponse est très simple. Premièrement, j'aime travailler la **nuit**, donc le noir était une obligation pour mes yeux ; un logiciel **sombre** permet de **mieux faire ressortir le canevas**, et notamment l'œuvre sur laquelle je travaille. Le rouge, c'est la passion, l'excitation, faire ressortir mon âme d'enfant avant d'attaquer un projet ! C'était ma couleur préférée quand j'étais petit, accessoirement. Et enfin, pourquoi tout est aussi… carré ? Simplement parce que j'aime les choses droites, parfaitement alignées et bien rangées : j'apprécie l'ordre. Tout ça pour donner l'interface d'aujourd'hui : **sombre, minimaliste, intense et bien organisée !**" },
      { type: "p", text: "**Bref, passons, voici toutes les fonctionnalités :**" },
    ],
  },
  {
    id: "accueil",
    title: "L'écran d'accueil",
    blocks: [
      { type: "p", text: "C'est la première chose que vous voyez. On y revient à tout moment par le bouton **Accueil** (tout à gauche de la barre de menus) ou **Fichier › Accueil**." },
      { type: "image", src: "home-screen", alt: "L'écran d'accueil de Lumitype" },
      { type: "fn", name: "Nouveau document", keys: ["file.new"], text: "Ouvre la boîte Nouveau document, avec tous les formats prêts à l'emploi." },
      { type: "fn", name: "Formats rapides", text: "Les formats que vous utilisez le plus, en un clic." },
      { type: "fn", name: "Formats", text: "Tous les formats, rangés par catégorie. Vos catégories personnelles s'exportent et s'importent en .json." },
      { type: "fn", name: "Récents", text: "Vos derniers projets et images. Un projet peut être **épinglé** pour rester en haut." },
      { type: "fn", name: "Modèles", text: "Des projets .graf livrés avec Lumitype, à ouvrir comme point de départ." },
      { type: "fn", name: "My ref", text: "Votre bibliothèque d'images de référence, voir plus bas." },
      { type: "fn", name: "Déposer une image", text: "Glissez une image dans la fenêtre pour l'ouvrir ; collez une capture avec [Ctrl] + [V] ; ou importez depuis une adresse web." },
      { type: "fn", name: "Recherche", text: "Cherche dans vos fichiers récents et dans cette notice." },
      { type: "fn", name: "Langue", text: "En bas de l'accueil : Français, English, Español, Deutsch. Change tout de suite, sans redémarrer." },
      { type: "fn", name: "Aperçus vidéo", text: "Survolez une vignette : un petit film montre la fonction en action." },
    ],
  },
  {
    id: "menus",
    title: "Les menus",
    blocks: [],
    subsections: [
      {
        id: "menu-fichier",
        title: "Fichier",
        blocks: [
          { type: "fn", name: "Accueil", keys: ["file.home"], text: "Affiche ou referme l'écran d'accueil." },
          { type: "fn", name: "Nouveau", keys: ["file.new"], text: "Crée un nouveau document." },
          { type: "fn", name: "Ouvrir", keys: ["file.open"], text: "Ouvre un projet, une image, un document." },
          { type: "fn", name: "Importer comme calque", keys: ["file.importLayer"], text: "Importe l'image comme un calque à l'intérieur du projet actuellement ouvert. Plusieurs images = plusieurs calques." },
          { type: "fn", name: "Importer comme objet dynamique", keys: ["file.placeSmart"], text: "Permet d'importer une image comme un calque déjà converti en objet dynamique." },
          { type: "fn", name: "Importer lié", keys: ["file.placeLinked"], text: "Permet d'utiliser une image directement dans le projet sans en importer une copie : le projet devient dépendant de l'image. Modifier l'image impacte donc le projet directement, sans exportation, et changer le nom de ce fichier le rendra indisponible dans le projet. Lumitype proposera alors **Relier…**" },
          { type: "fn", name: "Enregistrer", keys: ["file.save"], text: "Sauvegarde le projet .graf ; demande où le sauvegarder si c'est la première fois que vous enregistrez. Dans le contenu d'un objet dynamique, met à jour l'objet." },
          { type: "fn", name: "Enregistrer sous", keys: ["file.saveAs"], text: "Sauvegarde le projet .graf en précisant où vous voulez l'enregistrer." },
          { type: "fn", name: "Exporter", keys: ["file.exportPng"], text: "Exporte le plan de travail, ou tout le document, en image ou en PDF." },
          { type: "fn", name: "Fermer", keys: ["file.close"], text: "Ferme le projet." },
          { type: "image", src: "menu-file", alt: "Le menu Fichier" },
        ],
      },
      {
        id: "menu-edition",
        title: "Édition",
        blocks: [
          { type: "fn", name: "Édition|Annuler", keys: ["edit.undo"], text: "Annule la ou les actions effectuées précédemment." },
          { type: "fn", name: "Rétablir", keys: ["edit.redo"], text: "Rétablit une action que vous avez annulée." },
          { type: "fn", name: "Basculer le dernier état", keys: ["edit.toggleLastState"], text: "Fait un aller-retour entre « avant » et « après » votre dernière action. Pratique pour comparer un réglage d'un coup d'œil : un appui pour revenir en arrière, un second pour revenir en avant." },
          { type: "fn", name: "Couper", keys: ["edit.cut"], text: "Permet de **retirer** quelque chose pour le **coller ailleurs** : c'est une manière de déplacer quelque chose en deux actions composées. Il faut d'abord **sélectionner ce que vous souhaitez couper**, faire [Ctrl] + [X], puis aller à l'endroit où vous souhaitez le coller et faire [Ctrl] + [V]. Avec une sélection, ce sont les pixels sélectionnés qui sont coupés ; sans sélection, c'est le calque entier." },
          { type: "fn", name: "Copier", keys: ["edit.copy"], text: "En se basant sur le principe de **Couper**, Copier permet de faire une copie d'un calque, d'un texte, ou de n'importe quel élément sélectionnable dans un projet. La copie va aussi dans le presse-papiers du système, pour la coller dans un autre logiciel." },
          { type: "fn", name: "Coller", keys: ["edit.paste", "=Ctrl+V"], text: "Colle un élément qui a été copié ou coupé. Une image copiée depuis un autre logiciel, ou une capture d'écran, devient un nouveau calque." },
          { type: "note", text: "À savoir : vous pouvez copier un calque ou un groupe d'un **projet A**, et le coller dans un **projet B**." },
          { type: "fn", name: "Remplir : premier plan", keys: ["edit.fillForeground"], text: "Remplit le calque actuellement sélectionné de la couleur de premier plan choisie dans le panneau **Couleur**. S'il y a une sélection, seule la sélection est remplie." },
          { type: "fn", name: "Remplir : arrière-plan", keys: ["edit.fillBackground"], text: "Remplit le calque actuellement sélectionné de la couleur d'arrière-plan choisie dans le panneau **Couleur**." },
          { type: "image", src: "panel-color", alt: "Le panneau Couleur", caption: "Dans le panneau Couleur, la grande pastille en haut à gauche est la couleur de **premier plan**, la petite derrière elle la couleur d'**arrière-plan**." },
          { type: "fn", name: "Transformation manuelle", keys: ["edit.freeTransform"], text: "Transforme une image : modifier sa hauteur, sa largeur, de manière proportionnelle ou non, la faire pivoter. [Entrée] valide, [Échap] annule." },
          {
            type: "list",
            items: [
              "**Transformation normale** : [Ctrl] + [T], puis un simple clic gauche sur une poignée.",
              "**Un seul point** : [Ctrl] + clic gauche sur un coin, pour le déplacer seul.",
              "**Depuis le centre** : [Alt] + clic gauche sur une poignée.",
            ],
          },
          { type: "image", src: "transform", alt: "Le cadre de transformation manuelle" },
          { type: "fn", name: "Paramètres de couleur", keys: ["edit.colorSettings"], text: "Ouvre la boîte des profils de couleur, voir plus bas." },
          { type: "fn", name: "Préférences", keys: ["edit.preferences"], text: "Langue, unités, apparence, raccourcis, fichiers, performances." },
          { type: "fn", name: "Raccourcis clavier", keys: ["edit.shortcuts"], text: "Pour changer n'importe quel raccourci." },
          { type: "image", src: "menu-edit", alt: "Le menu Édition" },
        ],
      },
      {
        id: "parametres-couleur",
        title: "Paramètres de couleur",
        blocks: [
          { type: "fn", name: "RVB", text: "Liste des profils de couleur numérique pour le projet. RVB signifie Rouge, Vert, Bleu : c'est un mode de couleur dit additif, car il fonctionne en additionnant ces 3 couleurs." },
          { type: "fn", name: "CMJN", text: "Liste des profils de couleur d'impression pour le projet. CMJN signifie Cyan, Magenta, Jaune, Noir : c'est un mode de couleur dit soustractif, il crée les couleurs en absorbant (soustrayant) la lumière." },
          { type: "fn", name: "Intention", text: "Détermine comment les couleurs hors de la gamme imprimable sont traitées. **Colorimétrie relative** : le rendu reste parfaitement fidèle, et seules les couleurs hors gamme sont corrigées pour être imprimables. **Perceptive** : toutes les couleurs sont légèrement ajustées, ce qui garde des nuances différentes entre plusieurs couleurs hors gamme." },
          { type: "fn", name: "Compensation du point noir", text: "Si cochée, fait ressortir les détails des couleurs très sombres et évite qu'elles ne se fondent en un aplat." },
          { type: "fn", name: "Texte et filets noirs", text: "Le noir pur du projet est imprimé avec la seule encre Noir au lieu d'un mélange de toutes les encres. Cela évite des bords colorés." },
          { type: "fn", name: "Aplats noirs", text: "Ajoute les autres encres sous le noir des grandes surfaces pour approfondir sa densité à l'impression : c'est le « noir riche »." },
          { type: "fn", name: "Réglages Photoshop « Europe »", text: "Remet les réglages par défaut équivalents à ceux de Photoshop en Europe." },
          { type: "image", src: "dialog-color-settings", alt: "La boîte Paramètres de couleur" },
        ],
      },
      {
        id: "menu-image",
        title: "Image",
        blocks: [
          { type: "fn", name: "Mode", text: "Change le mode de couleur de tout le document." },
          {
            type: "list",
            items: [
              "**Bitmap** : noir ou blanc, sans gris. Une fenêtre règle la méthode de tramage.",
              "**Niveaux de gris** : du noir au blanc, sans couleur.",
              "**Couleurs RVB** : pour l'écran.",
              "**Couleurs CMJN** : pour l'impression. Une fenêtre choisit le profil de presse.",
              "**8 bits/couche** : la seule profondeur de couleur, toujours cochée.",
            ],
          },
          { type: "fn", name: "Format d'impression", keys: ["image.printSetup"], text: "Règle le format fini, les fonds perdus, la zone de sécurité, l'encrage maximal et la résolution. Voir **L'impression**." },
          { type: "fn", name: "Corriger l'encrage", keys: ["image.correctInk"], text: "Ramène automatiquement sous la limite les zones qui mettent trop d'encre. Le gris fait de cyan + magenta + jaune passe en noir seul." },
          { type: "fn", name: "Inverser", keys: ["image.invert"], text: "Chaque couleur devient son contraire : le noir devient blanc, le rouge devient cyan… Sur un masque, inverse le masque." },
          { type: "image", src: "menu-image", alt: "Le menu Image" },
        ],
      },
      {
        id: "menu-calque",
        title: "Calque",
        blocks: [
          { type: "fn", name: "Nouveau calque", keys: ["layer.new"], text: "Un calque vide au-dessus du calque actif." },
          { type: "fn", name: "Nouveau groupe", keys: ["layer.newGroup"], text: "Un dossier vide pour ranger des calques." },
          { type: "fn", name: "Grouper les calques", keys: ["layer.group"], text: "Met les calques sélectionnés dans un groupe." },
          { type: "fn", name: "Dupliquer le calque", keys: ["layer.duplicate"], text: "Une copie du calque, juste au-dessus." },
          { type: "fn", name: "Supprimer le calque", keys: ["layer.delete"], text: "La touche [Suppr] seule efface les pixels sélectionnés ; sans sélection, elle supprime le masque visé, sinon le calque." },
          { type: "fn", name: "Nouveau calque de réglage", text: "Crée un calque qui modifie les couleurs de ce qui est en dessous, sans toucher aux pixels. Il est écrêté sur le calque actif : il ne touche que lui (sauf Négatif, qui agit sur tout ce qui est dessous). Réglages disponibles : Développement, Luminosité/Contraste, Niveaux, Courbes, Exposition, Teinte/Saturation, Balance des couleurs, Noir et blanc, Mélangeur de couches, Correction sélective, Inverser, Négatif, Isohélie, Seuil, Transfert de dégradé." },
          { type: "fn", name: "Niveaux", keys: ["adjust.levels"], text: "Le réglage Niveaux, directement." },
          { type: "fn", name: "Teinte/Saturation", keys: ["adjust.hueSaturation"], text: "Le réglage Teinte/Saturation, directement." },
          { type: "fn", name: "Nouveau calque de remplissage", text: "Un calque entièrement rempli d'une **Couleur unie**, d'un **Dégradé** ou d'un **Motif**, modifiable à tout moment." },
          { type: "fn", name: "Style de calque", keys: ["layer.effects"], text: "**Effets de calque (fx)** ouvre la boîte des effets : ombre portée, lueur, contour, biseau, incrustation… Vous pouvez aussi **Copier**, **Coller** ou **Effacer** le style d'un calque." },
          { type: "fn", name: "Ajouter un masque de fusion", keys: ["layer.addMask"], text: "Un masque blanc : peindre en noir dessus cache une partie du calque, sans rien effacer." },
          { type: "fn", name: "Créer / annuler le masque d'écrêtage", keys: ["layer.toggleClipping"], text: "Le calque ne s'affiche plus que sur les pixels du calque du dessous." },
          { type: "fn", name: "Convertir en objet dynamique", keys: ["layer.convertToSmart"], text: "Voir **Les objets dynamiques**." },
          { type: "fn", name: "Modifier le contenu", keys: ["layer.editSmartContent"], text: "Ouvre le contenu de l'objet dynamique dans un onglet à part." },
          { type: "fn", name: "Remplacer le contenu", keys: ["layer.replaceSmartContent"], text: "Remplace l'image de l'objet par une autre, en gardant la position et la taille." },
          { type: "fn", name: "Nouvel objet dynamique par copie", keys: ["layer.newSmartByCopy"], text: "Une copie indépendante : modifier l'une ne change pas l'autre." },
          { type: "fn", name: "Exporter le contenu", keys: ["layer.exportSmartContent"], text: "Enregistre l'image d'origine de l'objet dans un fichier." },
          { type: "fn", name: "Incorporer le fichier lié", keys: ["layer.embedSmartLink"], text: "Un objet lié devient incorporé : le projet n'a plus besoin du fichier externe." },
          { type: "fn", name: "Relier", keys: ["layer.relinkSmart"], text: "Choisit un nouveau fichier pour un objet lié dont le fichier a disparu." },
          { type: "fn", name: "Réinitialiser la transformation", keys: ["layer.resetSmartTransform"], text: "L'objet retrouve sa taille et son angle d'origine." },
          { type: "fn", name: "Pixelliser le calque", keys: ["layer.rasterize"], text: "Un texte, une forme, un objet dynamique devient un simple calque de pixels." },
          { type: "fn", name: "Fusionner les calques", keys: ["layer.mergeGroup"], text: "Les calques sélectionnés deviennent un seul calque de pixels ; un groupe seul est aplati." },
          { type: "image", src: "menu-layer", alt: "Le menu Calque" },
        ],
      },
      {
        id: "menu-selection",
        title: "Sélection",
        blocks: [
          { type: "fn", name: "Tout sélectionner", keys: ["select.all"], text: "Sélectionne les pixels opaques du calque actif." },
          { type: "fn", name: "Désélectionner", keys: ["select.none"] },
          { type: "fn", name: "Intervertir", keys: ["select.invert"], text: "Ce qui était sélectionné ne l'est plus, et inversement." },
          { type: "fn", name: "Sélection avancée", keys: ["select.advanced"], text: "La boîte qui détoure : par couleur, par couche, par plage de tons ou par **sujet** automatique. Avec des pinceaux + / − / Affiner, et l'option **Décontaminer les couleurs** pour les cheveux et les bords fins. Sortie : sélection, masque de calque ou nouveau calque avec masque." },
          { type: "fn", name: "Contour progressif", keys: ["select.feather"], text: "Adoucit le bord de la sélection sur un rayon en pixels." },
          { type: "fn", name: "Modifier la sélection", keys: ["select.modify"], text: "**Dilater**, **Contracter** ou **Lisser** le contour." },
          { type: "fn", name: "Mode Masque rapide", keys: ["select.quickMask"], text: "La sélection devient une couche rouge que l'on peint au pinceau : noir pour retirer, blanc pour ajouter. La même touche ramène à la sélection." },
          { type: "image", src: "menu-select", alt: "Le menu Sélection" },
        ],
      },
      {
        id: "menu-filtre",
        title: "Filtre",
        blocks: [
          { type: "fn", name: "Dernier filtre", keys: ["filter.lastFilter"], text: "Refait le dernier filtre avec les mêmes réglages." },
          { type: "fn", name: "Galerie de filtres", keys: ["filter.gallery"], text: "Tous les filtres avec aperçu en direct. Clic droit sur un filtre pour l'ajouter aux **Favoris**." },
          { type: "fn", name: "Filtre Camera Raw", keys: ["filter.cameraRaw"], text: "La fenêtre de développement RAW appliquée au calque actif : exposition, blancs, noirs, clarté, voile, balance des blancs…" },
          { type: "fn", name: "Fluidité", keys: ["filter.liquify"], text: "Pousser, gonfler, pincer les pixels comme de la pâte." },
          { type: "p", text: "**Catégories** : chaque filtre a sa vignette et sa description au survol du menu." },
          ...FILTER_CATEGORIES,
          { type: "fn", name: "Favoris", text: "Vos filtres préférés, à portée de main." },
          { type: "note", text: "À savoir : un filtre appliqué sur un **objet dynamique** devient un **filtre dynamique** : il reste modifiable, et possède son propre masque." },
          { type: "image", src: "menu-filter", alt: "Le menu Filtre" },
        ],
      },
      {
        id: "menu-affichage",
        title: "Affichage",
        blocks: [
          { type: "fn", name: "Zoom avant / Zoom arrière", keys: ["view.zoomIn", "view.zoomOut"] },
          { type: "fn", name: "Taille écran", keys: ["view.fit"], text: "Ajuste le document, ou le plan de travail isolé, à la fenêtre." },
          { type: "fn", name: "100 %", keys: ["view.actualSize"], text: "Un pixel de l'image = un pixel de l'écran." },
          { type: "fn", name: "Réinitialiser la rotation de la vue", keys: ["view.resetRotation"], text: "Remet l'affichage droit après l'outil Rotation." },
          { type: "fn", name: "Vue miroir", keys: ["view.mirror"], text: "Retourne seulement l'affichage, pas l'image. Pour vérifier les proportions d'un dessin d'un œil neuf." },
          { type: "fn", name: "Règles", keys: ["view.toggleRulers"], text: "Dans l'unité choisie dans les Préférences. Avec un format d'impression, le zéro est au coin du format fini." },
          { type: "fn", name: "Afficher / masquer les repères", keys: ["view.toggleGuides"] },
          { type: "fn", name: "Verrouiller les repères", keys: ["view.lockGuides"] },
          { type: "fn", name: "Magnétisme", keys: ["view.toggleSnap"], text: "Les objets s'accrochent aux repères et aux bords." },
          { type: "fn", name: "Disposition de repères", keys: ["view.guideLayout"], text: "Crée une grille de colonnes, rangées et marges d'un coup ; des modèles sont fournis, et vous pouvez enregistrer les vôtres." },
          { type: "fn", name: "Effacer les repères", keys: ["view.clearGuides"] },
          { type: "fn", name: "Impression", text: "Voir **L'impression** : Fonds perdus, Zone de sécurité, Traits de coupe, Alerte d'encrage, Séparations, Vérifier avant impression." },
          { type: "fn", name: "Épreuve CMJN", keys: ["view.proofCmyk"], text: "Simule à l'écran le rendu imprimé avec le profil CMJN des Paramètres de couleur." },
          { type: "image", src: "menu-view", alt: "Le menu Affichage" },
        ],
      },
      {
        id: "menu-plans",
        title: "Plans de travail",
        blocks: [{ type: "p", text: "Un clic ouvre directement la boîte **Plans de travail** : voir **Les plans de travail**." }],
      },
      {
        id: "menu-copyright",
        title: "Copyright",
        blocks: [{ type: "fn", name: "Métadonnées par défaut", keys: ["copyright.metadata"], text: "Votre nom, le copyright et les mots-clés inscrits automatiquement dans chaque export." }],
      },
      {
        id: "menu-aide",
        title: "Aide",
        blocks: [
          { type: "fn", name: "Notice", keys: ["help.manual"], text: "Ouvre cette notice." },
          { type: "fn", name: "Signaler un bug", keys: ["help.reportBug"] },
          { type: "fn", name: "Ouvrir le dossier des journaux", keys: ["help.openLogs"] },
          { type: "fn", name: "À propos de Lumitype", keys: ["help.about"] },
        ],
      },
    ],
  },
  {
    id: "outils",
    title: "Les outils",
    blocks: [
      { type: "p", text: "Les outils sont empilés : **clic droit**, **appui long** ou le petit triangle montre les autres outils du groupe ; [Maj] + la lettre passe au suivant. Survolez un outil une seconde : une bulle montre un petit film de l'outil en action. La **barre d'options**, en haut, change selon l'outil ; elle garde toujours **Taille écran** et **100 %** à droite." },
      { type: "image", src: "toolbar", alt: "La barre d'outils" },
      { type: "p", text: "Survolez une carte : le film de l'outil se lance." },
      {
        type: "tools",
        tools: [
          { tool: "move", name: "Déplacement", keys: ["tool.move"] },
          { tool: "marquee", name: "Rectangle de sélection", keys: ["tool.marquee"] },
          { tool: "lasso", name: "Lasso", keys: ["tool.lasso"] },
          { tool: "wand", name: "Baguette magique", keys: ["tool.wand"] },
          { tool: "quickSelect", name: "Sélection rapide", keys: ["tool.wandCycle"] },
          { tool: "crop", name: "Recadrage", keys: ["tool.crop"] },
          { tool: "eyedropper", name: "Pipette", keys: ["tool.eyedropper"] },
          { tool: "heal", name: "Correcteur localisé", keys: ["tool.heal"] },
          { tool: "brush", name: "Pinceau", keys: ["tool.brush"] },
          { tool: "eraser", name: "Gomme", keys: ["tool.eraser"] },
          { tool: "fill", name: "Pot de peinture et Dégradé", keys: ["tool.fill"] },
          { tool: "pen", name: "Plume", keys: ["tool.pen"] },
          { tool: "pathSelect", name: "Sélection directe", keys: ["tool.pathSelect"] },
          { tool: "text", name: "Texte", keys: ["tool.text"] },
          { tool: "shape", name: "Formes", keys: ["tool.shape"] },
          { tool: "hand", name: "Main", keys: ["tool.hand"] },
          { tool: "zoom", name: "Zoom", keys: ["tool.zoom"] },
        ],
      },
      { type: "fn", name: "Déplacement", keys: ["tool.move"], text: "Déplace et transforme le calque. [Maj] + clic ajoute un calque au cadre commun ; [Alt] + glisser duplique ; les flèches déplacent de 1 px, [Maj] + flèches de 10 px. Option **Sélection automatique** : cliquer sur l'image sélectionne le calque ou le groupe sous le pointeur." },
      { type: "fn", name: "Rectangle de sélection / Ellipse de sélection", keys: ["tool.marquee"], text: "[Maj] ajoute, [Alt] soustrait, [Maj] + [Alt] garde l'intersection." },
      { type: "fn", name: "Lasso / Lasso polygonal", keys: ["tool.lasso"], text: "Libre à main levée, ou point par point ; double-clic ou [Entrée] ferme le tracé." },
      { type: "fn", name: "Baguette magique", keys: ["tool.wand"], text: "Sélectionne les pixels de couleur proche. Options : Tolérance, Pixels contigus, Lissage, Tous les calques, Échantillon (point, moyenne 3 × 3 ou 5 × 5). Glissez après le clic pour régler la tolérance en direct." },
      { type: "fn", name: "Sélection rapide", keys: ["tool.wandCycle"], text: "Peignez grossièrement : la sélection suit les contours toute seule. [Alt] retire." },
      { type: "fn", name: "Recadrage", keys: ["tool.crop"], text: "Libre, 1:1, 4:3, 3:2, 16:9, 9:16. Le cadre peut dépasser pour **agrandir** la zone de travail. [Entrée] ou double-clic valide." },
      { type: "fn", name: "Pipette", keys: ["tool.eyedropper"], text: "Prélève la couleur de premier plan. Avec le pinceau ou le pot en main, [Alt] donne une pipette temporaire." },
      { type: "fn", name: "Correcteur localisé", keys: ["tool.heal"], text: "Peignez sur un défaut : il disparaît au relâchement. Options : Taille, Dureté." },
      { type: "fn", name: "Pinceau", keys: ["tool.brush"], text: "Taille, Dureté, Opacité (touches 1 à 9, 0 = 100 %), Lissage du tracé, Pression → taille et Pression → opacité pour les tablettes. Le panneau Propriétés devient la bibliothèque de pinceaux." },
      { type: "fn", name: "Taille du pinceau", keys: ["brush.smaller", "brush.larger"], text: "Réduit ou augmente la taille du pinceau." },
      { type: "fn", name: "Gomme", keys: ["tool.eraser"], text: "Les mêmes réglages que le pinceau." },
      { type: "fn", name: "Pot de peinture", keys: ["tool.fill"], text: "Remplit une zone de couleur proche. Options : Tolérance, Pixels contigus." },
      { type: "fn", name: "Dégradé", keys: ["tool.fillCycle"], text: "Linéaire ou radial, de la couleur de premier plan à celle d'arrière-plan ; la pastille ouvre l'éditeur de dégradé. [Maj] contraint à 45°." },
      { type: "fn", name: "Plume", keys: ["tool.pen"], text: "Trace des courbes point par point. Le tracé peut devenir une **Sélection**, être rempli (**Fond**) ou dessiné (**Contour**)." },
      { type: "fn", name: "Définir une sélection depuis le tracé", keys: ["path.toSelection"] },
      { type: "fn", name: "Sélection directe", keys: ["tool.pathSelect"], text: "Déplace les points et les poignées d'un tracé ou d'une forme." },
      { type: "fn", name: "Texte", keys: ["tool.text"], text: "Cliquez pour un texte de point, ou tracez un cadre pour un **Texte dynamique** dont les lignes remplissent la largeur. [Ctrl] + [Entrée] valide, [Échap] annule. Police, taille, couleur, interligne, approche et alignement sont dans Propriétés." },
      { type: "fn", name: "Rectangle / Ellipse / Triangle / Polygone", keys: ["tool.shape"], text: "Formes vectorielles. Arrondi des angles, fond et contour se règlent dans Propriétés ; **Côtés** pour le polygone." },
      { type: "fn", name: "Main", keys: ["tool.hand", "=Space"], text: "Fait glisser la vue. Double-clic sur l'outil = Taille écran. [Espace] la donne temporairement." },
      { type: "fn", name: "Rotation de l'affichage", keys: ["tool.rotateView"], text: "Tourne la vue, pas l'image. [Maj] : pas de 15°. [Échap] ou double-clic sur l'outil = 0°." },
      { type: "fn", name: "Zoom", keys: ["tool.zoom"], text: "[Alt] + clic : zoom arrière. Double-clic sur l'outil = 100 %." },
    ],
  },
  {
    id: "panneaux",
    title: "Les panneaux",
    blocks: [],
    subsections: [
      {
        id: "panneau-couleur",
        title: "Couleur",
        blocks: [
          { type: "fn", name: "Roue", text: "La teinte sur le cercle, la saturation et la luminosité dans le triangle. Les curseurs **S** et **L** en dessous font la même chose, plus précisément." },
          { type: "fn", name: "Premier plan / Arrière-plan", keys: ["color.swap", "color.reset"], text: "Les deux pastilles. La première touche les permute, la seconde remet noir et blanc, comme le bouton **Défaut**." },
          { type: "fn", name: "Code hexadécimal", text: "Tapez ou collez une couleur, par exemple #ff0000." },
          { type: "fn", name: "Hors gamme CMJN", text: "En CMJN, une alerte prévient qu'une couleur n'est pas imprimable ; **Ramener dans la gamme** la corrige." },
        ],
      },
      {
        id: "panneau-proprietes",
        title: "Propriétés · Réglages · Historique",
        blocks: [
          { type: "fn", name: "Propriétés", text: "Nom et position du calque, puis selon le calque : les réglages d'un calque de réglage, le fond et le contour d'une forme, la police d'un texte, le fichier d'un objet dynamique. Pinceau en main : la bibliothèque de pinceaux." },
          { type: "fn", name: "Réglages", text: "Un clic crée un calque de réglage, écrêté sur le calque sélectionné." },
          { type: "fn", name: "Historique", text: "Toutes vos étapes ; cliquez sur l'une d'elles pour y revenir." },
        ],
      },
      {
        id: "panneau-texte",
        title: "Texte (dans Propriétés)",
        blocks: [
          { type: "fn", name: "Police, Style, Gras, Italique, Taille, Couleur, Interligne, Approche" },
          { type: "fn", name: "Alignement", text: "Gauche, droite, justifié." },
          { type: "fn", name: "Texte dynamique", text: "Largeur et hauteur du cadre, interlignage, retours à la ligne." },
          { type: "fn", name: "Styles de caractère", text: "Enregistrez un style pour le réappliquer : Nouveau, Redéfinir, **Kit de marque**, Importer d'un autre projet." },
        ],
      },
      {
        id: "panneau-calques",
        title: "Calques",
        blocks: [
          { type: "image", src: "panel-layers", alt: "Le panneau Calques" },
          { type: "fn", name: "Opacité et Fond", text: "Le Fond n'atténue pas les effets de calque." },
          { type: "fn", name: "Modes de fusion", keys: ["layer.blendNext", "layer.blendPrevious"], text: "Normal, Dissoudre · Obscurcir, Produit, Densité couleur +, Densité linéaire + · Éclaircir, Superposition, Densité couleur −, Densité linéaire − · Incrustation, Lumière tamisée, Lumière crue · Différence, Exclusion, Soustraire, Diviser · Teinte, Saturation, Couleur, Luminosité. Les raccourcis passent au mode suivant ou précédent." },
          { type: "fn", name: "Masques", text: "Masque de fusion lié ou délié au calque, **Masque noir** (tout caché), **Masque depuis la sélection**, masque d'écrêtage." },
          { type: "fn", name: "Filtres dynamiques", text: "Sur un objet dynamique : chaque filtre a sa ligne, son masque et ses options de fusion." },
          { type: "fn", name: "Groupes", text: "Dépliables, avec leur propre opacité et masque." },
          { type: "fn", name: "Verrous", text: "Pixels transparents, pixels de l'image, position, ou tout." },
          { type: "fn", name: "Œil", text: "[Alt] + clic n'affiche que ce calque." },
          { type: "fn", name: "Recherche de calque", keys: ["layer.search"] },
          { type: "fn", name: "Sélecteur de plan", text: "Tous les plans, ou un seul plan de travail isolé." },
          { type: "fn", name: "Clic droit", text: "Export rapide en PNG, Exporter sous, Exporter chaque calque séparément, Copier comme PNG, Convertir en texte de point / de paragraphe, Convertir le plan en groupe." },
        ],
      },
      {
        id: "panneau-pinceaux",
        title: "Pinceaux",
        blocks: [
          { type: "fn", name: "Dossiers", text: "Fournis : ronds de base et le pack Krita. Importés : .abr (Photoshop), .gbr (Gimp) ou une simple image ; glisser le fichier dans la fenêtre l'installe. Personnels : **Mes favoris**, **Mes pinceaux**." },
          { type: "fn", name: "Récemment utilisés", text: "Les 5 derniers." },
          { type: "fn", name: "Pinceaux|Réglages", text: "Forme de la pointe, Dispersion, Texture, Pinceau double, Mélangeur, Flux, Pression, Couleur dynamique, et **Réinitialiser la dynamique**." },
          { type: "fn", name: "Panneau", text: "Importer, Nouveau dossier, Enregistrer le pinceau courant, Vignettes ou Liste, Taille des aperçus." },
        ],
      },
      {
        id: "barre-etat",
        title: "Barre d'état (en bas)",
        blocks: [
          { type: "fn", name: "Temps de travail", text: "Le temps total passé sur le projet, enregistré dans le .graf, et le temps de cette session. Pause automatique après 5 minutes sans rien faire." },
          { type: "fn", name: "Nombre de calques", text: "Le détail par type au survol." },
          { type: "fn", name: "Taille", text: "Largeur × hauteur en pixels et résolution en ppp." },
          { type: "fn", name: "Légende d'impression", text: "Format fini, Fonds perdus, Zone de sécurité, et le nombre de zones au-dessus de la limite d'encre." },
          { type: "fn", name: "Indicateurs", text: "CMJN, Niveaux de gris, Bitmap, Épreuve, Miroir, Rotation, images par seconde si activé." },
        ],
      },
    ],
  },
  {
    id: "dialogues",
    title: "Les boîtes de dialogue",
    blocks: [],
    subsections: [
      {
        id: "dialogue-nouveau",
        title: "Nouveau document",
        blocks: [
          { type: "image", src: "dialog-new-document", alt: "La boîte Nouveau document" },
          { type: "fn", name: "Catégories", text: "Récents, Mes formats, Essentiels (A4, A3, A5 à 300 ppp, Écran HD, 4K, Carré, Story…), Impression, Cartes de visite, Instagram, LinkedIn, YouTube, TikTok, X, Reddit, Discord, Twitch, Kick, Signature e-mail." },
          { type: "fn", name: "Taille", text: "Largeur, Hauteur, Unité (pixels, millimètres, centimètres, pouces, points), Résolution, Portrait ou Paysage." },
          { type: "fn", name: "Couleurs", text: "RVB pour l'écran, CMJN pour l'impression." },
          { type: "fn", name: "Nouveau document|Fond", text: "Blanc ou Transparent." },
          { type: "fn", name: "Impression", text: "Avec fonds perdus (3 mm), Zone de sécurité (4 mm), Encrage max. (300 %)." },
        ],
      },
      {
        id: "dialogue-exporter",
        title: "Exporter",
        blocks: [
          { type: "image", src: "dialog-export", alt: "La boîte Exporter" },
          { type: "fn", name: "Formats", text: "PNG, JPEG, WebP (avec ou sans perte), TIFF, PDF, AVIF." },
          { type: "fn", name: "PDF", text: "Qualité maximale sans perte, Haute, Moyenne ou Légère. En CMJN, le PDF respecte la norme **PDF/X-4** des imprimeurs, avec **Inclure les fonds perdus** et **Traits de coupe**." },
          { type: "fn", name: "Couleurs", text: "RVB, CMJN (JPEG, TIFF, PDF), Gris (sauf WebP et AVIF), 1 bit pour un document en mode Bitmap (PNG, TIFF, PDF)." },
          { type: "fn", name: "Inclure le profil ICC", text: "Pour que les couleurs soient les mêmes partout." },
          { type: "fn", name: "Niveau de compression de l'image" },
          { type: "fn", name: "Résolution", text: "25, 50, 100, 150, 200 % ou Native." },
          { type: "fn", name: "Plan de travail", text: "Le plan actif, **Tous les plans** (un fichier par plan) ou **Document entier**." },
          { type: "fn", name: "Métadonnées", text: "Inscrire, description, mots-clés." },
          { type: "fn", name: "Poids estimé et aperçu", text: "Zoomable, avant d'exporter." },
        ],
      },
      {
        id: "dialogue-calques-separes",
        title: "Exporter chaque calque séparément",
        blocks: [{ type: "p", text: "Format, tailles multiples, suffixe, zone, marge, destination, et **Mémoriser pour l'export rapide**." }],
      },
      {
        id: "dialogue-effets",
        title: "Effets de calque",
        blocks: [{ type: "p", text: "Options de fusion ; Biseautage et estampage ; Contour ; Ombre interne ; Lueur interne ; Satin ; Incrustation de couleur, de dégradé, de motif ; Lueur externe ; Ombre portée." }],
      },
      {
        id: "dialogue-raccourcis",
        title: "Raccourcis clavier",
        blocks: [
          { type: "p", text: "Recherche, ajout ou retrait d'un raccourci. Un conflit est signalé en rouge : **Réattribuer** ou **Garder les deux**. **Défaut** sur une ligne, **Tout réinitialiser** pour tout. Enregistré dans keymap.json." },
          { type: "image", src: "dialog-shortcuts", alt: "La boîte Raccourcis clavier" },
        ],
      },
      {
        id: "dialogue-autres",
        title: "Autres boîtes",
        blocks: [{ type: "p", text: "**Calque de remplissage**, **Forme**, **Galerie de filtres**, **Fluidité**, **Développement RAW**, **Mode Bitmap**, **Mode CMJN**, **Métadonnées par défaut**, **Récupération**, **Signaler un bug**, **À propos**." }],
      },
    ],
  },
  {
    id: "impression",
    title: "L'impression",
    blocks: [
      { type: "p", text: "Tout ce qu'il faut pour envoyer un fichier propre à un imprimeur." },
      { type: "fn", name: "Format d'impression", keys: ["image.printSetup"], text: "**Image › Format d'impression**. Le **format fini** est la taille du papier une fois coupé. Les **fonds perdus** (3 mm) sont la marge qui dépasse, coupée au massicot : votre image doit les remplir pour éviter un liseré blanc. La **zone de sécurité** (4 mm) est la marge intérieure où rien d'important ne doit se trouver. **Étendre le contenu dans les fonds perdus** prolonge automatiquement les bords." },
      { type: "image", src: "dialog-print-setup", alt: "La boîte Format d'impression" },
      { type: "fn", name: "Encrage maximal", text: "La somme des 4 encres, C + M + J + N, ne doit pas dépasser une limite, en général **300 %** : au-delà, le papier gondole, sèche mal et les couleurs bavent. Lumitype conseille la valeur du profil choisi (FOGRA39 : 300 %, FOGRA52 : 280 %, GRACoL : 320 %)." },
      { type: "fn", name: "Affichage › Impression", text: "**Fonds perdus** : pointillés rouges. **Zone de sécurité** : pointillés bleus. **Traits de coupe** : aux coins. **Alerte d'encrage** : les zones au-dessus de la limite sont hachurées en orange, et une pipette d'encrage suit le pointeur avec C · M · J · N et le total. **Séparations** : voir une seule plaque C, M, J ou N." },
      { type: "image", src: "print-overlays", alt: "Fonds perdus, zone de sécurité et traits de coupe sur un document" },
      { type: "fn", name: "Corriger l'encrage", keys: ["image.correctInk"], text: "**Image › Corriger l'encrage** : ramène les zones hachurées sous la limite, sur le calque actif ou tous les calques." },
      { type: "fn", name: "Noirs", text: "Dans Paramètres de couleur : **Texte et filets noirs** imprime le noir pur avec la seule encre noire (pas de bords colorés) ; **Aplats noirs** met C60 M40 J40 sous le noir des grandes surfaces, pour un noir profond (le « noir riche »). Appliqué à l'export PDF/X-4." },
      { type: "fn", name: "Vérifier avant impression", keys: ["file.preflight"], text: "Contrôle tout d'un coup : résolution (300 ppp conseillés), document en RVB, encrage, texte hors zone de sécurité, fonds perdus vides, texte noir en quadri. Chaque point a ses boutons **Voir**, **Corriger…**, puis **Exporter en PDF…**" },
      { type: "fn", name: "Épreuve CMJN", keys: ["view.proofCmyk"], text: "L'écran simule le rendu imprimé." },
    ],
  },
  {
    id: "plans",
    title: "Les plans de travail",
    blocks: [
      { type: "p", text: "Plusieurs pages ou plusieurs formats dans un même projet (une carte de visite recto / verso, une story et un post…)." },
      { type: "fn", name: "Créer", keys: ["artboard.open"], text: "Menu **Plans de travail** : formats par catégorie, un clic montre l'aperçu, un double-clic crée le plan. Options : Planche, Marge, Fond (blanc, noir, sans fond), **Découper le contenu**." },
      { type: "image", src: "artboards", alt: "Des plans de travail sur la zone de travail" },
      { type: "fn", name: "Sur la zone de travail", text: "Le nom du plan est cliquable ; les boutons **+** ajoutent un plan à droite ou en dessous ([Alt] : avec son contenu) ; glissez pour déplacer, les poignées pour redimensionner." },
      { type: "fn", name: "Plan isolé", keys: ["view.fit"], text: "Cliquer sur le nom d'un plan l'isole : tout ce que vous faites reste dedans, les autres plans sont protégés. Taille écran cadre le plan isolé. Le sélecteur en haut du panneau Calques revient à **Tous les plans**." },
      { type: "fn", name: "Exporter", text: "Le plan actif par défaut, tous les plans en fichiers séparés, ou le document entier." },
      { type: "fn", name: "Convertir le plan en groupe", text: "Clic droit dans Calques." },
    ],
  },
  {
    id: "objets-dynamiques",
    title: "Les objets dynamiques",
    blocks: [
      { type: "p", text: "Un objet dynamique est un calque qui garde son image d'origine intacte. Vous pouvez le redimensionner dix fois, lui appliquer des filtres : rien n'est perdu." },
      { type: "fn", name: "Incorporé", text: "L'image est copiée dans le projet." },
      { type: "fn", name: "Lié", text: "Le projet pointe vers le fichier sur votre disque. Modifiez le fichier dans un autre logiciel : le projet se met à jour. Fichier déplacé ou renommé : « Fichier lié introuvable », puis **Relier…** Pour couper le lien : **Incorporer le fichier lié**." },
      { type: "fn", name: "Modifier le contenu", keys: ["layer.editSmartContent"], text: "S'ouvre dans un onglet ; [Ctrl] + [S] met à jour l'objet." },
      { type: "fn", name: "Filtres dynamiques", text: "Les filtres appliqués restent modifiables, avec leur masque." },
      { type: "note", text: "Astuce : déposez une image avec [Alt] enfoncé : elle arrive directement en objet dynamique." },
    ],
  },
  {
    id: "sauvegarde",
    title: "Sauvegarde, récupération et format .graf",
    short: "Sauvegarde et .graf",
    blocks: [
      { type: "fn", name: ".graf", text: "Le format de projet de Lumitype : calques, masques, textes, objets dynamiques, historique de récupération et temps de travail, tout est dedans." },
      { type: "fn", name: "Sauvegarde automatique", text: "Toutes les 3 minutes par défaut (de 2 à 60 dans les Préférences). Chaque document modifié est copié **à part**, historique compris : votre fichier .graf n'est jamais écrit à votre place. La copie est supprimée quand vous enregistrez ou fermez normalement." },
      { type: "fn", name: "Récupération", text: "Après une fermeture imprévue, la boîte **Récupération** propose chaque document : **Récupérer**, **Supprimer**, **Tout supprimer** ou **Plus tard**." },
      { type: "fn", name: "Documents ouverts", keys: ["doc.next", "doc.previous"], text: "Onglets en haut, avec les badges du mode de couleur et de l'épreuve. Les raccourcis passent au document suivant ou précédent." },
    ],
  },
  {
    id: "myref",
    title: "My ref",
    blocks: [
      { type: "p", text: "Votre bibliothèque d'images de référence, intégrée à l'accueil." },
      { type: "image", src: "myref", alt: "La bibliothèque My ref" },
      { type: "fn", name: "Dossier", text: "Vous choisissez un dossier sur votre disque : les images y restent en fichiers ordinaires, rien n'est enfermé dans une base." },
      { type: "fn", name: "Collections", text: "**Nouvelle collection…**, couverture, tri. Une image peut être dans plusieurs collections." },
      { type: "fn", name: "Survol", text: "En haut : la liste de ses collections, pour l'ajouter à une autre ou l'en retirer. En bas à droite : le **cœur** des favoris." },
      { type: "fn", name: "Favoris", text: "La collection de toutes les images au cœur rouge." },
      { type: "fn", name: "Visionneuse", text: "Collection, Tags, Note, Source, et **Intégrer au projet** pour l'ouvrir en calque dans le document actif." },
      { type: "fn", name: "Importer", text: "Déposez des fichiers depuis vos dossiers ou le navigateur. Les images supprimées vont dans le dossier .corbeille : rien n'est perdu." },
    ],
  },
  {
    id: "preferences",
    title: "Préférences et raccourcis",
    blocks: [
      { type: "p", text: "**Édition › Préférences**. Tout s'applique tout de suite." },
      { type: "image", src: "dialog-preferences", alt: "La boîte Préférences" },
      { type: "fn", name: "Général", text: "Langue ; unité des règles et des tailles." },
      { type: "fn", name: "Apparence", text: "Thèmes : OLED, Legacy, Charcoal, Gris, Purple, Burgundy, Navy Blue, Pine, Teal, Brown, Gold, Mint, Silver, Lime, Peach, Beige, Blanc. Couleur d'accent : 8 couleurs ou la vôtre en hexadécimal." },
      { type: "fn", name: "Raccourcis", text: "La disposition active, le nombre de raccourcis modifiés, et **Personnaliser…** pour les changer un par un." },
      { type: "fn", name: "Fichiers", text: "Sauvegarde automatique et son intervalle ; dossier d'enregistrement par défaut." },
      { type: "fn", name: "Performances", text: "**Mode portable** : économise mémoire et batterie, réglé automatiquement ou à la main. **Accélération matérielle** : la carte graphique calcule les aperçus. **Résolution dynamique** : affichage allégé pendant un geste, net au relâchement. **Mémoire réservée** à Lumitype, avec la jauge Documents / Historique / Cache / Carte graphique. **Disque de travail** : les anciennes étapes d'annulation passent sur le disque au lieu d'occuper la mémoire, avec son dossier et sa place maximale. **Cœurs utilisés**, **Mémoire vidéo**, **Étapes d'annulation**." },
    ],
    subsections: [
      {
        id: "raccourcis-a-connaitre",
        title: "Les raccourcis à connaître",
        blocks: [
          {
            type: "table",
            rows: [
              { label: "Nouveau · Ouvrir · Enregistrer", keys: ["file.new", "file.open", "file.save"] },
              { label: "Exporter", keys: ["file.exportPng"] },
              { label: "Annuler · Rétablir · Basculer", keys: ["edit.undo", "edit.redo", "edit.toggleLastState"] },
              { label: "Transformation manuelle", keys: ["edit.freeTransform"] },
              { label: "Nouveau calque · Dupliquer · Grouper · Fusionner", keys: ["layer.new", "layer.duplicate", "layer.group", "layer.mergeGroup"] },
              { label: "Masque d'écrêtage", keys: ["layer.toggleClipping"] },
              { label: "Tout sélectionner · Désélectionner · Intervertir", keys: ["select.all", "select.none", "select.invert"] },
              { label: "Masque rapide", keys: ["select.quickMask"] },
              { label: "Dernier filtre · Galerie", keys: ["filter.lastFilter", "filter.gallery"] },
              { label: "Taille écran · 100 %", keys: ["view.fit", "view.actualSize"] },
              { label: "Épreuve CMJN · Vue miroir", keys: ["view.proofCmyk", "view.mirror"] },
              { label: "Permuter les couleurs · Couleurs par défaut", keys: ["color.swap", "color.reset"] },
              { label: "Taille du pinceau", keys: ["brush.smaller", "brush.larger"] },
              { label: "Opacité du pinceau (0 = 100 %)", keys: ["=1 … 9", "=0"] },
              { label: "Document suivant · précédent", keys: ["doc.next", "doc.previous"] },
              { label: "Préférences · Paramètres de couleur · Raccourcis", keys: ["edit.preferences", "edit.colorSettings", "edit.shortcuts"] },
              { label: "Notice", keys: ["help.manual"] },
            ],
          },
        ],
      },
    ],
  },
  {
    id: "formats",
    title: "Formats de fichiers pris en charge",
    short: "Formats pris en charge",
    blocks: [
      { type: "fn", name: "Ouvrir", text: "PNG, JPEG, WebP, TIFF, AVIF, HEIC, RAW d'appareil photo (ARW, CR2, CR3, NEF, RAF, ORF, RW2, PEF, DNG…) et les projets .graf." },
      { type: "fn", name: "Exporter", text: "PNG, JPEG, WebP, TIFF, PDF et PDF/X-4, AVIF." },
      { type: "fn", name: "Pinceaux", text: ".abr Photoshop, .gbr Gimp, ou une image." },
      { type: "fn", name: "Formats et repères", text: "Vos catégories de formats et vos dispositions de repères s'exportent en .json." },
    ],
  },
];

/** Traduit un texte de la notice ; `contexte|texte` s'affiche sans son contexte (géré par `t`, voir i18n). */
export function tm(text: string): string {
  return t(text);
}

// ---- Images : par langue, sinon en anglais, sinon sans dossier de langue ; absente, le bloc est masqué.

const IMAGES = import.meta.glob<string>("../assets/manual/**/*.{png,jpg,webp}", { eager: true, query: "?url", import: "default" });
const EXTENSIONS = ["png", "jpg", "webp"] as const;

function imageIn(folder: string, name: string): string | null {
  for (const extension of EXTENSIONS) {
    const url = IMAGES[`../assets/manual/${folder}${name}.${extension}`];
    if (url) return url;
  }
  return null;
}

/** Adresse de l'image `name` dans la langue de l'interface (à lire dans un `$derived` : elle suit la langue). */
export function manualImage(name: string): string | null {
  return imageIn(`${i18n.locale}/`, name) ?? imageIn("en/", name) ?? imageIn("", name);
}

// ---- Ancres et recherche.

/** Ancre d'une ligne « fonction » : la section (ou sous-section) et son rang parmi les blocs. */
export const blockAnchor = (owner: string, index: number): string => `${owner}--${index}`;

export interface ManualHit {
  /** Ancre à atteindre. */
  anchor: string;
  /** Où se trouve la ligne (« Les menus › Fichier »), traduit. */
  where: string;
  /** Nom de la fonction ou titre, traduit. */
  title: string;
  /** Phrase, traduite et sans balisage. */
  text: string;
}

/** Toutes les entrées de la notice, dans la langue de l'interface : titres de sections et lignes « fonction ». */
export function manualEntries(): ManualHit[] {
  const out: ManualHit[] = [];
  const collect = (owner: string, where: string, blocks: ManualBlock[]) =>
    blocks.forEach((block, index) => {
      if (block.type !== "fn") return;
      const text = block.text ? plainText(tm(block.text)) : (block.items ?? []).map((item) => tm(item)).join(", ");
      out.push({ anchor: blockAnchor(owner, index), where, title: tm(block.name), text });
    });
  for (const section of MANUAL_SECTIONS) {
    const title = tm(section.title);
    out.push({ anchor: section.id, where: "", title, text: "" });
    collect(section.id, title, section.blocks);
    for (const sub of section.subsections ?? []) {
      const subTitle = tm(sub.title);
      out.push({ anchor: sub.id, where: title, title: subTitle, text: "" });
      collect(sub.id, `${title} › ${subTitle}`, sub.blocks);
    }
  }
  return out;
}

/** Comparaison sans accents ni casse : « ecretage » trouve « écrêtage ». */
const fold = (text: string): string => text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** Les entrées qui contiennent la recherche (titre d'abord, puis phrase), au plus `limit`. */
export function searchManual(query: string, limit = 30): ManualHit[] {
  const needle = fold(query.trim());
  if (!needle) return [];
  const entries = manualEntries();
  const byTitle = entries.filter((entry) => fold(entry.title).includes(needle));
  const byText = entries.filter((entry) => !byTitle.includes(entry) && fold(entry.text).includes(needle));
  return [...byTitle, ...byText].slice(0, limit);
}
