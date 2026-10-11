#!/usr/bin/env bash
# Construit Lumitype pour Linux, dans tous les formats de paquet, et les range dans export/sorties/.
#
#   ./export/linux.sh                      tous les formats possibles sur cette machine
#   ./export/linux.sh deb rpm              seulement ceux-là (formats : deb rpm arch flatpak appimage tar)
#   ./export/linux.sh --oui                installe sans demander ce qui manque pour construire
#   ./export/linux.sh --essai              compile seulement (sans paquet) puis lance Lumitype, pour essayer
#
# Moteur d'affichage : Chromium (CEF), fourni DANS le paquet — rien à installer en plus pour l'utilisateur. Il
# remplace WebKitGTK, qui plafonne vers 45-50 i/s en plein écran 4K. La première compilation télécharge Chromium
# (environ 300 Mo) ; les suivantes le réutilisent. LUMITYPE_MOTEUR=webkit ./export/linux.sh reconstruit l'ancienne
# version WebKitGTK.
#
# Ce que chaque format donne (version 1.1.0, PC 64 bits) :
#   Lumitype_1.1.0_amd64.deb              Debian, Ubuntu, Mint, Pop!_OS…         (apt)
#   Lumitype-1.1.0-1.x86_64.rpm           Fedora, Nobara, openSUSE, RHEL…        (dnf / zypper)
#   lumitype-1.1.0-1-x86_64.pkg.tar.zst   Arch, CachyOS, Manjaro, EndeavourOS…   (pacman)
#   Lumitype-1.1.0-x86_64.flatpak         toutes distributions                   (flatpak)
#   Lumitype_1.1.0_amd64.AppImage         toutes distributions, sans installation
#   Lumitype-1.1.0-x86_64.tar.gz          toutes distributions, à décompresser
#
# Le logiciel n'est compilé qu'UNE fois : le .deb, le .rpm et l'AppImage sortent de cette compilation, et les
# trois autres formats sont fabriqués à partir du .deb. C'est donc exactement le même programme partout.
# Un format impossible sur cette machine (le paquet Arch hors d'Arch, par exemple) est sauté, avec la raison.
#
# À lancer SUR une machine Linux : ces paquets ne peuvent pas être construits depuis Windows.
set -euo pipefail

cd "$(dirname "$0")/.."
RACINE="$(pwd)"
SORTIES="$RACINE/export/sorties"
TRAVAIL="$RACINE/export/.travail"
# Socle Flatpak : GNOME 47 (bibliothèques du bureau ; Chromium est fourni par Lumitype). Remplaçable : LUMITYPE_FLATPAK_GNOME=48 …
FLATPAK_GNOME="${LUMITYPE_FLATPAK_GNOME:-47}"

rouge() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
jaune() { printf '\033[33m%s\033[0m\n' "$*" >&2; }
vert() { printf '\033[32m%s\033[0m\n' "$*"; }
titre() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

# ---- Arguments
TOUS="deb rpm arch flatpak appimage tar"
DEMANDES=""
OUI=0
ESSAI=0
for argument in "$@"; do
  case "$argument" in
    --oui | -y) OUI=1 ;;
    --essai) ESSAI=1 ;;
    deb | rpm | arch | flatpak | appimage | tar) DEMANDES="$DEMANDES $argument" ;;
    pacman | pkg) DEMANDES="$DEMANDES arch" ;;
    *) rouge "Argument inconnu : $argument (formats : $TOUS ; options : --oui, --essai)"; exit 2 ;;
  esac
done
EXPLICITE=1
if [ -z "$DEMANDES" ]; then DEMANDES="$TOUS"; EXPLICITE=0; fi
demande() { [[ " $DEMANDES " == *" $1 "* ]]; }

# ---- Machine : famille de distribution (d'après /etc/os-release) et architecture
FAMILLE=inconnue
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  case " ${ID:-} ${ID_LIKE:-} " in
    *" arch "* | *" archlinux "* | *" cachyos "* | *" manjaro "*) FAMILLE=arch ;;
    *" fedora "* | *" rhel "* | *" centos "* | *" nobara "*) FAMILLE=fedora ;;
    *suse*) FAMILLE=suse ;;
    *" debian "* | *" ubuntu "*) FAMILLE=debian ;;
  esac
