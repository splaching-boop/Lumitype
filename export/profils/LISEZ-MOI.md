# Profils d'impression de l'épreuve CMJN

Profils ICC CMJN dont `export/lut-cmjn.py` tire les tables de l'épreuve (Affichage › Épreuve : …,
`ui/src/assets/proof/*.bin`). Seules les tables sont livrées dans l'application ; les profils restent ici, pour les
régénérer (`python3 export/lut-cmjn.py --tous`). Tous sont **libres de redistribution, sans restriction connue (CC0)**.

| Fichier                               | Condition d'impression                                              | Description interne        | Taille (octets) | SHA-256                                                            |
|---------------------------------------|---------------------------------------------------------------------|----------------------------|-----------------|--------------------------------------------------------------------|
| `FOGRA39L_coated.icc`                 | ISO 12647-2:2004, offset, papier couché (types 1 et 2)              | FOGRA39L Coated            | 121 368         | `b35713ef7eff09349d4c3249e5f377736d06d8a2671c54712971a3546bf17c57` |
| `FOGRA51_PSOcoated_v3_postext.icc`    | ISO 12647-2:2013, offset, couché premium (condition PSO Coated v3)  | FOGRA51 (postext, CC0)     | 230 920         | `c29ca236eb89613a1d9a0e4762f135dbfa3263f9f3e145153e3fbc2b2a728d4e` |
| `FOGRA52_PSOuncoated_v3_postext.icc`  | ISO 12647-2:2013, offset, non couché sans bois (PSO Uncoated v3)    | FOGRA52 (postext, CC0)     | 230 896         | `5e83ca1bb42d86d8c50364cf957729ece842cf687ab9f94637d4c7d5a3bb75ad` |
| `GRACoL_TR006_coated.icc`             | GRACoL 2006, couché n° 1 (CGATS TR 006)                             | GRACoL TR006 Coated        | 122 092         | `4105c8d5e239846fd9a6b73216b4bd5a6eb3389ef55fc59ae2642154c975746b` |
| `SWOP_TR003_coated_3.icc`             | SWOP 2006, couché n° 3 (CGATS TR 003)                               | SWOP TR003 Coated          | 122 072         | `dfafeb8d1614aa21b82ec8ecd867e59cea041ce2c80396ffa2ba72b2ca776269` |

Chaque fichier a été vérifié avec Pillow/LittleCMS (`ImageCms.ImageCmsProfile(…).profile.xcolor_space` = `CMYK`) ;
leur balise copyright dit « This profile is free of known copyright restrictions ».

## Provenance et licence

- **`FOGRA39L_coated.icc`** — profil de **colord** (projet freedesktop, `data/profiles/FOGRA39L_coated.iccprofile.xml`,
  https://github.com/hughsie/colord), calculé par ArgyllCMS à partir des données de caractérisation FOGRA39 de la
  Fogra ; licence déclarée par colord : **CC0** (`<license>CC0</license>` dans le XML source). Présent dans le projet
  avant le 10/10/2026 (balise de création : 13/08/2018). Ce n'est pas le fichier « ISO Coated v2 » de l'ECI, mais la
  même condition d'impression.
- **`FOGRA51_PSOcoated_v3_postext.icc`**, **`FOGRA52_PSOuncoated_v3_postext.icc`** — récupérés le 10/10/2026 dans le
  dépôt **postext** (https://github.com/drnachio/postext, licence du dépôt MIT, commit
  `849eb33ed13406bdb7476d20562389d0c05e5cec`, fichiers `packages/postext/icc/fogra51.icc` et `fogra52.icc`, renommés
  ici). D'après `packages/postext/icc/README.md`, ils ont été calculés avec ArgyllCMS 3.5.0 à partir des données de
  caractérisation publiques de la Fogra (https://www.color.org/chardata/FOGRA51.txt, `FOGRA52.txt`), avec la
  génération du noir des profils FOGRA de colord (`colprof -qm -kp 0.0 0.3 0.94 0.94 0.48 -l300`), et publiés sous
  **CC0**. Ce ne sont pas les fichiers de l'ECI (`PSOcoated_v3.icc`, `PSOuncoated_v3_FOGRA52.icc`), mais des profils
  de la même condition d'impression. Contrôle : aplats CMJN 100 % → L\*a\*b\* conformes aux valeurs FOGRA51/52
  (papier ≈ 95/1/−6 et 94/2/−10, noir ≈ 16 et 33).
- **`GRACoL_TR006_coated.icc`**, **`SWOP_TR003_coated_3.icc`** — profils de **colord** (`GRACoL_TR006_coated`,
  `SWOP_TR003_coated_3`, CC0), récupérés le 10/10/2026 dans le même dépôt postext (`gracol2006.icc`, `swop3.icc`,
  même commit), qui les reprend tels que colord 1.4.8 les installe (`/usr/share/color/icc/colord`).

## Profils demandés mais écartés

- **PSO Coated v3 / PSO Uncoated v3 de l'ECI** (fichiers originaux) : introuvables sur les sources accessibles
  (GitHub, npm, PyPI) avec une licence de redistribution vérifiable ; remplacés par les profils CC0 de postext
  ci-dessus, de même condition (FOGRA51, FOGRA52).
- **GRACoL 2013 / CRPC6** (Idealliance) : le seul fichier trouvé (`GRACoL2013_CRPC6.icc`, paquet npm
  `@imgly/plugin-print-ready-pdfs-web`) est en réalité le profil « SWOP2006_Coated3v2.icc » de 2007 mal nommé, et sa
  licence interdit toute modification ; écarté. Remplacé par GRACoL 2006 (TR 006, CC0), très proche de FOGRA39.
- **SWOP 2013 / CRPC5** : aucun fichier libre trouvé ; SWOP 2006 couché n° 3 (TR 003, CC0) le remplace.
- **Japan Color 2011 Coated** : aucun profil ni jeu de caractérisation libre trouvé ; non proposé.
- **Profils CMJN de Ghostscript** (`default_cmyk.icc`, `ps_cmyk.icc`) : licence AGPL, non retenus.

Pour un autre profil (celui de votre imprimeur, par exemple) : voir la section « Épreuve CMJN » de
`export/LISEZ-MOI.md`.
