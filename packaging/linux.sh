#!/usr/bin/env bash
# Construit Lumitype pour Linux — tous les formats de paquet — et, si demandé, les installateurs Windows, en une seule
# commande. Les fichiers arrivent dans packaging/out/ ; avec --release, ils sont aussi copiés sous leur nom de
# diffusion dans packaging/release/ avec SHA256SUMS.txt (prêts pour une Release GitHub).
#
#   ./packaging/linux.sh                       tous les formats Linux possibles sur cette machine
#   ./packaging/linux.sh deb rpm               seulement ceux-là (deb rpm arch flatpak appimage tar)
#   ./packaging/linux.sh 1.3.0                 fixe d'abord la version (partout), puis construit
#   ./packaging/linux.sh --windows             ajoute les 3 fichiers Windows (setup.exe, msi, portable) via cargo-xwin
#   ./packaging/linux.sh --release             copie sous les noms de diffusion + SHA256SUMS.txt
#   ./packaging/linux.sh --install             installe ensuite le paquet de cette distribution
#   ./packaging/linux.sh --oui                 installe sans demander ce qui manque pour construire
#   ./packaging/linux.sh --essai               compile seulement (sans paquet) puis lance Lumitype, pour essayer
#   ./packaging/linux.sh --compat              compile dans un conteneur Ubuntu 22.04 : paquets qui démarrent aussi sur
#                                              les distributions plus anciennes (Mint 21+, Ubuntu 22.04+, Debian 12+)
#   ./packaging/linux.sh 1.3.0 --windows --release --install     tout, d'un coup
#
# Moteur d'affichage : Chromium (CEF), fourni DANS le paquet — rien à installer en plus pour l'utilisateur. Il
# remplace WebKitGTK, qui plafonne vers 45-50 i/s en plein écran 4K. La première compilation télécharge Chromium
# (environ 300 Mo) ; les suivantes le réutilisent. LUMITYPE_MOTEUR=webkit ./packaging/linux.sh reconstruit l'ancienne
# version WebKitGTK.
#
# Ce que chaque format donne (version 1.2.88, PC 64 bits) :
#   Lumitype_1.2.88_amd64.deb              Debian, Ubuntu, Mint, Pop!_OS…         (apt)
#   Lumitype-1.2.88-1.x86_64.rpm           Fedora, Nobara, openSUSE, RHEL…        (dnf / zypper)
#   lumitype-1.2.88-1-x86_64.pkg.tar.zst   Arch, CachyOS, Manjaro, EndeavourOS…   (pacman)
#   Lumitype-1.2.88-x86_64.flatpak         toutes distributions                   (flatpak)
#   Lumitype_1.2.88_amd64.AppImage         toutes distributions, sans installation
#   Lumitype-1.2.88-x86_64.tar.gz          toutes distributions, à décompresser
#   --windows : Lumitype_1.2.88_x64-setup.exe, Lumitype_1.2.88_x64_fr-FR.msi, Lumitype-portable.exe
#
# Vitesse : le logiciel n'est compilé qu'UNE fois (le .deb, le .rpm et l'AppImage sortent de cette compilation) ;
# l'archive, le paquet Arch et le Flatpak sont ensuite fabriqués EN PARALLÈLE à partir du .deb ; les dépendances de
# l'interface ne sont réinstallées que si package-lock.json a changé ; la construction Windows tourne en même temps
# que l'empaquetage Linux. Un format impossible sur cette machine est sauté, avec la raison.
#
# Compatibilité : un exécutable Linux ne démarre que sur une glibc AU MOINS aussi récente que celle de la machine qui l'a
# compilé (« GLIBC_2.43 not found » sinon). Après chaque compilation, la glibc demandée est affichée avec les
# distributions qui l'ont ; au-delà de 2.35, un avertissement propose --compat, et --release refuse de publier.
# --compat compile dans un conteneur Ubuntu 22.04 (Podman ou Docker, préparé tout seul la première fois) : les
# paquets démarrent alors sur Mint 21 et plus, Ubuntu 22.04 et plus, Debian 12 et plus, Fedora, Arch…
# Seuil accepté pour --release : LUMITYPE_GLIBC_MAX=2.39 ./packaging/linux.sh --release (2.35 par défaut).
#
# À lancer SUR une machine Linux. Les scripts d'appui sont dans packaging/lib/.
set -euo pipefail

cd "$(dirname "$0")/.."
RACINE="$(pwd)"
# WSL : Windows ajoute ses dossiers au PATH (node.exe, npm, cargo de Windows…) ; ils ne servent à rien ici et trompent
# les vérifications (« npm présent, Node absent »). Seuls les outils Linux de la distribution comptent.
if grep -qi microsoft /proc/version 2>/dev/null; then
  PATH="$(printf '%s' "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd: -)"
  export PATH
  SOUS_WSL=1
else
  SOUS_WSL=0
fi
# Rust installé par rustup pour cet utilisateur, pas encore dans le PATH de cette console.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
SORTIES="$RACINE/packaging/out"
TRAVAIL="$RACINE/packaging/.work"
RELEASE="$RACINE/packaging/release"
LIB="$RACINE/packaging/lib"
# Socle Flatpak : GNOME 51 (bibliothèques du bureau ; Chromium est fourni par Lumitype). Un socle GNOME n'est suivi
# qu'environ un an (GNOME 47 : fin de vie le 15/10/2025) : le monter à chaque nouvelle version de GNOME, sinon
# « flatpak update » avertit les utilisateurs. Remplaçable : LUMITYPE_FLATPAK_GNOME=50 ./packaging/linux.sh flatpak
FLATPAK_GNOME="${LUMITYPE_FLATPAK_GNOME:-51}"

rouge() { printf '\033[31m%s\033[0m\n' "$*" >&2; }
jaune() { printf '\033[33m%s\033[0m\n' "$*" >&2; }
vert() { printf '\033[32m%s\033[0m\n' "$*"; }
titre() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

# ---- Arguments
TOUS="deb rpm arch flatpak appimage tar"
DEMANDES=""
OUI=0
ESSAI=0
WINDOWS=0
RELEASE_DEMANDE=0
INSTALLER_APRES=0
COMPAT=0
NOUVELLE_VERSION=""
ARGUMENTS=("$@")
for argument in "$@"; do
  case "$argument" in
    --oui | -y) OUI=1 ;;
    --essai) ESSAI=1 ;;
    --windows) WINDOWS=1 ;;
    --release) RELEASE_DEMANDE=1 ;;
    --install) INSTALLER_APRES=1 ;;
    --compat) COMPAT=1 ;;
    [0-9]*.[0-9]*.[0-9]*) NOUVELLE_VERSION="$argument" ;;
    deb | rpm | arch | flatpak | appimage | tar) DEMANDES="$DEMANDES $argument" ;;
    pacman | pkg) DEMANDES="$DEMANDES arch" ;;
    *) rouge "Argument inconnu : $argument (formats : $TOUS ; options : --windows --release --install --oui --essai --compat, ou un numéro de version)"; exit 2 ;;
  esac
done
EXPLICITE=1
if [ -z "$DEMANDES" ]; then DEMANDES="$TOUS"; EXPLICITE=0; fi
demande() { [[ " $DEMANDES " == *" $1 "* ]]; }