fi
case "$(uname -m)" in
  x86_64) ARCH=x86_64; ARCH_DEB=amd64 ;;
  aarch64) ARCH=aarch64; ARCH_DEB=arm64 ;;
  *) ARCH="$(uname -m)"; ARCH_DEB="$ARCH" ;;
esac
VERSION="$(grep -m1 '"version"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"
IDENTIFIANT="$(grep -m1 '"identifier"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"
echo "Lumitype $VERSION — ${PRETTY_NAME:-Linux} ($FAMILLE, $ARCH)"

# Installe des paquets de la distribution, après accord (ou directement avec --oui).
installer() {
  local commande
  case "$FAMILLE" in
    fedora) commande="sudo dnf install -y $*" ;;
    debian) commande="sudo apt-get install -y $*" ;;
    arch) commande="sudo pacman -S --needed --noconfirm $*" ;;
    suse) commande="sudo zypper install -y $*" ;;
    *) rouge "Distribution non reconnue : installez vous-même $*"; return 1 ;;
  esac
  echo "  À installer : $*"
  if [ "$OUI" -eq 0 ]; then
    if [ ! -t 0 ]; then rouge "  Lancez : $commande (ou relancez ce script avec --oui)"; return 1; fi
    read -r -p "  Lancer « $commande » ? [O/n] " reponse
    case "$reponse" in [nN]*) return 1 ;; esac
  fi
  eval "$commande"
}

IGNORES=()
ignorer() {
  IGNORES+=("$1 : $2")
  DEMANDES="$(echo " $DEMANDES " | sed "s/ $1 / /")"
  if [ "$EXPLICITE" -eq 1 ]; then rouge "  $1 impossible : $2"; else jaune "  $1 sauté : $2"; fi
}

# ---- Outils indispensables
titre "Vérification des outils"
manque=0
for outil in node npm cargo rustc; do
  if ! command -v "$outil" >/dev/null 2>&1; then rouge "Introuvable : $outil"; manque=1; fi
done
if [ "$manque" -eq 1 ]; then
  cat >&2 <<'AIDE'

Installez d'abord :
  - Node.js 20 ou plus récent : https://nodejs.org (ou le paquet « nodejs » de votre distribution)
  - Rust : curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
AIDE
  exit 1
fi
echo "  node $(node --version), rust $(rustc --version | cut -d' ' -f2)"

# ---- Bibliothèques de développement (WebKitGTK et compagnie), selon la distribution
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  titre "Bibliothèques de construction manquantes (webkit2gtk-4.1)"
  case "$FAMILLE" in
    fedora) installer webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel binutils &&
      sudo dnf group install -y c-development ;;
    debian) installer libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev ;;
    arch) installer webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool ;;
    suse) installer webkit2gtk3-devel libopenssl-devel curl wget file libappindicator3-1 librsvg-devel binutils &&
      sudo zypper install -y -t pattern devel_basis ;;
    *) false ;;
  esac || { rouge "Voir https://v2.tauri.app/start/prerequisites/ pour votre distribution."; exit 1; }
fi

# ---- Moteur d'affichage : Chromium par défaut (options passées à « tauri build »)
MOTEUR="${LUMITYPE_MOTEUR:-chromium}"
case "$MOTEUR" in
  chromium | cef)
    MOTEUR=chromium
    OPTIONS_TAURI=(--features cef --config src-tauri/tauri.cef.json)
    OPTIONS_CARGO=(-- --no-default-features)
    ;;
  webkit) OPTIONS_TAURI=(); OPTIONS_CARGO=() ;;
  *) rouge "LUMITYPE_MOTEUR inconnu : $MOTEUR (chromium ou webkit)"; exit 2 ;;
esac
echo "  Moteur d'affichage : $MOTEUR"

