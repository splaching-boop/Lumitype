# Calage du panneau Développement sur Lightroom

Ces scripts produisent `crates/engine/src/develop.bin`, la table de calibration que lit
`engine::calibration`.

## Principe

Une mire chromatique est passée dans Lightroom avec **un seul curseur** poussé à une valeur
connue. Le couple avant/après donne, pour chaque pixel, une correspondance `couleur -> couleur`.
On ajuste dessus une transformation en quatre étages :

1. matrice 3 x 3, ou polynôme racine à six termes quand il mesure plus juste, en lumière
   linéaire ou directement en sRGB — les quatre variantes sont essayées, la meilleure est gardée ;
2. courbe de 129 points par canal, **lissée puis rendue croissante** ;
3. correction résiduelle de 65 points indexée par la luminance, **réservée aux neutres** ;
4. correction indexée par la teinte, sur 96 secteurs, **pondérée par la chroma**.

Les étages 3 et 4 sont complémentaires : l'un s'éteint là où l'autre agit.

## Trois pièges, mesurés puis évités

- **Courbes non monotones.** Un creux de deux niveaux dans une courbe de transfert se voit
  comme une bande verticale dans un dégradé. D'où la régression isotone.
- **Sur-ajustement au bruit.** Les médianes par tranche relevées sur des copies d'écran
  compressées ne sont pas lisses. Le lissage de Whittaker les régularise — et fait *baisser*
  l'écart, preuve que le bruit n'apportait rien.
- **Les gris écrasés par les couleurs.** La mire est à 80 % faite de teintes saturées : un
  résidu ajusté sur tous les pixels fait virer les gris. D'où la séparation neutres / couleurs.

## Ce qui reste faible

- Seules les **valeurs mesurées** sont exactes (écart 2,3 sur 255). Entre deux mesures, le
  mélange est linéaire et l'écart monte à 12–40. Mesurer -50 et +50 réglerait ça.
- `Contraste`, `Texture` et `Clarté` n'ont jamais été capturés ; `Rouge primaire` seulement
  vers le négatif. Ces cas gardent l'ancienne formule.
- La mire ne contient que des teintes saturées et des gris. Une mire **Hald CLUT** couvrirait
  tout le cube des couleurs et permettrait de valider le calage ailleurs que sur ces deux
  familles.

## Relancer

```
python3 export.py          # écrit crates/engine/src/develop.bin (utilise v5.py)
```

Les captures Lightroom sont attendues dans `/tmp/mires` : `ref.png` et une image par réglage.
