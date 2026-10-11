#!/usr/bin/env python3
"""Écrit les icônes validées (propositions 1 et 2) dans ui/src/assets/icons/<nom>.svg, et les curseurs de la
plume en PNG dans ui/src/assets/cursors/. Relancer après avoir retouché icons.py ou v2.py. Les PNG des curseurs
sont rendus par tools/icones/curseurs.mjs (Node + @resvg/resvg-js), lancé à la fin."""
import os, subprocess, sys
HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
from icons import ICONS
from v2 import NEW

icons = {name: body for name, (_, body) in ICONS.items()}
icons.update({name: NEW[name][1] for name in ("link", "pipette", "pen", "lockAll")})

out = os.path.join(ROOT, "ui/src/assets/icons")
for name, body in icons.items():
    # Affichée comme masque par l'interface : seule la forme compte ; elle prend la couleur du texte.
    svg = f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="#000" stroke="#000" style="color:#000">{body}</svg>\n'
    open(os.path.join(out, f"{name}.svg"), "w", encoding="utf-8").write(svg)
print(f"{len(icons)} icônes écrites dans {out}")

# Curseurs de la plume : forme blanche cernée de noir (lisible sur toute image), 32 × 32 px.
CURSORS = {"pen": NEW["pen"][1], "pen-add": NEW["penAdd"][1], "pen-delete": NEW["penDelete"][1], "pen-convert": NEW["penConvert"][1], "pen-free": NEW["penFree"][1]}
work = os.path.join(HERE, ".curseurs")
os.makedirs(work, exist_ok=True)
for name, body in CURSORS.items():
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="-1 -1 26 26">'
        '<defs><filter id="contour" x="-20%" y="-20%" width="140%" height="140%">'
        '<feMorphology in="SourceAlpha" operator="dilate" radius="1.1" result="large"/>'
        '<feFlood flood-color="#000"/><feComposite in2="large" operator="in" result="noir"/>'
        '<feMerge><feMergeNode in="noir"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>'
        f'<g filter="url(#contour)" fill="#fff" stroke="#fff" style="color:#fff">{body}</g></svg>'
    )
    open(os.path.join(work, f"{name}.svg"), "w", encoding="utf-8").write(svg)
subprocess.run(["node", os.path.join(HERE, "curseurs.mjs")], check=True)