# ---- Polices incluses (src-tauri/fonts) : sans elles, Tauri refuse de compiler (ressource `fonts/**/*` introuvable).
if ! find src-tauri/fonts -type f \( -iname '*.ttf' -o -iname '*.otf' \) -print -quit 2>/dev/null | grep -q .; then
  rouge "Le dossier src-tauri/fonts est vide ou absent : les polices incluses manquent. Décompressez l'archive complète du projet."
  exit 1
fi

# ---- Essai : compilation seule, puis lancement (aucun paquet)
if [ "$ESSAI" -eq 1 ]; then
  titre "Dépendances de l'interface"
  [ -d node_modules ] || { [ -f package-lock.json ] && npm ci || npm install; }
  titre "Compilation pour essai ($MOTEUR) — sans paquet"
  "$(dirname "$0")/sync-modeles.sh"
  npm run tauri build -- --no-bundle "${OPTIONS_TAURI[@]}" "${OPTIONS_CARGO[@]}"
  vert "Prêt : $RACINE/target/release/lumitype"
  exec "$RACINE/target/release/lumitype"
fi

# ---- Ce que demande chaque format en plus
titre "Formats"
if ! command -v ar >/dev/null 2>&1 && { demande arch || demande flatpak || demande tar; }; then
  installer binutils || true
fi
if demande arch; then
  if [ "$FAMILLE" != arch ] || ! command -v makepkg >/dev/null 2>&1; then
    ignorer arch "le paquet pacman ne se fabrique que sous Arch, CachyOS, Manjaro… (outil makepkg)"
  elif [ "$(id -u)" -eq 0 ]; then
    ignorer arch "makepkg refuse de tourner en administrateur : relancez ce script sans sudo"
  fi
fi
if demande flatpak; then
  if ! command -v flatpak-builder >/dev/null 2>&1; then
    installer flatpak flatpak-builder || true
  fi
  if ! command -v flatpak-builder >/dev/null 2>&1; then
    ignorer flatpak "flatpak-builder introuvable (paquet « flatpak-builder »)"
  else
    # Socle GNOME, téléchargé une seule fois (environ 1 Go), dans le dossier de l'utilisateur.
    if ! flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo ||
      ! flatpak install --user --noninteractive flathub "org.gnome.Platform//$FLATPAK_GNOME" "org.gnome.Sdk//$FLATPAK_GNOME"; then
      ignorer flatpak "socle org.gnome.Platform//$FLATPAK_GNOME impossible à télécharger depuis Flathub (connexion ?)"
    fi
  fi
fi
DEMANDES="$(echo $DEMANDES)"
if [ -z "$DEMANDES" ]; then rouge "Aucun format possible sur cette machine."; exit 1; fi
echo "  À produire : $DEMANDES"

# ---- Compilation, une seule fois : Tauri fabrique le .deb (base des autres), le .rpm et l'AppImage
BUNDLES=""
for format in deb rpm appimage; do demande "$format" && BUNDLES="$BUNDLES,$format"; done
if demande arch || demande flatpak || demande tar; then [[ "$BUNDLES" == *deb* ]] || BUNDLES="$BUNDLES,deb"; fi
BUNDLES="${BUNDLES#,}"

titre "Dépendances de l'interface"
[ -d node_modules ] || { [ -f package-lock.json ] && npm ci || npm install; }

titre "Compilation ($BUNDLES) — dix à vingt minutes la première fois"
# L'outil qui fabrique l'AppImage (linuxdeploy) est lui-même une AppImage : il lui faut FUSE 2, absent de
# beaucoup de distributions récentes. Cette variable lui fait extraire son contenu et s'exécuter sans FUSE ;
# elle est sans effet quand FUSE est disponible, donc on la pose toujours.
export APPIMAGE_EXTRACT_AND_RUN=1
# Certaines AppImages écrivent leur dossier temporaire ici : sans cela, linuxdeploy échoue sans rien dire.
export TMPDIR="${TMPDIR:-/tmp}"
"$(dirname "$0")/sync-modeles.sh"
DEBUT="$(mktemp)"
trap 'rm -f "$DEBUT"' EXIT
echec_tauri=0
npm run tauri build -- --bundles "$BUNDLES" "${OPTIONS_TAURI[@]}" "${OPTIONS_CARGO[@]}" || echec_tauri=1

