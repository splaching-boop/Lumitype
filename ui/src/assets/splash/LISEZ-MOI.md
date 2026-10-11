# Bannière de lancement

La bannière est une petite fenêtre sans bordure de **1040 × 430 px** (rapport 2,42), affichée pendant que l'application se prépare.
Deux façons de la personnaliser, en déposant un fichier dans ce dossier :

| Fichier | Effet |
|---|---|
| `banner.jpg` (ou `.png`, `.webp`) | **Bannière complète** : votre image, seule, sur toute la fenêtre. Elle porte elle-même le nom du logiciel. Le logiciel n'ajoute que l'étape de chargement (pastille en bas à gauche) et un filet d'avancement au ras du bas. |
| `background.jpg` (ou `.png`, `.webp`) | **Image de fond** : elle occupe la partie droite et se fond dans un panneau sombre où le logiciel écrit « Lumitype », la version et les crédits. Ignorée si une bannière complète existe. |

- **Taille conseillée de la bannière complète : 2080 × 860 px** (le double de la fenêtre, pour les écrans denses) ; au minimum 1040 × 430.
  Un autre rapport est accepté : l'image est recadrée pour remplir la fenêtre, le centre est conservé (1920 × 792 convient tel quel).
- Gardez libre le coin inférieur gauche (environ 300 × 50 px) : la pastille de chargement s'y pose.
- JPEG de qualité 85 à 90 ou WebP, moins de 600 Ko : l'image est embarquée dans l'application.
- Prise en compte à la prochaine compilation. Sans aucun fichier, un fond dégradé est utilisé.
