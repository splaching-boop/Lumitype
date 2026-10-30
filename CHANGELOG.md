# Journal des versions

Le mode d'emploi complet est dans l'application : **Accueil › Notice** (ou **Aide › Notice**, F1).

## 1.2.88 — 2026-10-23
Mise à jour du 29 octobre 2026 (même numéro de version) :
- **Pinceau et gomme en temps réel** : le trait est peint par la carte graphique directement dans la texture du calque, identique au pixel près à celui du moteur ; ce qui s'affiche pendant le geste reste tel quel, plus de « net puis flou » ni d'attente entre deux traits.
- **Annuler / Rétablir en rafale** : un seul saut dans l'historique pour tous les appuis, les pixels des zones annulées renvoyés avec la réponse ; miniatures relues au repos.
- **Plans de travail** : un import, un collage ou un nouveau calque fait « au-dessus » d'un plan (ou dans un plan isolé) entre dans ce plan, ajusté et centré sur son cadre, au lieu de couvrir tous les plans ; plus de liseré autour des plans inactifs (cadre du plan actif et table de montage calés au pixel) ; le fond du plan devient un calque de pixels ordinaire dès qu'on retire son cadenas.
- **Clic dans un plan de travail** : la sélection automatique respecte la découpe des plans et les masques ; un clic dans un plan ne saisit plus le fond du plan voisin (ni ne le décale en glissant), et le fond géré d'un plan ne se saisit plus au clic.
- **Repères par plan de travail** : un repère appartient au plan actif, n'est dessiné et n'aimante que dans son cadre, suit le plan quand il bouge et disparaît avec lui ; Disposition de repères, Verrouiller et Effacer agissent sur le plan actif seulement, et sont grisés sans plan actif ; les repères des anciens projets rejoignent le plan où ils tombent.
- **Raccourcis** : Ctrl+Alt+lettre (Vue miroir, Basculer le dernier état…) fonctionne sous Windows, où Ctrl+Alt est rapporté comme AltGr.
- **Peinture** : un calque qui ne se peint pas (remplissage, texte, forme, réglage, verrouillé…) est refusé avant le geste avec la raison, sans point parasite.
- **Linux** : paquets compilés contre la glibc 2.35 (`packaging/linux.sh --compat`, WSL Ubuntu-22.04 sous Windows), Flatpak sur GNOME 51 ; corrige « GLIBC_2.43 introuvable » sur Linux Mint 22.
- README : nouvelles captures (plans de travail, pinceaux, My ref, visionneuse, thèmes, galerie de filtres), liste des fonctionnalités et feuille de route à jour.
- **Notice intégrée dans l'accueil, en 4 langues** (français, anglais, espagnol, allemand) : toutes les fonctionnalités, avec sommaire, raccourcis réels de l'utilisateur, captures et aperçus vidéo des outils au survol ; recherche de l'accueil branchée dessus ; **Aide › Notice** (F1). Le journal et les tutoriels sont retirés.
- **Préférences repensées** : une ligne par réglage (nom, une phrase, commande à droite), cinq sections (Général, Apparence, Raccourcis, Fichiers, Performances), explications repliées sous « Comment ça marche » et « Réglages avancés », fenêtre de hauteur fixe, langue et unité des règles réglables ici, et « Rétablir les réglages par défaut » qui agit sur toutes les sections.
- **Thèmes** : treize nouveaux (Charcoal, Purple, Burgundy, Navy Blue, Pine, Teal, Brown, Gold, Mint, Silver, Lime, Peach, Beige) ; Vanille devient Beige.
- **Audit** : une vingtaine de bugs corrigés (vignettes My ref, fichiers corrompus, enregistrement sûr, toasts sous les fenêtres modales, boucle d'effet de la Sélection avancée…), glitchs du thème clair, et optimisations (cache de rendu, déplacement de sélection, pinceau, export sans figer l'interface).
- **Dépôt** rangé : noms en anglais, scripts de construction unifiés (`packaging/linux.sh`, `packaging/windows.bat`), fichiers de travail retirés.

## 1.2.8 — 2026-10-23
**Temps passé sur chaque projet**
- **Temps de travail** : en bas à gauche, enregistré dans le .graf, il reprend à l'ouverture
- **Temps actif seulement** : pause après 5 minutes sans geste
- **Nombre de calques**, avec le détail par type au survol

