// Vérifie le choix entre composition GPU et moteur processeur, sans navigateur : npm run test:gpu
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/gpu-plan.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-gpu-plan.mjs");
fs.writeFileSync(out, compiled.outputText);
const { gpuPlan, tilesRect, intersect } = await import(pathToFileURL(out).href);

const layer = (id, parent, index, extra = {}) => ({
  id, parent, index, depth: 0, offset: [0, 0], name: `L${id}`, isGroup: false, expanded: true, childCount: 0,
  visible: true, opacity: 1, fill: 1, blend: "normal", locks: {}, clipped: false, hasMask: false, maskEnabled: false,
  text: null, adjustment: null, shape: null, effects: [], ...extra,
});
const doc = (layers) => ({ layers });

// Pile simple : du bas vers le haut, masqués ignorés, opacité × fond.
let plan = gpuPlan(doc([
  layer(3, null, 2, { opacity: 0.5, fill: 0.8, offset: [10, -4] }),
  layer(1, null, 0),
  layer(2, null, 1, { visible: false }),
]));
assert.equal(plan.ok, true);
assert.deepEqual(plan.layers.map((l) => l.id), [1, 3], "du bas vers le haut, sans le calque masqué");
assert.ok(Math.abs(plan.layers[1].alpha - 0.4) < 1e-9);
assert.deepEqual(plan.layers[1].offset, [10, -4]);

// Groupe passe-à-travers : son contenu est intercalé à sa place ; groupe masqué : tout son contenu disparaît.
plan = gpuPlan(doc([
  layer(1, null, 0),
  layer(10, null, 1, { isGroup: true, blend: "pass_through" }),
  layer(11, 10, 0),
  layer(12, 10, 1),
  layer(2, null, 2),
  layer(20, null, 3, { isGroup: true, blend: "pass_through", visible: false }),
  layer(21, 20, 0),
]));
assert.deepEqual(plan.layers.map((l) => l.id), [1, 11, 12, 2]);

// Effets de calque : ceux fondus à part se fondent dessous, puis le calque, puis ceux du dessus. Les calques écrêtés
// sur une base à effets s'arrêtent à ses pixels seuls ; sur un groupe à effets (effets ignorés), à rien.
const fx = [{ type: "stroke", enabled: true }];
plan = gpuPlan(doc([
  layer(1, null, 0, { effects: fx }),
  layer(2, null, 1, { clipped: true }),
  layer(10, null, 2, { isGroup: true, blend: "normal", effects: fx }),
  layer(11, 10, 0),
  layer(3, null, 3, { clipped: true }),
]));
assert.equal(plan.ok, true);
assert.equal(plan.simple, false);
assert.deepEqual(plan.effects, [1]);
assert.deepEqual(plan.raws, [1]);
assert.deepEqual(plan.program.ops, [
  { op: "load", target: 1, layer: 1 },
  { op: "clip", target: 1, source: { layer: 2 }, mode: "normal", layer: 2, shape: 1 },
  { op: "effects", target: 0, layer: 1, side: "under" },
  { op: "blend", target: 0, source: { buffer: 1 }, mode: "normal", layer: 1 },
  { op: "effects", target: 0, layer: 1, side: "over" },
  { op: "clear", target: 1 },
  { op: "blend", target: 1, source: { layer: 11 }, mode: "normal", layer: 11 },
  { op: "clip", target: 1, source: { layer: 3 }, mode: "normal", layer: 3, shape: "empty" },
  { op: "blend", target: 0, source: { buffer: 1 }, mode: "normal", layer: 10 },
]);
// Un effet désactivé ne compte pas.
plan = gpuPlan(doc([layer(1, null, 0, { effects: [{ type: "stroke", enabled: false }] })]));
assert.equal(plan.simple, true);
assert.deepEqual(plan.effects, []);

// Pile simple : superposition directe, sans tampon.
plan = gpuPlan(doc([layer(1, null, 0), layer(2, null, 1, { opacity: 0.5 })]));
assert.equal(plan.simple, true);
assert.deepEqual(plan.program, { ops: [
  { op: "blend", target: 0, source: { layer: 1 }, mode: "normal", layer: 1 },
  { op: "blend", target: 0, source: { layer: 2 }, mode: "normal", layer: 2 },
], buffers: 1 });

