# Démonstrations filmées

Une vidéo par guide, **nommée d'après l'identifiant de l'article** — pas d'après l'outil :
`outil-deplacement.mp4`, `outil-selection.mp4`, `outil-lasso.mp4`, `outil-baguette.mp4`,
`outil-selection-rapide.mp4`, `outil-recadrage.mp4`, `outil-pipette.mp4`, `outil-correcteur.mp4`,
`outil-pinceau.mp4`, `outil-gomme.mp4`, `outil-degrade-pot.mp4`, `outil-plume.mp4`,
`outil-texte.mp4`, `outil-selection-directe.mp4`, `outil-forme.mp4`, `outil-main.mp4`,
`outil-zoom.mp4`.

Le même fichier sert à trois endroits : la **bulle de la barre d'outils**, le **bandeau du guide**
et le **lecteur dans le corps du guide**. La correspondance outil → article est la table
`TOOL_DEMO` de `lib/media.ts` ; c'est le seul endroit à compléter si un outil change d'identifiant.

## Deux encodages, et pourquoi

Chaque film est livré **en `.webm` et en `.mp4`**, sous le même nom. Le lecteur essaie le WebM en
premier : c'est le seul décodé partout sans greffon. Les compilations libres de Chromium et
WebKitGTK sur Fedora **n'ont pas H.264** par défaut, et un `.mp4` seul y resterait noir. Le MP4 est
gardé en second pour les systèmes qui préfèrent H.264.

Convertir un nouveau film :

```bash
ffmpeg -i demo.mp4 -c:v libvpx-vp9 -crf 34 -b:v 0 -row-mt 1 -an demo.webm
```

Format : **480 × 270**, sans piste audio, quelques secondes à une trentaine, sous 450 Ko par
encodage. La lecture se fait en boucle et sans son : une vidéo avec du son ne serait jamais
entendue. Une source plus fine (960 × 540) serait préférable pour le lecteur du guide, qui affiche
le film sur toute la largeur de la fenêtre de lecture.

Tant qu'un fichier manque, la bulle et le bandeau retombent sur l'icône de l'outil, et le corps du
guide affiche l'animation de remplacement avec la mention « Démonstration à venir ». Rien d'autre à
modifier : déposer le fichier au bon nom suffit.
