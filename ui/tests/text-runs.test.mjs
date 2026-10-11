// Portions stylées du texte, sans navigateur : npm run test:runs
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/text-runs.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-text-runs.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const r = await import(pathToFileURL(out).href);

const red = [255, 0, 0];
const base = { content: "Hello world", fontFamily: "Roboto", weight: 400, italic: false, color: [0, 0, 0], runs: [] };
const run = (start, end, extra = { color: red }) => ({ start, end, ...extra });

// Recalage après une frappe : « world » (6..11) est stylé.
const styled = [run(6, 11)];
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello, world"), [run(7, 12)], "insertion avant : la portion est repoussée");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello world!"), [run(6, 12)], "frappe à la fin : la portion se prolonge");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello wOrld"), [run(6, 11)], "remplacement à l'intérieur");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello Xworld"), [run(7, 12)], "frappe juste avant : hors de la portion");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello "), [], "portion entièrement supprimée");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hellorld"), [run(5, 8)], "suppression à cheval sur le début");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "X"), [], "tout remplacé");

// Appliquer un style à une plage, puis un autre qui la chevauche : les propriétés se cumulent.
let text = { ...base, runs: r.applyStyle(base, 6, 11, { color: red }) };
assert.deepEqual(text.runs, [run(6, 11)]);
text = { ...text, runs: r.applyStyle(text, 8, 11, { fontFamily: "Inter" }) };
assert.deepEqual(text.runs, [run(6, 8), run(8, 11, { fontFamily: "Inter", color: red })], "la couleur est gardée là où la police change");
// Revenir au style du calque supprime la portion.
assert.deepEqual(r.applyStyle({ ...base, runs: [run(6, 11)] }, 6, 11, { color: [0, 0, 0] }), []);

// Style d'une plage : commun, ou null quand il varie (affiché « ? »).
assert.deepEqual(r.styleOver(text, 0, 5), { fontFamily: "Roboto", weight: 400, italic: false, color: [0, 0, 0] });
assert.deepEqual(r.styleOver(text, 0, 11), { fontFamily: null, weight: 400, italic: false, color: null }, "tout sélectionné : police et couleur mixtes");
assert.deepEqual(r.styleOver(text, 9, 9).fontFamily, "Inter", "curseur seul : style de la lettre qui précède");

// Réglage pour tout le calque : la propriété est retirée des portions, les autres restent.
const recolored = r.applyToLayer(text, { color: [0, 0, 255] });
assert.deepEqual(recolored.runs, [run(8, 11, { fontFamily: "Inter" })]);
assert.deepEqual(recolored.color, [0, 0, 255]);
console.log("OK — portions de texte : recalage à la frappe, cumul des styles, valeurs mixtes.");
