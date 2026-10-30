#!/usr/bin/env python3
"""Convertit un pack de pinceaux Krita (.bundle) en pointes intégrées à Lumitype.

    python3 tools/krita-brushes.py Brushs.bundle

Écrit `crates/engine/assets/brushes/NN.png` (masque en niveaux de gris : 0 = rien, 255 = encre) et
`crates/engine/assets/brushes/brushes.tsv` (fichier, nom, espacement), que `brushes::builtin_tips` embarque.

Lumitype peint avec une pointe (un masque) et un espacement : de chaque réglage Krita (.kpp), on garde donc ce
qui fait sa forme — pointe ronde automatique (diamètre, ratio, fondu, courbe de douceur, angle), image ou
pinceau GIMP de la pointe, texture du papier et pinceau de masquage cuits dans la pointe, hachures dessinées.
Les dynamiques de Krita (courbes de pression, d'inclinaison…) ne sont pas reprises : la dynamique de Lumitype
(panneau du pinceau) les remplace.
"""
import base64
import io
import math
import sys
import zipfile
import zlib
import xml.etree.ElementTree as ET
from pathlib import Path

import numpy as np
from PIL import Image

SIDE = 256  # côté des pointes calculées ; les pointes images gardent leurs proportions, plafonnées à 384 px
MAX_IMAGE = 384
OUT = Path(__file__).resolve().parent.parent / "crates/engine/assets/brushes"

# Réglage Krita -> nom affiché (en français : l'interface traduit). L'ordre est celui de la liste.
NAMES = {
    "b) Basic-1": "Marqueur rond",
    "b) Basic-2 Opacity": "Basique opacité",
    "b) Basic-3 Flow": "Basique flux",
    "b) Basic-5 Size": "Basique taille",
    "b) Airbrush Soft": "Aérographe doux",
    "c) Pencil-2": "Crayon",
    "c) Pencil-3 Large 4B": "Crayon 4B large",
    "c) Pencil-5 Tilted": "Crayon incliné",
    "d) Ink-3 Gpen": "Encre G-pen",
    "f) Charcoal Rock Soft": "Fusain roche",
    "j) WaterC Basic Round-Grunge": "Aquarelle ronde grunge",
    "j) WaterC Flat Decay Tilt": "Aquarelle plate",
    "u) Pixel Art": "Pixel art",
    "u) Pixel Art Fill": "Pixel art remplissage",
    "y) Screentone Moire": "Trame moirée",
    "y) Screentone Pressure": "Trame pression",
    "a) Eraser Circle": "Gomme ronde",
    "a) Eraser Soft": "Gomme douce",
}


def chunks(data):
    i = 8
    while i < len(data):
        n = int.from_bytes(data[i:i + 4], "big")
        yield data[i + 4:i + 8], data[i + 8:i + 8 + n]
        i += 12 + n


def preset_xml(data):
    for kind, body in chunks(data):
        if body.startswith(b"preset\0"):
            rest = body[len(b"preset\0"):]
            if kind == b"zTXt":
                return zlib.decompress(rest[1:]).decode("utf-8")
            if kind == b"tEXt":
                return rest.decode("utf-8")
    raise ValueError("réglage introuvable dans le .kpp")


def grid(w, h, angle=0.0, ratio=1.0):
    """Coordonnées centrées (-1..1) tournées de `angle` (radians), l'axe vertical étiré par `ratio`."""
    ys, xs = np.mgrid[0:h, 0:w].astype(np.float64)
    u = (xs + 0.5 - w / 2) / (w / 2)
    v = (ys + 0.5 - h / 2) / (h / 2)
    c, s = math.cos(angle), math.sin(angle)
    ru, rv = u * c + v * s, -u * s + v * c
    return ru, rv / max(ratio, 0.05)


def curve(points, x):
    """Courbe de Krita « x,y;x,y;… » évaluée en x (linéaire par morceaux, prolongée aux bouts)."""
    pts = sorted(tuple(map(float, p.split(","))) for p in points.strip(";").split(";") if p)
    return np.interp(x, [p[0] for p in pts], [p[1] for p in pts])


