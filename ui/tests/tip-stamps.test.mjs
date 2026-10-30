// Empreintes d'un trait de pointe côté interface (ui/src/lib/tip-stamps.ts), comparées à celles du moteur :
// npm run test:tip-stamps. La fixture est produite par le moteur (crates/engine/src/paint.rs,
// `tip_stamps_match_the_interface_fixture`, avec LUMI_WRITE_FIXTURE=1) : mêmes réglages, mêmes points, mêmes empreintes.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/tip-stamps.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-tip-stamps.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { TipStamper, packStamps, packDuals } = await import(pathToFileURL(out).href);

const fixture = JSON.parse(fs.readFileSync(path.join(here, "fixtures/tip-stamps.json"), "utf8"));
assert.ok(fixture.cases.length >= 4, "cas de la fixture");

/** Écart permis : arrondis des fonctions trigonométriques (le moteur et le navigateur n'ont pas les mêmes). */
const close = (a, b, what) => {
  assert.equal(a.length, b.length, `${what} : nombre`);
  a.forEach((row, i) => row.forEach((v, k) => assert.ok(Math.abs(v - b[i][k]) <= 1e-3 * Math.max(1, Math.abs(b[i][k])), `${what} n° ${i}, valeur ${k} : ${v} au lieu de ${b[i][k]}`)));
};

let total = 0;
for (const { settings, points, stamps, duals } of fixture.cases) {
  const stamper = new TipStamper(settings);
  // Les points arrivent par lots, comme ceux du stylet.
  const got = { stamps: [], duals: [] };
  for (let i = 0; i < points.length; i += 5) {
    const part = stamper.add(points.slice(i, i + 5));
    got.stamps.push(...part.stamps);
    got.duals.push(...part.duals);
  }
  const last = stamper.finish();
  got.stamps.push(...last.stamps);
  got.duals.push(...last.duals);
  close(packStamps(got.stamps), stamps, `${settings.name} — empreinte`);
  close(packDuals(got.duals), duals, `${settings.name} — empreinte du pinceau double`);
  total += got.stamps.length + got.duals.length;
}

// Même geste, mêmes empreintes : rien n'est tiré au sort ailleurs que dans le générateur du trait.
const [first] = fixture.cases;
const twice = () => {
  const s = new TipStamper(first.settings);
  return [...s.add(first.points).stamps, ...s.finish().stamps];
};
assert.deepEqual(twice(), twice(), "déterministe");

console.log(`OK — empreintes de pointe identiques au moteur : ${fixture.cases.length} réglages, ${total} empreintes.`);
