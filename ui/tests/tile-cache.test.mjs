// Vérifie la logique d'affichage cohérent du cache de tuiles, sans GPU : npm run test:tiles
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/viewport/tile-cache.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-tile-cache.mjs");
fs.writeFileSync(out, compiled.outputText);
const { TileCache } = await import(pathToFileURL(out).href);

// Moteur factice : répond à la demande, avec la révision courante et un contenu identifiable.
let revision = 1;
const pending = [];
const fetchTile = (level, x, y) =>
  new Promise((resolve) => {
    pending.push(() => {
      const buffer = new ArrayBuffer(9);
      const view = new DataView(buffer);
      view.setBigUint64(0, BigInt(revision), true);
      view.setUint8(8, revision); // le « contenu » de la tuile est sa révision
      resolve(buffer);
    });
  });
const flush = async (count = pending.length) => {
  pending.splice(0, count).forEach((answer) => answer());
  await new Promise((resolve) => setTimeout(resolve, 0));
};
const gl = { deleteTexture() {} };
const cache = new TileCache(gl, fetchTile, (payload) => ({ texture: { content: payload[0] }, uniform: 0 }), () => {}, 64);

let frame = 0;
/** Simule un rendu : parcourt deux tuiles visibles et retourne le contenu affiché de chacune. */
const render = () => {
  frame += 1;
  cache.beginFrame();
  const shown = [cache.resolve(0, 0, 0, 0, frame), cache.resolve(0, 1, 0, 0, frame)].map((t) => t?.texture?.content ?? null);
  const swapped = cache.promote();
  return { shown, swapped, settled: cache.settled };
};

// Premier affichage : rien, puis les deux tuiles à la révision 1.
assert.deepEqual(render().shown, [null, null]);
await flush();
assert.deepEqual(render(), { shown: [1, 1], swapped: false, settled: true });

// Le document change entièrement (déplacement d'un calque) : révision 2.
revision = 2;
cache.invalidateAll(2);
assert.equal(render().settled, false);
await flush(1); // UNE SEULE des deux tuiles est arrivée
const partial = render();
assert.deepEqual(partial.shown, [1, 1], "aucun mélange : l'ancien état reste affiché en entier");
assert.equal(partial.swapped, false);
await flush();
assert.equal(render().swapped, true, "les deux tuiles sont échangées ensemble");
assert.deepEqual(render(), { shown: [2, 2], swapped: false, settled: true });

// Une réponse calculée avant la modification signalée est ignorée, puis redemandée.
cache.invalidateAll(3);
render();
await flush(); // le moteur factice répond encore « révision 2 »
assert.deepEqual(render().shown, [2, 2]);
revision = 3;
await flush();
render();
assert.deepEqual(render().shown, [3, 3]);

// Trait de pinceau : la tuile touchée s'affiche dès son arrivée, sans attendre sa voisine.
revision = 4;
cache.invalidate([[0, 0]], 0, 4, true);
cache.invalidate([[1, 0]], 0, 4, false);
render();
await flush(1);
assert.deepEqual(render().shown, [4, 3], "la tuile du trait n'attend pas");
await flush();
render();
assert.deepEqual(render().shown, [4, 4]);
console.log("OK — cache de tuiles : échange groupé, réponses périmées, exception du pinceau.");
