#!/usr/bin/env bash
# Copie les modèles de `modeles/` dans `src-tauri/templates/`, embarqué par Tauri. Miroir exact :
# un modèle retiré du dossier disparaît de l'application à la construction suivante.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p src-tauri/templates
find src-tauri/templates -maxdepth 1 \( -name '*.graf' -o -name '*.txt' \) -delete
shopt -s nullglob
n=0
for f in modeles/*.graf; do
  cp "$f" src-tauri/templates/
  base="${f%.graf}"
  [ -f "$base.txt" ] && cp "$base.txt" src-tauri/templates/
  n=$((n + 1))
done
echo "Modèles embarqués : $n"
