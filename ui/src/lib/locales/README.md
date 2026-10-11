# Dictionnaires de l'interface

Le **français est la langue de référence** : chaque chaîne est écrite en français dans le code et
passe par `t("…")` (`lib/i18n.svelte.ts`) au moment de l'affichage. Les guides (`lib/articles/*.ts`)
suivent la même règle, phrase par phrase, via `localizeArticle`.

- `keys.fr.json` : la liste des chaînes françaises connues, dans l'ordre du code. Régénérée par
  `python3 ../../../scripts/i18n-extract.py` (voir ce script) après un ajout de textes.
- `en.json`, `es.json`, `de.json` : « chaîne française » → traduction. Une chaîne absente s'affiche en
  français : rien ne casse, on la voit et on la traduit.

Conventions : les `{variables}` gardent leur nom ; `**gras**` et `[Touche]` sont un balisage léger
rendu par l'interface ; les noms de touches se traduisent aussi (« Maj » → « Shift », « Umschalt »,
« Mayús »), `chordLabel` les passe par le dictionnaire.
