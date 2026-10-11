# À faire — chantiers en attente

## Accélération GPU sous Linux

Objectif : que la carte graphique soit réellement utilisée et que la manipulation soit nettement plus fluide.

Fait en beta86 (29/09/2026) :
- Accélération WebKitGTK exigée au démarrage (`hardware_acceleration_policy = Always`, WebGL) — `src-tauri/src/main.rs`.
  `LUMITYPE_SOFTWARE=1` rend la main à WebKitGTK si un pilote pose problème.
- Tout le document composé en WebGL2 : masques, écrêtage, 21 modes de fusion, groupes isolés et en Transfert,
  calques de réglage (formules exactes du moteur, pas de table approchée), effets de calque (dessinés par le moteur,
  fondus par la carte graphique) — `ui/src/lib/gpu-plan.ts` (programme,
  traduction de `render.rs`) et `ui/src/viewport/gpu-layers.ts` (exécution). Parité vérifiée contre le moteur :
  `npm run test:gpu-parity` (44 documents, écart maximal 3/255).
- Aperçus sans moteur sur toute pile : déplacement (calques, groupes, pixels ou masque seuls), curseur d'opacité
  (calques et groupes), curseurs des réglages.
- Vulkan retiré : prototype « surface native » supprimé, `crates/gpu` limité à OpenGL (référence des formules, hors application).

Diagnostic Linux fait le 02/10 (Fedora 44, Radeon RX 7900 XT, écran 4K 240 Hz) : la carte est bien utilisée, mais
WebKitGTK plafonne à 60 i/s (GNOME Web aussi ; Firefox atteint 240). X11 forcé reste le meilleur réglage (Wayland
natif : 27 i/s ; sans DMABUF : 7 i/s). Lumitype tenait 50 i/s sous ce plafond.

Fait en beta89 (02/10/2026) :
- Résolution dynamique pendant les gestes (`ui/src/lib/resolution.ts`) : le canevas passe à ×0,75 puis ×0,5 quand
  les images dépassent le rafraîchissement, net au relâchement. Réglage dans Préférences, facteur dans la barre d'état.
- Fenêtre native OpenGL (`src-tauri/src/native_view.rs`, reprise du prototype de septembre, Vulkan retiré) : la zone
  de travail dessinée hors de la webview, synchronisée sur l'écran. Automatique sous Linux, décochable dans les
  Préférences ; témoin de plantage qui la désactive seule. Non testée sur une vraie carte depuis cette machine.

Corrections beta89b → beta89e (02/10/2026), d'après les essais sur la machine Linux :
- b : « display handle does not match » — tao ouvrait un nouveau `XOpenDisplay` à chaque lecture du handle ;
  les handles sont lus une fois et partagés (`Handles`).
- c/d : fil de rendu résilient (`catch_unwind`, erreurs wgpu consignées, retrait après 1 s d'échecs), fenêtre
  jamais focalisable (le clavier restait à la webview), cadence non émise au repos.
- e : **tuiles composées par la carte invisibles (damier partout)** — cause confirmée dans le code de wgpu-hal
  (`gles/device.rs` : « Unable to create a sampled texture binding for non-zero array layer ») : OpenGL ne peut pas
  lier une seule couche d'un tableau de textures comme texture 2D, la liaison échouait et la tuile se dessinait
  transparente. La fenêtre native échantillonne maintenant le tableau entier (`texture_2d_array`, couche choisie
  dans le shader) — `crates/gpu/tests/display.rs` reproduit ce dessin sur OpenGL logiciel et le compare à la
  composition.
- f : l'application entière reproduite ici sous Xvfb (Mesa llvmpipe, `npm run tauri build -- --no-bundle` puis
  `LUMITYPE_NATIVE_LOG=1 target/release/lumitype fichier.graf`), trois causes de plus trouvées et corrigées, chacune
  reproduite hors de Lumitype dans `crates/gpu/examples/window_quad.rs` (`LUMITYPE_REPRO_WIN=…`) :
  1. **GTK efface l'image OpenGL** : sous X11, GTK 3 dessine la fenêtre dans un double tampon puis le colle sur la
     fenêtre à chaque redessin (expose, déplacement, redimensionnement) — l'image présentée juste avant disparaît
     (écran noir, ou couleur du thème). Corrigé : plus de double tampon (`gtk_widget_set_double_buffered(false)`)
     et signal `draw` arrêté.
  2. **La fenêtre passait derrière la principale** : transiente, mais rien ne la remontait quand la principale
     reprenait le focus (sans gestionnaire de fenêtres, elle restait dessous). Corrigé : `gdk_window_raise` après
     chaque affichage, déplacement, redimensionnement et focus de la principale.
  3. **Aucune commande ne fonctionnait** : le masque d'entrée (la souris traverse la fenêtre) posé avant `show()`
     est perdu quand GTK affiche la fenêtre ; reposé après chaque affichage, il survit ensuite.
  Vérifié sous Xvfb : image complète composée par la carte, déplacement d'un calque à la souris avec repères
  intelligents, retrait sous les menus, raccourcis. `LUMITYPE_NATIVE_LOG=1` décrit sur le terminal chaque demande
  de la webview, l'état GTK de la fenêtre (visible/mappée) et les trois premières images.

