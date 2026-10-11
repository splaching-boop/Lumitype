# Icônes de Lumitype

Déposez ici un fichier par icône. Dès qu'un fichier `<nom>.png` (ou `<nom>.svg`) existe, il remplace l'icône intégrée du même nom ;
supprimez-le pour revenir à l'icône d'origine. Rien d'autre à modifier : l'icône est reprise à la prochaine compilation.

## Format en pixels

| | |
|---|---|
| Fichier | **PNG, 96 × 96 px**, fond transparent |
| Couleur | une seule, blanc ou noir : le fichier sert de masque, l'interface le recolore (gris au repos, blanc quand l'élément est actif) |
| Zone utile | **80 × 80 px**, soit une marge de **8 px** de chaque côté |
| Trait | **6 px** conseillé, bouts et angles arrondis ; un dessin plein fonctionne aussi |
| Alignement | calez les traits droits sur la grille de 4 px : ils restent nets une fois réduits |

L'icône s'affiche entre 12 et 24 px selon l'endroit. 96 px couvre un écran très dense (×4) ; en dessous de 48 × 48 px elle deviendrait floue.
SVG accepté aussi : `viewBox="0 0 24 24"`, trait de 1,6 (la même grille, 1 unité = 4 px).

## Fichiers d'aide

- `_gabarit-icones.svg` et `_gabarit-icones.png` : la planche-guide, avec chaque case à l'échelle 2, son nom de fichier, le quadrillage, la zone utile et l'icône actuelle en filigrane.
- `_modeles-png/` : l'icône actuelle de chaque nom, en PNG 96 × 96 px blanc sur transparent. Partez de ces fichiers, redessinez, puis enregistrez le résultat **dans ce dossier-ci** sous le même nom.

Les fichiers et dossiers dont le nom commence par `_` sont ignorés par l'application.

## Noms attendus

### Outils

| Fichier | Rôle |
|---|---|
| `move.png` | Déplacement |
| `marquee.png` | Sélection rectangulaire |
| `lasso.png` | Lasso |
| `wand.png` | Baguette magique |
| `quickSelect.png` | Sélection rapide |
| `crop.png` | Recadrage |
| `pipette.png` | Pipette |
| `heal.png` | Correcteur localisé |
| `brush.png` | Pinceau |
| `eraser.png` | Gomme |
| `bucket.png` | Dégradé / Pot de peinture |
| `pen.png` | Plume |
| `pathSelect.png` | Sélection directe |
| `type.png` | Texte |
| `shape.png` | Forme |
| `hand.png` | Main |
| `zoom.png` | Zoom |

### Panneau Calques

| Fichier | Rôle |
|---|---|
| `eye.png` | Visibilité du calque (œil) |
| `chevron.png` | Déplier / replier un groupe |
| `clip.png` | Flèche d'écrêtage |
| `textLayer.png` | Calque de texte |
| `shapeRect.png` | Calque de forme : rectangle |
| `shapeEllipse.png` | Calque de forme : ellipse |
| `shapePolygon.png` | Calque de forme : triangle, polygone |
| `group.png` | Groupe / Nouveau groupe |
| `adjustment.png` | Calque de réglage |
| `lockTransparency.png` | Verrou : pixels transparents |
| `lockPixels.png` | Verrou : pixels de l'image |
| `lockPosition.png` | Verrou : position |
| `lockAll.png` | Verrou : tout / cadenas du calque |
| `link.png` | Lier les calques |
| `fx.png` | Effets de calque (fx) |
| `mask.png` | Masque de fusion |
| `newLayer.png` | Nouveau calque |
| `trash.png` | Supprimer |

### Divers

| Fichier | Rôle |
|---|---|
| `close.png` | Fermer |
| `caret.png` | Liste déroulante |
| `plus.png` | Ajouter |
