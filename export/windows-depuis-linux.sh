#!/usr/bin/env bash
# Compile l'installateur Windows (.exe NSIS) depuis Linux (expérimental) et le range dans export/sorties/.
set -euo pipefail
RACINE="$(cd "$(dirname "$0")/.." && pwd)"
SORTIES="$RACINE/export/sorties"
cd "$RACINE"
mkdir -p "$SORTIES"

manque=()
for c in makensis clang lld-link llvm-ar; do command -v "$c" >/dev/null || manque+=("$c"); done
if [ ${#manque[@]} -gt 0 ]; then
  echo "Outils manquants : ${manque[*]}"
  echo "Installez-les :  sudo dnf install -y nsis clang lld llvm"
  exit 1
fi
rustup target add x86_64-pc-windows-msvc
command -v cargo-xwin >/dev/null || cargo install --locked cargo-xwin

npm run tauri build -- --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis

trouve=0
for f in "$RACINE"/target/x86_64-pc-windows-msvc/release/bundle/nsis/*.exe; do
  [ -f "$f" ] || continue
  cp -f "$f" "$SORTIES/" && echo "OK  $SORTIES/$(basename "$f")" && trouve=1
done
[ "$trouve" = 1 ] || { echo "Aucun .exe produit : voir les erreurs ci-dessus."; exit 1; }