# ---- WSL, projet dans un dossier Windows (/mnt/c/…) : la construction se fait dans une copie côté Linux
# (~/.cache/lumitype-wsl/projet), puis les paquets reviennent dans packaging/out/ du dossier Windows. Le disque Windows
# vu de WSL est 3 à 5 fois plus lent, et ses particularités (droits, liens, espaces dans le chemin) font échouer
# l'AppImage (linuxdeploy) et le Flatpak (flatpak-builder). La copie est mise à jour à chaque fois (seuls les fichiers
# modifiés sont recopiés) ; sa compilation (target/) et ses dépendances (node_modules/) sont gardées d'une fois sur
# l'autre. LUMITYPE_SUR_PLACE=1 construit directement dans le dossier Windows, comme avant.
if [ "$SOUS_WSL" -eq 1 ] && [[ "$RACINE" == /mnt/* ]] && [ "${LUMITYPE_SUR_PLACE:-0}" != 1 ] && [ "${LUMITYPE_COPIE_WSL:-0}" != 1 ]; then
  COPIE="${XDG_CACHE_HOME:-$HOME/.cache}/lumitype-wsl/projet"
  titre "WSL : construction dans une copie côté Linux ($COPIE)"
  command -v rsync >/dev/null 2>&1 || { sudo apt-get update -q && sudo apt-get install -y rsync; } || { rouge "rsync est nécessaire : sudo apt install rsync"; exit 1; }
  mkdir -p "$COPIE"
  rsync -a --delete --exclude '/target/' --exclude '/node_modules/' --exclude '/packaging/out/' --exclude '/packaging/.work/' \
    --exclude '/packaging/release/' --exclude '/.git/' --exclude '/ClaudeBackups/' "$RACINE/" "$COPIE/"
  echo "  Copie à jour."
  code=0
  (cd "$COPIE" && LUMITYPE_COPIE_WSL=1 bash ./packaging/linux.sh "${ARGUMENTS[@]}") || code=$?
  # Paquets (et numéro de version, s'il a été changé) rapportés dans le dossier Windows.
  mkdir -p "$RACINE/packaging/out"
  [ -d "$COPIE/packaging/out" ] && rsync -a "$COPIE/packaging/out/" "$RACINE/packaging/out/"
  # Journaux (compilation, flatpak, tar…) rapportés aussi, pour pouvoir les ouvrir depuis Windows.
  [ -d "$COPIE/packaging/.work" ] && mkdir -p "$RACINE/packaging/.work" && rsync -a --include '*.log' --exclude '*' "$COPIE/packaging/.work/" "$RACINE/packaging/.work/"
  [ -d "$COPIE/packaging/release" ] && mkdir -p "$RACINE/packaging/release" && rsync -a "$COPIE/packaging/release/" "$RACINE/packaging/release/"
  if [ -n "$NOUVELLE_VERSION" ]; then
    for fichier in src-tauri/tauri.conf.json package.json src-tauri/Cargo.toml Cargo.toml; do cp -f "$COPIE/$fichier" "$RACINE/$fichier"; done
  fi
  [ "$code" -eq 0 ] && vert "Paquets rapportés dans packaging/out/ (dossier Windows)." || rouge "Échec dans la copie WSL (code $code) : voir les messages ci-dessus. Journaux : $COPIE/packaging/.work/"
  exit "$code"
fi

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
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]; }

# ---- Compatibilité glibc : plus haute version demandée par l'exécutable et par Chromium, et distributions qui l'ont.
# Un programme compilé ici ne démarre que sur une glibc au moins aussi récente que celle de cette machine.
GLIBC_MAX_DIFFUSION="${LUMITYPE_GLIBC_MAX:-2.35}"
glibc_requise() { # fichiers… → « 2.35 » (vide si rien n'est lisible)
  local f
  for f in "$@"; do
    [ -f "$f" ] || continue
    if command -v objdump >/dev/null 2>&1; then objdump -T "$f" 2>/dev/null; else readelf -V --wide "$f" 2>/dev/null; fi
  done | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/^GLIBC_//' | sort -Vu | tail -n1
}
# Glibc de chaque version de distribution (la plus ancienne qui suffit est retenue pour chaque famille).
GLIBC_DISTRIBUTIONS="
Linux Mint 20|2.31
Linux Mint 21|2.35
Linux Mint 22|2.39
Ubuntu 20.04|2.31
Ubuntu 22.04|2.35
Ubuntu 24.04|2.39
Ubuntu 25.04|2.41
Ubuntu 26.04|2.43
Debian 11|2.31
Debian 12|2.36
Debian 13|2.41
Fedora 36|2.35
Fedora 40|2.39
Fedora 41|2.40
Fedora 42|2.41
Fedora 43|2.42
Fedora 44|2.43"
distributions_compatibles() { # version glibc → « Linux Mint 21 et plus, Ubuntu 22.04 et plus… »
  local requise="$1" famille vu="" sortie="" nom glibc
  while IFS='|' read -r nom glibc; do
    [ -n "$nom" ] || continue
    famille="${nom% *}"
    [[ "$vu" == *"|$famille|"* ]] && continue
    if version_ge "$glibc" "$requise"; then vu="$vu|$famille|"; sortie="$sortie, $nom et plus"; fi
  done <<<"$GLIBC_DISTRIBUTIONS"
  echo "${sortie#, }"
}
# Vérifie la construction qui vient de se faire. 0 : diffusable ; 1 : trop récente (avertissement, ou refus avec --release).
verifier_glibc() {
  local cible="${CARGO_TARGET_DIR:-$RACINE/target}/release" requise
  requise="$(glibc_requise "$cible/lumitype" "$cible"/cef-bundle/*.so)"
  if [ -z "$requise" ]; then jaune "  glibc demandée : illisible (objdump et readelf absents ?)"; return 0; fi
  if version_ge "$GLIBC_MAX_DIFFUSION" "$requise"; then
    vert "  glibc demandée : $requise — démarre sur $(distributions_compatibles "$requise")"
    return 0
  fi
  local message="glibc demandée : $requise (au-delà de $GLIBC_MAX_DIFFUSION) — ces paquets ne démarrent QUE sur $(distributions_compatibles "$requise").
  Sur une distribution plus ancienne : « version 'GLIBC_$requise' not found » au lancement.
  Paquets pour tout le monde : ./packaging/linux.sh --compat (conteneur Ubuntu 22.04, glibc 2.35)."
  if [ "$RELEASE_DEMANDE" -eq 1 ]; then
    rouge "  $message"
    rouge "  --release refusé. Seuil plus haut, en connaissance de cause : LUMITYPE_GLIBC_MAX=$requise ./packaging/linux.sh --release"
  else
    jaune "  $message"
  fi
  return 1
}

# Installe le paquet de cette distribution qui vient d'être produit (--install).
installer_paquet_produit() {
  local version paquet commande
  version="$(grep -m1 '"version"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"
  trouver() { find "$SORTIES" -maxdepth 1 -name "$1" -print -quit 2>/dev/null; }
  case "$FAMILLE" in
    debian) paquet="$(trouver "Lumitype_${version}_*.deb")"; commande=(sudo apt install -y) ;;
    fedora) paquet="$(trouver "Lumitype-${version}-*.rpm")"; commande=(sudo dnf install -y) ;;
    suse) paquet="$(trouver "Lumitype-${version}-*.rpm")"; commande=(sudo zypper install -y --allow-unsigned-rpm) ;;
    arch) paquet="$(trouver "lumitype-${version//-/_}-*.pkg.tar.*")"; commande=(sudo pacman -U --noconfirm) ;;
    *) paquet="" ;;
  esac
  if [ -z "$paquet" ]; then
    rouge "Aucun paquet de Lumitype $version pour cette distribution dans packaging/out/ : rien n'est installé."
  else
    titre "Installation de $(basename "$paquet")"
    "${commande[@]}" "$paquet"
    vert "Lumitype $version est installé. Fermez-le et rouvrez-le s'il tournait déjà."
  fi
}

# ---- --compat : la même construction, dans un conteneur Ubuntu 22.04 (glibc 2.35), puis retour ici.
# Le projet est monté tel quel (les paquets arrivent dans packaging/out/ comme d'habitude) ; la compilation garde son
# propre dossier target/ et ses caches (~/.cache/lumitype-compat), pour ne jamais se mélanger avec celle de cette
# machine. L'image (outils de construction, Rust, Node 20) est préparée une seule fois : packaging/lib/Containerfile.ubuntu22.
if [ "$COMPAT" -eq 1 ] && [ "${LUMITYPE_DANS_CONTENEUR:-0}" != 1 ]; then
  [ "$ESSAI" -eq 1 ] && { rouge "--essai et --compat ne vont pas ensemble : --essai lance Lumitype sur cette machine."; exit 2; }
  [ "$WINDOWS" -eq 1 ] && { rouge "--windows se construit sans --compat (les fichiers Windows ne dépendent pas de la glibc)."; exit 2; }
  if command -v podman >/dev/null 2>&1; then CONTENEUR=podman
  elif command -v docker >/dev/null 2>&1; then CONTENEUR=docker
  else
    rouge "--compat demande Podman ou Docker :"
    rouge "  Fedora : sudo dnf install podman    Debian, Ubuntu, Mint : sudo apt install podman    Arch : sudo pacman -S podman"
    exit 1
  fi
  # Formats possibles dans le conteneur : le paquet Arch (makepkg) et le Flatpak (bac à sable) restent sur cette machine.
  FORMATS_CONTENEUR=""
  for format in $DEMANDES; do
    case "$format" in
      arch | flatpak) [ "$EXPLICITE" -eq 1 ] && jaune "  $format : non fabriqué avec --compat (lancez-le sans --compat)" ;;
      *) FORMATS_CONTENEUR="$FORMATS_CONTENEUR $format" ;;
    esac
  done
  [ -n "$FORMATS_CONTENEUR" ] || { rouge "Aucun format possible avec --compat (deb rpm appimage tar)."; exit 2; }
  TRANSMIS=()
  for argument in "${ARGUMENTS[@]}"; do
    case "$argument" in --compat | --install | deb | rpm | arch | pacman | pkg | flatpak | appimage | tar) ;; *) TRANSMIS+=("$argument") ;; esac
  done
  TRANSMIS+=($FORMATS_CONTENEUR --oui)
  IMAGE="lumitype-build:ubuntu22.04-3"
  CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/lumitype-compat"
  mkdir -p "$CACHE/target" "$CACHE/cargo-registry" "$CACHE/npm"
  titre "Construction compatible : conteneur Ubuntu 22.04 (glibc 2.35), avec $CONTENEUR"
  if ! "$CONTENEUR" image inspect "$IMAGE" >/dev/null 2>&1; then
    echo "  Première fois : préparation de l'image (5 à 10 minutes, environ 2 Go, gardée pour les fois suivantes)"
    "$CONTENEUR" build -t "$IMAGE" -f "$LIB/Containerfile.ubuntu22" "$LIB" || { rouge "Préparation de l'image impossible (connexion ?)."; exit 1; }
  fi
  # « z » : étiquette SELinux partagée (Fedora), sans effet ailleurs. Docker tourne en administrateur dans le
  # conteneur : les fichiers produits sont rendus à l'utilisateur à la fin (Podman sans root n'en a pas besoin).
  PROPRIETAIRE=""
  [ "$CONTENEUR" = docker ] && PROPRIETAIRE="$(id -u):$(id -g)"
  OPTIONS_TTY=(); [ -t 0 ] && [ -t 1 ] && OPTIONS_TTY=(-t)
  echo "  Formats : $FORMATS_CONTENEUR"
  code=0
  "$CONTENEUR" run --rm "${OPTIONS_TTY[@]}" \
    -v "$RACINE:/src:z" -v "$CACHE/target:/src/target:z" \
    -v "$CACHE/cargo-registry:/usr/local/cargo/registry:z" -v "$CACHE/npm:/root/.npm:z" \
    -e LUMITYPE_DANS_CONTENEUR=1 -e LUMITYPE_MOTEUR="${LUMITYPE_MOTEUR:-chromium}" \
    -e LUMITYPE_GLIBC_MAX="$GLIBC_MAX_DIFFUSION" -e PROPRIETAIRE="$PROPRIETAIRE" \
    -w /src "$IMAGE" bash -c '
      ./packaging/linux.sh "$@"; code=$?
      if [ -n "$PROPRIETAIRE" ]; then chown -R "$PROPRIETAIRE" packaging/out packaging/.work packaging/release node_modules target 2>/dev/null || true; fi
      exit $code' _ "${TRANSMIS[@]}" || code=$?
  [ "$code" -eq 0 ] || { rouge "La construction dans le conteneur a échoué (code $code)."; exit "$code"; }
  vert "Paquets compatibles dans packaging/out/ (glibc 2.35 : Mint 21+, Ubuntu 22.04+, Debian 12+, Fedora, Arch…)"
  [ "$INSTALLER_APRES" -eq 1 ] && installer_paquet_produit
  exit 0
fi

# ---- Numéro de version, s'il est demandé : tauri.conf.json (paquets, noms des fichiers, « À propos ») et package.json
if [ -n "$NOUVELLE_VERSION" ]; then
  titre "Version $NOUVELLE_VERSION"
  for fichier in "$RACINE/src-tauri/tauri.conf.json" "$RACINE/package.json"; do
    # Seule la ligne "version" de premier niveau est touchée ; les versions des dépendances ne bougent pas.
    sed -i -E '0,/^(\s*)"version":\s*"[^"]*"/s//\1"version": "'"$NOUVELLE_VERSION"'"/' "$fichier"
    grep -q "\"version\": \"$NOUVELLE_VERSION\"" "$fichier" || { rouge "Impossible de mettre à jour $fichier"; exit 1; }
  done
  sed -i -E '0,/^version = "[^"]*"/s//version = "'"$NOUVELLE_VERSION"'"/' "$RACINE/src-tauri/Cargo.toml" "$RACINE/Cargo.toml"
  echo "  tauri.conf.json, package.json, Cargo.toml à jour"
fi
VERSION="$(grep -m1 '"version"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"
IDENTIFIANT="$(grep -m1 '"identifier"' "$RACINE/src-tauri/tauri.conf.json" | cut -d'"' -f4)"
echo "Lumitype $VERSION — ${PRETTY_NAME:-Linux} ($FAMILLE, $ARCH, glibc $(ldd --version 2>/dev/null | head -n1 | grep -o '[0-9]\+\.[0-9]\+$' || echo '?'))"
[ "${LUMITYPE_DANS_CONTENEUR:-0}" = 1 ] && echo "  (dans le conteneur de --compat)"

# Installe des paquets de la distribution, après accord (ou directement avec --oui).
installer() {
  local commande s="sudo "
  # Déjà administrateur (conteneur de --compat) : pas de sudo, qui n'y est d'ailleurs pas installé.
  [ "$(id -u)" = 0 ] && s=""
  case "$FAMILLE" in
    fedora) commande="${s}dnf install -y $*" ;;
    debian) commande="${s}apt-get install -y $*" ;;
    arch) commande="${s}pacman -S --needed --noconfirm $*" ;;
    suse) commande="${s}zypper install -y $*" ;;
    *) rouge "Distribution non reconnue : installez vous-même $*"; return 1 ;;
  esac
  echo "  À installer : $*"
  if [ "$OUI" -eq 0 ]; then
    if [ ! -t 0 ]; then rouge "  Lancez : $commande (ou relancez ce script avec --oui)"; return 1; fi
    read -r -p "  Lancer « $commande » ? [O/n] " reponse
    case "$reponse" in [nN]*) return 1 ;; esac
  fi
  # Debian/Ubuntu : la liste des paquets d'une installation neuve (WSL surtout) est vide ou périmée ; sans « update »,
  # apt ne trouve ni nasm, ni ninja-build, ni libxdo-dev. Le dépôt « universe » d'Ubuntu, où ils sont, est activé au besoin.
  if [ "$FAMILLE" = debian ] && [ "${APT_A_JOUR:-0}" -eq 0 ]; then
    if [ "${ID:-}" = ubuntu ] && command -v add-apt-repository >/dev/null 2>&1; then ${s}add-apt-repository -y universe >/dev/null 2>&1 || true; fi
    ${s}apt-get update -q || true
    APT_A_JOUR=1
  fi
  eval "$commande"
}

# Node.js $NODE_MIN+ : sous Debian, Ubuntu et Mint, celui de la distribution est souvent trop ancien (12 sur Ubuntu
# 22.04, 18 sur Debian 12) ; il vient alors du dépôt officiel NodeSource, qui remplace le paquet nodejs (npm compris).
# Ailleurs (Fedora, Arch, openSUSE), le paquet de la distribution est assez récent.
installer_node() {
  echo "  À installer : Node.js $NODE_MIN (dépôt officiel NodeSource, https://github.com/nodesource/distributions)"
  if [ "$OUI" -eq 0 ]; then
    if [ ! -t 0 ]; then return 1; fi
    read -r -p "  Installer Node.js $NODE_MIN depuis NodeSource ? [O/n] " reponse
    case "$reponse" in [nN]*) return 1 ;; esac
  fi
  # Le Node de la distribution est retiré d'abord : ses paquets (npm, libnode-dev, libnode72…) contiennent des
  # fichiers que le nodejs de NodeSource apporte lui-même, et dpkg refuserait de les écraser (« trying to overwrite
  # /usr/include/node/common.gypi »). Une installation interrompue est réparée au passage.
  local anciens
  anciens="$(dpkg-query -W -f='${db:Status-Abbrev} ${Package}\n' 'nodejs' 'nodejs-doc' 'npm' 'libnode*' 'node-*' 2>/dev/null |
    awk '$1 ~ /^.i/ || $1 ~ /^iF/ || $1 ~ /^iU/ {print $2}' | grep -v '^$' || true)"
  if [ -n "$anciens" ] && ! apt-cache policy nodejs 2>/dev/null | grep -q '\*\*\*.*nodesource'; then
    echo "  Retrait du Node.js de la distribution : $(echo $anciens)"
    sudo dpkg --configure -a || true
    sudo apt-get remove -y $anciens || sudo dpkg --remove --force-remove-reinstreq $anciens || return 1
  fi
  sudo apt-get install -y ca-certificates curl gnupg &&
    curl -fsSL "https://deb.nodesource.com/setup_${NODE_MIN}.x" | sudo -E bash - &&
    sudo apt-get install -y nodejs
}

# Rust par rustup (installation officielle, pour cet utilisateur, sans question), puis chargé dans cette console.
installer_rust() {
  echo "  À installer : Rust (rustup, https://rustup.rs)"
  if [ "$OUI" -eq 0 ]; then
    if [ ! -t 0 ]; then return 1; fi
    read -r -p "  Installer Rust avec rustup ? [O/n] " reponse
    case "$reponse" in [nN]*) return 1 ;; esac
  fi
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path &&
    . "$HOME/.cargo/env"
}

IGNORES=()
ignorer() {
  IGNORES+=("$1 : $2")
  DEMANDES="$(echo " $DEMANDES " | sed "s/ $1 / /")"
  if [ "$EXPLICITE" -eq 1 ]; then rouge "  $1 impossible : $2"; else jaune "  $1 sauté : $2"; fi
}

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

# ---- Vérification de TOUTES les dépendances, avant de commencer quoi que ce soit
# Tout est contrôlé d'un coup et résumé dans un tableau ; ce qui manque et que la distribution fournit est installé
# en une seule fois (après accord, ou directement avec --oui), puis tout est revérifié. La compilation ne démarre que
# si rien ne manque : plus d'échec au bout de dix minutes pour un outil absent.
RUST_MIN="1.88"
NODE_MIN=20
DISQUE_MIN_GO=15
# Nom du paquet de la distribution pour une dépendance (vide : pas de paquet, installation manuelle).
paquet_pour() {
  case "$FAMILLE:$1" in
    *:nasm) echo nasm ;;
    fedora:cmake | debian:cmake | arch:cmake | suse:cmake) echo cmake ;;
    fedora:ninja) echo ninja-build ;; debian:ninja) echo ninja-build ;; arch:ninja) echo ninja ;; suse:ninja) echo ninja ;;
    fedora:cxx) echo gcc-c++ ;; debian:cxx) echo g++ ;; arch:cxx) echo gcc ;; suse:cxx) echo gcc-c++ ;;
    fedora:pkg-config) echo pkgconf-pkg-config ;; debian:pkg-config) echo pkg-config ;; arch:pkg-config) echo pkgconf ;; suse:pkg-config) echo pkg-config ;;
    # libxkbcommon-x11, libxcb-xkb et les morceaux de NSS : embarqués dans l'AppImage (packaging/lib/cef-bundle.sh).
    fedora:webkit) echo "webkit2gtk4.1-devel openssl-devel librsvg2-devel libxdo-devel libappindicator-gtk3-devel libxkbcommon-x11 nss nss-softokn nss-softokn-freebl" ;;
    debian:webkit) echo "libwebkit2gtk-4.1-dev libssl-dev librsvg2-dev libxdo-dev libayatana-appindicator3-dev libxkbcommon-x11-0 libxcb-xkb1 libnss3" ;;
    arch:webkit) echo "webkit2gtk-4.1 openssl librsvg xdotool libappindicator-gtk3" ;;
    suse:webkit) echo "webkit2gtk3-devel libopenssl-devel librsvg-devel libappindicator3-1" ;;
    *:curl) echo curl ;; *:file) echo file ;; *:ar) echo binutils ;;
    fedora:node | debian:node | arch:node) echo "nodejs npm" ;; suse:node) echo "nodejs22 npm22" ;;
    fedora:windows | debian:windows | arch:windows) echo "nsis clang lld llvm" ;; suse:windows) echo "nsis clang lld llvm" ;;
    *) echo "" ;;
  esac
}

ETATS=()     # lignes du tableau : « OK|nom|détail » ou « KO|nom|détail »
A_INSTALLER=()
A_LA_MAIN=()
etat() { ETATS+=("$1|$2|$3"); }
manque_paquet() { # nom affiché, clé de paquet, détail
  local p; p="$(paquet_pour "$2")"
  etat KO "$1" "$3"
  if [ -n "$p" ]; then A_INSTALLER+=($p); else A_LA_MAIN+=("$1 : $3"); fi
}

RUST_MANQUE=0
NODE_MANQUE=0
verifier_dependances() {
  ETATS=(); A_INSTALLER=(); A_LA_MAIN=(); RUST_MANQUE=0; NODE_MANQUE=0
  # Node.js et npm. Debian/Ubuntu/Mint : absent ou trop ancien, il est installé depuis NodeSource (installer_node),
  # car le paquet de la distribution serait lui aussi trop ancien.
  local n=""
  command -v node >/dev/null 2>&1 && n="$(node --version | tr -d v)"
  if [ -n "$n" ] && version_ge "$n" "$NODE_MIN"; then etat OK "Node.js" "$n"
  elif [ "$FAMILLE" = debian ]; then etat KO "Node.js" "${n:+$n trouvé, }$NODE_MIN ou plus requis (installable depuis NodeSource)"; NODE_MANQUE=1
  elif [ -n "$n" ]; then etat KO "Node.js" "$n trouvé, $NODE_MIN ou plus requis"; A_LA_MAIN+=("Node.js $NODE_MIN+ : https://nodejs.org (ou nvm)")
  else manque_paquet "Node.js" node "absent"; fi
  if command -v npm >/dev/null 2>&1; then etat OK "npm" "$(npm --version)"
  elif [ "$NODE_MANQUE" -eq 1 ]; then etat KO "npm" "absent (fourni avec Node.js de NodeSource)"
  else manque_paquet "npm" node "absent"; fi
  # Rust
  if command -v rustc >/dev/null 2>&1 && command -v cargo >/dev/null 2>&1; then
    local r; r="$(rustc --version | cut -d' ' -f2)"
    if version_ge "$r" "$RUST_MIN"; then etat OK "Rust" "$r"; else etat KO "Rust" "$r trouvé, $RUST_MIN ou plus requis"; A_LA_MAIN+=("Rust $RUST_MIN+ : rustup update stable"); fi
  else etat KO "Rust" "absent"; RUST_MANQUE=1; fi
  # Compilateur C/C++ et outils de construction
  if command -v cc >/dev/null 2>&1 && command -v c++ >/dev/null 2>&1; then etat OK "Compilateur C/C++" "$(c++ --version | head -n1)"; else manque_paquet "Compilateur C/C++" cxx "absent (gcc/g++)"; fi
  if command -v pkg-config >/dev/null 2>&1; then etat OK "pkg-config" "$(pkg-config --version)"; else manque_paquet "pkg-config" pkg-config "absent"; fi
  # Bibliothèques de développement (fenêtre, réseau, icônes) : vérifiées par pkg-config
  if command -v pkg-config >/dev/null 2>&1 && pkg-config --exists webkit2gtk-4.1 gtk+-3.0 openssl librsvg-2.0 2>/dev/null; then
    etat OK "Bibliothèques GTK / WebKit / OpenSSL" "webkit2gtk-4.1 $(pkg-config --modversion webkit2gtk-4.1)"
  else
    manque_paquet "Bibliothèques GTK / WebKit / OpenSSL" webkit "webkit2gtk-4.1, gtk+-3.0, openssl ou librsvg introuvable"
  fi
  # Chromium embarqué (CEF) : son enveloppe C++ se compile avec CMake et Ninja
  if [ "$MOTEUR" = chromium ]; then
    if command -v cmake >/dev/null 2>&1; then etat OK "CMake (Chromium embarqué)" "$(cmake --version | head -n1 | awk '{print $3}')"; else manque_paquet "CMake (Chromium embarqué)" cmake "absent"; fi
    if command -v ninja >/dev/null 2>&1 || command -v ninja-build >/dev/null 2>&1; then etat OK "Ninja (Chromium embarqué)" "$( (ninja --version || ninja-build --version) 2>/dev/null)"; else manque_paquet "Ninja (Chromium embarqué)" ninja "absent"; fi
  fi
  # nasm : assembleur de l'encodeur AVIF (rav1e), indispensable sur PC x86-64 (rav1e s'en passe sur ARM)
  if [ "$ARCH" = x86_64 ]; then
    if command -v nasm >/dev/null 2>&1; then etat OK "nasm (export AVIF)" "$(nasm -v | awk '{print $3}')"; else manque_paquet "nasm (export AVIF)" nasm "absent"; fi
  fi
  # Petits outils utilisés par la construction et les paquets
  command -v curl >/dev/null 2>&1 && etat OK "curl" "présent" || manque_paquet "curl" curl "absent (téléchargement de Chromium)"
  command -v file >/dev/null 2>&1 && etat OK "file" "présent" || manque_paquet "file" file "absent (AppImage)"
  if demande arch || demande flatpak || demande tar; then
    command -v ar >/dev/null 2>&1 && etat OK "ar (binutils)" "présent" || manque_paquet "ar (binutils)" ar "absent (paquets Arch, Flatpak, tar)"
  fi
  # Windows depuis Linux (--windows) : NSIS, clang/lld (édition de liens MSVC) et cargo-xwin
  if [ "$WINDOWS" -eq 1 ]; then
    local manquants=""
    for c in makensis clang lld-link llvm-ar; do command -v "$c" >/dev/null 2>&1 || manquants="$manquants $c"; done
    if [ -z "$manquants" ]; then etat OK "Windows (nsis, clang, lld, llvm)" "présents"; else manque_paquet "Windows (nsis, clang, lld, llvm)" windows "absents :$manquants"; fi
    command -v rustup >/dev/null 2>&1 && etat OK "rustup (cible Windows)" "présent" || { etat KO "rustup (cible Windows)" "absent"; A_LA_MAIN+=("rustup : https://rustup.rs"); }
  fi
  # Polices incluses : sans elles, Tauri refuse de compiler (ressource fonts/**/* introuvable)
  if find src-tauri/fonts -type f \( -iname '*.ttf' -o -iname '*.otf' \) -print -quit 2>/dev/null | grep -q .; then etat OK "Polices incluses" "src-tauri/fonts"
  else etat KO "Polices incluses" "src-tauri/fonts vide ou absent"; A_LA_MAIN+=("Polices : décompressez l'archive COMPLÈTE du projet (dossier src-tauri/fonts)"); fi
  # Place sur le disque (compilation Rust + Chromium)
  local libre; libre="$(df -Pk "$RACINE" | awk 'NR==2 {print int($4/1048576)}')"
  # En dessous de 5 Go la compilation échouera à coup sûr ; entre 5 et 15 Go, simple avertissement.
  if [ "${libre:-0}" -ge "$DISQUE_MIN_GO" ]; then etat OK "Espace disque" "$libre Go libres"
  elif [ "${libre:-0}" -ge 5 ]; then etat ATTENTION "Espace disque" "$libre Go libres ($DISQUE_MIN_GO Go conseillés : la compilation peut manquer de place)"
  else etat KO "Espace disque" "$libre Go libres, $DISQUE_MIN_GO Go conseillés"; A_LA_MAIN+=("Espace disque : libérez de la place (ex. supprimez target/debug ou packaging/.work)"); fi
}

