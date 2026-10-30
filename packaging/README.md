# Construire et distribuer Lumitype

**Une commande par système**, qui vérifie les outils, installe ce qui manque, compile une seule fois et range tous les
fichiers à distribuer dans `packaging/out/`. Avec `--release` / `-Release`, ils sont aussi copiés sous leur nom de
diffusion dans `packaging/release/` avec `SHA256SUMS.txt`, prêts pour la Release GitHub.

| Système | Commande | Produit |
|---|---|---|
| **Windows** | double-clic sur `packaging\windows.bat` (ou `npm run package:windows`) | les **3 fichiers Windows** : `Lumitype_…-setup.exe`, `Lumitype_….msi`, `Lumitype-portable.exe` |
| **Linux** | `./packaging/linux.sh` (ou `npm run package:linux`) | les **6 paquets Linux** (deb, rpm, Arch, Flatpak, AppImage, tar.gz) et, avec `--windows`, les 3 fichiers Windows |
| **Windows → paquets Linux** | double-clic sur `packaging\linux.bat` (lance `linux.sh` dans WSL 2) | les 6 paquets Linux, dans le même `packaging\out\` |

```
./packaging/linux.sh 1.3.0 --windows --release --install   # version, tout construire, release, installer ici
packaging\windows.bat 1.3.0 -Release                        # Windows : version + 3 fichiers + release
```

Tout ce qui est ci-dessous est le détail de ces deux scripts. Les scripts d'appui qu'ils appellent sont dans
`packaging/lib/` (`sync-templates.*` : modèles embarqués ; `cef-bundle.sh` : fichiers Chromium pour les paquets Linux).

## Ce qui accélère la construction

- Le logiciel n'est **compilé qu'une fois** ; le .deb, le .rpm et l'AppImage sortent de cette compilation.
- L'archive, le paquet Arch et le Flatpak sont fabriqués **en parallèle** à partir du .deb (journaux dans
  `packaging/.work/*.log`).
- Avec `--windows`, la construction Windows tourne **en même temps** que l'empaquetage Linux, dans son propre dossier
  de compilation (`target/windows/`).
- Les dépendances de l'interface ne sont réinstallées que si `package-lock.json` a changé.
- Chromium (300 Mo) n'est téléchargé qu'une fois ; le `.rpm` est compressé en zstd ; l'archive en gzip rapide.
- Compilation Rust incrémentale : ne supprimez pas `target/` entre deux versions.

## Windows : rien à installer à la main

`packaging\windows.bat` vérifie ce qu'il faut et **installe lui-même ce qui manque**, avec `winget` (fourni avec
Windows 10 et 11) : Build Tools C++ de Visual Studio (environ 3 Go, la première fois seulement), Rust, Node.js,
NASM (l'assembleur de l'encodeur AVIF, ajouté au PATH de la console) et WebView2 s'il manquait. Windows demande l'autorisation d'installer : acceptez. Ensuite il construit et range
les fichiers dans `packaging\out\`.

```
packaging\windows.bat          :: construit (et installe ce qui manque)
packaging\windows.bat 0.2.0    :: fixe d'abord la version, puis construit
packaging\windows.bat -Essai   :: compile seulement (sans installateur) et lance Lumitype, pour essayer
```

Sous Windows, le moteur d'affichage est **WebView2**, le Chromium fourni par Windows : même moteur de rendu que le
Chromium embarqué des paquets Linux, sans rien à embarquer.

## Linux : tous les formats

`./packaging/linux.sh` compile Lumitype une seule fois, puis produit dans `packaging/out/` (version 1.2.88, PC 64 bits) :

| Fichier | Installateur | Distributions |
|---|---|---|
| `Lumitype_1.2.88_amd64.deb` | `apt` | Debian, Ubuntu, Mint, Pop!_OS, Zorin, elementary |
| `Lumitype-1.2.88-1.x86_64.rpm` | `dnf`, `zypper` | Fedora, Nobara, openSUSE, RHEL, Rocky, Alma |
| `lumitype-1.2.88-1-x86_64.pkg.tar.zst` | `pacman` | Arch, CachyOS, Manjaro, EndeavourOS, Garuda |
| `Lumitype-1.2.88-x86_64.flatpak` | `flatpak` | toutes |
| `Lumitype_1.2.88_amd64.AppImage` | aucun | toutes, sans installation |
| `Lumitype-1.2.88-x86_64.tar.gz` | aucun | toutes : décompresser, puis `./installer.sh` (dans le dossier de l'utilisateur) |

**Avant de compiler quoi que ce soit**, le script vérifie toutes les dépendances d'un coup et affiche un tableau
(✓ présent, ✗ manquant, ! avertissement) : Node.js 20+, npm, Rust 1.88+, compilateur C/C++, pkg-config, bibliothèques
GTK / WebKit / OpenSSL / librsvg, CMake et Ninja (Chromium embarqué), `nasm` (encodeur AVIF, PC x86-64), curl, file,
binutils (selon les formats), polices incluses et espace disque (15 Go conseillés). Ce que la distribution fournit est
installé en une seule fois (`dnf`, `apt`, `pacman` ou `zypper`, après accord ; `--oui` pour ne pas demander), puis tout
est revérifié ; s'il manque encore quelque chose, le script s'arrête avec la liste et les commandes à lancer, sans avoir
rien compilé. Avec `--windows`, nsis, clang, lld et llvm sont vérifiés aussi.

Un format impossible sur la machine est sauté, avec la raison :

- **paquet Arch** : seulement sous Arch, CachyOS, Manjaro… (outil `makepkg`), et sans `sudo` ;
- **Flatpak** : télécharge une fois le socle GNOME 51 depuis Flathub (environ 1 Go) ; autre version :
  `LUMITYPE_FLATPAK_GNOME=50 ./packaging/linux.sh flatpak`. Un socle GNOME n'est suivi qu'environ un an : le monter
  à chaque version de GNOME (GNOME 47 : fin de vie le 15/10/2025). Sous Ubuntu (WSL compris), un flatpak-builder trop
  ancien est mis à jour depuis le PPA officiel `flatpak/stable`, après accord.
- **AppImage** : si linuxdeploy échoue, l'AppImage est refaite seule en mode détaillé (`packaging/.work/appimage.log`)
  et ses lignes d'erreur sont affichées ; les bibliothèques de Chromium absentes de la machine sont signalées.

```
./packaging/linux.sh                 # tous les formats possibles ici
./packaging/linux.sh deb appimage    # seulement ceux-là : deb rpm arch flatpak appimage tar
./packaging/linux.sh --essai         # compile seulement (sans paquet) et lance Lumitype, pour essayer
```

Les projets `.graf` sont associés à Lumitype dans tous les formats installés : double-clic pour les ouvrir.

## Windows depuis Linux (`--windows`)

`./packaging/linux.sh --windows` compile aussi Lumitype pour Windows (cargo-xwin : en-têtes et bibliothèques MSVC
téléchargés une fois ; `nsis clang lld llvm` à installer, le script le propose) et produit `…-setup.exe` et
`Lumitype-portable.exe`. Le `.msi` (WiX) ne se fabrique que sous Windows, ou sous Linux si `wine` est installé.

## Version, release et installation

```
./packaging/linux.sh 1.3.0                 # passe d'abord la version à 1.3.0, partout (tauri.conf.json, package.json, Cargo.toml)
./packaging/linux.sh --release             # copie sous les noms de diffusion + SHA256SUMS.txt dans packaging/release/
./packaging/linux.sh --install             # construit, puis installe le paquet de cette distribution
```

`--install` choisit le paquet de la machine (`.deb` avec apt, `.rpm` avec dnf ou zypper, `.pkg.tar.zst` avec pacman)
et met à jour une version déjà présente ; préférences, raccourcis et pinceaux sont conservés.

## Linux : moteur d'affichage Chromium

Les paquets Linux embarquent **Chromium** (CEF) comme moteur d'affichage, à la place de WebKitGTK qui plafonne
vers 45-50 images par seconde en plein écran 4K. **L'utilisateur n'installe rien** : Chromium est dans
`/usr/lib/Lumitype/` (environ 300 Mo décompressé, 100 à 130 Mo de plus dans chaque paquet).

- La première construction télécharge Chromium (environ 300 Mo, depuis `cef-builds.spotifycdn.com`) ; les
  suivantes le réutilisent.
- Ancienne version avec la webview du système (WebKitGTK) : `LUMITYPE_MOTEUR=webkit ./packaging/linux.sh`.
- Windows n'est pas concerné : il garde WebView2 (un essai de Chromium embarqué sous Windows a été abandonné).
- Journal de Chromium, en cas de problème d'affichage : `~/.cache/dev.imageeditor.app/chromium.log`.
- Chromium tourne en X11 (XWayland), **sans plafond de cadence** : l'affichage suit la fréquence de l'écran (144, 240 Hz). Retour au vsync à 60 Hz si besoin : `LUMITYPE_VSYNC=1 lumitype`. (Le mode Wayland natif, `LUMITYPE_WAYLAND=1`, ne gère qu'une fenêtre et ne démarre pas.)
- Le `.rpm` est compressé en zstd (comme les paquets Fedora) : bien plus rapide à fabriquer que le gzip par défaut.
- Compilation à la main : `npm run tauri build -- --no-bundle --features cef --config src-tauri/tauri.cef.json -- --no-default-features`.

## À savoir

- **Chaque paquet se construit sur son système** : l'exécutable Windows sous Windows, les paquets Linux sous Linux
  (une machine virtuelle ou WSL2 conviennent pour Linux). Pour tout produire d'un coup sans seconde machine,
  l'action GitHub `tauri-apps/tauri-action` construit les deux en parallèle à chaque version.
- **Première construction : dix à vingt minutes** (tout est compilé en version optimisée). Les suivantes sont bien plus rapides.
- **Linux : compatibilité (glibc).** Un paquet ne démarre que sur une glibc au moins aussi récente que celle de la machine
  qui l'a construit : construit sur Fedora 44 (glibc 2.43), il refuse Linux Mint 22 (« GLIBC_2.43 not found »).
  `linux.sh` affiche après chaque compilation la glibc demandée et les distributions où les paquets démarrent ; au-delà
  de 2.35, il avertit, et `--release` refuse de publier (seuil réglable : `LUMITYPE_GLIBC_MAX=2.39`).
  **Paquets pour tout le monde : `./packaging/linux.sh --compat`** compile dans un conteneur Ubuntu 22.04 (glibc 2.35,
  Podman ou Docker, image préparée seule la première fois, `packaging/lib/Containerfile.ubuntu22`) : Linux Mint 21+,
  Ubuntu 22.04+, Debian 12+, Fedora, Arch. Le .deb, le .rpm, l'AppImage et l'archive s'y fabriquent ; le paquet Arch et
  le Flatpak restent à faire sans `--compat`. Compilation et caches à part (`~/.cache/lumitype-compat`).
  Depuis Windows, `linux.bat` choisit d'office la distribution WSL `Ubuntu-22.04` si elle est installée
  (`wsl --install -d Ubuntu-22.04`).
- **Version** : celle de `src-tauri/tauri.conf.json` (`"version"`) ; c'est elle qui apparaît dans le nom des fichiers.
  Pensez à l'augmenter, avec celle de `package.json`, avant chaque diffusion.
- **Icône** : celle de `src-tauri/icons/`. Après avoir remplacé le logo :
  `npm run tauri icon ui/src/assets/logo/logo-1024.png`.
- **AppImage et FUSE** : l'outil qui la fabrique est lui-même une AppImage et réclame FUSE 2. Le script contourne ce
  besoin (`APPIMAGE_EXTRACT_AND_RUN=1`) ; si l'erreur `failed to run linuxdeploy` persiste, installez FUSE
  (`sudo dnf install fuse fuse-libs` sous Fedora, `sudo apt install libfuse2` sous Ubuntu) ou contentez-vous du `.rpm` et du `.deb`,
  qui n'en ont pas besoin.
- **Windows, « éditeur inconnu »** : sans certificat de signature de code, SmartScreen avertit l'utilisateur
  (Informations complémentaires > Exécuter quand même).
- **Couleurs CMJN (Édition › Paramètres de couleur…, Maj+Ctrl+K)** : un seul CMJN pour tout Lumitype, comme
  Photoshop. Le profil de presse (les cinq livrés dans `crates/engine/assets/cmyk/`, libres de redistribution, ceux installés sur
  l'ordinateur, par Photoshop notamment, ou un fichier chargé), l'intention de rendu et la compensation du point noir
  servent à l'épreuve écran (Affichage › Épreuve CMJN, Ctrl+Y), au mode Image › Mode › Couleurs CMJN et à l'export
  CMJN. Les tables sont calculées par LittleCMS, compilé avec le moteur (crate `lcms2`, fonction `static` : rien à
  installer). Un profil chargé est recopié dans le dossier de réglages (`profils-cmjn/`) ; un projet en mode CMJN avec
  un profil qui n'est pas livré le garde dans son archive (`profils/<id>.icc`). Réglage enregistré : `couleurs`.

## Installer le résultat

```
sudo apt install ./packaging/out/Lumitype_*.deb                 # Debian, Ubuntu, Mint
sudo dnf install ./packaging/out/Lumitype-*.rpm                 # Fedora, Nobara
sudo zypper install ./packaging/out/Lumitype-*.rpm              # openSUSE
sudo pacman -U ./packaging/out/lumitype-*.pkg.tar.zst           # Arch, CachyOS, Manjaro
flatpak install --user ./packaging/out/Lumitype-*.flatpak       # toutes distributions
chmod +x packaging/out/Lumitype_*.AppImage && ./packaging/out/Lumitype_*.AppImage   # sans installation
tar -xzf Lumitype-*.tar.gz && ./Lumitype-*/installer.sh           # sans droits d'administrateur
```
