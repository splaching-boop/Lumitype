# Captures d'écran de la notice

Harnais qui fait tourner l'interface de Lumitype dans Chromium (Playwright), sans Tauri ni moteur Rust, pour
produire les images de la notice en anglais (thème Legacy, accent rouge) dans `ui/src/assets/manual/en/`.

## Relancer

```sh
node tools/screenshots/capture.mjs                    # toutes les captures
node tools/screenshots/capture.mjs menu-file toolbar  # seulement celles-ci
OUT=/tmp/essai node tools/screenshots/capture.mjs     # écrire ailleurs (pour essayer)
```

Prérequis : `npm install` à la racine (Vite, Svelte), Playwright et son Chromium, `sharp` (installés localement ou
globalement : le script cherche aussi dans `npm root -g`). Ne pas lancer `playwright install` si Chromium est déjà là.

Le script démarre le serveur de Vite sur `ui/` (port 5199 ou suivant), ouvre une page neuve par capture
(fenêtre 1440 × 900, `deviceScaleFactor: 2`, WebGL2 par SwiftShader), recadre sur l'élément utile avec 16 px de
marge et enregistre un PNG optimisé par `sharp` (sans perte, ou palette réduite si l'image dépasse 250 Ko).
En fin de course, il affiche le poids de chaque image et la liste des **commandes non simulées** : après un
changement de l'interface, c'est par là qu'on voit ce que le faux moteur doit apprendre. Une capture qui échoue
laisse `<nom>.failed.png` (la fenêtre entière au moment de l'échec) à côté des autres.

Toujours regarder les images produites : texte en anglais, rien de coupé, pas d'écran vide ni de message d'erreur.

## Fichiers

- `capture.mjs` : serveur, navigateur, liste des captures (`SHOTS`), recadrage, optimisation.
- `mock-tauri.js` : injecté avant le chargement de la page. Il définit `window.__TAURI_INTERNALS__` et
  `window.__TAURI_EVENT_PLUGIN_INTERNALS__` (invoke, transformCallback, convertFileSrc, metadata, événements) et
  répond aux commandes de `ui/src/lib/ipc.ts` et `ui/src/lib/myref.ts` : réglages (langue `en`), documents générés
  (affiche A4 300 ppp « Summer poster » : calques Title, Shape, Photo avec masque, Background ; variante avec format
  d'impression ; deux plans de travail), tuiles composées en JS (OffscreenCanvas), miniatures, récents, modèles,
  bibliothèque My ref fictive. Une commande inconnue répond `null` et s'écrit `[mock] inconnue : …` dans la console.
  Les images sont des compositions abstraites générées (dégradés, collines, cercles), aucune œuvre existante.

## Ajouter une capture

Ajouter une entrée à `SHOTS` dans `capture.mjs` :

```js
"dialog-feather": {
  scene: "poster",                 // poster (défaut), print ou artboards
  async run(page) {
    await openDocument(page);      // Fichier › Nouveau, puis Créer : le document de la scène
    await action(page, "select.feather"); // identifiant d'action de ui/src/lib/actions.ts
    return page.locator("dialog[open]"); // élément(s) à cadrer ; un tableau = leur union
  },
},
```

`full: true` garde toute la fenêtre. Outils disponibles : `openDocument`, `action` (exécute une action du catalogue,
plus robuste qu'un clic dans les menus), `openMenu(page, "View")`, `selectLayer(page, "Photo")`, et
`mod(page, "/src/state/app.svelte.ts", "(m) => …")` pour toucher directement l'état de l'interface (le serveur de
Vite sert les mêmes modules que l'application). Si l'interface appelle une nouvelle commande, l'ajouter à
`handlers` dans `mock-tauri.js`.

`FRENCH_LEFTOVERS` (dans `capture.mjs`) remplace, juste avant la prise de vue, les quelques chaînes que
l'interface anglaise affiche encore en français : retirer chaque ligne dès que l'interface la traduit elle-même.