## 1.2.7 — 2026-10-22
**Collections, résolution et vérification plus justes**
- **My ref** : la visionneuse range l'image dans plusieurs collections, le cœur remplace l'étoile
- **Résolution** : 300 ppp par défaut et modifiable pour tous les modèles, et dans Format d'impression
- **Ctrl+0 / Ctrl+1** cadrent le plan isolé
- **Vérification** : les fonds perdus ne sont signalés que s'il reste du papier nu, l'encrage conseillé est indiqué

## 1.2.6 — 2026-10-22
**Plans de travail : isolés pour de bon, exportés un par un**
- **Plan isolé** : clics, traits, sélections et Suppr n'agissent plus sur les plans voisins cachés
- **Exporter › Plan de travail** : le plan où vous travaillez par défaut, tous les plans (un fichier chacun) ou le document entier

## 1.2.5 — 2026-10-22
**Impression : fonds perdus, encrage et PDF/X-4**
- **Nouveau document** : RVB ou CMJN, 300 ppp par défaut, fonds perdus, zone de sécurité, encrage max., et les unités mm, cm, pouces ou points
- **Affichage › Impression** : cadres, traits de coupe, alerte d'encrage hachurée et séparations C/M/J/N
- **Image › Corriger l'encrage…** : ramène sous 300 % en un clic
- **PDF/X-4** : boîtes de coupe, traits de coupe, noir du texte en 100 % N
- **My ref** : au survol, ajoutez l'image à vos collections et cœur animé

## 1.2.4 — 2026-10-20
**Pinceau : récents, taille et couleur dynamique**
- **Récemment utilisés** : vos 5 derniers pinceaux dans le panneau
- **Taille exponentielle** : le détail au début, la vitesse à la fin
- **Couleur dynamique** : une nuance très proche à chaque trait
- **Glisser un .abr** dans la fenêtre l'installe

## 1.2.3 — 2026-10-19
**Groupes en direct, sans attente**
- **Groupes en direct** : Ctrl+T et déplacement sans préparation
- **Ouverture claire** : une carte au centre montre l'arrivée des calques
- **Ctrl+Z en rafale** : un seul rafraîchissement pour toute la série
- **Netteté garantie** : jamais sous 35 % pendant un geste, nette dès que la souris s'arrête

## 1.2.2 — 2026-10-19
**Mode portable et cache sans gel**
- **Mode portable** automatique (carte graphique intégrée), ou choisi à la main
- **Cache de groupe sans gel** : préparé en fond, envoyé à la carte graphique par petites parts
- **Ctrl+T suivants quasi instantanés** : ce qui n'a pas changé est repris
- **Reprise automatique** si la carte graphique est perdue

## 1.2.1 — 2026-10-19
**Groupes optimisés et pinceau en direct**
- **Ctrl+T sur un groupe** : une image optimisée sert à tous ses déplacements
- **Groupes en Transfert** optimisés par paquets, rendu exact
- **Pinceau en direct** : vraie pointe, pression, dispersion, pinceau double, texture
- **Thème Blanc**, plans de travail créés d'un double-clic

## 1.2.0 — 2026-10-16
**20 filtres créatifs, l'export PDF et CMJN, et My ref**
- **Galerie de filtres** par catégories : Truchet (15 motifs), CRT, Risographie, Datamosh…
- **Export PDF** et **CMJN** avec profil ICC et poids estimé
- **My ref** : vos références rangées dans votre propre dossier
- **Outils empilés**, rotation de l'affichage (R), G = pot de peinture
- **8 réglages** et **calques de remplissage** en plus
- **Styles de calque** : biseautage, satin, motif ; groupes 14× plus fluides

## 1.2.0 (lots des 16 et 17/10/2026) — détail
## Couleurs : un seul CMJN (Édition › Paramètres de couleur…, Maj+Ctrl+K)
- Comme Photoshop : **un seul espace CMJN** (profil de presse, intention de rendu, compensation du point noir) pour
  l'épreuve écran (**Affichage › Épreuve CMJN**, Ctrl+Y, désormais un simple interrupteur), le mode
  **Image › Mode › Couleurs CMJN** et l'export CMJN (PDF, TIFF, JPEG).
- Profils proposés : les cinq livrés, **ceux installés sur l'ordinateur** (dont ceux de Photoshop, par exemple
  « U.S. Web Coated (SWOP) v2 », trouvés dans les dossiers d'Adobe et du système), et **Charger un profil ICC…**
  (recopié dans les réglages de Lumitype). Réglages par défaut : Coated FOGRA39, colorimétrie relative, compensation
  du point noir (« Réglages Photoshop « Europe » »).
