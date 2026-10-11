<p align="center">
  <img src="docs/images/banner.webp" alt="Lumitype : rapide, moderne, gratuit" width="100%">
</p>

<p align="center"><a href="README.md">English</a> · <b>Français</b></p>

<p align="center">
  <img alt="Version" src="https://img.shields.io/badge/version-1.1.0-ec2a3c">
  <img alt="Plateformes" src="https://img.shields.io/badge/plateforme-Linux%20%7C%20Windows-2da44e">
  <img alt="Moteur" src="https://img.shields.io/badge/moteur-Rust-b7410e">
  <img alt="Interface" src="https://img.shields.io/badge/interface-Svelte%205-ff3e00">
  <img alt="Coque" src="https://img.shields.io/badge/coque-Tauri%202-24c8db">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-blue">
</p>

Le nom Lumitype vient d'une machine à composer le texte. J'ai utilisé Claude AI pour développer ce logiciel, dont les principaux points forts sont les performances, la gratuité et l'interface moderne.

## ⬇ Télécharger Lumitype 1.1.0

**Choisissez votre système.** Gratuit, sans compte.

| Système | Paquet | Installation |
|---|---|---|
| **Windows 10 / 11** | [Lumitype-Windows-Setup.exe](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Windows-Setup.exe) | Double-clic sur l'installateur |
| **Fedora** · RHEL · openSUSE | [Lumitype-Fedora-x86_64.rpm](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Fedora-x86_64.rpm) | `sudo dnf install ./Lumitype-Fedora-x86_64.rpm` |
| **Debian** · Ubuntu · Mint | [Lumitype-Debian-Ubuntu-amd64.deb](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Debian-Ubuntu-amd64.deb) | `sudo apt install ./Lumitype-Debian-Ubuntu-amd64.deb` |
| **Tout Linux** : AppImage | [Lumitype-Linux-x86_64.AppImage](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Linux-x86_64.AppImage) | `chmod +x Lumitype-Linux-x86_64.AppImage && ./Lumitype-Linux-x86_64.AppImage` |
| **Tout Linux** : Flatpak | [Lumitype-Linux.flatpak](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Linux.flatpak) | `flatpak install --user ./Lumitype-Linux.flatpak` |
| **Tout Linux** : archive portable | [Lumitype-Linux-portable.tar.gz](https://github.com/splaching-boop/Lumitype/releases/latest/download/Lumitype-Linux-portable.tar.gz) | `tar -xzf Lumitype-Linux-portable.tar.gz` |

Sous Windows, le logiciel utilise WebView2 (inclus).

[Toutes les versions](https://github.com/splaching-boop/Lumitype/releases) · [Vérifier les empreintes (SHA256SUMS.txt)](https://github.com/splaching-boop/Lumitype/releases/latest/download/SHA256SUMS.txt)

<p align="center">
  <img src="docs/images/interface.webp" alt="Interface de travail de Lumitype" width="100%">
  <br>
  <sub><i>Interface de travail – Lumitype</i></sub>
</p>

## Fonctionnalités

<table>
<tr>
<td width="33%" valign="top">
<img src="docs/images/layers.webp" alt="Panneau des calques">
<h3>Organisation des calques</h3>
<p>Masques de fusion, masques d'écrêtage, groupes : j'ai essayé de reproduire fidèlement la méthode de travail de Photoshop.</p>
</td>
<td width="33%" valign="top">
<img src="docs/images/cmyk-preview.webp" alt="Aperçu RVB et CMJN">
<h3>Aperçu CMJN</h3>
<p>Pour celles et ceux qui travaillent sur des affiches imprimées, un aperçu CMJN est disponible dans le logiciel !</p>
</td>
<td width="33%" valign="top">
<img src="docs/images/home.webp" alt="Écran d'accueil">
<h3>Accueil simple</h3>
<p>Retrouvez vos projets récents, ou glissez n'importe quelle image depuis votre navigateur pour commencer un projet !</p>
</td>
</tr>
</table>

### Repères aimantés

Aimantez vos images, la plume ou le texte aux repères : bien plus pratique !

<!-- TODO : ajouter une image, par exemple docs/images/magnetic-guides.webp -->

### Une sélection de guides complète

<p align="center">
  <img src="docs/images/guides-dialog.webp" alt="Fenêtre des guides de composition" width="80%">
</p>

Aidez-vous des guides prédéfinis pour réaliser votre affiche, votre bannière ou tout autre contenu graphique !

<details>
<summary><b>Toutes les fonctionnalités</b></summary>

- **Interface** : moderne, raccourcis clavier basés sur ceux de Photoshop
- **Fichiers** : projet `.graf` (v7) ; ouverture PNG, JPEG, WebP, TIFF, AVIF, HEIC, DNG ; import par adresse web ou Ctrl V
- **Calques** : groupes imbriqués, masques de fusion et d'écrêtage, 21 modes de fusion, opacité et fond, verrous, objets dynamiques
- **Effets de calque** : ombres, contours, dégradés, accélérés par le matériel (GPU)
- **Sélection** : rectangle, ellipse, lasso, baguette magique, ajout, soustraction, intersection
- **Peinture** : pinceau, gomme, pot de peinture, pipette, plume, dégradé, transfert de dégradé
- **Texte** : éditeur sur le canvas, 45 familles incluses, graisses avec aperçu, repli de police
- **Réglages et filtres** : développement, luminosité, niveaux, menu Filtre avec vignettes inspirées de Lightroom
- **Transformation** : cadre, rotation, recadrage
- **Couleur** : roue chromatique, aperçu CMJN avec 5 profils ICC
- **Export** : JPEG, PNG, WebP avec estimation du poids
- **Performance** : tuiles, mémoire dynamique réglable (RAM et VRAM), disque de travail compressé, rendu GPU
- **Historique** et récupération après plantage
- **Composition** : de nombreux guides prédéfinis pour vous aider à composer vos travaux !

</details>

## Pourquoi Lumitype

Lumitype ne se différencie pas spécialement des autres projets existants, mais un soin particulier est apporté pour que le logiciel reste utilisable, sans trop de bugs ni de problèmes !

## État du projet et limites connues

Le projet est développé uniquement le week-end, à cause de mon emploi du temps. Je le rends open source : vous êtes libres de l'améliorer ou de faire un fork de Lumitype sans problème. Servez-vous ! De toute façon, même si j'ai initié le projet, Claude AI est le véritable développeur de cette application !

## Feuille de route

Problèmes connus et améliorations futures :

- Outil de transformation parfois imprécis selon les circonstances
- Transformation imprécise avec les masques de fusion
- Plus de filtres à l'avenir
- Meilleure prise en charge des brushes (non testés dans la version actuelle)
- Prise en charge des images RAW

## Crédits et licence

**Auteur :** Claude AI (Fable 5.1, Opus 5.5 et Sonnet 5.5)
**Direction de Claude :** Antoine Deg.

**Licence :** [MIT](LICENSE). Libre d'utilisation, de modification et de redistribution. Les polices incluses conservent leur propre licence (SIL OFL, voir les fichiers `OFL.txt` dans `src-tauri/fonts/`).

---

## Pour les développeurs

<details>
<summary><b>Compiler depuis les sources</b></summary>

| | Linux (Debian/Ubuntu) | Windows 10/11 |
|---|---|---|
| Rust ≥ 1.88 | `curl https://sh.rustup.rs -sSf \| sh` | <https://rustup.rs> (toolchain MSVC) |
| Node ≥ 20 | paquet `nodejs` ou nvm | <https://nodejs.org> |
| Système | voir ci-dessous | « Desktop development with C++ » (VS 2022 Build Tools) ; WebView2 est préinstallé |

```sh
# Linux uniquement
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev pkg-config build-essential
```

Commandes (identiques sous Linux et Windows) :

```sh
npm install            # dépendances de l'interface + CLI Tauri
npm run tauri dev      # lancement de développement (rechargement à chaud)
npm run tauri build    # binaire et installateurs dans target/release/bundle/
cargo test -p engine   # tests unitaires du moteur
cargo bench -p engine  # benchmarks sur une image 8000 x 8000
npm run check          # vérifications TypeScript + Svelte
npm test               # toutes les vérifications de l'interface
```

Pilote NVIDIA propriétaire sous Linux, fenêtre blanche ou qui scintille : `WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri dev`.

La construction des paquets Linux et Windows (deb, rpm, AppImage, Flatpak, installateur) est décrite dans [`export/LISEZ-MOI.md`](export/LISEZ-MOI.md). Pour renommer les paquets d'une release et générer les empreintes : `export/renommer-release.sh`.

</details>

<details>
<summary><b>Organisation du projet</b></summary>

```
crates/engine/   moteur d'image : tuiles, document, rendu, E/S (sans dépendance à Tauri)
src-tauri/       coque desktop : état partagé et commandes exposées à l'interface
ui/              interface Svelte 5 : thème, raccourcis, viewport WebGL2, panneaux
```

</details>

<details>
<summary><b>Le format de projet <code>.graf</code> (version 7)</b></summary>

Un projet `.graf` est une archive zip non compressée contenant **tout** le document : `manifest.json` (nom et version du format), `document.json` (arbre des calques), `layers/<id>/…png` (tuiles RGBA, alpha droit) et `masks/<id>/…png` (tuiles de masque en niveaux de gris). Aucun fichier externe n'est référencé : une image importée est copiée dans le projet à l'import, le fichier s'ouvre donc tel quel sur une autre machine.

Un document sans objets dynamiques est écrit en version 6 (les anciennes versions de Lumitype l'ouvrent encore) ; un document avec objets dynamiques est écrit en version 7. Détails et règles de version dans `crates/engine/src/io.rs`.

Seule réserve : les polices. Un calque de texte mémorise son rendu, il s'affiche donc à l'identique partout ; pour *modifier* son texte avec le même aspect, la police doit être installée sur la machine (sinon une police de repli est utilisée). Les anciens fichiers `.iep` s'ouvrent toujours.

</details>
