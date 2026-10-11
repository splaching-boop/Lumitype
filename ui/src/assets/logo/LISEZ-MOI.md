# Logo de Lumitype

Logo de l'auteur, décliné en plusieurs tailles. Pour le changer, remplacez ces fichiers en gardant leurs noms :

| Fichier | Usage |
|---|---|
| `logo-256.png` | Interface : fenêtre « À propos », bannière de lancement |
| `logo-16.png`, `logo-32.png` | Icône de fenêtre et barre des tâches |
| `logo-64.png`, `logo-128.png`, `logo-256.png` | Raccourcis, lanceurs Linux |
| `logo-512.png` | Boutiques d'applications, pages web |
| `logo-1024.png` | **Source des icônes de l'installateur** |

Format : carré, fond transparent (le logo porte son propre fond s'il en a un), marge de sécurité d'environ 8 %.

## Icônes de l'installateur (Windows, Linux)

Une fois `logo-1024.png` remplacé, régénérez tous les formats attendus par Tauri (`.ico`, `.icns`, PNG) :

```
npm run tauri icon ui/src/assets/logo/logo-1024.png
```

Les fichiers sont écrits dans `src-tauri/icons/` et repris à la prochaine construction.
