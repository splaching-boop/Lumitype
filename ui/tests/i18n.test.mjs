// Vérifie la traduction (lib/i18n.svelte.ts), sans navigateur : npm run test:i18n
// Recherche exacte, variables `{nom}`, et messages déjà remplis (erreurs du moteur) retrouvés par motif.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
// Le module importe l'IPC (pour enregistrer la langue) et les dictionnaires : l'IPC est remplacé par un
// stub, les dictionnaires sont lus ici ; `$state` de Svelte est remplacé par une simple valeur.
const locales = (code) => JSON.stringify(JSON.parse(fs.readFileSync(path.join(here, `../src/lib/locales/${code}.json`), "utf8")));
const source = fs
  .readFileSync(path.join(here, "../src/lib/i18n.svelte.ts"), "utf8")
  .replace(/^import \{ api \} from ".\/ipc";$/m, "const api = { saveSettings: async () => undefined, loadSettings: async () => null };")
  .replace(/^import (\w+) from ".\/locales\/(\w+)\.json";$/gm, (_, name, code) => `const ${name} = ${locales(code)};`)
  .replace("$state<Locale>(\"fr\")", "\"fr\"")
  .replace("document.documentElement.lang = next;", "");
const out = path.join(here, "../../node_modules/.cache-i18n.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { i18n, t } = await import(pathToFileURL(out).href);

// ---- Français : la chaîne telle quelle, variables remplies.
assert.equal(t("Réglages"), "Réglages");
assert.equal(t("{n} calques", { n: 3 }), "3 calques");

// ---- Anglais : recherche exacte, puis variables.
i18n.locale = "en";
assert.equal(t("Réglages"), "Adjustments");
assert.equal(t("{n} calques", { n: 3 }), "3 layers");
assert.equal(t("Chaîne inconnue de l'interface"), "Chaîne inconnue de l'interface", "sans traduction : le français, jamais vide");

// ---- Message déjà rempli par le moteur : retrouvé par son motif, les morceaux variables à leur place.
assert.equal(t("l'objet dynamique « Logo » n'existe plus dans son document"), "the smart object “Logo” no longer exists in its document");
assert.equal(t("ce contenu est un document : exportez-le en .graf"), "this content is a document: export it as .graf");
assert.equal(t("3 fichier(s) lié(s) introuvable(s)"), "3 linked file(s) not found", "variable nommée {n}");

// ---- Allemand et espagnol : même mécanique.
i18n.locale = "de";
assert.equal(t("l'objet dynamique « Logo » n'existe plus dans son document"), "das Smartobjekt „Logo“ existiert in seinem Dokument nicht mehr");
i18n.locale = "es";
assert.equal(t("Réglages").length > 0, true);

console.log("i18n : ok");
