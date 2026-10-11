// Vérifie la résolution dynamique (lib/resolution.ts), sans navigateur : npm run test:resolution
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/resolution.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-resolution.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { DynamicResolution } = await import(pathToFileURL(out).href);

// Un geste dont les images durent `gapMs`, pendant `frames` images ; retourne les facteurs successifs.
const run = (dyn, start, gapMs, frames) => {
  const seen = [];
  for (let i = 0; i < frames; i++) if (dyn.gestureFrame(start + i * gapMs)) seen.push(dyn.factor);
  return seen;
};

// Machine qui suit (60 Hz, 16,7 ms par image) : la résolution ne bouge pas.
let dyn = new DynamicResolution();
assert.deepEqual(run(dyn, 1000, 1000 / 60, 60), []);
assert.equal(dyn.factor, 1);
assert.equal(dyn.endGesture(), false);

// Écran à 60 Hz, images à 20 ms (50 i/s) : après trois images lentes, un cran plus bas ; toujours trop lent, encore un.
dyn = new DynamicResolution();
run(dyn, 0, 1000 / 60, 10); // mesure du rafraîchissement
let seen = run(dyn, 1000, 20, 10);
assert.deepEqual(seen, [0.75, 0.5]);
// Fin du geste : image nette, et le prochain geste repart directement à 0,5.
assert.equal(dyn.endGesture(), true);
assert.equal(dyn.factor, 1);
assert.equal(dyn.gestureFrame(5000), true);
assert.equal(dyn.factor, 0.5);
// À 0,5 les images repassent sous le budget : remontée après 24 images rapides, puis encore 24.
seen = run(dyn, 5000 + 20, 1000 / 60 - 0.5, 60);
assert.deepEqual(seen, [0.75, 1]);

// Désactivée : rien ne bouge, même à 40 ms par image.
dyn = new DynamicResolution(false);
assert.deepEqual(run(dyn, 0, 40, 30), []);
assert.equal(dyn.factor, 1);
// Réactivée en cours de route.
dyn.setEnabled(true);
run(dyn, 10000, 1000 / 60, 5);
assert.deepEqual(run(dyn, 11000, 40, 4), [0.75]);
dyn.setEnabled(false);
assert.equal(dyn.factor, 1, "désactiver remet la résolution de l'écran");

// Écran 240 Hz : l'intervalle estimé descend à 4,2 ms, et 8 ms par image est alors « lent ».
dyn = new DynamicResolution();
run(dyn, 0, 1000 / 240, 20);
assert.deepEqual(run(dyn, 100, 8, 4), [0.75]);

console.log("OK — résolution dynamique : descente par paliers, mémoire du geste précédent, remontée, désactivation.");
