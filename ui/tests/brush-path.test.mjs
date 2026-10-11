// Empreintes du pinceau côté interface, sans navigateur : npm run test:brush
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/brush-path.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-brush-path.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { DabPath } = await import(pathToFileURL(out).href);

// Trait droit : les gélules s'enchaînent bout à bout, du premier au dernier point, sans trou.
const line = new DabPath(20, false);
const dabs = [...line.add([0, 0, 1]), ...line.add([100, 0, 1]), ...line.finish()];
assert.deepEqual(dabs[0], { ax: 0, ay: 0, ar: 10, bx: 0, by: 0, br: 10 }, "premier point : un cercle");
for (let i = 2; i < dabs.length; i++) assert.ok(Math.abs(dabs[i].ax - dabs[i - 1].bx) < 1e-9 && dabs[i].ay === 0, "gélules jointives");
assert.ok(Math.abs(dabs.at(-1).bx - 100) < 1e-9, "le trait atteint le dernier point");
assert.ok(dabs.length >= 8, "la courbe est découpée en plusieurs gélules");

// Angle droit : la courbe passe par (137,5 ; 62,5) comme dans le moteur, jamais par le coin.
const corner = new DabPath(4, false);
const bend = [...corner.add([50, 50, 1]), ...corner.add([150, 50, 1]), ...corner.add([150, 150, 1]), ...corner.finish()];
assert.ok(bend.some((d) => Math.hypot(d.bx - 137.5, d.by - 62.5) < 2.5), "même courbe que le moteur");
assert.ok(bend.every((d) => Math.hypot(d.bx - 150, d.by - 50) > 10), "coin arrondi");

// Un gros pinceau trace moins de gélules, plus longues.
const count = (size) => {
  const path = new DabPath(size, false);
  return [...path.add([0, 0, 1]), ...path.add([400, 0, 1]), ...path.add([800, 300, 1]), ...path.finish()].length;
};
assert.ok(count(1000) < count(10) / 3, `${count(1000)} gélules contre ${count(10)}`);

// Pression : rayon proportionnel, jamais sous un demi-pixel ; bruit de capteur ignoré.
const pen = new DabPath(40, true);
assert.equal(pen.add([0, 0, 0.25])[0].ar, 5);
assert.equal(new DabPath(40, true).add([0, 0, 0])[0].ar, 0.5);
assert.deepEqual(pen.add([0.3, 0.2, 1]), []);
// Pinceau flou : empreintes rondes espacées de 25 % du diamètre, la distance étant reportée d'un segment à l'autre.
const soft = new DabPath(40, false, 0.25); // espacement de test : 25 % (le pinceau flou utilise 2,5 %)
const stamps = [...soft.add([0, 0, 1]), ...soft.add([50, 0, 1]), ...soft.add([100, 0, 1]), ...soft.finish()];
assert.ok(stamps.every((d) => d.ax === d.bx && d.ay === d.by && d.ar === 20), "des cercles, pas des gélules");
const gaps = stamps.slice(1).map((d, i) => d.ax - stamps[i].ax);
assert.ok(gaps.every((g) => Math.abs(g - 10) < 1e-6), `espacement régulier de 10 px : ${gaps}`);
assert.equal(stamps.length, 11, "de 0 à 100 px : 11 empreintes");
console.log("OK — tracé du pinceau : gélules jointives, lissage identique au moteur, pression.");