# Paquet produit par CETTE construction (plus récent que son début), ou rien.
produit() {
  local dossier
  for dossier in "$RACINE/target/release/bundle" "$RACINE/src-tauri/target/release/bundle"; do
    if [ -d "$dossier" ]; then find "$dossier" -maxdepth 2 -type f -name "$1" -newer "$DEBUT" -print -quit; fi
  done | head -n1
}

mkdir -p "$SORTIES"
PRODUITS=()
ranger() {
  cp -f "$1" "$SORTIES/$2"
  PRODUITS+=("$2  ($(du -h "$SORTIES/$2" | cut -f1))")
}

titre "Rangement dans export/sorties/"
DEB="$(produit "*.deb")"
demande deb && { [ -n "$DEB" ] && ranger "$DEB" "Lumitype_${VERSION}_${ARCH_DEB}.deb" || ignorer deb "la compilation a échoué"; }
if demande rpm; then
  RPM="$(produit "*.rpm")"
  [ -n "$RPM" ] && ranger "$RPM" "$(basename "$RPM")" || ignorer rpm "la compilation a échoué"
fi
if demande appimage; then
  APPIMAGE="$(produit "*.AppImage")"
  [ -n "$APPIMAGE" ] && ranger "$APPIMAGE" "$(basename "$APPIMAGE")" ||
    ignorer appimage "échec de linuxdeploy (installez FUSE 2 : fuse2 / libfuse2 / fuse-libs)"
fi
if [ -z "$DEB" ]; then
  for format in arch flatpak tar; do demande "$format" && ignorer "$format" "il se fabrique à partir du .deb, qui a échoué"; done
fi

# Contenu du .deb (usr/bin, usr/lib/Lumitype, usr/share…) dans un dossier.
extraire_deb() {
  rm -rf "$2" && mkdir -p "$2"
  (cd "$2" && ar x "$1" && tar -xf data.tar.* && rm -f data.tar.* control.tar.* debian-binary)
}

# ---- Archive portable : le programme et ses fichiers, plus un installateur pour l'utilisateur courant
if demande tar; then
  titre "Archive portable (.tar.gz)"
  NOM="Lumitype-$VERSION-$ARCH"
  rm -rf "$TRAVAIL/tar" && mkdir -p "$TRAVAIL/tar/$NOM"
  extraire_deb "$DEB" "$TRAVAIL/tar/deb"
  cp -a "$TRAVAIL/tar/deb/usr/." "$TRAVAIL/tar/$NOM/"
  cat >"$TRAVAIL/tar/$NOM/installer.sh" <<'INSTALLER'
#!/usr/bin/env bash
# Installe Lumitype pour l'utilisateur courant (dans ~/.local), sans droits d'administrateur.
#   ./installer.sh              installe
#   ./installer.sh --retirer    désinstalle
# Le moteur d'affichage (Chromium) est fourni dans lib/Lumitype : rien d'autre à installer.
set -euo pipefail
ICI="$(cd "$(dirname "$0")" && pwd)"
DEST="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$HOME/.local/bin"
if [ "${1:-}" = "--retirer" ]; then
  rm -f "$BIN/lumitype" "$DEST/applications/Lumitype.desktop"
  rm -rf "$HOME/.local/lib/Lumitype"
  find "$DEST/icons/hicolor" -name lumitype.png -delete 2>/dev/null || true
  rm -f "$DEST/mime/packages/lumitype.xml" && update-mime-database "$DEST/mime" 2>/dev/null || true
  echo "Lumitype est désinstallé (vos préférences sont conservées)."
  exit 0
