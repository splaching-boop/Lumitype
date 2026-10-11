#!/usr/bin/env bash
# Appelé par Tauri juste avant la fabrication des paquets (build.beforeBundleCommand de src-tauri/tauri.cef.json) :
# rassemble les fichiers de Chromium (CEF) dans target/release/cef-paquet/, que les paquets installent dans
# /usr/lib/Lumitype/ à côté des ressources de Lumitype. L'exécutable (/usr/bin/lumitype) les y trouve seul.
set -euo pipefail
cd "$(dirname "$0")/.."
CIBLE="${CARGO_TARGET_DIR:-$(pwd)/target}/release"
# Dossier de la distribution CEF téléchargée à la compilation (celui qui contient libcef.so), le plus récent.
SOURCE="$(ls -dt "$CIBLE"/build/cef-dll-sys-*/out/*/ 2>/dev/null | while read -r d; do [ -f "$d/libcef.so" ] && { echo "$d"; break; }; done)"
if [ -z "$SOURCE" ]; then
  echo "cef-paquet : distribution CEF introuvable sous $CIBLE/build (compiler avec --features cef)" >&2
  exit 1
fi
PAQUET="$CIBLE/cef-paquet"
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
  echo "cef-paquet : « strip » introuvable (paquet binutils) — bibliothèques laissées telles quelles, paquets très lourds" >&2
fi
echo "cef-paquet : $(du -sh "$PAQUET" | cut -f1) de fichiers Chromium prêts dans $PAQUET"
