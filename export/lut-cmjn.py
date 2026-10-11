#!/usr/bin/env python3
"""
Génère la table 3D de l'épreuve CMJN (Affichage › Épreuve CMJN) à partir d'un profil ICC de presse.

Chaque couleur sRGB est convertie en CMJN par le profil (ce que ferait l'imprimeur), puis ramenée en sRGB
(ce que l'écran peut en montrer). Le résultat est une grille N×N×N de triplets RVB 8 bits, lue telle quelle
par la carte graphique (texture 3D, interpolation trilinéaire) : le shader n'a aucune formule à connaître.

Format du fichier : N³ × 3 octets, ordre rouge le plus rapide, puis vert, puis bleu
(octet = ((b*N + g)*N + r)*3 + canal). N est déduit de la taille du fichier.

Usage :
  python3 export/lut-cmjn.py                                   # FOGRA39 → ui/src/assets/proof/fogra39.bin
  python3 export/lut-cmjn.py --tous                            # les cinq profils livrés (table PROFILS ci-dessous)
  python3 export/lut-cmjn.py --profil mon.icc --sortie x.bin   # autre profil (celui de l'imprimeur…)
  python3 export/lut-cmjn.py --intention perceptuel

Dépendances : Pillow avec LittleCMS (`pip install pillow`).
Profils fournis : export/profils/*.icc, tous libres de redistribution (provenance, licence et empreintes dans
export/profils/LISEZ-MOI.md). Un profil ajouté ici doit l'être aussi dans ui/src/lib/proof-profiles.ts
(libellé du menu) et dans ui/src/viewport/renderer.ts (fichier de la table).
"""
import argparse
import pathlib
import sys

try:
    from PIL import Image, ImageCms
except ImportError:  # pragma: no cover
    sys.exit("Pillow est requis : pip install pillow")

RACINE = pathlib.Path(__file__).resolve().parent.parent
# Profils livrés : profil ICC (export/profils) → table (ui/src/assets/proof), pour --tous.
PROFILS = {
    "fogra39": "FOGRA39L_coated.icc",
    "fogra51": "FOGRA51_PSOcoated_v3_postext.icc",
    "fogra52": "FOGRA52_PSOuncoated_v3_postext.icc",
    "gracol2006": "GRACoL_TR006_coated.icc",
    "swop2006c3": "SWOP_TR003_coated_3.icc",
}
INTENTIONS = {
    "relatif": ImageCms.Intent.RELATIVE_COLORIMETRIC,
    "perceptuel": ImageCms.Intent.PERCEPTUAL,
}


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profil", default=RACINE / "export/profils/FOGRA39L_coated.icc", type=pathlib.Path)
    p.add_argument("--sortie", default=RACINE / "ui/src/assets/proof/fogra39.bin", type=pathlib.Path)
    p.add_argument("--taille", default=33, type=int, help="points par axe (33 par défaut, 65 pour plus de finesse)")
    p.add_argument("--intention", default="relatif", choices=INTENTIONS, help="intention de rendu vers le CMJN")
    p.add_argument("--tous", action="store_true", help="régénère la table de chaque profil livré (ignore --profil et --sortie)")
    args = p.parse_args()

    if args.tous:
        for nom, fichier in PROFILS.items():
            generer(RACINE / "export/profils" / fichier, RACINE / f"ui/src/assets/proof/{nom}.bin", args.taille, args.intention)
    else:
        generer(args.profil, args.sortie, args.taille, args.intention)


def generer(profil: pathlib.Path, sortie: pathlib.Path, n: int, intention: str) -> None:
    """Écrit la table n³ de l'épreuve selon `profil` (intention choisie vers le CMJN, relative + BPC au retour)."""
    srgb = ImageCms.createProfile("sRGB")
    presse = ImageCms.getOpenProfile(str(profil))
    bpc = ImageCms.Flags.BLACKPOINTCOMPENSATION
    vers_cmjn = ImageCms.buildTransform(srgb, presse, "RGB", "CMYK", renderingIntent=INTENTIONS[intention], flags=bpc)
    vers_ecran = ImageCms.buildTransform(presse, srgb, "CMYK", "RGB", renderingIntent=ImageCms.Intent.RELATIVE_COLORIMETRIC, flags=bpc)

    # Une image n × n² : ligne = b*n + g, colonne = r.
    grille = Image.new("RGB", (n, n * n))
    px = grille.load()
    for b in range(n):
        for g in range(n):
            for r in range(n):
                px[r, b * n + g] = (r * 255 // (n - 1), g * 255 // (n - 1), b * 255 // (n - 1))
    epreuve = ImageCms.applyTransform(ImageCms.applyTransform(grille, vers_cmjn), vers_ecran)

    sortie.parent.mkdir(parents=True, exist_ok=True)
    sortie.write_bytes(epreuve.tobytes())
    print(f"{sortie} : {n}³ points, {sortie.stat().st_size} octets, profil « {ImageCms.getProfileDescription(presse).strip()} », intention {intention}")


if __name__ == "__main__":
    main()