fi
mkdir -p "$BIN" "$HOME/.local/lib" "$DEST/applications"
install -m755 "$ICI/bin/lumitype" "$BIN/lumitype"
[ -d "$ICI/lib/Lumitype" ] && rm -rf "$HOME/.local/lib/Lumitype" && cp -a "$ICI/lib/Lumitype" "$HOME/.local/lib/Lumitype"
cp -a "$ICI/share/icons" "$DEST/" 2>/dev/null || true
[ -d "$ICI/share/mime" ] && cp -a "$ICI/share/mime" "$DEST/" && update-mime-database "$DEST/mime" 2>/dev/null || true
sed "s|^Exec=lumitype|Exec=$BIN/lumitype|" "$ICI/share/applications/Lumitype.desktop" >"$DEST/applications/Lumitype.desktop"
update-desktop-database "$DEST/applications" 2>/dev/null || true
echo "Lumitype est installé : menu des applications, ou « $BIN/lumitype »."
INSTALLER
  chmod +x "$TRAVAIL/tar/$NOM/installer.sh"
  tar -C "$TRAVAIL/tar" -czf "$SORTIES/$NOM.tar.gz" "$NOM"
  PRODUITS+=("$NOM.tar.gz  ($(du -h "$SORTIES/$NOM.tar.gz" | cut -f1))")
fi

# ---- Paquet Arch (pacman) : le contenu du .deb, décrit par un PKGBUILD et emballé par makepkg
if demande arch; then
  titre "Paquet Arch / CachyOS (.pkg.tar.zst)"
  rm -rf "$TRAVAIL/arch" && mkdir -p "$TRAVAIL/arch"
  cp "$DEB" "$TRAVAIL/arch/Lumitype.deb"
  cat >"$TRAVAIL/arch/PKGBUILD" <<PKGBUILD