def auto_mask(mg, angle):
    """Pointe ronde automatique de Krita : cercle ou carré, fondu par défaut, gaussien ou en courbe.

    Le « fondu » de Krita est la part PLEINE du rayon : 1 = bord franc, 0 = fondu depuis le centre.
    """
    ratio = float(mg.get("ratio", 1))
    u, v = grid(SIDE, SIDE, angle, ratio)
    square = mg.get("type") == "rect"
    d = np.maximum(abs(u), abs(v)) if square else np.hypot(u, v)
    solid = min(float(mg.get("hfade", 1)), float(mg.get("vfade", 1)))
    edge = np.clip((1 - d) * SIDE / 2, 0, 1)  # bord lissé sur un pixel
    kind = mg.get("id", "default")
    if kind == "soft" and mg.get("softness_curve"):
        m = curve(mg.get("softness_curve"), np.clip(d, 0, 1)) * edge
    elif solid >= 0.999:
        m = edge
    elif kind == "gauss":
        # Plein jusqu'à `solid`, puis une cloche qui s'éteint au bord.
        t = np.clip((d - solid) / max(1 - solid, 1e-3), 0, None)
        m = np.exp(-4.0 * t * t) * edge
    else:
        m = np.clip((1 - d) / max(1 - solid, 1e-3), 0, 1) * edge
    return np.clip(m, 0, 1)


def image_mask(img):
    """Image de pointe : l'alpha si elle en a un vrai, sinon le sombre (encre noire sur fond blanc)."""
    img = img.convert("RGBA")
    a = np.asarray(img).astype(np.float64) / 255
    alpha = a[..., 3]
    luma = a[..., 0] * 0.299 + a[..., 1] * 0.587 + a[..., 2] * 0.114
    return alpha if alpha.min() < 0.999 else 1 - luma


def gih_first(data):
    """Premier pinceau d'un pinceau animé GIMP (.gih) : deux lignes d'en-tête, puis des .gbr à la suite."""
    end = data.index(b"\n", data.index(b"\n") + 1) + 1
    gbr = data[end:]
    header, _version, w, h, bpp = (int.from_bytes(gbr[i:i + 4], "big") for i in range(0, 20, 4))
    pixels = np.frombuffer(gbr[header:header + w * h * bpp], np.uint8).reshape(h, w, bpp)
    return (pixels[..., 3] if bpp == 4 else pixels[..., 0]).astype(np.float64) / 255


def embedded_pattern(params):
    """Texture du papier du réglage (PNG en base64, lui-même en base64), en niveaux de gris 0..1."""
    raw = params.get("Texture/Pattern/Pattern")
    if not raw:
        return None
    png = base64.b64decode(base64.b64decode(raw))
    img = Image.open(io.BytesIO(png)).convert("L")
    return np.asarray(img).astype(np.float64) / 255


def resize(mask, w, h):
    img = Image.fromarray((np.clip(mask, 0, 1) * 255).astype(np.uint8), "L")
    return np.asarray(img.resize((w, h), Image.LANCZOS)).astype(np.float64) / 255


def fit(mask):
    """Plafonne une pointe image à MAX_IMAGE px de côté, proportions gardées."""
    h, w = mask.shape
    k = min(1.0, MAX_IMAGE / max(w, h))
    return resize(mask, max(1, round(w * k)), max(1, round(h * k))) if k < 1 else mask