- Les tables sont calculées par **LittleCMS** compilé avec le moteur (crate `lcms2`, rien à installer) : mêmes
  conversions que les logiciels de PAO. Un document en mode CMJN garde son espace ; un profil non livré est gardé
  dans le projet (`profils/<id>.icc`) et la copie de secours.
- Les anciens menus « Épreuve : FOGRA39… » et le choix du profil dans l'export disparaissent (l'export affiche le
  profil utilisé).

## Divers
- Groupe Remplissage : **Pot de peinture** en premier, **Dégradé** ensuite.
- **Suppr sur le dernier calque** : il est remplacé par un calque vide « Calque 1 » (une seule étape d'historique).

## Miniatures des RAW sous Linux
- Installé par le paquet `.rpm`/`.deb` : `/usr/share/thumbnailers/lumitype.thumbnailer`. Fichiers (Nautilus), Thunar,
  Nemo et Caja montrent la miniature des RAW (CR3, NEF, ARW, DNG, RAF… les formats que Lumitype lit) : aperçu
  embarqué par l'appareil, sinon développement. Lumitype s'en charge lui-même (`lumitype --miniature -s 256 photo.cr3
  miniature.png`), sans fenêtre. Pas de miniatures pour les `.graf`. L'AppImage ne s'installe pas dans le système :
  pas de miniatures avec elle.

## Installateur Windows aux couleurs de Lumitype
- Pages d'accueil et de fin sur fond noir chaud, panneau au logo, bandeau sombre en haut des autres étapes, icône de
  Lumitype, « Lumitype 1.2.0 » au bas de la fenêtre, cases de la page de fin lisibles. Choix de la langue au départ
  (français, anglais, espagnol, allemand). L'installateur MSI reprend les mêmes images.
- Modèle NSIS : `src-tauri/windows/installer.nsi`, copie de celui de Tauri 2.12.0 avec trois ajouts (chercher
  LUMITYPE) ; à reporter sur le nouveau modèle à chaque mise à jour de Tauri.

## Visionneuse Lumitype
- Une fenêtre à part pour regarder ses images et celles du dossier : `lumitype --visionneuse photo.jpg`, raccourci
  « Lumitype Visionneuse » (menu Démarrer sous Windows, menu des applications sous Linux), « Ouvrir avec » et
  Applications par défaut (elle n'est jamais imposée).
- JPEG, PNG, WebP, AVIF, GIF, BMP, TIFF, HEIC et tous les RAW. Molette sur l'image : zoom ; molette sur la pellicule :
  image précédente / suivante ; bouton et touche F11 : plein écran ; diaporama (F5) ; comparaison côte à côte ;
  histogramme, format, profil ICC, prise de vue (EXIF) ; Pivoter enregistré sans perte (JPEG par l'orientation EXIF,
  PNG / TIFF / WebP sans perte réencodés) ; Copier ; Imprimer ; Corbeille du système ; Afficher dans le dossier ;
  « Modifier dans Lumitype » ouvre l'image dans l'éditeur (celui déjà ouvert la reçoit).
- Sous Windows, `lumitype-visionneuse.exe` est un lien dur vers `lumitype.exe` (aucun doublon sur le disque).

### Filtres et images
- **Galerie de filtres** par catégories, 20 nouveaux filtres (Truchet et ses 15 motifs, CRT, Risographie, Datamosh…).
- **Menu Filtre** en sous-menus par catégorie, **Dernier filtre (Ctrl+F)**, Galerie (Ctrl+Alt+F).
- **Inverser** exact (255 − v, comme Photoshop) : plus de blocs blancs dans les hautes lumières.
- **RAW d'appareil photo** : ARW, CR2, CR3, NEF, RAF, ORF, RW2, PEF, DNG… (29 formats), visibles dans la fenêtre
  d'ouverture quelle que soit la casse de l'extension.
- **Fenêtre Développement RAW** (uniquement pour les RAW) : balance des blancs (préréglages, pipette, température,
  teinte), tonalité avec Auto, présence, bruit, netteté, vignetage, histogramme, Alt = zones écrêtées, série de RAW
  avec « Appliquer à tous les suivants ». Option **objet dynamique** : le RAW est gardé dans le projet et se
  redéveloppe par double-clic (Ctrl+Z possible). Un projet qui en contient s'enregistre en format 8, refusé
  proprement par les versions précédentes.

### Affichage et outils
- **Rotation de l'affichage (R)** sans déformation et fluide : l'image est composée une fois, seule la rotation change.
- **Redimensionnement / rotation / perspective d'un calque fluides** : aperçu par la carte graphique pendant le geste,
  calcul du moteur au relâchement, rendu final à la validation.
- **Texte** : plus de surlignage décalé au premier clic.
- **Suppr** sur la vignette du masque retire le masque (le calque reste).
- **Outils empilés** comme Photoshop, **G = Pot de peinture**, **Rotation de l'affichage** dans le groupe de la Main.
- **Déplacement de groupes fluide** : le groupe déplacé devient une image temporaire en cache (avec ce qui est dessous
  et dessus), assemblée par la carte graphique pendant le geste ; un seul calcul exact au relâchement. Indicateur
  « composition / déplacement » avec l'option « Afficher les images par seconde ».
- Onglet du document : « (CMJN/8) » ; épreuve CMJN discrète dans la barre d'état (pastille quadri).
- Menu Édition : « Remplir : premier plan / arrière-plan ».

### Calques
- **Bouton de réglage** : Courbes, Exposition, Balance des couleurs, Noir et blanc, Mélangeur de couches, Correction
  sélective, Isohélie, Seuil ; **calques de remplissage** Couleur unie, Dégradé, Motif.
- **Bouton fx** : Biseautage et estampage, Satin, Incrustation de motif, Options de fusion, Copier / Coller / Effacer
  le style. Menu Calque en sous-menus.

### Export
- **Export AVIF** : le plus léger (souvent 20 à 50 % de moins que le WebP), transparence, vitesse Rapide / Équilibrée /
  Maximale, poids estimé. La compilation demande `nasm` (`sudo dnf install nasm`).
- **Export PDF** (4 qualités, poids estimé) et **CMJN** pour PDF, TIFF et JPEG, avec « Inclure le profil ICC ».

### Accueil
- **Nouveautés** refaites : versions, guides rangés par thème, astuces, recherche ; notes de la 1.2.0 et trois
  nouveaux guides (Truchet, filtres rétro, galerie). Sur l'accueil, le volet Nouveautés est replié par défaut.
- **Formats rapides** rangés du plus récent au plus ancien (formats personnalisés compris).
- **Page Formats par plateforme** : Instagram, LinkedIn, YouTube, TikTok (publications, carrousels, stories,
  miniatures, bannières…), en Standard et HD ; mêmes catégories dans « Nouveau document ».
- **Logo officiel** sur l'accueil, la barre de titre, À propos et la bannière.

### My ref
- Bibliothèque d'images de référence dans votre propre dossier (collections = dossiers, infos en JSON à côté des
  images), titre manuscrit animé, glisser-déposer, visionneuse (renommer, déplacer, supprimer de la collection).
- Menus Plus et Trier refaits (tri mémorisé), pastille retour, **clic droit** sur une collection (Renommer, Ouvrir le
  dossier, Changer la couverture, Supprimer), confirmation dans l'application, **couverture au choix**.
- Vignettes en **WebP** (environ 2 fois plus légères ; les anciennes se convertissent seules).

Traductions anglais, espagnol, allemand à jour.

### Ajouts de fin de journée
- **Texte en bloc** (paragraphe) : cliquer-glisser avec l'outil Texte trace un cadre ; retour à la ligne automatique,
  justification, débordement signalé ; en édition, cadre en pointillés gris (poignées = taille du bloc, le texte se
  recompose) ; Ctrl+T reste un cadre bleu de transformation ; clic droit « Convertir en texte de paragraphe / de point ».
- **Image › Mode** : Bitmap… (seuil, diffusion, trame), Niveaux de gris (vrais fichiers gris à l'export), Couleurs RVB,
  Couleurs CMJN… (mode de travail : épreuve permanente, alerte hors gamme, export CMJN par défaut).
- **Filtre › Filtre Camera Raw…** (Maj+Ctrl+A) : la fenêtre de développement sur n'importe quel calque.
- **Recherche dans le panneau Calques** (Ctrl+Maj+F) : par nom, et par pastilles (texte, fx, masque, caché, mode:produit…).
- **Formats** : X, Reddit, Discord, signature e-mail, cartes de visite (européenne, américaine, japonaise, avec ou sans
  fond perdu) ; **catégories personnelles** (créer, renommer, ajouter ses formats, exporter / importer en .json).
- Galerie de filtres ouverte sur « Tous », sans Fluidité.
- Format .graf : version 9 pour les projets avec texte de paragraphe ou en mode Gris / Bitmap / CMJN (refusés
  proprement par les versions précédentes) ; les autres projets restent lisibles par la 1.1.0.
