#!/usr/bin/env bash
# Nouvelle version de Lumitype : fixe le numéro de version si demandé, construit les paquets Linux (voir linux.sh),
# puis installe celui de cette machine si demandé.
#
#   ./export/maj.sh                    tous les formats possibles sur cette machine
#   ./export/maj.sh 0.2.0              passe d'abord la version à 0.2.0 (partout : paquets, « À propos », noms des fichiers)
#   ./export/maj.sh --install          construit, puis installe le paquet de cette distribution :
#                                        .deb (apt), .rpm (dnf, zypper) ou .pkg.tar.zst (pacman)
#   ./export/maj.sh 0.2.0 --install deb appimage     formats au choix : deb rpm arch flatpak appimage tar
#
# À lancer sur la machine Linux, depuis le dossier du projet.
set -euo pipefail

cd "$(dirname "$0")/.."
RACINE="$(pwd)"
SORTIES="$RACINE/export/sorties"

rouge() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
vert() { printf '\033[32m%s\033[0m\n' "$*"; }
titre() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

# ---- Arguments : un numéro de version, --install, et les options de linux.sh (formats, --oui)
VERSION=""
INSTALLER=0
TRANSMIS=()
for argument in "$@"; do
  case "$argument" in
    --install) INSTALLER=1 ;;
    [0-9]*.[0-9]*.[0-9]*) VERSION="$argument" ;;
    *) TRANSMIS+=("$argument") ;;
  esac
done

# ---- Famille de distribution : décide du paquet à installer
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

# ---- Numéro de version, s'il est demandé (les deux fichiers doivent rester d'accord)
if [ -n "$VERSION" ]; then
  titre "Version $VERSION"
  for fichier in "$RACINE/src-tauri/tauri.conf.json" "$RACINE/package.json"; do
    # Seule la ligne "version" de premier niveau est touchée ; les versions des dépendances ne bougent pas.
    sed -i -E '0,/^(\s*)"version":\s*"[^"]*"/s//\1"version": "'"$VERSION"'"/' "$fichier"
    grep -q "\"version\": \"$VERSION\"" "$fichier" || { rouge "Impossible de mettre à jour $fichier"; exit 1; }
  done
  echo "  tauri.conf.json et package.json à jour"
fi
ACTUELLE="$(grep -m1 '"version"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"

# ---- Construction : tous les formats (ou ceux demandés), un échec partiel n'empêche pas d'installer le reste
"$(dirname "$0")/linux.sh" "${TRANSMIS[@]}" || echo "(certains formats n'ont pas pu être produits, voir plus haut)"

[ "$INSTALLER" -eq 1 ] || exit 0

# ---- Installation sur cette machine : le paquet de sa distribution. Une version déjà présente est mise à jour ;
# préférences, raccourcis et pinceaux sont conservés.
trouver() { find "$SORTIES" -maxdepth 1 -name "$1" -print -quit; }
case "$FAMILLE" in
  debian) PAQUET="$(trouver "Lumitype_${ACTUELLE}_*.deb")"; COMMANDE=(sudo apt install -y) ;;
  fedora) PAQUET="$(trouver "Lumitype-${ACTUELLE}-*.rpm")"; COMMANDE=(sudo dnf install -y) ;;
  suse) PAQUET="$(trouver "Lumitype-${ACTUELLE}-*.rpm")"; COMMANDE=(sudo zypper install -y --allow-unsigned-rpm) ;;
  arch) PAQUET="$(trouver "lumitype-${ACTUELLE//-/_}-*.pkg.tar.*")"; COMMANDE=(sudo pacman -U --noconfirm) ;;
  *) PAQUET="" ;;
esac
if [ -z "$PAQUET" ]; then
  rouge "Aucun paquet de Lumitype $ACTUELLE pour cette distribution dans export/sorties/ : rien n'est installé."
  rouge "L'AppImage ou le Flatpak, eux, fonctionnent partout."
  exit 1
fi
titre "Installation de $(basename "$PAQUET")"
"${COMMANDE[@]}" "$PAQUET"
vert "Lumitype $ACTUELLE est installé. Fermez-le et rouvrez-le s'il tournait déjà."