def with_texture(mask, pattern, scale, invert):
    """Texture cuite dans la pointe : le grain du papier, à l'échelle du réglage, module l'encre."""
    if pattern is None:
        return mask
    h, w = mask.shape
    # Échelle de Krita relative à la taille du pinceau : on ramène le motif à la pointe.
    ph, pw = pattern.shape
    k = max(0.15, min(4.0, scale * 2))
    tile = resize(pattern, max(4, round(pw * k)), max(4, round(ph * k)))
    reps = (h // tile.shape[0] + 2, w // tile.shape[1] + 2)
    tex = np.tile(tile, reps)[:h, :w]
    tex = 1 - tex if not invert else tex  # sombre du papier = encre qui accroche
    tex = np.clip((tex - tex.min()) / max(tex.max() - tex.min(), 1e-6), 0, 1)
    return mask * (0.25 + 0.75 * tex)


def hatching(mask, angle, spacing_px=9, width=0.45):
    """Hachures (moteur de Krita propre à la trame) : traits parallèles dans la pointe."""
    h, w = mask.shape
    ys, xs = np.mgrid[0:h, 0:w].astype(np.float64)
    along = xs * math.sin(angle) - ys * math.cos(angle)
    phase = (along / spacing_px) % 1.0
    lines = np.clip((width - np.abs(phase - 0.5) * 2 + 0.5) * 2.5, 0, 1)
    return mask * lines


def spacing_of(brush):
    if brush is None:
        return 0.1
    if brush.get("useAutoSpacing") == "1":
        return round(min(0.3, max(0.04, float(brush.get("autoSpacingCoeff", 1)) * 0.12)), 3)
    return round(min(1.0, max(0.03, float(brush.get("spacing", 0.1)))), 3)


def convert(bundle):
    z = zipfile.ZipFile(bundle)
    files = {Path(n).name: z.read(n) for n in z.namelist() if n.startswith("brushes/")}
    presets = {}
    for name in z.namelist():
        if name.endswith(".kpp"):
            root = ET.fromstring(preset_xml(z.read(name)))
            presets[Path(name).stem] = (root, {p.get("name"): (p.text or "") for p in root.iter("param")})

    OUT.mkdir(parents=True, exist_ok=True)
    for old in OUT.glob("*.png"):
        old.unlink()
    rows = []
    for index, (key, label) in enumerate(NAMES.items(), start=1):
        root, params = presets[key]
        paintop = root.get("paintopid")
        definition = params.get("brush_definition")
        brush = ET.fromstring(definition) if definition else None
        angle = float(brush.get("angle", 0)) if brush is not None else 0.0
        if paintop == "roundmarker":
            mask = auto_mask({"id": "default", "hfade": "1", "vfade": "1"}, 0)  # marqueur : rond plein, bord net
        elif brush is not None and brush.get("type") == "auto_brush":
            mask = auto_mask(brush.find("MaskGenerator").attrib, angle)
        elif brush is not None and brush.get("filename") in files:
            data = files[brush.get("filename")]
            mask = gih_first(data) if brush.get("filename").endswith(".gih") else image_mask(Image.open(io.BytesIO(data)))
            mask = fit(mask)
            if abs(angle) > 1e-3:
                img = Image.fromarray((mask * 255).astype(np.uint8), "L").rotate(math.degrees(angle), Image.BICUBIC, expand=True)
                mask = np.asarray(img).astype(np.float64) / 255
        else:
            raise SystemExit(f"{key} : pointe non reconnue ({paintop})")

        # Pinceau de masquage (une seconde pointe qui découpe la première) : cuit dans la pointe.
        if params.get("MaskingBrush/Enabled") == "true":
            masking = params.get("MaskingBrush/Preset/brush_definition")
            mb = ET.fromstring(masking) if masking else None
            if mb is not None and mb.get("filename") in files:
                cut = image_mask(Image.open(io.BytesIO(files[mb.get("filename")])))
                mask = mask * (0.2 + 0.8 * resize(cut, mask.shape[1], mask.shape[0]))
        if params.get("Texture/Pattern/Enabled") == "true":
            mask = with_texture(mask, embedded_pattern(params), float(params.get("Texture/Pattern/Scale", 1)), params.get("Texture/Pattern/Invert") == "true")
        if paintop == "hatchingbrush":
            # Angle des hachures du réglage (en degrés), sinon celui de la pointe.
            degrees = params.get("Hatching/angle")
            diameter = float(brush.find("MaskGenerator").get("diameter", SIDE)) if brush is not None else SIDE
            separation = float(params.get("Hatching/separation", 9))
            thickness = float(params.get("Hatching/thickness", separation / 2))
            mask = hatching(mask, math.radians(float(degrees)) if degrees else angle, separation * SIDE / diameter, thickness / separation)

        # Le plus encré de la pointe vaut 100 % : l'opacité se règle ensuite dans Lumitype (l'aérographe de Krita,
        # par exemple, plafonne à 40 % et compte sur l'accumulation ; ici, il serait resté pâle à jamais).
        mask = mask / max(float(mask.max()), 1e-6)
        # Bords : rien ne doit toucher le cadre (la pointe est posée tournée et redimensionnée par le moteur).
        mask = np.pad(mask, 2)
        name = f"{index:02}.png"
        Image.fromarray((np.clip(mask, 0, 1) * 255 + 0.5).astype(np.uint8), "L").save(OUT / name, optimize=True)
        spacing = spacing_of(brush)
        # Pointe ronde au bord fondu : Krita lisse le trait par son flux ; ici, chaque empreinte s'ajoute, et un
        # espacement large laisse des perles. On les rapproche (le rendu est alors celui d'un trait continu).
        soft = brush is not None and brush.get("type") == "auto_brush" and float(np.mean(mask[mask > 0.02] < 0.9)) > 0.25
        if soft and paintop != "hatchingbrush" and params.get("Texture/Pattern/Enabled") != "true":
            spacing = min(spacing, 0.04)
        rows.append(f"{name}\t{label}\t{spacing}")
        print(f"{name}  {label:26} {mask.shape[1]}x{mask.shape[0]}  espacement {spacing}")
    (OUT / "brushes.tsv").write_text("\n".join(rows) + "\n", encoding="utf-8")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    convert(sys.argv[1])
