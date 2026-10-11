#!/usr/bin/env python3
"""Régénère le guide des icônes (ui/src/assets/icons/_gabarit-icones.svg) et les SVG sources des modèles PNG,
à partir du catalogue ui/src/lib/icons.ts. À relancer quand une icône est ajoutée au catalogue.
Le rendu en PNG se fait ensuite avec tools/gabarit-icones.mjs (Node + @resvg/resvg-js)."""
import json, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "ui/src/assets/icons")
src = open(os.path.join(ROOT, "ui/src/lib/icons.ts"), encoding="utf-8").read()
body = src[src.index("export const ICONS = {"):src.index("} as const;")]
paths = dict(re.findall(r'^\s+(\w+): "([^"]+)",', body, re.M))

LABELS = {"move": "Déplacement", "marquee": "Sélection rectangulaire", "lasso": "Lasso", "wand": "Baguette magique", "crop": "Recadrage", "pipette": "Pipette", "brush": "Pinceau", "eraser": "Gomme", "bucket": "Dégradé / Pot de peinture", "pen": "Plume", "type": "Texte", "pathSelect": "Sélection directe", "shape": "Forme", "hand": "Main", "zoom": "Zoom", "eye": "Visibilité du calque (œil)", "close": "Fermer", "heal": "Correcteur localisé", "quickSelect": "Sélection rapide", "chevron": "Déplier / replier un groupe", "clip": "Flèche d'écrêtage", "textLayer": "Calque de texte", "shapeRect": "Calque de forme : rectangle", "shapeEllipse": "Calque de forme : ellipse", "shapePolygon": "Calque de forme : triangle, polygone", "group": "Groupe / Nouveau groupe", "adjustment": "Calque de réglage", "lockTransparency": "Verrou : pixels transparents", "lockPixels": "Verrou : pixels de l'image", "lockPosition": "Verrou : position", "lockAll": "Verrou : tout / cadenas du calque", "link": "Lier les calques", "fx": "Effets de calque (fx)", "mask": "Masque de fusion", "newLayer": "Nouveau calque", "trash": "Supprimer", "caret": "Liste déroulante", "plus": "Ajouter"}
GROUPS = [
    ("Outils", ["move", "marquee", "lasso", "wand", "quickSelect", "crop", "pipette", "heal", "brush", "eraser", "bucket", "pen", "pathSelect", "type", "shape", "hand", "zoom"]),
    ("Panneau Calques", ["eye", "chevron", "clip", "textLayer", "shapeRect", "shapeEllipse", "shapePolygon", "group", "adjustment", "lockTransparency", "lockPixels", "lockPosition", "lockAll", "link", "fx", "mask", "newLayer", "trash"]),
    ("Divers", ["close", "caret", "plus"]),
]
listed = {name for _, names in GROUPS for name in names}
missing = [name for name in paths if name not in listed]
if missing:
    sys.exit(f"Icônes du catalogue absentes du guide : {missing} — ajoutez-les à GROUPS et LABELS.")

CELL, SCALE, COLS, PAD = 96, 2, 6, 48
box, colw, rowh = CELL * SCALE, CELL * SCALE + 56, CELL * SCALE + 74
rows = sum((len(names) + COLS - 1) // COLS for _, names in GROUPS)
W, H = PAD * 2 + COLS * colw - 56, PAD * 2 + 150 + rows * rowh + len(GROUPS) * 56
o = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="Geist, Inter, Arial, sans-serif">',
     f'<rect width="{W}" height="{H}" fill="#111113"/>',
     f'<text x="{PAD}" y="{PAD+24}" fill="#ececee" font-size="26" font-weight="600">Lumitype — guide des icônes</text>',
     f'<text x="{PAD}" y="{PAD+56}" fill="#c4c4ca" font-size="15">Format : PNG 96 × 96 px, fond transparent, une seule couleur (blanc ou noir : l\'interface la recolore). Zone utile : 80 × 80 px, marge de 8 px (pointillés bleus).</text>',
     f'<text x="{PAD}" y="{PAD+80}" fill="#c4c4ca" font-size="15">Trait conseillé : 6 px, bouts et angles arrondis. Quadrillage : un carreau = 4 px, trait fort tous les 16 px. Les cases sont dessinées ici à l\'échelle 2.</text>',
     f'<text x="{PAD}" y="{PAD+104}" fill="#8b8b93" font-size="15">Nom du fichier = nom écrit au-dessus de la case, à déposer dans ui/src/assets/icons/. SVG accepté aussi (viewBox 0 0 24 24, trait 1,6). Icône actuelle en filigrane.</text>']
y = PAD + 150
for title, names in GROUPS:
    o += [f'<text x="{PAD}" y="{y+22}" fill="#ececee" font-size="19" font-weight="600">{title}</text>', f'<path d="M{PAD} {y+34}H{W-PAD}" stroke="#2a2a2f"/>']
    y += 56
    for i, name in enumerate(names):
        x, cy = PAD + (i % COLS) * colw, y + (i // COLS) * rowh
        gx, gy, m = x, cy + 44, 8 * SCALE
        o += [f'<text x="{x}" y="{cy+14}" fill="#ececee" font-size="15" font-weight="600">{name}.png</text>',
              f'<text x="{x}" y="{cy+33}" fill="#8b8b93" font-size="12.5">{LABELS[name]}</text>',
              f'<rect x="{gx}" y="{gy}" width="{box}" height="{box}" fill="#1b1b1e" stroke="#3a3a40"/>']
        for k in range(1, 24):
            p = k * 4 * SCALE
            o.append(f'<path d="M{gx+p} {gy}v{box}M{gx} {gy+p}h{box}" stroke="{"#26262b" if k % 4 else "#3a3a42"}" stroke-width="0.7"/>')
        o += [f'<rect x="{gx+m}" y="{gy+m}" width="{box-2*m}" height="{box-2*m}" fill="none" stroke="#4d8dff" stroke-width="1" stroke-dasharray="4 4"/>',
              f'<g transform="translate({gx} {gy}) scale({box/24})" fill="none" stroke="#ffffff" stroke-opacity="0.3" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="{paths[name]}"/></g>',
              f'<text x="{gx+box}" y="{gy+box+15}" fill="#55555c" font-size="11" text-anchor="end">96 × 96 px</text>']
    y += ((len(names) + COLS - 1) // COLS) * rowh
o.append("</svg>")
open(os.path.join(OUT, "_gabarit-icones.svg"), "w", encoding="utf-8").write("\n".join(o))

# Sources des modèles PNG (rendus par gabarit-icones.mjs) et tableau des noms pour le LISEZ-MOI.
work = os.path.join(ROOT, "tools/.icones")
os.makedirs(work, exist_ok=True)
for name, d in paths.items():
    open(os.path.join(work, f"{name}.svg"), "w").write(f'<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 24 24" fill="none" stroke="#ffffff" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="{d}"/></svg>')
json.dump({"names": list(paths)}, open(os.path.join(work, "names.json"), "w"))

readme = os.path.join(OUT, "LISEZ-MOI.md")
text = open(readme, encoding="utf-8").read()
table = ["## Noms attendus", ""]
for title, names in GROUPS:
    table += [f"### {title}", "", "| Fichier | Rôle |", "|---|---|"] + [f"| `{n}.png` | {LABELS[n]} |" for n in names] + [""]
open(readme, "w", encoding="utf-8").write(text[:text.index("## Noms attendus")] + "\n".join(table))
print(f"{len(paths)} icônes, planche {W} x {H}")
