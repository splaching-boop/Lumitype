# Exporter Lumitype

Deux commandes, une par système. Chacune vérifie les outils nécessaires, construit l'application en
version finale, puis range les fichiers à distribuer dans `export/sorties/`.

| Système | Commande | Résultat dans `export/sorties/` |
|---|---|---|
| **Windows** | double-clic sur `export\windows.bat` (ou `npm run export:windows`) | `Lumitype_…-setup.exe` (installateur à distribuer), `Lumitype_….msi`, `Lumitype-portable.exe` (sans installation) |
| **Linux** | `./export/linux.sh` (ou `npm run export:linux`) | les six formats ci-dessous |

## Windows : rien à installer à la main

`export\windows.bat` vérifie ce qu'il faut et **installe lui-même ce qui manque**, avec `winget` (fourni avec
Windows 10 et 11) : Build Tools C++ de Visual Studio (environ 3 Go, la première fois seulement), Rust, Node.js,
et WebView2 s'il manquait. Windows demande l'autorisation d'installer : acceptez. Ensuite il construit et range
les fichiers dans `export\sorties\`.

```
export\windows.bat          :: construit (et installe ce qui manque)
export\windows.bat 0.2.0    :: fixe d'abord la version, puis construit
export\windows.bat -Essai   :: compile seulement (sans installateur) et lance Lumitype, pour essayer
```

Sous Windows, le moteur d'affichage est **WebView2**, le Chromium fourni par Windows : même moteur de rendu que le
Chromium embarqué des paquets Linux, sans rien à embarquer.

## Linux : tous les formats

`./export/linux.sh` compile Lumitype une seule fois, puis produit dans `export/sorties/` (version 1.1.0, PC 64 bits) :

| Fichier | Installateur | Distributions |
|---|---|---|
| `Lumitype_1.1.0_amd64.deb` | `apt` | Debian, Ubuntu, Mint, Pop!_OS, Zorin, elementary |
| `Lumitype-1.1.0-1.x86_64.rpm` | `dnf`, `zypper` | Fedora, Nobara, openSUSE, RHEL, Rocky, Alma |
| `lumitype-1.1.0-1-x86_64.pkg.tar.zst` | `pacman` | Arch, CachyOS, Manjaro, EndeavourOS, Garuda |
| `Lumitype-1.1.0-x86_64.flatpak` | `flatpak` | toutes |
| `Lumitype_1.1.0_amd64.AppImage` | aucun | toutes, sans installation |
| `Lumitype-1.1.0-x86_64.tar.gz` | aucun | toutes : décompresser, puis `./installer.sh` (dans le dossier de l'utilisateur) |

Le script reconnaît la distribution et propose d'installer ce qui manque (`dnf`, `apt`, `pacman` ou `zypper` ;
`--oui` pour ne pas demander). Un format impossible sur la machine est sauté, avec la raison :

- **paquet Arch** : seulement sous Arch, CachyOS, Manjaro… (outil `makepkg`), et sans `sudo` ;
- **Flatpak** : télécharge une fois le socle GNOME 47 depuis Flathub (environ 1 Go) ; autre version :
  `LUMITYPE_FLATPAK_GNOME=48 ./export/linux.sh flatpak`.

```
./export/linux.sh                 # tous les formats possibles ici
./export/linux.sh deb appimage    # seulement ceux-là : deb rpm arch flatpak appimage tar
./export/linux.sh --essai         # compile seulement (sans paquet) et lance Lumitype, pour essayer
```

Les projets `.graf` sont associés à Lumitype dans tous les formats installés : double-clic pour les ouvrir.

## Mise à jour (Linux)

```
./export/maj.sh                   # construit tous les formats, range dans export/sorties/
./export/maj.sh 0.2.0             # passe d'abord la version à 0.2.0, partout
./export/maj.sh 0.2.0 --install   # construit, puis installe le paquet de cette distribution
```

`--install` choisit le paquet de la machine (`.deb` avec apt, `.rpm` avec dnf ou zypper, `.pkg.tar.zst` avec pacman)
et met à jour une version déjà présente ; préférences, raccourcis et pinceaux sont conservés.

## Linux : moteur d'affichage Chromium

Les paquets Linux embarquent **Chromium** (CEF) comme moteur d'affichage, à la place de WebKitGTK qui plafonne
vers 45-50 images par seconde en plein écran 4K. **L'utilisateur n'installe rien** : Chromium est dans
`/usr/lib/Lumitype/` (environ 300 Mo décompressé, 100 à 130 Mo de plus dans chaque paquet).

- La première construction télécharge Chromium (environ 300 Mo, depuis `cef-builds.spotifycdn.com`) ; les
  suivantes le réutilisent.
- Ancienne version avec la webview du système (WebKitGTK) : `LUMITYPE_MOTEUR=webkit ./export/linux.sh`.
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
- **Linux : construire sur la plus ancienne distribution visée.** Un paquet construit sur Fedora 42 peut refuser de démarrer
  sur un système plus ancien (version de la glibc).
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
- **Épreuve CMJN (Affichage › Épreuve : …)** : simulation d'impression selon l'un des profils de presse
  d'`export/profils/` (tous libres de redistribution ; provenance, licences et empreintes dans
  `export/profils/LISEZ-MOI.md`), une table 3D 33³ précalculée par profil dans `ui/src/assets/proof/` :

  | Entrée du menu                          | Profil ICC                              | Table             |
  |-----------------------------------------|-----------------------------------------|-------------------|
  | Épreuve : FOGRA39 (couché)              | `FOGRA39L_coated.icc`                   | `fogra39.bin`     |
  | Épreuve : FOGRA51 (PSO couché v3)       | `FOGRA51_PSOcoated_v3_postext.icc`      | `fogra51.bin`     |
  | Épreuve : FOGRA52 (PSO non couché v3)   | `FOGRA52_PSOuncoated_v3_postext.icc`    | `fogra52.bin`     |
  | Épreuve : GRACoL 2006 (couché n° 1)     | `GRACoL_TR006_coated.icc`               | `gracol2006.bin`  |
  | Épreuve : SWOP 2006 (couché n° 3)       | `SWOP_TR003_coated_3.icc`               | `swop2006c3.bin`  |

  Une seule épreuve à la fois ; re-choisir l'entrée active l'éteint. « Épreuve CMJN » (Ctrl+Y) rallume le dernier
  profil choisi, gardé dans les réglages (`proof`). L'onglet du document et la barre d'état affichent le profil
  (« CMJN FOGRA39 »). Intention relative colorimétrique avec compensation du point noir : le blanc du papier devient
  le blanc de l'écran (pas de simulation de la teinte du papier). Régénérer toutes les tables :
  `python3 export/lut-cmjn.py --tous` (Pillow requis), puis reconstruire. Pour le profil de votre imprimeur :
  `python3 export/lut-cmjn.py --profil son.icc --sortie ui/src/assets/proof/<nom>.bin`, puis le déclarer dans
  `ui/src/lib/proof-profiles.ts` (identifiant, nom court), `ui/src/viewport/renderer.ts` (fichier de la table),
  `ui/src/lib/actions.ts` et `ui/src/lib/action-catalog.ts` (entrée du menu).

## Installer le résultat

```
sudo apt install ./export/sorties/Lumitype_*.deb                 # Debian, Ubuntu, Mint
sudo dnf install ./export/sorties/Lumitype-*.rpm                 # Fedora, Nobara
sudo zypper install ./export/sorties/Lumitype-*.rpm              # openSUSE
sudo pacman -U ./export/sorties/lumitype-*.pkg.tar.zst           # Arch, CachyOS, Manjaro
flatpak install --user ./export/sorties/Lumitype-*.flatpak       # toutes distributions
chmod +x export/sorties/Lumitype_*.AppImage && ./export/sorties/Lumitype_*.AppImage   # sans installation
tar -xzf Lumitype-*.tar.gz && ./Lumitype-*/installer.sh           # sans droits d'administrateur
```
