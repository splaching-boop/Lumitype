#!/usr/bin/env bash
# Copie les paquets de export/sorties vers export/release sous des noms sans version
# + génère SHA256SUMS.txt. Les originaux ne sont jamais modifiés.
set -euo pipefail
cd "$(dirname "$0")"
SRC="sorties"; DST="release"
mkdir -p "$DST"
copy() { # motif  nom_final
  local f; f=$(ls $SRC/$1 2>/dev/null | head -n1 || true)
  if [ -n "$f" ]; then cp -f "$f" "$DST/$2"; echo "OK   $2  <-  $f"; else echo "ABSENT  $2 (motif $1)"; fi
}
copy 'Lumitype-*.x86_64.rpm'   Lumitype-Fedora-x86_64.rpm
copy 'Lumitype_*_amd64.deb'    Lumitype-Debian-Ubuntu-amd64.deb
copy 'Lumitype_*_amd64.AppImage' Lumitype-Linux-x86_64.AppImage
copy 'Lumitype-*-x86_64.flatpak' Lumitype-Linux.flatpak
copy 'Lumitype-*-x86_64.tar.gz'  Lumitype-Linux-portable.tar.gz
copy 'Lumitype*.exe'           Lumitype-Windows-Setup.exe
( cd "$DST" && rm -f SHA256SUMS.txt && sha256sum Lumitype-* > SHA256SUMS.txt && cat SHA256SUMS.txt )
echo "Fichiers prêts dans export/$DST/ : à déposer dans la Release GitHub."