- g (03/10, après l'essai « beaucoup plus fluide, quelques artefacts, 40-45 i/s ») :
  - **Carrés au mauvais contenu** : une tuile dont la couche vidéo était reprise par une autre (hors écran un instant,
    pendant un zoom) restait notée « à jour » et n'était jamais redemandée ; le trou était comblé par une tuile
    d'attente d'un autre niveau, composée avant la retouche. Corrigé : tuile évincée = oubliée (`evicted`), tuiles
    d'attente des autres niveaux jetées dès qu'une retouche les touche, réserve passée de 128 à 256 couches
    (un écran 4K à 100 % en montre 160).
  - **Cadence** : à chaque révision, la fenêtre native redemandait TOUTES les tuiles visibles (135 en 4K) ; le
    moteur tient maintenant un journal de ce que chaque révision a changé (`Session::changes_since`, test
    `changes_since_reports_touched_tiles…`) et seules les tuiles touchées sont refaites — un déplacement de petit
    calque n'en recompose que 3 à 8. Boucle de rendu qui attend (1 ms) au lieu de tourner à vide pendant que les
    tuiles arrivent. `LUMITYPE_NATIVE_LOG=1` imprime chaque seconde : tuiles composées (carte / processeur), temps
    de composition, envois, temps de dessin — c'est ce qu'il faut lire pour savoir où passent les 40-45 i/s.
  - Piste suivante si la cadence reste basse : les calques décalés d'un nombre de pixels non multiple de 256 sont
    ré-extraits (`read_rect`) et renvoyés à la carte à chaque révision ; garder les tuiles de chaque calque
    résidentes et décaler dans le shader (comme l'affichage WebGL) supprimerait ces envois.

- h (03/10, mode normal — webview) : mesuré chez l'utilisateur, `requestAnimationFrame` tourne à 240 i/s (script
  `webkit-cadence.py`) ; la limite du mode normal est le coût par pixel d'une image 4K (fenêtre réduite : 160-200 i/s).
  Fait :
  - **Piles** (`STACK_SHADER`, `stackRuns`) : les suites de calques Normal sans masque sont fondues en une passe de
    8 calques au plus (une lecture-écriture de la cible au lieu d'une par calque) ; le chemin « direct » passe aussi
    par les piles. Parité moteur inchangée (écart max 3).
  - **Recomposition partielle** (`partialRegion`) : pendant un aperçu de déplacement (ou d'opacité), seule l'union
    des anciennes et nouvelles emprises du calque est recomposée dans la cible 0, conservée d'une image à l'autre ;
    identique au recalcul complet au pixel près sur les 43 documents à programme (vérifié dans `test:gpu-parity`).
  - **Mesure** : « Afficher les images par seconde » ajoute « JS x ms · carte y ms » (temps du code, puis `gl.finish`).
  - Cibles RGBA8 essayées (`GpuLayers.floatBuffers = false`, `PARITY_FLOAT=0`) : parité partout sauf mode
    Saturation (écart 38) → demi-flottants gardés.
  - Fenêtre native : ciseaux au document (plus de bande de tuiles sous le document).

- beta90 (03/10) :
  - **Déplacement du plan de travail (« freeze », 35 i/s, pire 50 ms)** : chaque événement souris (jusqu'à 1 000 par
    seconde) déclenchait un rendu, une mise à jour des règles (réallocation des canevas + `getComputedStyle`) et un
    envoi à la fenêtre native. Tout est maintenant regroupé par image (`queuePan`, `queuePointer`, `queueNativeSpec`
    dans `Viewport.svelte`), les règles ne redessinent que si la caméra ou la taille change et leur marqueur est un
    élément DOM déplacé par `transform` ; cibles WebGL de réserve conservées par taille (`spareTargets`).
  - **Affichage › Épreuve CMJN** (Ctrl+Y) : simulation d'impression quadri (encres C, M, J, N aux teintes réelles,
    sans profil ICC ni gain de point), appliquée dans la zone de travail WebGL (`drawProof`, `PROOF_SHADER`) et dans
    la fenêtre native (`proofed()` en WGSL) ; pastille « CMJN » dans la barre d'état. Document et exports restent en
    RVB. Vérifié sous Xvfb : les deux rendus suivent la formule à 1/255 près (hors repères). Piège trouvé à l'essai :
    le canevas est créé sans alpha, la copie d'écran vers une texture RGBA8 était refusée (image noire) → RGB8.
  - **Clic droit sur un calque ou un dossier › Couleur** : sept teintes (rouge, orange, jaune, vert, bleu, violet,
    gris) ou aucune ; enregistrée dans le `.graf` (`LayerLabel`, `set_label`, annulable), appliquée à toute la
    sélection si le calque visé en fait partie ; un dossier teinte aussi ses calques (barre et fond légers).
  - **Menu Copyright › Métadonnées par défaut…** : prénom, nom, mention de copyright (proposée « © année Prénom
    Nom »), site ou contact, mots-clés, description, cases « inscrire », « Créé avec Lumitype », « date de l'export » ;
    enregistrées dans `metadata.json`. Inscrites par le moteur dans chaque export (`crates/engine/src/metadata.rs`) :
    PNG (`iTXt` Author/Copyright/Description/Keywords/Source/Software/Creation Time + XMP), JPEG (APP1 EXIF +
    APP1 XMP), WebP (`VP8X` + `EXIF` + `XMP `), TIFF (IFD0 réécrit : Artist, Copyright, ImageDescription,
    Software, DateTime, XMP tag 700). La fenêtre Exporter montre l'auteur, permet de décocher l'inscription et de
    donner une description et des mots-clés propres à l'image ; le poids affiché les compte. Relus sous Xvfb avec
    PIL et ImageMagick dans les quatre formats.
  - TIFF : tag `ExtraSamples` (alpha non associé) ajouté et `Predictor` inutile retiré — libtiff (Photoshop, GIMP,
    ImageMagick) se plaignait des deux à chaque export.
  - **Tab dans un texte** insère une tabulation (taquets tous les 8 espaces, `TAB_WIDTH`) au lieu de quitter la saisie.
  - **Raccourcis derrière une fenêtre modale** : reproduit sous Xvfb — avec la fenêtre Exporter ouverte, les frappes
    atteignaient le document (un chiffre réglait l'opacité, « G » changeait d'outil, Suppr aurait supprimé le calque).
    `onKeyDown` ignore désormais toute frappe tant qu'un `<dialog>` est ouvert. Et une modification longue (filtre
    appliqué à un grand calque…) s'affiche dans la barre d'activité au-delà de 0,4 s au lieu de laisser croire que
    Suppr « ne fait rien » (les modifications d'un document se suivent dans l'ordre). Le cas exact signalé (« Suppr ne
    supprime pas le calque si une fenêtre de filtre ou d'export est ouverte ») reste à préciser avec l'utilisateur.
  - Fenêtre native : si elle reçoit le clavier (focus), elle le rend aussitôt à la fenêtre principale (`keyboard_taken`).

- beta90b (03/10, essais en mode normal WebGL : « cadre bleu décalé en bougeant vite, 45 i/s, légère latence ») :
  - **Cadre bleu en retard d'une image sur l'image (en avance, en fait)** : la caméra changeait dans un
    `requestAnimationFrame` (`queuePan`, animation du zoom) et le dessin WebGL était redemandé pour l'image SUIVANTE,
    alors que le cadre (DOM) se mettait à jour tout de suite. Mesuré dans Chromium : 4–5 px d'écart à chaque image
    d'un glisser à 4,5 px/image, 0 après correction. Une seule boucle par image maintenant (`beforeFrame` dans
    `Viewport.svelte`) : tâches de l'image (déplacement cumulé, pas de zoom, pointeur), `flushSync`, puis dessin
    dans le même rappel. Une image de latence en moins sur tout ; la description de la fenêtre native part aussi
    dans la même image.
  - **Cadre resté sur l'ancien calque** : calque A sélectionné, glisser directement sur B (sélection automatique) →
    B bouge mais le cadre de A suivait la souris (l'effet qui recadre est suspendu pendant un glisser). Le cadre
    est retiré dès que la sélection change et reposé sur B (décalé de ce qu'il a déjà parcouru) quand son étendue
    arrive.
  - **Calque et masque décalés un instant après un déplacement sans le masque** : les deux textures étaient
    rechargées et le calque restait à l'ancienne place en attendant. Seule la partie réécrite (masque) est relue,
    les pixels suivent la position immédiatement (`invalidate`, test dans `gpu-layers.test.mjs`).
  - 45 i/s avec ou sans accélération matérielle : confirme que la limite n'est pas la composition mais le transport
    de l'image 4K par WebKitGTK (voir h) ; seule la fenêtre OpenGL la contourne.

- beta91 (03/10) — **diagnostic final de la cadence sous Linux** (mesures chez l'utilisateur, 4K 240 Hz) :
  carte graphique jamais saturée (44 %, « eff 10 % »), processeur à 90 % libre, image composée en 2 ms ; carte
  forcée à pleine fréquence : aucun changement ; `glxgears` plein écran via XWayland : 240 i/s ; **page WebKitGTK
  presque vide : 240 i/s en petite fenêtre, 49 i/s en plein écran** ; Chromium et Firefox plein écran : 240 i/s.
  → Le plafond de 45 i/s est celui de WebKitGTK en plein écran 4K, indépendant de Lumitype (la fenêtre native
  OpenGL plafonnait aussi parce qu'elle reçoit la caméra au rythme de la webview).
  - **Variante Chromium (CEF)** : `npm run tauri build -- --no-bundle --features cef`. Runtime CEF de Tauri
    (non encore publié par Tauri) copié dans `crates/tauri-runtime-cef` et adapté à Tauri 2.12 (voir son
    `PORTAGE-LUMITYPE.md`). Alias `rt::Runtime` / `rt::AppHandle` (`src-tauri/src/rt.rs`) : le reste du code ne
    dépend pas du moteur. Sous Chromium : pas de fenêtre native OpenGL (inutile), pas de réglages WebKitGTK,
    `LUMITYPE_WAYLAND=1` pour essayer Chromium en Wayland natif. Les binaires CEF (~300 Mo) sont téléchargés à la
    première compilation et copiés à côté de l'exécutable (`rpath $ORIGIN`). Vérifié ici par `cargo check
    --features cef-check` (le téléchargement de CEF est bloqué dans l'environnement de développement) : la
    compilation complète et l'exécution restent à faire sur la machine Linux.
  - beta92 : **export/linux.sh construit la version Chromium** (paquets deb, rpm, arch, flatpak, AppImage, tar).
    WebKitGTK n'est plus compilé dans cette version (fonction `webkit` par défaut, retirée par
    `--no-default-features`). Les fichiers de Chromium sont rassemblés juste avant l'emballage
    (`export/cef-paquet.sh`, `build.beforeBundleCommand` de `src-tauri/tauri.cef.json`) et installés dans
    `/usr/lib/Lumitype/` ; l'exécutable les trouve par son `rpath` (`$ORIGIN/../lib/Lumitype`) et désigne leurs
    données à Chromium (`cef_files_switches`). `./export/linux.sh --essai` : compilation sans paquet puis lancement.
    Non vérifié ici au-delà du téléchargement de CEF (bloqué) : l'emballage d'un dossier entier par `files`
    (deb/rpm/AppImage) et le démarrage depuis /usr/bin restent à confirmer sur la machine Linux.
  - Restent à faire avec Chromium (en attente d'accord) : plantage sur Ctrl+O (boîte GTK → portail XDG),
    plafond de 60 i/s (Wayland natif ?), retrait de la fenêtre native OpenGL et des optimisations WebKit, aperçus
    vidéo des outils (vides, déjà avant Chromium).
  - À reprendre si Chromium est retenu : le redessin à chaque mouvement de souris introduit en beta90b
    (`queuePointer` passé dans `beforeFrame`) — à remettre hors de la boucle de dessin.

- beta93 (03/10, Chromium confirmé chez l'utilisateur : « fonctionne parfaitement à quelques exceptions ») :
  - **Chromium livré dans les paquets** (`export/linux.sh`, voir beta92) ; `libcef.so` débarrassée de ses symboles de
    débogage (1,4 Go → 311 Mo pour tout Chromium), langues de Lumitype seulement ; `.rpm` compressé en zstd.
  - **Fenêtre native OpenGL retirée** (1 900 lignes de `native_view.rs`, `crates/gpu`, commandes, réglage des
    Préférences, pastille « OpenGL », `native-overlay`, `overlay-selector`), ainsi que les réglages propres à WebKitGTK
    (X11 forcé, accélération exigée, `webkit2gtk`). Fonctions Cargo : `wry` (webview du système, par défaut, Windows)
    et `cef` (Chromium, Linux via export/linux.sh) ; alias `rt::Runtime` / `rt::AppHandle`.
  - **Recomposition partielle retirée** (gpu-layers) : tout le document visible est recomposé à chaque image.
  - **Calque et masque échangés ensemble** sur la carte (`place`/`staged` dans gpu-layers) : les pixels relus d'un
    calque attendent son masque relu, et inversement — plus de pixels neufs sous l'ancien masque. Test ajouté.
  - **Pointeur hors de la boucle de dessin** : bouger la souris sans cliquer ne redessine plus la zone de travail
    (régression beta90b, cause des 240 i/s perdus au repos).
  - **Zoom à la molette** : un seul modèle, cible + glissement exponentiel (45 ms) ; le défilement doux de Chromium
    (petits et grands pas mélangés par cran) faisait repartir l'animation et le zoom arrière remontait par moments.
    Vérifié dans Chromium : suite de zooms monotone.
  - **Films des bulles d'outil et des guides** : lus depuis la mémoire (`loadFilm`, adresse `blob:`) — le protocole de
    Tauri sert les fichiers d'un bloc sans requêtes partielles et les lecteurs vidéo refusaient. CSP : `media-src
    'self' blob:` et `connect-src 'self'` (le `fetch` du film était bloqué). Vérifié dans Chromium (bulle et lecteur).
  - **Images de l'interface non glissables** (`onDragStart`) : Chromium joignait une image de l'aide glissée comme
    fichier, que Lumitype importait.
  - **Plantage sur Ctrl+O** : boîtes Ouvrir/Enregistrer par le portail de bureau (`tauri-plugin-dialog` en
    `xdg-portal`, sans GTK) — GTK et Chromium se disputaient la fenêtre X11 et Lumitype quittait après une seconde.
    À confirmer sur la machine Linux (pas de portail sous Xvfb).
  - Plafond de 60 i/s sous Chromium (X11/XWayland) : à mesurer ; essai `LUMITYPE_WAYLAND=1`.
  - Confirmé par l'utilisateur (03/10) : Ctrl+O, films, zoom OK ; toujours 60 i/s, stable (« pas grave »).

- beta94 (04/10) :
  - **Épreuve CMJN sur vrai profil de presse** : le modèle d'encres maison (`proofCmyk` en GLSL) est remplacé par une
    table 3D 33³ calculée par LittleCMS depuis `export/profils/FOGRA39L_coated.icc` (ISO Coated v2, libre de droits),
    sRGB → CMJN (relatif + compensation du point noir) → sRGB. Shader : une lecture de `sampler3D` trilinéaire.
    Script `export/lut-cmjn.py` (`--profil`, `--intention`, `--taille`) ; fichier `ui/src/assets/proof/fogra39.bin`
    (108 Ko), chargé à la première épreuve. Vérifié : écart ≤ 2,7/255 avec la référence CPU ; rendu conforme à la
    mire fournie par l'utilisateur (vert 0,255,0 → 102,179,56 ; bleu → 53,61,144).

- beta94b (04/10, retours Chromium) :
  - **Fenêtre maximisée au premier lancement** : sous X11, `maximized: true` n'est qu'un message au gestionnaire de
    fenêtres, ignoré tant que la fenêtre (créée cachée derrière la bannière) n'est pas affichée. `reveal_main_window`
    redemande `maximize()` après `show()`.
  - **Glisser une image depuis un navigateur** : le glisser visait la fenêtre winit (`XdndAware`), qui ne garde que les
    `file://`. Le runtime pose `XdndProxy` (fenêtre winit → fenêtre Chromium, et Chromium → elle-même) dans
    `reparent()` : Chromium reçoit le glisser entier et la page retrouve `dataTransfer` (`onDrop` d'App.svelte).
    Vérifié par `cargo check --features cef-check` seulement ; à confirmer sur la machine Linux.
  - **Alt + clic entre deux calques** (écrêtage) : le geste est lu sur le clic lui-même (`onRowPointerDown`, Alt
    enfoncé, à 5 px du bord d'une ligne) au lieu d'un bouton qui n'existait que si l'état d'Alt avait été vu par une
    frappe ou un mouvement ; la ligne bleue reste un simple repère. Non reproduit ici (Chromium sans écran : OK avant
    comme après) ; si Alt reste ignoré sous Chromium/X11, vérifier Alt + clic sur l'œil (solo) et Alt+Ctrl+G.

- beta94c (04/10) :
  - **Glisser-déposer sous Linux/Chromium (fichiers d'un dossier, images d'un site)** : Mutter remet le glisser à la
    fenêtre de premier niveau (winit), pas à la page — `XdndProxy` (beta94b) n'y change rien sous GNOME. L'interface
    écoute donc aussi les événements de fenêtre Tauri (`onDragDropEvent` dans App.svelte → `importLocalPaths` ou
    `importFromUrl`, anti-doublon de 1,5 s avec le dépôt de la page, zone de dépôt allumée à l'entrée).
  - **winit-x11 copié et corrigé** (`crates/winit-x11`, `[patch.crates-io]`, voir `PORTAGE-LUMITYPE.md`) : adresses
    `http(s)://` acceptées ; pour un glisser web, le HTML est lu ensuite et son `<img src>` passe devant l'adresse de
    la page. Tests unitaires (`cargo test --lib dnd::tests` dans le crate) ; branchement vérifié dans Chromium sans
    écran (événements Tauri simulés). Vrai glisser à confirmer sur la machine Linux.

- beta95 (05/10) — **Chromium embarqué aussi sous Windows** (même runtime que Linux, pour un comportement et des
  correctifs communs) :
  - `export/windows.ps1` : moteur `chromium` par défaut (`--features cef --config src-tauri\tauri.cef-windows.json
    -- --no-default-features`), CMake vérifié (celui des Build Tools, sinon installé), WebView2 seulement avec
    `-Moteur webview2` ; option `-Essai` (compile et lance, comme `linux.sh --essai`) ; version portable en
    `Lumitype-portable.zip` (Chromium doit être à côté de l'exécutable).
  - `export/cef-paquet.ps1` (équivalent de cef-paquet.sh) et `src-tauri/tauri.cef-windows.json` (fichiers de
    Chromium posés à côté de Lumitype.exe par NSIS et MSI, `webviewInstallMode: skip`). Le dossier `cef-paquet` est
    créé vide avant la compilation : tauri-build vérifie les ressources dès ce moment.
  - Runtime : `windows` 0.61 → 0.62 (types `HWND` de tauri-runtime 2.12). Compilation croisée Windows vérifiée ici
    (Chromium et WebView2) ; **premier lancement réel à faire sous Windows** (code Windows du runtime jamais exécuté).

- beta95b/c (05/10) — premiers essais Windows :
  - `cef-dll-sys` impose Ninja à CMake : `windows.ps1` le trouve (Build Tools) ou l'installe (winget, sinon zip).
  - **Fenêtre blanche** : l'exécutable n'avait que le manifeste minimal de Tauri, sans la section `supportedOS`
    qu'exige Chromium. Manifeste complet `src-tauri/windows/lumitype.exe.manifest` (repris de cefsimple, ASCII
    seulement pour rc.exe), intégré par build.rs (`WindowsAttributes::app_manifest`). Intégration vérifiée dans les
    ressources compilées (compilation croisée).

- beta95d (06/10) — fenêtre toujours blanche sous Windows (7 processus Lumitype : Chromium démarre) :
  - **Chargement direct des pages de l'application** : le runtime créait le navigateur sur une page interne
    blanche et n'allait à la vraie page qu'après une réponse DevTools, qui ne venait pas sous Windows.
    Filet de sécurité de 1,5 s pour les autres adresses.
  - **CEF servi par la boucle winit** sous Windows (toutes les 4 ms), en plus de la pompe à messages jamais éprouvée.
  - **Journal Chromium** automatique : `%LOCALAPPDATA%\dev.imageeditor.app\chromium.log` (Linux :
    `~/.cache/dev.imageeditor.app/chromium.log`). À envoyer si la fenêtre reste blanche.
  - Voir `crates/tauri-runtime-cef/PORTAGE-LUMITYPE.md`. Compilation croisée Windows et Linux vérifiées.

- beta95e (07/10) : **WebView2 redevient le défaut sous Windows** (`windows.bat`), Chromium embarqué en option
  `-Moteur chromium` tant qu'il n'est pas au point (beta95d à essayer, puis journal `chromium.log`).

- beta96 (10/10) — **Chromium abandonné sous Windows**, retrait de tout ce qui n'y servait qu'à ça :
  `export/cef-paquet.ps1`, `src-tauri/tauri.cef-windows.json`, `src-tauri/windows/lumitype.exe.manifest` (build.rs
  revient à `tauri_build::build()`), options `-Moteur`/CMake/Ninja/portable zip de `windows.ps1` (garde `-Essai`),
  chargement direct et pompe Windows du runtime (revenus à l'état beta95c). Gardés : `windows` 0.62 dans le runtime
  et le journal `chromium.log` (utile sous Linux). Windows : WebView2, comme avant beta95.

- beta97 (10/10) :
  - **Jauge de chargement** (barre d'état, variante « mini-jauge ») : `GpuLayers.progress()` compte les textures de
    calques posées / attendues depuis le dernier repos de la file ; le viewport la relève toutes les 100 ms et ne
    l'affiche qu'après 300 ms (`app.loading`), fondu à la fin. `StatusBar.svelte` : « Chargement ▮▮▮▯ 62 % ».
  - **Polices incluses** (`src-tauri/fonts/`, 45 familles, 61 Mo, OFL/UFL, voir LISEZ-MOI.md) : ressources Tauri,
    recensées par le moteur AVANT celles du système (`engine::text::set_bundled_fonts_dir`, base fontdb construite à
    la main) ; `FontFamily.bundled` ; sélecteur en sections « Incluses » / « Système », échantillon chargé depuis le
    fichier (`bundled_font_file` → `FontFace`, à l'apparition de la ligne). Instances statiques (cosmic-text ne règle
    pas les axes variables). Test moteur `bundled_fonts_are_listed_and_rendered` ; vérifié en vrai sous Xvfb.
  - **Bords droits partout** : 251 `border-radius` passés en variables (`--radius-xs/s/m/l/pill`, tokens.css), toutes
    à 0 ; anciennes valeurs en commentaire pour revenir en arrière d'un chiffre. Les cercles (50 % : pastilles,
    poignées, curseurs) restent ronds. Bannière de lancement alignée.

- beta98 (10/10) — **Lumitype** :
  - **Renommage Grafism → Lumitype** partout (nom du produit, titres, binaire `lumitype`, paquets, `/usr/lib/Lumitype`,
    `lumitype.desktop`, type MIME `application/x-lumitype`, variables `LUMITYPE_*`, scripts d'export, docs, locales,
    monogramme « LT »). Conservés : extension `.graf`, identifiant `dev.imageeditor.app` (préférences, cache et
    récents restent en place), marqueur interne `grafism-recuperation` des copies de secours, crate `image-editor`,
    logo PNG de l'auteur (à refaire par lui). Sur une machine où Grafism était installé : `sudo dnf remove grafism`
    (ou `apt remove grafism`) une fois, le nouveau paquet ne le remplace pas.
  - **Thème « affiche » (E)** : noir chaud (`#121111`…`#221f20`), accent unique rouge corail `#ec2a3c` (fini le bleu),
    `--accent-strong/-deep/-shadow`, boutons principaux rouges ; accueil : bande rouge fondue sur la barre latérale,
    filet rouge en tête de la carte Nouveau, onglet actif à filet, survols à liseré rouge, pastille LT rouge.
  - **Panneau Calques** façon Photoshop, dans le thème : lignes jointives de 34 px séparées d'un filet, colonne œil
    fixe (verte sur la ligne active, rouge si masqué), retrait 22 px/niveau jusqu'au 5ᵉ puis 12 px, vignettes 30 px au
    format de l'image, texte 11,5 px, dossier redessiné en contour (`assets/icons/group.svg`, l'ancien dans le backup).

- beta98c (10/10) :
  - **Œil des calques** : la couleur de la cellule est l'étiquette de couleur du calque (comme Photoshop), pas la
    sélection ; calque masqué = icône grisée. Plus de vert à la sélection.
  - **Barre de titre fusionnée** : la fenêtre principale n'a plus de barre du système (`decorations: false`,
    `shadow: true` dans `tauri.conf.json`). La barre de menus (40 px) porte la marque LT + Lumitype à gauche, les
    menus, Exporter, puis réduire / agrandir / fermer (`WindowControls.svelte`, Fermer rouge au survol). Saisir le
    vide de la barre déplace la fenêtre, double-clic agrandit ou restaure (`MenuBar.svelte`). Huit zones invisibles de
    6 px le long des bords redimensionnent la fenêtre (`WindowEdges.svelte`, cachées quand elle est agrandie ; sous
    Windows la bordure native reste aussi active grâce à `shadow`). Permissions `core:window:allow-minimize /
    toggle-maximize / is-maximized / start-dragging / start-resize-dragging` dans `capabilities/default.json`.
    Même traitement Linux et Windows ; Fermer passe par la demande de fermeture habituelle (propose d'enregistrer).
  - **Liste des polices** : parcourir au clavier faisait défiler la liste sous la souris immobile, et le survol
    reprenait la main (sélection qui remonte avant de redescendre). Le survol est ignoré tant que la souris n'a pas
    réellement bougé (`pointerMoved`, `FontPicker.svelte`).
  - **Onglet Propriétés** : un calque de texte ou de réglage qui devient actif (créé depuis l'onglet Réglages ou le
    menu Calque, clic dans la liste des calques ou sur le plan de travail) ouvre l'onglet Propriétés (`App.svelte`).
    Avant, créer un réglage laissait l'onglet Réglages affiché : ses curseurs semblaient ne jamais apparaître.
  - **Curseur de rotation** redessiné : arc ouvert avec une flèche à chaque bout, tourné selon l'angle survolé (le
    sommet de l'arc pointe vers l'extérieur du cadre). Dessiné par `lib/cursors.ts` (un SVG par degré, en cache) ;
    le jeton `--cursor-rotate` de tokens.css disparaît.
  - **Liste des styles** (graisses) du texte : plus un `<select>` natif mais une liste façon polices
    (`StylePicker.svelte`) : chaque ligne rendue dans sa graisse, aperçu sur le texte au survol et aux flèches,
    retour au style d'avant en quittant la liste, Échap. Deux graisses de même nom (Roboto Thin déclaré 250 et
    Light 300) sont distinguées par leur poids.
  - **Alt + pot de peinture / dégradé** : pipette temporaire, comme avec le pinceau (`Viewport.svelte`, `sampling`).
  - **Fenêtre Exporter** plus haute (jusqu'à 900 px) et plus large (1280 px) : la colonne des réglages tient sans
    défiler dès 1080p.

- beta99 (10/10) :
  - **Déplacement des dossiers sans rechargement** : un dossier bouge d'un bloc, masque compris (`movedParts()` dans
    `Viewport.svelte` répond « both » pour un groupe ; `Document::translate_parts` renvoie sur `translate_layers`
    pour un groupe : le masque suit par la position, sans être recopié ; avant, la copie décalée du masque faisait
    recharger sa texture à chaque pas, contenu laissé sur place). **Textures refusées mémorisées**
    (`GpuLayers.rejected`, `stillRejected`) : un calque trop grand ou hors budget n'est plus relu en entier à chaque
    image pour être refusé à nouveau (jauge « Chargement » en boucle, gestes retardés) ; le refus est oublié dès que
    le calque, le budget ou la place libre change. **Miniatures différées** (`lib/engine.ts`,
    `MOVED_THUMBNAILS_AFTER_MS` = 250 ms) : un calque seulement déplacé (`dirty.repainted` absent de sa liste)
    attend un temps de repos avant que sa miniature ne soit relue, au lieu d'une relecture de tout le dossier à chaque
    pas. **Emprise du masque** (`Session::refresh`) : après un déplacement, les tuiles à recomposer sont celles de
    l'emprise des pixels ET celle du masque, avant et après (un dossier n'a pas de pixels : seule l'emprise de son
    masque compte ; sans elle, ce qu'il cachait restait en place). Le moteur vérifie la taille d'une texture AVANT de
    lire ses pixels (`LayerFetcher`, `limit` = côté et nombre de pixels maximaux) : un calque trop grand ne renvoie
    que son en-tête.
  - **Plume** (`PathOverlay.svelte`) : compteur `inFlight` des éditions en cours — la copie de travail n'est
    resynchronisée depuis le document qu'une fois toutes revenues (un mouvement pendant une poignée tirée relançait
    une copie complète du tracé et effaçait ce qui venait d'être posé) ; glisser en posant un point ne crée des
    poignées qu'au-delà du même seuil de 3 px qu'à la fermeture (`newHandle`, un clic qui tremble ne fait plus un faux
    angle) ; une poignée collée à son point laisse le point l'emporter à la saisie (poignées vs point) ; clic sur le
    dernier point posé : on ne le supprime pas, on reprend sa courbe (`fresh: false`, « Régler la courbe », aucun état
    d'historique si rien n'a bougé) ; après une annulation, le prochain clic repart du dernier sous-tracé encore ouvert.
  - **Suppr** (`actions.ts` `edit.deleteKey`) : le masque de fusion étant visé, la sélection y est masquée
    (`Document::clear_selected_mask`, `onMask` de l'opération `clearPixels`, à proportion de la sélection ; sans
    sélection, tout le masque) ; un clic dans le panneau Calques y laisse le focus (`keepFocus`, `data-panel="layers"`
    focalisable) et Suppr y supprime alors le calque, pas les pixels ; **Maj+Suppr** supprime toujours le calque, même
    avec une sélection et le focus sur l'image (`keymap-photoshop.ts`). Un calque caché se supprime comme les autres.
  - **Transformation** : tirer un coin suit la souris au plus près, par projection sur la diagonale
    (`diagonalScale` dans `lib/geometry.ts`, `TransformOverlay.svelte`), au lieu de n'écouter que l'axe dominant.
  - **Pot de peinture et baguette** (`selection.rs`) **13 à 18 × plus rapides** : cache des écarts Oklab par couleur
    (`Distance`, table à adressage direct de 4096 cases : sur un aplat, la conversion n'est faite qu'une fois), bord
    lissé par `mask_value`, masque écrit tuile par tuile dans le seul rectangle englobant de la région (`write_rect`,
    tuile vide non stockée) ; le remplissage ne touche que les tuiles réellement couvertes par la région
    (`TileStore::any_nonzero_in`, `Document::within_mask`), pas tout son rectangle : seules celles-là sont
    retransmises à l'affichage.
  - **WebP avec perte** (`io.rs`, crate `webp` = libwebp) : VP8 + alpha à la qualité du curseur (25 à 35 % plus léger
    que le JPEG), case **Sans perte** (VP8L) dans la fenêtre Exporter (`ExportOptions::lossless`,
    `ExportFormat::is_lossy` : l'aperçu montre le format lui-même quand il perd) ; limite du format à 16 383 px de côté.
  - **Curseurs `range` unifiés** (`theme/base.css`) : un seul dessin pour toute l'interface au lieu de celui de chaque
    moteur (bleu sous WebKit, gris sous WebView2) : piste fine à bords droits du ton des champs, bouton blanc rond ;
    `.tinted` pour les pistes colorées des réglages.
  - **Mémoire dynamique avec plafond** (comme « Utilisation de la mémoire » de Photoshop) :
    - **Mémoire vive** : Préférences › Performances › « Mémoire vive utilisable par Lumitype », en % de la mémoire de
      la machine (défaut 50 %, 10 à 90 %, « 50 % · 32 Gio sur 64 Gio »). Le moteur ne réserve rien d'avance :
      `tiles::resident_bytes` compte exactement les tuiles vivantes (documents, historique, caches) ; le cache de
      rendu est dynamique (`render::cache_budget_for` = plafond − tout le reste, minimum 128 Mio, vidé puis
      reconstruit à la demande) ; les caches de tous les documents sont inscrits au registre de `engine::memory`
      (`register_caches`, `trim_caches`, `cache_bytes_total`) ; au-delà du plafond, les caches sont vidés d'abord,
      puis la session modifiée abandonne ses plus anciens états d'annulation (`Session::enforce_memory_cap`, jamais
      moins de 5, jamais l'état courant). Plafond envoyé par `set_performance` (`memoryPercent`) et appliqué au
      démarrage (`loadPerformance`). L'ancien `cacheMb` (auto = ¼ de la mémoire vive / manuel) disparaît ; une clé
      `cacheMb` ou `gpuMemoryMb` dans un fichier existant est ignorée (`mergeStoredPerformance`).
    - **Jauge** (commande `memory_status`, relevée toutes les secondes seulement tant que la boîte est ouverte sur
      Performances) : documents ouverts / historique / cache de rendu / libre sous le plafond, et « Utilisation
      actuelle : X Gio sur Y Gio ».
    - **Mémoire vidéo** : « Mémoire vidéo utilisable » en % de la mémoire vidéo détectée (défaut 50 %, 10 à 80 %,
      « 50 % · 12 Gio sur 24 Gio détectés »). Détection dans `src-tauri/src/gpu_info.rs`, dans l'ordre : Vulkan
      (crate `ash`, chargeur ouvert à l'exécution, plus grand tas local de la carte dédiée la mieux dotée ; carte
      intégrée signalée « partagée »), Linux `/sys/class/drm/card*/device/mem_info_vram_total`, Windows DXGI
      (`DXGI_ADAPTER_DESC1.DedicatedVideoMemory`, crate `windows` 0.62 `Win32_Graphics_Dxgi`), sinon « VRAM non
      détectée, 2 Gio supposés ». Carte intégrée (< 1 Gio dédié, partagée ou inconnue) : plafond = min(% × VRAM,
      ¼ de la mémoire vive). Ce plafond est le budget de `GpuLayers` (`gpuMemoryBudgetMb`) ; l'ancien réglage manuel
      en Mio disparaît. L'allocation reste dynamique : au-delà, le document est composé par le processeur
      (`plan.ok = false`), jamais de plantage.
    - Tests : moteur `the_dynamic_cache_respects_the_memory_cap`, `a_low_memory_cap_prunes_the_oldest_states_but_never_below_the_minimum`,
      `the_status_counts_current_documents_once…` ; interface `ui/tests/performance.test.mjs` (`npm run test:performance`).
      Reste à vérifier à la main sous Windows : détection DXGI/Vulkan et affichage de la jauge.

- **1.0.0 (10/10)** — première version stable :
  - **Cadence débridée sous Linux** (`src-tauri/src/rt.rs`) : switches Chromium `disable-frame-rate-limit` et
    `disable-gpu-vsync` ; XWayland annonçait 60 Hz à Chromium même sur un écran 240 Hz. Le compositeur présente la
    fenêtre à la cadence réelle ; Lumitype ne dessine que quand quelque chose change. `LUMITYPE_VSYNC=1` : retour au
    vsync. Le mode Wayland natif du runtime (`LUMITYPE_WAYLAND=1`) n'accepte qu'une fenêtre : abandonné.
  - **Texte** : Échap annule vraiment (frappe en attente abandonnée avant `cancelGesture` + `undo`), un calque de
    texte vidé est supprimé (aussi en changeant d'outil ou de calque), Ctrl+T pendant la saisie ouvre la
    transformation (`TextEditorOverlay.svelte`, `finishTextEdit` dans `lib/text-edit.svelte.ts`).
  - **Éditeur de dégradé** : l'arrêt ajouté se glisse et se retire aussitôt ; double-clic sur un arrêt = couleur
    (`GradientStops.svelte`).
  - **Panneaux flottants** (Effets, éditeur de dégradé, couleur, menu pinceau) : les raccourcis d'édition ne passent
    plus derrière (Suppr supprimait le calque) ; seuls ceux de la vue restent (`panelOwnsKeyboard`, `lib/keymap.ts`).
  - **Menus contextuels** robustes sous Chromium embarqué : ni la perte de focus ni le relâchement du clic droit ne
    les referment aussitôt (`lib/menu-dismiss.ts`).
  - **Panneau Calques** : un dossier ne se dépose plus dans un de ses descendants ; après suppression, le calque
    voisin du dessous est sélectionné (comme Photoshop).
  - `<output>` dans la barre d'options, chaînes manquantes traduites (en/es/de complets), icône de page.
  - Version **1.0.0** partout (`Cargo.toml`, `package.json`, `tauri.conf.json`, noms des paquets).

- **1.0.1 (10/10)** :
  - **Gel « ne répond pas »** autour du champ Police : un glisser du texte sélectionné lançait, sous Linux, la boucle
    de glisser imbriquée de Chromium qui bloquait la fenêtre. Plus aucun glisser hors des éléments déclarés glissables
    (`onDragStart`, `App.svelte`) ; le champ Police ne sélectionne plus son texte (vide à l'ouverture, police courante
    en indication).
  - **Listes Police et Style** : un clic sur la barre de défilement ne les referme plus.
  - **Rotation** : les anneaux de l'outil Déplacement sont découpés au contour du cadre (`clip-path`) ; l'intérieur
    est toujours « déplacer », même sur un petit objet.
  - **Calques verrouillés** (cadenas ou position, sur le calque ou un dossier parent) : pas de cadre, Ctrl+T refusé
    avec un message, la sélection automatique les traverse (`Document::pick_layer`), l'outil Déplacement le signale.

- **1.0.2 (10/10)** :
  - **Glisser-déposer du panneau Calques réécrit** (`lib/layer-drag.svelte.ts`) : événements de pointeur, seuil de
    4 px, fantôme, défilement automatique, dépôt sur un onglet. Plus aucun glisser natif de Chromium dans l'interface :
    fini le calque « collé » au pointeur et les gels de la boucle de glisser sous Linux. Mêmes opérations qu'avant
    (déplacer, ranger, Alt = dupliquer, copie de masque, refus d'un dossier dans son descendant).
  - **Transformation en attente validée avant toute autre opération** (crochet `setBeforeEdit` dans `lib/engine.ts`) :
    rotation puis Ctrl+J donnait deux calques différents. Ctrl+T ne reprend un cadre que s'il porte les calques
    sélectionnés ; l'ancien cadre disparaît dès que la sélection change ; `{#key}` sur le cadre ; rien n'est rouvert
    pendant qu'une poignée est tirée ; étendues lues dans la file du document.
  - **Modes de fusion sous Chromium** : garde contre le faux `blur` de fenêtre (`dismissGuard`), choix à l'appui.
  - **Attente des textures pendant un geste** limitée aux calques repeints (`whenSettled(layers)`, `GpuLayers.pending`).
  - **Repli de police** : liste courte (polices incluses puis système courantes), base de polices réduite pour
    cosmic-text, une seule recherche par caractère inconnu ; graisse absente → graisse la plus proche de la même famille.
  - **Disque de travail** (`crates/engine/src/scratch.rs`) : au-delà du plafond de mémoire, les états anciens de
    l'historique partent sur disque (LZ4) au lieu d'être supprimés ; relus à l'annulation. Préférences : case, dossier,
    quota 2–200 Gio (20 par défaut), « Sur le disque » dans la jauge.
  - **Effets de calque sur la carte graphique pendant le réglage** (`viewport/gpu-effects.ts`) : les sept effets, écart
    ≤ 1/255 avec le moteur ; le moteur fait le rendu exact au relâchement. Rendu logiciel (llvmpipe) : chemin moteur.
  - **Épreuve CMJN** : FOGRA39, FOGRA51 (PSO couché v3), FOGRA52 (PSO non couché v3), GRACoL 2006, SWOP 2006 — profils
    CC0 (`export/profils/LISEZ-MOI.md`), une LUT chacun ; nom du profil dans l'onglet et la barre d'état.
  - Fermeture des Effets par Échap : plus d'erreur.

- **1.1.0 (10/10) — Objets dynamiques** (maquette `objets-dynamiques-maquette.png`) :
  - **Moteur** (`crates/engine/src/smart.rs`) : `LayerKind::Smart { content, transform, raster }` ; contenu incorporé
    (document complet, image d'origine conservée à l'octet près) ou lié (rechargé s'il change, « introuvable » sinon,
    dernier rendu gardé) ; transformation paramétrique (agrandir après réduire ne perd rien : le rendu repart toujours
    de l'aplat pleine résolution, réductions par paliers) ; instances (Ctrl+J, copier-coller) partageant le contenu ;
    peinture et filtres refusés (« pixellisez-le ou modifiez son contenu »), masque autorisé ; mémoire comptée une fois.
  - **Ops** : `convertToSmart`, `placeSmartObject`, `replaceSmartContent`, `relinkSmart`, `embedSmartLink`,
    `resetSmartTransform`, `newSmartByCopy`, `rasterize` ; commandes `smart_content_open/commit/export`,
    `smart_links_check`.
  - **Format .graf v7** : contenus stockés une fois (table `contents`), instances restaurées ; un document sans objet
    dynamique s'écrit toujours en v6 (lisible par les anciennes versions).
  - **Interface** : pastille sur la vignette (objet, lié, introuvable, ×N instances), double-clic sur la vignette =
    modifier le contenu dans un onglet (Ctrl+S y met à jour l'objet et toutes ses instances), menus contextuel et
    Calque › Objets dynamiques, Propriétés (L/H liés, rotation, position, échelle en % de l'original), Fichier ›
    Importer comme objet dynamique / Importer lié, dépôt avec Alt, vérification des liens au retour du focus.
  - Tests : 268 moteur, 18 étapes d'interface sous Chromium, essai réel complet sous Xvfb.

Reste à faire :
1. Objets dynamiques, étape 2 : filtres dynamiques (modifiables, masquables). Cadre de transformation tourné avec
   l'objet (aujourd'hui l'outil Déplacement montre l'emprise droite). Liens imbriqués dans un contenu non vérifiés.
