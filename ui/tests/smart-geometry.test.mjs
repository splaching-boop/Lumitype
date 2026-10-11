// Vérifie les calculs du panneau Propriétés des objets dynamiques, sans navigateur : npm run test:smart
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const cache = path.join(here, "../../node_modules");
const compile = (file, out, rewrite = (s) => s) => {
  const source = fs.readFileSync(path.join(here, "../src/lib", file), "utf8");
  const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
  fs.writeFileSync(path.join(cache, out), rewrite(js));
};
compile("geometry.ts", ".cache-smart-geometry-base.mjs");
compile("smart-geometry.ts", ".cache-smart-geometry.mjs", (js) => js.replace(/from "\.\/geometry"/, 'from "./.cache-smart-geometry-base.mjs"'));
const g = await import(pathToFileURL(path.join(cache, ".cache-smart-geometry.mjs")).href);

const apply = (m, [x, y]) => {
  const w = m[6] * x + m[7] * y + m[8];
  return [(m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w];
};
const close = (a, b, eps = 1e-6) => assert.ok(Math.abs(a[0] - b[0]) < eps && Math.abs(a[1] - b[1]) < eps, `${JSON.stringify(a)} != ${JSON.stringify(b)}`);

/** Objet 1200 × 640 affiché à 50 %, coin haut gauche en (100, 50), tourné de `deg` degrés (sens horaire) autour de ce coin. */
function smart(deg = 0, scale = 0.5) {
  const r = (deg * Math.PI) / 180;
  const [c, s] = [Math.cos(r), Math.sin(r)];
  const at = (x, y) => [100 + (x * c - y * s) * scale, 50 + (x * s + y * c) * scale];
  return { width: 1200, height: 640, scaleX: scale, scaleY: scale, rotationDeg: deg, x: 100, y: 50, corners: [at(0, 0), at(1200, 0), at(1200, 640), at(0, 640)] };
}

// Taille affichée et pourcentage.
assert.deepEqual(g.displayedSize(smart()), { width: 600, height: 320 });
assert.equal(g.percentLabel(0.4333), "43,3 %");

// Redimensionner : le coin haut gauche reste, les autres suivent les axes du contenu (même tourné).
for (const deg of [0, -12, 30]) {
  const s = smart(deg);
  const m = g.resizeMatrix(s, 900, 480);
  assert.ok(m);
  const want = smart(deg, 0.75).corners;
  s.corners.forEach((p, i) => close(apply(m, p), want[i]));
}
assert.equal(g.resizeMatrix(smart(), 0, 100), null, "taille nulle refusée");

// Proportions liées par la chaîne.
assert.deepEqual(g.proportional(smart(), "width", 300), { width: 300, height: 160 });
assert.deepEqual(g.proportional(smart(), "height", 640), { width: 1200, height: 640 });

// Rotation absolue autour du centre : de 0 à 90°, le centre ne bouge pas et le coin haut gauche tourne.
{
  const s = smart();
  const m = g.rotateMatrix(s, 90);
  const center = [100 + 300, 50 + 160];
  close(apply(m, center), center);
  close(apply(m, s.corners[0]), [center[0] + 160, center[1] - 300]);
}

// Position : translation pure du coin haut gauche.
close(apply(g.moveMatrix(smart(), 180, 120), [100, 50]), [180, 120]);

console.log("smart-geometry : OK");