# Paquet produit par export/linux.sh à partir du .deb de la même construction.
pkgname=lumitype
pkgver=${VERSION//-/_}
pkgrel=1
pkgdesc="Éditeur d'images raster : calques, masques, sélections, texte, réglages et effets"
arch=('$ARCH')
license=('MIT' 'Apache-2.0')
depends=('webkit2gtk-4.1' 'gtk3' 'hicolor-icon-theme' 'desktop-file-utils' 'shared-mime-info')
options=('!strip' '!debug')
source=('Lumitype.deb')
noextract=('Lumitype.deb')
sha256sums=('SKIP')

package() {
  bsdtar -xf "\$srcdir/Lumitype.deb" -C "\$srcdir"
  bsdtar -xf "\$srcdir"/data.tar.* -C "\$pkgdir"
}
PKGBUILD
  if (cd "$TRAVAIL/arch" && PKGDEST="$SORTIES" makepkg -f --nodeps --noconfirm); then
    PAQUET="$(find "$SORTIES" -maxdepth 1 -name "lumitype-${VERSION//-/_}-1-$ARCH.pkg.tar.*" -print -quit)"
    PRODUITS+=("$(basename "$PAQUET")  ($(du -h "$PAQUET" | cut -f1))")
  else
    ignorer arch "makepkg a échoué (voir plus haut)"
  fi
fi

# ---- Flatpak : le contenu du .deb dans le socle GNOME, en un seul fichier installable partout
if demande flatpak; then
  titre "Flatpak (socle GNOME $FLATPAK_GNOME)"
  rm -rf "$TRAVAIL/flatpak" && mkdir -p "$TRAVAIL/flatpak"
  cp "$DEB" "$TRAVAIL/flatpak/Lumitype.deb"
  cat >"$TRAVAIL/flatpak/$IDENTIFIANT.metainfo.xml" <<METAINFO
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>$IDENTIFIANT</id>
  <name>Lumitype</name>
  <summary>Éditeur d'images raster</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>MIT OR Apache-2.0</project_license>
  <description><p>Calques, masques, sélections, texte, tracés, réglages non destructifs, filtres et effets de calque.</p></description>
  <launchable type="desktop-id">$IDENTIFIANT.desktop</launchable>
  <releases><release version="$VERSION"/></releases>
</component>
METAINFO
  # Accès : écran (Wayland et X11), carte graphique, dossier personnel (ouvrir et enregistrer partout), réseau
  # (importer une image depuis une adresse web). Chromium est fourni dans /app/lib/Lumitype.
  cat >"$TRAVAIL/flatpak/$IDENTIFIANT.json" <<MANIFESTE
{
  "id": "$IDENTIFIANT",
  "runtime": "org.gnome.Platform",
  "runtime-version": "$FLATPAK_GNOME",
  "sdk": "org.gnome.Sdk",
  "command": "lumitype",
  "finish-args": ["--socket=x11", "--socket=wayland", "--share=ipc", "--device=dri", "--filesystem=home", "--share=network"],
  "modules": [
    {
      "name": "lumitype",
      "buildsystem": "simple",
      "sources": [
        { "type": "file", "path": "Lumitype.deb" },
        { "type": "file", "path": "$IDENTIFIANT.metainfo.xml" }
      ],
      "build-commands": [
        "mkdir deb && cd deb && ar x ../Lumitype.deb && tar -xf data.tar.*",
        "install -Dm755 deb/usr/bin/lumitype /app/bin/lumitype",
        "if [ -d deb/usr/lib/Lumitype ]; then mkdir -p /app/lib && cp -a deb/usr/lib/Lumitype /app/lib/Lumitype; fi",
        "install -Dm644 deb/usr/share/applications/Lumitype.desktop /app/share/applications/$IDENTIFIANT.desktop",
        "sed -i 's/^Icon=.*/Icon=$IDENTIFIANT/' /app/share/applications/$IDENTIFIANT.desktop",
        "for icone in deb/usr/share/icons/hicolor/*/apps/lumitype.png; do taille=\$(basename \$(dirname \$(dirname \$icone))); case \$taille in *@*) continue ;; esac; install -Dm644 \$icone /app/share/icons/hicolor/\$taille/apps/$IDENTIFIANT.png; done",
        "if [ -f deb/usr/share/mime/packages/lumitype.xml ]; then sed 's/icon name=\"lumitype\"/icon name=\"$IDENTIFIANT\"/' deb/usr/share/mime/packages/lumitype.xml > lumitype-mime.xml && install -Dm644 lumitype-mime.xml /app/share/mime/packages/$IDENTIFIANT.xml; fi",
        "install -Dm644 $IDENTIFIANT.metainfo.xml /app/share/metainfo/$IDENTIFIANT.metainfo.xml"
      ]
    }
  ]
}
MANIFESTE
  NOM="Lumitype-$VERSION-$ARCH.flatpak"
  if (cd "$TRAVAIL/flatpak" &&
    flatpak-builder --user --force-clean --disable-rofiles-fuse --repo=depot construction "$IDENTIFIANT.json" &&
    flatpak build-bundle depot "$SORTIES/$NOM" "$IDENTIFIANT" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo); then
    PRODUITS+=("$NOM  ($(du -h "$SORTIES/$NOM" | cut -f1))")
  else
    ignorer flatpak "flatpak-builder a échoué (voir plus haut)"
  fi
fi

# ---- Bilan
titre "Bilan"
if [ "${#PRODUITS[@]}" -eq 0 ]; then
  rouge "Aucun paquet produit."
  [ "$echec_tauri" -eq 1 ] && rouge "La compilation a échoué plus haut."
  exit 1
fi
for ligne in "${PRODUITS[@]}"; do vert "  $ligne"; done
for ligne in "${IGNORES[@]}"; do jaune "  sauté — $ligne"; done

cat <<AIDE

Paquets dans export/sorties/. Pour installer :
  Debian, Ubuntu, Mint    : sudo apt install ./export/sorties/Lumitype_*.deb
  Fedora, Nobara          : sudo dnf install ./export/sorties/Lumitype-*.rpm
  openSUSE                : sudo zypper install ./export/sorties/Lumitype-*.rpm
  Arch, CachyOS, Manjaro  : sudo pacman -U ./export/sorties/lumitype-*.pkg.tar.zst
  Flatpak (partout)       : flatpak install --user ./export/sorties/Lumitype-*.flatpak
  AppImage (partout)      : chmod +x export/sorties/Lumitype_*.AppImage && ./export/sorties/Lumitype_*.AppImage
  Archive (partout)       : tar -xzf Lumitype-*.tar.gz && ./Lumitype-*/installer.sh
AIDE
# Un format demandé explicitement et impossible est un échec ; un format sauté par défaut ne l'est pas.
[ "$EXPLICITE" -eq 1 ] && [ "${#IGNORES[@]}" -gt 0 ] && exit 1
exit 0
