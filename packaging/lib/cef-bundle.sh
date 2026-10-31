#!/usr/bin/env bash
# Appelé par Tauri juste avant la fabrication des paquets (build.beforeBundleCommand de src-tauri/tauri.cef.json) :
# rassemble les fichiers de Chromium (CEF) dans target/release/cef-bundle/, que les paquets installent dans
# /usr/lib/Lumitype/ (.deb, .rpm) ou /usr/lib/ (AppImage), à côté des ressources de Lumitype. L'exécutable les y trouve seul.
set -euo pipefail
cd "$(dirname "$0")/../.."
CIBLE="${CARGO_TARGET_DIR:-$(pwd)/target}/release"
# Dossier de la distribution CEF téléchargée à la compilation (celui qui contient libcef.so), le plus récent.
SOURCE="$(ls -dt "$CIBLE"/build/cef-dll-sys-*/out/*/ 2>/dev/null | while read -r d; do [ -f "$d/libcef.so" ] && { echo "$d"; break; }; done)"
if [ -z "$SOURCE" ]; then
  echo "cef-bundle : distribution CEF introuvable sous $CIBLE/build (compiler avec --features cef)" >&2
  exit 1
fi
PAQUET="$CIBLE/cef-bundle"
rm -rf "$PAQUET" && mkdir -p "$PAQUET/locales"
# Bibliothèques, ressources et données : tout ce qui est à plat dans la distribution, sauf la documentation, les
# fichiers de construction et l'assistant de bac à sable (Lumitype tourne sans bac à sable).
for fichier in "$SOURCE"/*; do
  [ -f "$fichier" ] || continue
  case "$(basename "$fichier")" in
    *.txt | *.cmake | CMakeLists.txt | archive.json | chrome-sandbox) continue ;;
  esac
  cp -f "$fichier" "$PAQUET/"
done
# Langues de Chromium (menus contextuels, messages) : celles de Lumitype seulement. en-US est obligatoire, c'est
# aussi la langue de repli pour toutes les autres.
for langue in en-US en-GB fr de es es-419; do
  [ -f "$SOURCE/locales/$langue.pak" ] && cp -f "$SOURCE/locales/$langue.pak" "$PAQUET/locales/"
done
# Symboles de débogage retirés : libcef.so passe d'environ 1,4 Go à 200-250 Mo. Ils ne servent qu'à déboguer
# Chromium lui-même. Les fichiers de target/release/ (compilation) ne sont pas touchés : seule cette copie l'est.
if command -v strip >/dev/null 2>&1; then
  for bibliotheque in "$PAQUET"/*.so "$PAQUET"/*.so.*; do
    [ -f "$bibliotheque" ] && strip --strip-unneeded "$bibliotheque"
  done
else
  echo "cef-bundle : « strip » introuvable (paquet binutils) — bibliothèques laissées telles quelles, paquets très lourds" >&2
fi
echo "cef-bundle : $(du -sh "$PAQUET" | cut -f1) de fichiers Chromium prêts dans $PAQUET"

# AppImage seulement : bibliothèques ouvertes À LA VOLÉE (dlopen). linuxdeploy ne voit que les bibliothèques LIÉES aux
# exécutables : sans cette copie, l'AppImage les prend sur le système, ou plante si elles manquent. Elles vont dans
# /usr/lib de l'AppImage (src-tauri/tauri.cef.json), le dossier où linuxdeploy dirige l'exécutable et où se trouve
# libcef.so. Les .deb et .rpm, eux, les déclarent en dépendances : ils ne les embarquent pas.
#   - libxkbcommon-x11, libxcb-xkb : gestion du clavier sous X11 (crate xkbcommon-dl). Sans elles : « Library
#     libxkbcommon-x11.so could not be loaded » (test d'AppImageHub).
#   - libsoftokn3, libfreebl3, libfreeblpriv3, libnssckbi : morceaux de NSS (chiffrement de Chromium) chargés par
#     libnss3, que linuxdeploy embarque. NSS les cherche d'abord à côté d'elle-même ; absents, elle prenait ceux du
#     système, d'une autre version : « NSSUTIL_3.108 not found », plantage au démarrage. Tous viennent donc de la même
#     machine que libnss3, pour être de la même version.
EMBARQUEES="$CIBLE/appimage-libs"
rm -rf "$EMBARQUEES" && mkdir -p "$EMBARQUEES"
# Chemin réel d'une bibliothèque 64 bits : liste du système (ldconfig), puis les dossiers où Debian/Ubuntu et Fedora
# rangent les morceaux de NSS (absents de cette liste sur Debian/Ubuntu : sous-dossier nss/).
trouver() {
  local nom="$1" chemin dossier
  # awk lit toute la liste (pas de « exit ») : arrêté plus tôt, ldconfig recevait SIGPIPE et, avec pipefail, tout le
  # script s'arrêtait (code 141).
  chemin="$( (/sbin/ldconfig -p 2>/dev/null || ldconfig -p 2>/dev/null) | awk -v n="$nom" '$1 == n && /x86-64/ && !trouve { print $NF; trouve = 1 }')"
  if [ -n "$chemin" ] && [ -e "$chemin" ]; then echo "$chemin"; return; fi
  for dossier in /usr/lib/x86_64-linux-gnu/nss /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib64/nss /usr/lib /usr/lib/nss; do
    if [ -e "$dossier/$nom" ]; then echo "$dossier/$nom"; return; fi
  done
}
for nom in libxkbcommon-x11.so.0 libxkbcommon.so.0 libxcb-xkb.so.1 libsoftokn3.so libfreebl3.so libfreeblpriv3.so libnssckbi.so; do
  chemin="$(trouver "$nom")"
  if [ -z "$chemin" ]; then
    echo "cef-bundle : $nom introuvable sur cette machine. Debian/Ubuntu : sudo apt install libxkbcommon-x11-0 libxcb-xkb1 libnss3 ; Fedora : sudo dnf install libxkbcommon-x11 nss nss-softokn nss-softokn-freebl" >&2
    exit 1
  fi
  # Le fichier réel (pas le lien symbolique), sous le nom que cherche le programme.
  cp -fL "$chemin" "$EMBARQUEES/$nom"
done
echo "cef-bundle : bibliothèques de l'AppImage prêtes dans $EMBARQUEES ($(ls "$EMBARQUEES" | tr '\n' ' '))"
