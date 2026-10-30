// Vérifie la file des fenêtres « Développement RAW » (lib/raw-queue.ts), sans navigateur : node ui/tests/raw-queue.test.mjs
// Un RAW déposé pendant qu'une fenêtre est ouverte attend son tour (rien n'est annulé), et « n sur N » en tient compte.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/raw-queue.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-raw-queue.mjs");
fs.writeFileSync(out, compiled.outputText);
const { rawBatch, endRawBatch, startRaw, rawInTurn, onSeriesCount } = await import(pathToFileURL(out).href);

const counts = [];
onSeriesCount((series) => counts.push(series.count));
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

// Même déroulé que lib/actions.ts : chaque appelant boucle sur ses fichiers ; chaque RAW prend son tour, et sa
// « fenêtre » reste ouverte jusqu'à ce qu'on y réponde (`answer`).
const log = [];
const windows = [];
const develop = (name, part) =>
  rawInTurn(async () => {
    const position = startRaw(part);
    log.push(`${name} ${position.index}/${position.count}`);
    await new Promise((resolve) => windows.push({ name, resolve }));
    log.push(`${name} fermé`);
  });
async function caller(names, skipped = 0) {
  const part = rawBatch(names.length + skipped);
  try {
    for (const name of names) await develop(name, part);
  } finally {
    endRawBatch(part);
  }
}

// Ouverture de deux RAW ; pendant que la fenêtre du premier est ouverte, un troisième est déposé.
const first = caller(["a", "b"]);
await tick();
assert.deepEqual(log, ["a 1/2"]);
const dropped = caller(["c"], 1); // un fichier du dépôt sera refusé (format) : il sort du compte à la fin
await tick();
assert.equal(windows.length, 1, "une seule fenêtre à la fois");
assert.equal(counts.at(-1), 4, "« n sur N » de la fenêtre ouverte mis à jour");
windows.shift().resolve();
await tick();
assert.deepEqual(log, ["a 1/2", "a fermé", "c 2/4"], "le RAW déposé passe après le courant, sans rien annuler");
windows.shift().resolve();
await tick();
assert.equal(counts.at(-1), 3, "fichier refusé retiré du compte");
assert.deepEqual(log.slice(3), ["c fermé", "b 3/3"]);
windows.shift().resolve();
await Promise.all([first, dropped]);
assert.equal(windows.length, 0);

// Série terminée : la suivante repart de 1.
const next = caller(["d"]);
await tick();
assert.equal(log.at(-1), "d 1/1");
windows.shift().resolve();
await next;

// Une erreur pendant un tour ne bloque pas la file.
await rawInTurn(async () => {
  throw new Error("RAW illisible");
}).catch(() => undefined);
let ran = false;
await rawInTurn(async () => void (ran = true));
assert.ok(ran);

console.log("OK — file des développements RAW : une fenêtre à la fois, ordre des demandes, compte de la série.");
