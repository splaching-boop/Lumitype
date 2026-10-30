// Couleur dynamique (ui/src/lib/color-jitter.ts) : chaque trait, une nuance très proche de la couleur choisie.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/color-jitter.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-color-jitter.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { jitterColor } = await import(pathToFileURL(out).href);

// Hasard au milieu : la couleur revient telle quelle (conversion exacte).
for (const c of [[63, 111, 154], [214, 72, 60], [0, 0, 0], [255, 255, 255], [128, 128, 128]]) assert.deepEqual(jitterColor(c, () => 0.5), c, `${c}`);
// Extrêmes : jamais loin (≤ 26 niveaux par canal sur ces couleurs), et bien différente.
let seed = 7;
const random = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
for (const c of [[63, 111, 154], [214, 72, 60], [96, 150, 70]]) {
  let differs = 0;
  for (let i = 0; i < 200; i++) {
    const j = jitterColor(c, random);
    const gap = Math.max(...j.map((v, k) => Math.abs(v - c[k])));
    assert.ok(gap <= 26, `${c} → ${j} : écart ${gap}`);
    if (gap > 0) differs += 1;
  }
  assert.ok(differs > 150, "presque toujours une autre nuance");
}
console.log("OK — couleur dynamique : nuances très proches, jamais tout à fait la même.");
