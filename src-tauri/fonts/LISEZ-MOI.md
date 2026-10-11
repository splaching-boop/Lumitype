# Polices incluses avec Lumitype

Chaque dossier est une famille, livrée telle quelle dans les paquets (ressources Tauri `fonts/**`) et recensée par le
moteur AVANT les polices du système (`engine::text::set_bundled_fonts_dir`) : un projet s'affiche pareil sur toute
machine. Le sélecteur de police les montre en tête, section « Incluses ».

- Fichiers : instances statiques (une par graisse et style), car le moteur de texte (cosmic-text) ne règle pas les
  axes des polices variables ; une police variable n'est gardée que lorsqu'elle n'a pas d'instances statiques
  (Bitcount Ink, Climate Crisis, Geist Pixel, Kablammo, Sixtyfour Convergence : une seule graisse).
- Pour alléger : une seule taille optique (Inter 24pt, Fraunces 72pt, Google Sans), une seule chasse (Roboto, Fredoka,
  Sour Gummy sans Condensed/Expanded). Le nom de famille affiché est celui du fichier (« Inter 24pt »).
- Licences : SIL Open Font License 1.1 (`OFL.txt` dans chaque dossier), Ubuntu Font Licence (`UFL.txt`),
  Mountains of Christmas (`LICENSE.txt`). Toutes autorisent la redistribution avec le logiciel.

Ajouter une police : déposer un dossier avec ses `.ttf`/`.otf` (et sa licence) ici, puis reconstruire.
