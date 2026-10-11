# Lumitype

Éditeur d'images raster. Projet initié par Antoine Deg., codé par l'IA Claude (Anthropic).

État : étapes 1 à 10 du plan, plus recadrage, fluidité et masque rapide.

Étape 1 : fenêtre, canevas, zoom/déplacement, ouverture et enregistrement.
Étape 2 : calques (groupes, masques de fusion et d'écrêtage, 21 modes, glisser-déposer), historique,
sélecteur de couleurs, pinceau et gomme.

## Prérequis

| | Linux (Debian/Ubuntu) | Windows 10/11 |
|---|---|---|
| Rust ≥ 1.88 | `curl https://sh.rustup.rs -sSf \| sh` | <https://rustup.rs> (toolchain MSVC) |
| Node ≥ 20 | paquet `nodejs` ou nvm | <https://nodejs.org> |
| Système | voir ci-dessous | « Desktop development with C++ » (Build Tools VS 2022) ; WebView2 est préinstallé |

```sh
# Linux uniquement
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev pkg-config build-essential
```

## Commandes (identiques sous Linux et Windows)

```sh
npm install            # dépendances de l'interface + CLI Tauri
npm run tauri dev      # lancement en développement (rechargement à chaud de l'interface)
npm run tauri build    # binaire et installeurs dans target/release/bundle/
cargo test -p engine   # tests unitaires du moteur
cargo bench -p engine  # mesures sur une image 8000 x 8000
npm run check          # vérification TypeScript + Svelte
npm test               # toutes les vérifications de l'interface d'un coup
npm run test:geometry  # homographie et magnétisme de la transformation libre
npm run test:tiles     # affichage cohérent : échange groupé des tuiles
npm run smoke          # monte l'interface dans un DOM simulé : attrape les écrans vides au démarrage
```

Pilote NVIDIA propriétaire sous Linux, fenêtre vide ou scintillante :
`WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri dev`.

## Arborescence

```
crates/engine/   moteur d'image : tuiles, document, rendu, E/S (aucune dépendance à Tauri)
src-tauri/       coque desktop : état partagé et commandes exposées à l'interface
ui/              interface Svelte 5 : thème, keymap, viewport WebGL2, panneaux
```

## Format de projet `.graf`, version 6

Archive zip non compressée : `manifest.json` (nom et version du format),
`document.json` (arbre de calques), `layers/<id>/…png` (tuiles RGBA, alpha droit),
`masks/<id>/…png` (tuiles de masque en niveaux de gris). Les projets version 1 sont convertis à l'ouverture.
Détails et règles de versionnage dans `crates/engine/src/io.rs`.

## Vignettes du menu Filtre

Au survol d'un filtre, le menu montre l'image de référence (`ui/src/assets/filters/reference.jpg`),
moitié gauche filtrée, moitié droite d'origine. Ces vignettes sont produites par le moteur lui-même,
donc fidèles au filtre réel. À régénérer quand un filtre change ou s'ajoute :

```
cargo run -p engine --release --example filter_previews -- ui/src/assets/filters/reference.jpg ui/src/assets/filters
```

## Un fichier autonome

Un projet `.graf` est une archive zip qui contient **tout** le document : la structure des calques
(`document.json`) et les pixels de chaque calque et de chaque masque, en PNG sans perte. Il ne fait
référence à aucun fichier extérieur : une image importée est copiée dans le projet au moment de
l'import. Le fichier s'ouvre donc tel quel sur une autre machine.

Seule réserve : les polices. Un calque de texte enregistre son rendu, donc il s'affiche à l'identique
partout ; mais pour en **modifier** le texte avec la même apparence, la police doit être installée sur
la machine (sinon une police de repli est utilisée). Les anciens fichiers `.iep` s'ouvrent toujours.
