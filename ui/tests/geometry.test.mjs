// Vérifie la géométrie de la transformation libre, sans navigateur : npm run test:geometry
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/geometry.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-geometry.mjs");
fs.writeFileSync(out, compiled.outputText);
const g = await import(pathToFileURL(out).href);

const rect = [{ x: 10, y: 20 }, { x: 110, y: 20 }, { x: 110, y: 70 }, { x: 10, y: 70 }];
const apply = (m, p) => {
  const w = m[6] * p.x + m[7] * p.y + m[8];
  return { x: (m[0] * p.x + m[1] * p.y + m[2]) / w, y: (m[3] * p.x + m[4] * p.y + m[5]) / w };
};
const close = (a, b) => assert.ok(Math.abs(a.x - b.x) < 1e-6 && Math.abs(a.y - b.y) < 1e-6, `${JSON.stringify(a)} != ${JSON.stringify(b)}`);

// Identité, translation, et un vrai quadrilatère en perspective : les quatre coins doivent tomber juste.
for (const quad of [
  rect,
  rect.map((p) => ({ x: p.x + 7, y: p.y - 3 })),
  [{ x: 0, y: 0 }, { x: 200, y: 30 }, { x: 180, y: 140 }, { x: -20, y: 90 }],
]) {
  const m = g.homography(rect, quad);
  assert.ok(m, "homographie attendue");
  rect.forEach((p, i) => close(apply(m, p), quad[i]));
}
assert.deepEqual(g.homography(rect, rect).map((v) => Math.round(v * 1e9) / 1e9 + 0), [1, 0, 0, 0, 1, 0, 0, 0, 1]); // « + 0 » : -0 devient 0
assert.equal(g.homography(rect, [rect[0], rect[0], rect[0], rect[0]]), null, "cadre écrasé en un point");

assert.ok(g.isUsableQuad(rect));
assert.ok(!g.isUsableQuad([rect[0], rect[2], rect[1], rect[3]]), "cadre croisé refusé");
assert.ok(g.pointInQuad({ x: 50, y: 40 }, rect) && !g.pointInQuad({ x: 5, y: 40 }, rect));
close(g.rotate({ x: 1, y: 0 }, { x: 0, y: 0 }, Math.PI / 2), { x: 0, y: 1 });
assert.deepEqual(g.decompose({ x: 3, y: 8 }, { x: 1, y: 0 }, { x: 0, y: 2 }), [3, 4]);

// Coin à proportions conservées : un seul facteur, celui de la projection de la souris sur la diagonale.
{
  const [anchor, handle] = [rect[0], rect[2]]; // point fixe en haut à gauche, coin bas droit tiré (100 x 50)
  const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} != ${b}`);
  near(g.diagonalScale(anchor, handle, handle), 1); // souris immobile
  near(g.diagonalScale(anchor, handle, { x: 210, y: 120 }), 2); // sur la diagonale, deux fois plus loin
  // Hors de la diagonale : le cadre suit la projection, sans sauter d'un axe à l'autre quand les écarts se croisent.
  near(g.diagonalScale(anchor, handle, { x: 210, y: 20 }), (200 * 100 + 0 * 50) / (100 * 100 + 50 * 50));
  near(g.diagonalScale(anchor, handle, { x: 10, y: 120 }), (0 * 100 + 100 * 50) / (100 * 100 + 50 * 50));
  near(g.diagonalScale(anchor, handle, { x: -90, y: -30 }), -1); // de l'autre côté du point fixe : retournement
  near(g.diagonalScale(anchor, anchor, { x: 50, y: 50 }), 1); // diagonale dégénérée : rien ne bouge
}

// Magnétisme : le plus proche gagne, rien au-delà de la tolérance.
assert.equal(g.snapOffset([98, 148, 198], [0, 150, 300], 8), 2);
assert.equal(g.snapOffset([40], [0, 150, 300], 8), 0);
console.log("OK — géométrie : homographie, cadre, échelle proportionnelle, magnétisme.");
