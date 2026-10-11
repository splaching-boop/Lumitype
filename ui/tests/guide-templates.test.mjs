// Vérifie la géométrie des modèles de repères, sans navigateur : npm run test:guides
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/guide-templates.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-guide-templates.mjs");
fs.writeFileSync(out, compiled.outputText);
const t = await import(pathToFileURL(out).href);

const near = (a, b, tol = 1e-3) => assert.ok(Math.abs(a - b) < tol, `${a} ≈ ${b}`);

// Tous les modèles restent dans le plan de travail, sans repère sur un bord ni doublon.
for (const template of t.TEMPLATES) {
  const layout = template.build("bottomLeft");
  for (const axis of ["vertical", "horizontal"]) {
    const list = layout[axis];
    assert.ok(Array.isArray(list), `${template.id} : ${axis} manquant`);
    for (const value of list) assert.ok(value > 0.001 && value < 0.999, `${template.id} : repère hors plan (${value})`);
    assert.equal(new Set(list.map((v) => v.toFixed(5))).size, list.length, `${template.id} : repère en double`);
  }
  assert.ok(template.name && template.note && template.group, `${template.id} : libellé manquant`);
}

// La règle des tiers tombe au tiers, quel que soit le format — les proportions ne dépendent de rien d'autre.
const thirds = t.templateById("thirds").build("bottomLeft");
assert.deepEqual(thirds.vertical.map((v) => v.toFixed(4)), ["0.3333", "0.6667"]);
assert.equal(thirds.hints.dots.length, 4);

// La grille d'or tombe sur les deux sections.
const grid = t.templateById("goldenGrid").build("bottomLeft");
assert.deepEqual(grid.vertical, [0.382, 0.618]);

const spiral = t.templateById("goldenSpiral");
assert.ok(spiral.turnable, "la spirale doit être pivotable");

const upright = spiral.build("bottomLeft");
// Six repères retenus, trois par axe : 0,618 — la coupe d'or — puis 0,708 et 0,764.
assert.equal(upright.vertical.length + upright.horizontal.length, 6);
for (const list of [upright.vertical, upright.horizontal]) {
  const sorted = [...list].sort((a, b) => a - b);
  near(sorted[0], 0.618);
  near(sorted[1], 0.708);
  near(sorted[2], 0.764);
}

// La spirale est continue : chaque arc repart exactement là où le précédent s'arrête.
const arcs = upright.hints.arcs;
assert.ok(arcs.length >= 6, "spirale trop courte");
for (let i = 1; i < arcs.length; i++) {
  near(arcs[i].sx, arcs[i - 1].ex, 1e-9);
  near(arcs[i].sy, arcs[i - 1].ey, 1e-9);
}
// Et elle se resserre : chaque quart est plus petit que le précédent.
for (let i = 1; i < arcs.length; i++) assert.ok(arcs[i].rx < arcs[i - 1].rx, `arc ${i} plus grand que le précédent`);

// Les quatre coins sont des miroirs exacts l'un de l'autre, et l'œil se déplace bien de coin en coin.
const eyes = Object.fromEntries(t.CORNERS.map((corner) => [corner.id, spiral.build(corner.id).hints.eye]));
assert.ok(eyes.bottomLeft[0] > 0.5 && eyes.bottomLeft[1] > 0.5, "bas gauche : l'œil se referme en bas à droite du plan");
near(eyes.bottomRight[0], 1 - eyes.bottomLeft[0]);
near(eyes.bottomRight[1], eyes.bottomLeft[1]);
near(eyes.topLeft[1], 1 - eyes.bottomLeft[1]);
near(eyes.topRight[0], 1 - eyes.bottomLeft[0]);
near(eyes.topRight[1], 1 - eyes.bottomLeft[1]);
// Un seul miroir inverse le sens de rotation, deux miroirs le rétablissent.
assert.equal(spiral.build("bottomLeft").hints.arcs[0].sweep, 1);
assert.equal(spiral.build("bottomRight").hints.arcs[0].sweep, 0);
assert.equal(spiral.build("topRight").hints.arcs[0].sweep, 1);
assert.equal(spiral.build("topLeft").hints.arcs[0].sweep, 0);

// Les modèles non pivotables ignorent le coin demandé.
for (const template of t.TEMPLATES.filter((item) => !item.turnable)) {
  assert.deepEqual(template.build("topRight"), template.build("bottomLeft"), `${template.id} ne devrait pas pivoter`);
}

// Les grilles donnent bien les traits intérieurs, pas les bords.
assert.equal(t.templateById("cols12").build("bottomLeft").vertical.length, 11);
assert.equal(t.templateById("grid12x8").build("bottomLeft").horizontal.length, 7);

assert.ok(t.GROUPS.length >= 4 && t.GROUPS[0] === "Composition");
assert.equal(t.templateById("inconnu"), undefined);

console.log(`modèles de repères : ${t.TEMPLATES.length} modèles vérifiés`);