afficher_dependances() {
  local ligne statut nom detail marge
  for ligne in "${ETATS[@]}"; do
    IFS='|' read -r statut nom detail <<<"$ligne"
    # Colonne alignée en caractères (printf compte les octets : les accents la décaleraient).
    marge=$((38 - $(printf '%s' "$nom" | LC_ALL=C.UTF-8 wc -m))); [ "$marge" -lt 1 ] && marge=1
    case "$statut" in
      OK) printf '  \033[32m✓\033[0m %s%*s%s\n' "$nom" "$marge" "" "$detail" ;;
      ATTENTION) printf '  \033[33m!\033[0m %s%*s%s\n' "$nom" "$marge" "" "$detail" ;;
      *) printf '  \033[31m✗\033[0m %s%*s%s\n' "$nom" "$marge" "" "$detail" ;;
    esac
  done
}

titre "Vérification des dépendances (moteur d'affichage : $MOTEUR)"
[ "$SOUS_WSL" -eq 1 ] && echo "  (WSL : les outils de Windows sont ignorés, seuls ceux de la distribution comptent)"
verifier_dependances
afficher_dependances
if [ ${#A_INSTALLER[@]} -gt 0 ] || [ "$RUST_MANQUE" -eq 1 ] || [ "$NODE_MANQUE" -eq 1 ]; then
  titre "Installation de ce qui manque"
  INSTALLE=0
  if [ ${#A_INSTALLER[@]} -gt 0 ]; then
    # Doublons retirés, un seul appel au gestionnaire de paquets pour tout.
    mapfile -t A_INSTALLER < <(printf '%s\n' "${A_INSTALLER[@]}" | sort -u)
    if installer "${A_INSTALLER[@]}"; then
      INSTALLE=1
      [ "$FAMILLE" = fedora ] && ! command -v c++ >/dev/null 2>&1 && sudo dnf group install -y c-development || true
    fi
  fi
  if [ "$NODE_MANQUE" -eq 1 ]; then installer_node && INSTALLE=1 || A_LA_MAIN+=("Node.js $NODE_MIN : curl -fsSL https://deb.nodesource.com/setup_${NODE_MIN}.x | sudo -E bash - && sudo apt install -y nodejs"); fi
  if [ "$RUST_MANQUE" -eq 1 ]; then installer_rust && INSTALLE=1 || A_LA_MAIN+=("Rust : curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"); fi
  if [ "$INSTALLE" -eq 1 ]; then
    hash -r
    titre "Nouvelle vérification"
    verifier_dependances
    afficher_dependances
  fi
fi
if printf '%s\n' "${ETATS[@]}" | grep -q '^KO|'; then
  rouge ""
  rouge "Il manque encore des dépendances : rien n'a été compilé."
  for aide in "${A_LA_MAIN[@]}"; do rouge "  - $aide"; done
  if [ ${#A_INSTALLER[@]} -gt 0 ]; then
    case "$FAMILLE" in
      fedora) rouge "  - Paquets : sudo dnf install ${A_INSTALLER[*]}" ;;
      debian) rouge "  - Paquets : sudo apt install ${A_INSTALLER[*]}" ;;
      arch) rouge "  - Paquets : sudo pacman -S ${A_INSTALLER[*]}" ;;
      suse) rouge "  - Paquets : sudo zypper install ${A_INSTALLER[*]}" ;;
    esac
  fi
  rouge "  Guide général : https://v2.tauri.app/start/prerequisites/"
  exit 1
fi
vert "  Tout est prêt."

# Dépendances de l'interface : réinstallées seulement si package-lock.json a changé depuis la dernière fois
# (empreinte gardée dans node_modules/.lumitype-lock), au lieu d'un npm ci d'une minute à chaque construction.
dependances_interface() {
  local empreinte marque="node_modules/.lumitype-lock"
  empreinte="$(sha256sum package-lock.json 2>/dev/null | cut -d' ' -f1)"
  if [ -d node_modules ] && [ -f "$marque" ] && [ "$(cat "$marque")" = "$empreinte" ]; then echo "  à jour"; return; fi
  if [ -f package-lock.json ]; then npm ci --no-audit --no-fund; else npm install --no-audit --no-fund; fi
  echo "$empreinte" >"$marque"
}

# ---- Essai : compilation seule, puis lancement (aucun paquet)
if [ "$ESSAI" -eq 1 ]; then
  titre "Dépendances de l'interface"
  dependances_interface
  titre "Compilation pour essai ($MOTEUR) — sans paquet"
  bash "$LIB/sync-templates.sh"
  npm run tauri build -- --no-bundle "${OPTIONS_TAURI[@]}" "${OPTIONS_CARGO[@]}"
  verifier_glibc || true
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
  # Ubuntu 22.04 (WSL compris) fournit flatpak 1.12 et flatpak-builder 1.2 : trop anciens pour les socles GNOME
  # récents (appstream-compose absent, socles refusés). Le dépôt officiel de Flatpak pour Ubuntu (PPA flatpak/stable)
  # apporte les versions à jour.
  version_builder="$(flatpak-builder --version 2>/dev/null | grep -o '[0-9][0-9.]*' | head -n1)"
  if [ "${ID:-}" = ubuntu ] && [ -n "$version_builder" ] && ! version_ge "$version_builder" 1.4; then
    echo "  flatpak-builder $version_builder d'Ubuntu trop ancien pour le socle GNOME $FLATPAK_GNOME : mise à jour depuis le PPA officiel flatpak/stable"
    if [ "$OUI" -eq 1 ] || { [ -t 0 ] && read -r -p "  Ajouter le PPA flatpak/stable et mettre à jour flatpak ? [O/n] " reponse && [[ ! "$reponse" =~ ^[nN] ]]; }; then
      sudo add-apt-repository -y ppa:flatpak/stable && sudo apt-get update -q && sudo apt-get install -y flatpak flatpak-builder || jaune "  Mise à jour de flatpak impossible : on continue avec la version $version_builder"
    fi
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
dependances_interface

titre "Compilation ($BUNDLES) — dix à vingt minutes la première fois"
# L'outil qui fabrique l'AppImage (linuxdeploy) est lui-même une AppImage : il lui faut FUSE 2, absent de
# beaucoup de distributions récentes. Cette variable lui fait extraire son contenu et s'exécuter sans FUSE ;
# elle est sans effet quand FUSE est disponible, donc on la pose toujours.
export APPIMAGE_EXTRACT_AND_RUN=1
# Certaines AppImages écrivent leur dossier temporaire ici : sans cela, linuxdeploy échoue sans rien dire.
export TMPDIR="${TMPDIR:-/tmp}"
# Le « strip » embarqué dans linuxdeploy est ancien : il échoue sur les bibliothèques récentes (« unknown type
# [0x13] section .relr.dyn ») et fait échouer toute l'AppImage. Les bibliothèques des distributions et celles de
# Chromium (packaging/lib/cef-bundle.sh) sont déjà allégées : on s'en passe.
export NO_STRIP=true
# Lancé par bash : un dépôt envoyé depuis Windows perd le droit d'exécution des scripts.
bash "$LIB/sync-templates.sh"
DEBUT="$(mktemp)"
trap 'rm -f "$DEBUT"' EXIT
echec_tauri=0
# Sortie de la compilation gardée dans un journal : en cas d'échec d'un format, ses vraies lignes d'erreur sont affichées.
mkdir -p "$TRAVAIL"
JOURNAL_COMPILATION="$TRAVAIL/compilation.log"
npm run tauri build -- --bundles "$BUNDLES" "${OPTIONS_TAURI[@]}" "${OPTIONS_CARGO[@]}" 2>&1 | tee "$JOURNAL_COMPILATION" || echec_tauri=1

titre "Compatibilité"
GLIBC_OK=1
verifier_glibc || GLIBC_OK=0
if [ "$GLIBC_OK" -eq 0 ] && [ "$RELEASE_DEMANDE" -eq 1 ]; then exit 1; fi

# Paquet produit par CETTE construction (plus récent que son début), ou rien.
produit() {
  local dossier
  for dossier in "$RACINE/target/release/bundle" "$RACINE/src-tauri/target/release/bundle"; do
    if [ -d "$dossier" ]; then find "$dossier" -maxdepth 2 -type f -name "$1" -newer "$DEBUT" -print -quit; fi
  done | head -n1
}

mkdir -p "$SORTIES"
PRODUITS=()

# ---- Windows (--windows) : les 3 fichiers (setup.exe, msi, portable), construits EN MÊME TEMPS que l'empaquetage
# Linux ci-dessous, dans leur propre dossier de compilation (target/windows) pour ne pas bloquer cargo. Compilation
# croisée par cargo-xwin (en-têtes et bibliothèques MSVC téléchargés une fois) ; installateur NSIS par makensis ;
# MSI par WiX quand « wine » est présent, sauté sinon (WiX ne tourne que sous Windows ou Wine).
JOURNAL_WINDOWS="$TRAVAIL/windows.log"
PID_WINDOWS=""
if [ "$WINDOWS" -eq 1 ]; then
  titre "Windows : construction en arrière-plan (journal : packaging/.work/windows.log)"
  mkdir -p "$TRAVAIL"
  (
    rustup target add x86_64-pc-windows-msvc >/dev/null
    command -v cargo-xwin >/dev/null 2>&1 || cargo install --locked cargo-xwin
    BUNDLES_WIN=nsis
    command -v wine >/dev/null 2>&1 && BUNDLES_WIN=nsis,msi
    CARGO_TARGET_DIR="$RACINE/target/windows" npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles "$BUNDLES_WIN"
  ) >"$JOURNAL_WINDOWS" 2>&1 &
  PID_WINDOWS=$!
fi
ranger() {
  cp -f "$1" "$SORTIES/$2"
  PRODUITS+=("$2  ($(du -h "$SORTIES/$2" | cut -f1))")
}

# Contenu de l'AppImage contrôlé après fabrication. linuxdeploy dirige l'exécutable vers usr/lib (son rpath remplace
# celui de build.rs) : Chromium et ses données doivent y être (sinon « Invalid file descriptor to ICU data »), avec
# les bibliothèques ouvertes à la volée (sinon prises sur le système, d'une autre version, ou absentes).
verifier_appimage() {
  local dossier manquants="" fichier
  dossier="$(mktemp -d)"
  chmod +x "$1"
  if ! (cd "$dossier" && "$1" --appimage-extract >/dev/null 2>&1); then
    rm -rf "$dossier"; rouge "  AppImage : extraction impossible pour la vérification"; return 1
  fi
  for fichier in libcef.so icudtl.dat v8_context_snapshot.bin locales/en-US.pak libEGL.so libGLESv2.so \
                 libxkbcommon-x11.so.0 libxkbcommon.so.0 libxcb-xkb.so.1 \
                 libnss3.so libnssutil3.so libsmime3.so libnspr4.so libsoftokn3.so libfreebl3.so libfreeblpriv3.so libnssckbi.so; do
    [ -e "$dossier/squashfs-root/usr/lib/$fichier" ] || manquants="$manquants $fichier"
  done
  rm -rf "$dossier"
  if [ -n "$manquants" ]; then rouge "  AppImage : absents de usr/lib :$manquants"; return 1; fi
  vert "  AppImage vérifiée : Chromium, ses données, NSS et le clavier X11 sont dans usr/lib"
}

titre "Rangement dans packaging/out/"
DEB="$(produit "*.deb")"
demande deb && { [ -n "$DEB" ] && ranger "$DEB" "Lumitype_${VERSION}_${ARCH_DEB}.deb" || ignorer deb "la compilation a échoué"; }
if demande rpm; then
  RPM="$(produit "*.rpm")"
  [ -n "$RPM" ] && ranger "$RPM" "$(basename "$RPM")" || ignorer rpm "la compilation a échoué"
fi
if demande appimage; then
  APPIMAGE="$(produit "*.AppImage")"
  if [ -z "$APPIMAGE" ] && [ -x "$RACINE/target/release/lumitype" ]; then
    # Tauri ne montre pas pourquoi linuxdeploy échoue : nouvel essai de l'AppImage seule (sans recompiler), en mode
    # détaillé, dans packaging/.work/appimage.log.
    jaune "  AppImage : échec de linuxdeploy, nouvel essai détaillé (journal : packaging/.work/appimage.log)"
    # Bibliothèques demandées par Chromium et absentes de cette machine : linuxdeploy doit les embarquer et échoue sans elles.
    MANQUANTES="$(for f in "$RACINE"/target/release/cef-bundle/*.so; do ldd "$f" 2>/dev/null; done | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ')"
    if [ -n "$MANQUANTES" ]; then
      jaune "  Bibliothèques de Chromium absentes de cette machine : $MANQUANTES"
      if [ "$FAMILLE" = debian ]; then
        installer libnss3 libnspr4 libgbm1 libxss1 libcups2 libxkbcommon0 libatk-bridge2.0-0 libxcomposite1 libxdamage1 libxrandr2 libdrm2 libxshmfence1 "$(apt-cache show libasound2t64 >/dev/null 2>&1 && echo libasound2t64 || echo libasound2)" || true
      fi
    fi
    touch "$DEBUT"
    npm run tauri bundle -- --bundles appimage --verbose "${OPTIONS_TAURI[@]}" >"$TRAVAIL/appimage.log" 2>&1 || true
    APPIMAGE="$(produit "*.AppImage")"
  fi
  if [ -n "$APPIMAGE" ] && ! verifier_appimage "$APPIMAGE"; then
    if [ "$RELEASE_DEMANDE" -eq 1 ]; then rouge "  AppImage incomplète : pas de publication (--release)."; exit 1; fi
    jaune "  AppImage incomplète : elle risque de ne pas démarrer sur d'autres machines."
  fi
  if [ -n "$APPIMAGE" ]; then ranger "$APPIMAGE" "$(basename "$APPIMAGE")"
  else
    # AppImage d'une construction précédente retirée : sinon packaging/release/ la reprenait comme si elle était neuve.
    rm -f "$SORTIES"/Lumitype_*_amd64.AppImage "$RELEASE/Lumitype-x86_64.AppImage"
    ignorer appimage "échec de linuxdeploy (journal : packaging/.work/appimage.log)"
    grep -iE 'error|erreur|failed|could not|not found|cannot|unable' "$TRAVAIL/appimage.log" 2>/dev/null | grep -v '^\s*$' | tail -n 20 >&2 || true
  fi
fi
if [ -z "$DEB" ]; then
  for format in arch flatpak tar; do demande "$format" && ignorer "$format" "il se fabrique à partir du .deb, qui a échoué"; done
fi

# Contenu du .deb (usr/bin, usr/lib/Lumitype, usr/share…) dans un dossier.
extraire_deb() {
  rm -rf "$2" && mkdir -p "$2"
  (cd "$2" && ar x "$1" && tar -xf data.tar.* && rm -f data.tar.* control.tar.* debian-binary)
}

# ---- Les trois formats dérivés du .deb (archive, Arch, Flatpak) se fabriquent en parallèle : chacun dans son
# dossier de travail, avec son journal ; les résultats sont relevés à la fin.
mkdir -p "$TRAVAIL"
PIDS=()
NOMS_PARALLELES=()
lancer() { # nom, fonction
  ( "$2" ) >"$TRAVAIL/$1.log" 2>&1 &
  PIDS+=($!); NOMS_PARALLELES+=("$1")
}

# ---- Archive portable : le programme et ses fichiers, plus un installateur pour l'utilisateur courant
fabriquer_tar() {
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
  # gzip rapide (-1) : l'archive est à peine plus grosse, et bien plus vite faite.
  tar -C "$TRAVAIL/tar" -I 'gzip -1' -cf "$SORTIES/$NOM.tar.gz" "$NOM"
}
demande tar && lancer tar fabriquer_tar

# ---- Paquet Arch (pacman) : le contenu du .deb, décrit par un PKGBUILD et emballé par makepkg
fabriquer_arch() {
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
  (cd "$TRAVAIL/arch" && PKGDEST="$SORTIES" makepkg -f --nodeps --noconfirm)
}
demande arch && lancer arch fabriquer_arch

# ---- Flatpak : le contenu du .deb dans le socle GNOME, en un seul fichier installable partout
fabriquer_flatpak() {
  titre "Flatpak (socle GNOME $FLATPAK_GNOME)"
  rm -rf "$TRAVAIL/flatpak" && mkdir -p "$TRAVAIL/flatpak"
  cp "$DEB" "$TRAVAIL/flatpak/Lumitype.deb"
  # flatpak-builder antérieur à 1.3 (Ubuntu 22.04, Debian 12) lance « appstream-compose », un outil que les socles
  # GNOME récents ne fournissent plus (« bwrap: execvp appstream-compose: No such file or directory ») ; les versions
  # suivantes passent par appstreamcli. Avec un ancien flatpak-builder, cette étape est sautée : la fiche AppStream
  # est quand même installée dans le Flatpak (share/metainfo), seul son catalogue précompilé manque.
  local version_builder COMPOSE_APPSTREAM=""
  version_builder="$(flatpak-builder --version 2>/dev/null | grep -o '[0-9][0-9.]*' | head -n1)"
  if [ -n "$version_builder" ] && ! version_ge "$version_builder" 1.3; then
    COMPOSE_APPSTREAM='
  "appstream-compose": false,'
    echo "  flatpak-builder $version_builder : composition AppStream sautée (outil absent des socles GNOME récents)"
  fi
  # Fiche AppStream (logithèques, Flathub) : valide pour appstreamcli validate (date de version, développeur,
  # adresse du projet, classification d'âge), sinon appstream-compose refuse le Flatpak.
  cat >"$TRAVAIL/flatpak/$IDENTIFIANT.metainfo.xml" <<METAINFO
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>$IDENTIFIANT</id>
  <name>Lumitype</name>
  <summary>Éditeur d'images raster</summary>
  <developer id="io.github.splaching-boop">
    <name>Lumitype</name>
  </developer>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>MIT OR Apache-2.0</project_license>
  <description>
    <p>Calques, masques, sélections, texte, tracés, réglages non destructifs, filtres et effets de calque.</p>
  </description>
  <launchable type="desktop-id">$IDENTIFIANT.desktop</launchable>
  <url type="homepage">https://github.com/splaching-boop/Lumitype</url>
  <url type="bugtracker">https://github.com/splaching-boop/Lumitype/issues</url>
  <content_rating type="oars-1.1"/>
  <releases>
    <release version="$VERSION" date="$(date +%Y-%m-%d)"/>
  </releases>
</component>
METAINFO
  # Vérification avant la construction : une erreur de la fiche est montrée telle quelle, au lieu du seul
  # « appstream-compose failed » de flatpak-builder.
  if command -v appstreamcli >/dev/null 2>&1; then appstreamcli validate --no-net --explain "$TRAVAIL/flatpak/$IDENTIFIANT.metainfo.xml" || true; fi
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
  "build-options": { "no-debuginfo": true, "strip": false },$COMPOSE_APPSTREAM
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
  (cd "$TRAVAIL/flatpak" &&
    flatpak-builder --user --force-clean --disable-rofiles-fuse --repo=depot construction "$IDENTIFIANT.json" &&
    flatpak build-bundle depot "$SORTIES/$NOM" "$IDENTIFIANT" --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo)
}
demande flatpak && lancer flatpak fabriquer_flatpak

# ---- Attente des fabrications parallèles, et relevé de ce qu'elles ont produit
if [ ${#PIDS[@]} -gt 0 ]; then
  titre "Fabrication en parallèle : ${NOMS_PARALLELES[*]}"
  for i in "${!PIDS[@]}"; do
    nom="${NOMS_PARALLELES[$i]}"
    if wait "${PIDS[$i]}"; then
      case "$nom" in
        tar) F="$SORTIES/Lumitype-$VERSION-$ARCH.tar.gz" ;;
        arch) F="$(find "$SORTIES" -maxdepth 1 -name "lumitype-${VERSION//-/_}-1-$ARCH.pkg.tar.*" -print -quit)" ;;
        flatpak) F="$SORTIES/Lumitype-$VERSION-$ARCH.flatpak" ;;
      esac
      [ -n "$F" ] && [ -f "$F" ] && PRODUITS+=("$(basename "$F")  ($(du -h "$F" | cut -f1))") || ignorer "$nom" "aucun fichier produit (journal : packaging/.work/$nom.log)"
    else
      ignorer "$nom" "échec (journal : packaging/.work/$nom.log)"
      # Le message utile précède souvent la dernière ligne (« appstream-compose failed ») : les lignes d'erreur d'abord.
      grep -iE '^(E|W):|error|erreur|failed|not found|introuvable' "$TRAVAIL/$nom.log" | tail -n 20 >&2 || true
      tail -n 8 "$TRAVAIL/$nom.log" >&2 || true
    fi
  done
fi

# ---- Windows : attente, puis rangement des 3 fichiers
if [ -n "$PID_WINDOWS" ]; then
  titre "Windows : attente de la fin de la construction"
  if wait "$PID_WINDOWS"; then
    CIBLE_WIN="$RACINE/target/windows/x86_64-pc-windows-msvc/release"
    for f in "$CIBLE_WIN"/bundle/nsis/*-setup.exe "$CIBLE_WIN"/bundle/msi/*.msi; do
      [ -f "$f" ] && ranger "$f" "$(basename "$f")"
    done
    [ -f "$CIBLE_WIN/lumitype.exe" ] && ranger "$CIBLE_WIN/lumitype.exe" "Lumitype-portable.exe"
    command -v wine >/dev/null 2>&1 || IGNORES+=("msi : WiX ne tourne pas sans « wine » (le .msi se fabrique sous Windows : packaging/windows.bat)")
  else
    IGNORES+=("windows : la construction a échoué (journal : packaging/.work/windows.log)")
    tail -n 20 "$JOURNAL_WINDOWS" >&2 || true
  fi
fi

# ---- Release : copie sous les noms de diffusion (sans version) et empreintes SHA-256, pour la Release GitHub
if [ "$RELEASE_DEMANDE" -eq 1 ]; then
  titre "Release : packaging/release/"
  mkdir -p "$RELEASE"
  copier_release() { # motif, nom final
    local f; f="$(find "$SORTIES" -maxdepth 1 -name "$1" -print -quit)"
    if [ -n "$f" ]; then cp -f "$f" "$RELEASE/$2"; echo "  $2  <-  $(basename "$f")"; else jaune "  absent : $2"; fi
  }
  copier_release 'Lumitype-*.x86_64.rpm' Lumitype-Fedora-x86_64.rpm
  copier_release 'Lumitype_*_amd64.deb' Lumitype-Debian-Ubuntu-amd64.deb
  copier_release 'lumitype-*.pkg.tar.*' Lumitype-Arch-x86_64.pkg.tar.zst
  # Sans « Linux » : toutes les AppImage sont pour Linux (règle de nommage d'AppImageHub).
  copier_release 'Lumitype_*_amd64.AppImage' Lumitype-x86_64.AppImage
  copier_release 'Lumitype-*-x86_64.flatpak' Lumitype-Linux.flatpak
  copier_release 'Lumitype-*-x86_64.tar.gz' Lumitype-Linux-portable.tar.gz
  copier_release 'Lumitype_*-setup.exe' Lumitype-Windows-Setup.exe
  copier_release 'Lumitype_*.msi' Lumitype-Windows.msi
  copier_release 'Lumitype-portable.exe' Lumitype-Windows-portable.exe
  (cd "$RELEASE" && rm -f SHA256SUMS.txt && sha256sum Lumitype-* >SHA256SUMS.txt && cat SHA256SUMS.txt)
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

Paquets dans packaging/out/. Pour installer :
  Debian, Ubuntu, Mint    : sudo apt install ./packaging/out/Lumitype_*.deb
  Fedora, Nobara          : sudo dnf install ./packaging/out/Lumitype-*.rpm
  openSUSE                : sudo zypper install ./packaging/out/Lumitype-*.rpm
  Arch, CachyOS, Manjaro  : sudo pacman -U ./packaging/out/lumitype-*.pkg.tar.zst
  Flatpak (partout)       : flatpak install --user ./packaging/out/Lumitype-*.flatpak
  AppImage (partout)      : chmod +x packaging/out/Lumitype_*.AppImage && ./packaging/out/Lumitype_*.AppImage
  Archive (partout)       : tar -xzf Lumitype-*.tar.gz && ./Lumitype-*/installer.sh
AIDE
# ---- Installation sur cette machine (--install) : le paquet de sa distribution. Une version déjà présente est mise
# à jour ; préférences, raccourcis et pinceaux sont conservés.
[ "$INSTALLER_APRES" -eq 1 ] && installer_paquet_produit

# Un format demandé explicitement et impossible est un échec ; un format sauté par défaut ne l'est pas.
[ "$EXPLICITE" -eq 1 ] && [ "${#IGNORES[@]}" -gt 0 ] && exit 1
exit 0
