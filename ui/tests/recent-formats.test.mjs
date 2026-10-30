// Formats rapides et liste des formats (lib/formats.ts, lib/recent-formats.ts), sans navigateur : npm run test:formats
// Test de la logique des formats rapides (lib/recent-formats.ts), sans navigateur.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { fileURLToPath } from "node:url";
import ts from "typescript";
const here = path.dirname(fileURLToPath(import.meta.url));
const lib = path.join(here, "../src/lib");
const out = path.join(here, "../../node_modules/.cache-recent-formats");
fs.mkdirSync(out, { recursive: true });
const js = (file) => ts.transpileModule(fs.readFileSync(path.join(lib, file), "utf8"), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
fs.writeFileSync(path.join(out, "formats.mjs"), js("formats.ts"));
fs.writeFileSync(
  path.join(out, "recent-formats.mjs"),
  js("recent-formats.ts")
    .replace(/import \{ writable \} from "svelte\/store";/, "const writable = (v) => ({ v, set(x) { this.v = x; }, update(f) { this.v = f(this.v); } });")
    .replace(/from "\.\/formats"/, 'from "./formats.mjs"')
    .replace(/import \{ api \} from "\.\/ipc";/, "const api = { loadSettings: async () => null, saveSettings: async () => undefined };"),
);
const F = await import(pathToFileURL(path.join(out, "formats.mjs")).href);
const R = await import(pathToFileURL(path.join(out, "recent-formats.mjs")).href);
const titles = (list) => list.map((q) => (q.format ? q.format.label + (q.hd ? " HD" : "") : `${q.width}x${q.height}`));

// Sans historique : l'ordre par défaut.
assert.deepEqual(titles(R.quickFormats([])), ["Écran HD", "4K", "A4 · 300 ppp", "Carré", "Story"]);
// Story utilisé : premier, les autres reculent d'un cran, sans doublon.
let h = R.pushHistory([], R.fromFormat(F.formatById("story")));
assert.deepEqual(titles(R.quickFormats(h)), ["Story", "Écran HD", "4K", "A4 · 300 ppp", "Carré"]);
// Puis une taille libre 1200 × 628 à 72 ppp : carte normale en tête, Story recule.
h = R.pushHistory(h, { width: 1200, height: 628, dpi: 72, transparent: false });
const q = R.quickFormats(h);
assert.deepEqual(titles(q), ["1200x628", "Story", "Écran HD", "4K", "A4 · 300 ppp"]);
assert.equal(q[0].format, null);
// Réutiliser Story : pas de doublon dans l'historique.
h = R.pushHistory(h, R.fromFormat(F.formatById("story")));
assert.equal(h.length, 2);
assert.deepEqual(titles(R.quickFormats(h)), ["Story", "1200x628", "Écran HD", "4K", "A4 · 300 ppp"]);
// La Story d'Instagram a la même taille : même document, elle remplace la Story par défaut (sans doublon).
h = R.pushHistory(h, R.fromFormat(F.formatById("instagram-story")));
assert.deepEqual(titles(R.quickFormats(h)), ["Story", "1200x628", "Écran HD", "4K", "A4 · 300 ppp"]);
assert.equal(R.quickFormats(h)[0].formatId, "instagram-story");
// HD : taille HD retenue, libellé « HD ».
h = R.pushHistory(h, R.fromFormat(F.formatById("instagram-portrait"), true));
assert.equal(R.quickFormats(h)[0].width, 2160);
assert.deepEqual(titles(R.quickFormats(h)).slice(0, 1), ["Publication portrait HD"]);
// Fond transparent : un autre document que le même format en blanc.
h = R.pushHistory(h, { width: 1920, height: 1080, dpi: 72, transparent: true });
assert.equal(R.quickFormats(h).filter((x) => x.width === 1920).length, 2);
// Taille venue d'une série de la fenêtre Nouveau document : retrouvée dans le catalogue.
assert.equal(R.describe({ width: 3840, height: 2160, dpi: 72, transparent: false }).format?.id, "4k");
// Lecture prudente : entrées abîmées ignorées, limite respectée.
assert.deepEqual(R.parseHistory('[{"width":10,"height":20,"dpi":72},{"width":"x"},null,{"width":0,"height":1,"dpi":1}]'), [{ width: 10, height: 20, dpi: 72, transparent: false }]);
assert.deepEqual(R.parseHistory("pas du json"), []);
let long = [];
for (let i = 1; i <= 30; i++) long = R.pushHistory(long, { width: i, height: i, dpi: 72, transparent: false });
assert.equal(long.length, R.HISTORY_LIMIT);
// Recherche de la page Formats.
const insta = F.FORMAT_CATEGORIES.find((c) => c.id === "instagram");
assert.equal(insta.formats.filter((f) => F.matchesFormat(f, insta, "story")).length, 1);
assert.equal(insta.formats.filter((f) => F.matchesFormat(f, insta, "1080 x 1350")).length, 2);
assert.equal(insta.formats.filter((f) => F.matchesFormat(f, insta, "2160×2700")).length, 2);
// Catalogue : identifiants uniques, tout à 72 ppp sauf l'impression.
const ids = F.ALL_FORMATS.map((f) => f.id);
assert.equal(new Set(ids).size, ids.length);
assert.ok(F.ALL_FORMATS.every((f) => f.dpi === (F.FORMAT_CATEGORIES.find((c) => c.formats.includes(f)).id === "impression" || F.FORMAT_CATEGORIES.find((c) => c.formats.includes(f)).id === "cartes" ? 300 : 72)));
console.log("formats rapides : ok");