// Mode de fusion et masque : le programme n'est plus « simple », le masque est à charger.
plan = gpuPlan(doc([layer(1, null, 0), layer(2, null, 1, { blend: "multiply", hasMask: true, maskEnabled: true })]));
assert.equal(plan.ok, true);
assert.equal(plan.simple, false);
assert.deepEqual(plan.masks, [2]);
assert.deepEqual(plan.program.ops[1], { op: "blend", target: 0, source: { layer: 2 }, mode: "multiply", layer: 2 });

// Écrêtage (comme le moteur) : la base est chargée dans un tampon, les calques écrêtés s'y fondent, puis la base
// (avec son mode) rejoint l'image. Un réglage écrêté ne corrige que sa base.
plan = gpuPlan(doc([
  layer(1, null, 0),
  layer(2, null, 1, { blend: "screen" }),
  layer(3, null, 2, { clipped: true, blend: "overlay" }),
  layer(4, null, 3, { clipped: true, adjustment: { type: "invert" } }),
  layer(5, null, 4, { clipped: true, visible: false }),
]));
assert.deepEqual(plan.program, { ops: [
  { op: "blend", target: 0, source: { layer: 1 }, mode: "normal", layer: 1 },
  { op: "load", target: 1, layer: 2 },
  { op: "clip", target: 1, source: { layer: 3 }, mode: "overlay", layer: 3 },
  { op: "adjust", target: 1, mode: "normal", layer: 4 },
  { op: "blend", target: 0, source: { buffer: 1 }, mode: "screen", layer: 2 },
], buffers: 2 });
assert.deepEqual(plan.adjustments, [{ id: 4, adjustment: { type: "invert" } }]);
assert.deepEqual(plan.layers.map((l) => l.id), [1, 2, 3], "le calque écrêté masqué n'est pas chargé");

// Groupe isolé : composé à part dans un tampon, puis fondu avec son mode et son opacité.
plan = gpuPlan(doc([
  layer(10, null, 0, { isGroup: true, blend: "normal", opacity: 0.5 }),
  layer(11, 10, 0),
  layer(12, 10, 1, { adjustment: { type: "invert" } }),
]));
assert.deepEqual(plan.program, { ops: [
  { op: "clear", target: 1 },
  { op: "blend", target: 1, source: { layer: 11 }, mode: "normal", layer: 11 },
  { op: "adjust", target: 1, mode: "normal", layer: 12 },
  { op: "blend", target: 0, source: { buffer: 1 }, mode: "normal", layer: 10 },
], buffers: 2 });

// Groupe en Transfert : à 100 % sans masque, ses calques rejoignent directement l'image ; sinon, interpolation
// entre l'image d'avant et celle d'après (aussi pendant l'aperçu du curseur d'opacité de ce groupe).
const passing = (extra) => doc([layer(1, null, 0), layer(10, null, 1, { isGroup: true, blend: "pass_through", ...extra }), layer(11, 10, 0, { blend: "multiply" })]);
assert.deepEqual(gpuPlan(passing({})).program.ops.map((o) => o.op), ["blend", "blend"]);
for (const plan of [gpuPlan(passing({ opacity: 0.4 })), gpuPlan(passing({ hasMask: true, maskEnabled: true })), gpuPlan(passing({}), { live: 10 })]) {
  assert.deepEqual(plan.program.ops, [
    { op: "blend", target: 0, source: { layer: 1 }, mode: "normal", layer: 1 },
    { op: "copy", target: 1, source: 0 },
    { op: "blend", target: 0, source: { layer: 11 }, mode: "multiply", layer: 11 },
    { op: "mix", target: 0, before: 1, layer: 10 },
  ]);
}

// Géométrie des mises à jour partielles.
assert.deepEqual(tilesRect([[1, 2], [3, 2], [2, 4]], 256), [256, 512, 768, 768]);
assert.equal(tilesRect([], 256), null);
assert.deepEqual(intersect([0, 0, 100, 100], [50, 60, 100, 100]), [50, 60, 50, 40]);
assert.equal(intersect([0, 0, 10, 10], [20, 20, 5, 5]), null);

console.log("OK — composition GPU : choix du chemin, ordre des calques, mises à jour partielles.");
