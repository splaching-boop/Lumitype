#!/usr/bin/env bash
# Branche les nouvelles icônes : fichiers .graf (Linux), et retire l'ancienne bannière jpg.
set -euo pipefail
cd "$(dirname "$0")"
rm -f ui/src/assets/splash/banner.jpg
python3 - <<'PY'
import json
p="src-tauri/tauri.conf.json"
c=json.load(open(p,encoding="utf-8"))
tailles=(48,64,128,256,512)
for cible in ("deb","rpm","appimage"):
    f=c["bundle"]["linux"][cible].setdefault("files",{})
    for s in tailles:
        f[f"/usr/share/icons/hicolor/{s}x{s}/mimetypes/application-x-lumitype.png"]=f"linux/mimetypes/application-x-lumitype-{s}.png"
json.dump(c,open(p,"w",encoding="utf-8"),indent=2,ensure_ascii=False)
open(p,"a").write("\n")
PY
echo "OK : icônes branchées. Recompilez : npm run tauri build"
