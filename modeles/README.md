# Modèles livrés avec Lumitype

Déposez ici les projets `.graf` à proposer dans **Accueil › Modèles**. Rien d'autre à faire :
au moment de construire (`export\windows.bat`, `export/linux.sh`, `export/maj.sh`), ils sont copiés
dans l'application et apparaissent, dans l'ordre alphabétique, avec un aperçu calculé au premier
affichage.

- Le **nom** affiché est celui du fichier, sans extension, tirets et soulignés remplacés par des
  espaces (`Affiche_concert.graf` → « Affiche concert »).
- Facultatif : un fichier texte du même nom (`Affiche_concert.txt`) donne la **description** sous
  la carte ; sans lui, elle indique les dimensions du document.
- Un modèle s'ouvre toujours **comme copie** : l'utilisateur ne peut pas l'écraser.

Ce dossier n'est pas lu à l'exécution : c'est `src-tauri/templates/` (rempli par les scripts) qui
est embarqué. Pour tester en développement (`npm run tauri dev`), lancez `export/sync-modeles.sh`
(ou `.ps1`) une fois.
