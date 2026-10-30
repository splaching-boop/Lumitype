// Portions stylées du texte, sans navigateur : npm run test:runs
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/text-runs.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-text-runs.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const r = await import(pathToFileURL(out).href);

const red = [255, 0, 0];
const base = { content: "Hello world", fontFamily: "Roboto", weight: 400, italic: false, color: [0, 0, 0], runs: [] };
const run = (start, end, extra = { color: red }) => ({ start, end, ...extra });

// Recalage après une frappe : « world » (6..11) est stylé.
const styled = [run(6, 11)];
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello, world"), [run(7, 12)], "insertion avant : la portion est repoussée");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello world!"), [run(6, 12)], "frappe à la fin : la portion se prolonge");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello wOrld"), [run(6, 11)], "remplacement à l'intérieur");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello Xworld"), [run(7, 12)], "frappe juste avant : hors de la portion");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hello "), [], "portion entièrement supprimée");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "Hellorld"), [run(5, 8)], "suppression à cheval sur le début");
assert.deepEqual(r.adjustRuns(styled, "Hello world", "X"), [], "tout remplacé");

// Appliquer un style à une plage, puis un autre qui la chevauche : les propriétés se cumulent.
let text = { ...base, runs: r.applyStyle(base, 6, 11, { color: red }) };
assert.deepEqual(text.runs, [run(6, 11)]);
text = { ...text, runs: r.applyStyle(text, 8, 11, { fontFamily: "Inter" }) };
assert.deepEqual(text.runs, [run(6, 8), run(8, 11, { fontFamily: "Inter", color: red })], "la couleur est gardée là où la police change");
// Revenir au style du calque supprime la portion.
assert.deepEqual(r.applyStyle({ ...base, runs: [run(6, 11)] }, 6, 11, { color: [0, 0, 0] }), []);

// Style d'une plage : commun, ou null quand il varie (affiché « ? »).
// Police, graisse, italique, couleur (les autres réglages — corps, approche, casse, style de caractère — ont leur test).
const four = ({ fontFamily, weight, italic, color }) => ({ fontFamily, weight, italic, color });
assert.deepEqual(four(r.styleOver(text, 0, 5)), { fontFamily: "Roboto", weight: 400, italic: false, color: [0, 0, 0] });
assert.deepEqual(four(r.styleOver(text, 0, 11)), { fontFamily: null, weight: 400, italic: false, color: null }, "tout sélectionné : police et couleur mixtes");
assert.deepEqual(r.styleOver(text, 9, 9).fontFamily, "Inter", "curseur seul : style de la lettre qui précède");

// Réglage pour tout le calque : la propriété est retirée des portions, les autres restent.
const recolored = r.applyToLayer(text, { color: [0, 0, 255] });
assert.deepEqual(recolored.runs, [run(8, 11, { fontFamily: "Inter" })]);
assert.deepEqual(recolored.color, [0, 0, 255]);
console.log("OK — portions de texte : recalage à la frappe, cumul des styles, valeurs mixtes.");

// Cadre d'un texte de paragraphe (texte en bloc) : poignées, glissé de l'outil Texte, repère du texte.
const frameSource = fs.readFileSync(path.join(here, "../src/lib/text-frame.ts"), "utf8");
const frameOut = path.join(here, "../../node_modules/.cache-text-frame.mjs");
fs.writeFileSync(frameOut, ts.transpileModule(frameSource, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const f = await import(pathToFileURL(frameOut).href);
const handle = (hx, hy) => ({ hx, hy });

// Coin bas droit : la taille suit la souris, l'origine ne bouge pas.
assert.deepEqual(f.resizeFrame(handle(1, 1), 200, 100, { x: 260.4, y: 149.6 }, false), { shift: [0, 0], width: 260, height: 150 });
// Poignée gauche : le bord gauche suit, l'origine recule d'autant ; le bord droit reste en place.
assert.deepEqual(f.resizeFrame(handle(0, 0.5), 200, 100, { x: -30, y: 999 }, false), { shift: [-30, 0], width: 230, height: 100 }, "un bord ne change qu'une dimension");
// Un côté ne passe jamais au-delà de l'autre.
assert.deepEqual(f.resizeFrame(handle(0, 0), 200, 100, { x: 500, y: 500 }, false), { shift: [192, 92], width: 8, height: 8 });
// Maj : proportion gardée. Par un coin, la plus grande variation l'emporte ; par un bord, l'autre dimension suit, centrée.
assert.deepEqual(f.resizeFrame(handle(1, 1), 200, 100, { x: 300, y: 110 }, true), { shift: [0, 0], width: 300, height: 150 });
assert.deepEqual(f.resizeFrame(handle(0, 0), 200, 100, { x: -200, y: 0 }, true), { shift: [-200, -100], width: 400, height: 200 }, "coin haut gauche : l'opposé reste en place");
assert.deepEqual(f.resizeFrame(handle(1, 0.5), 200, 100, { x: 400, y: 50 }, true), { shift: [0, -50], width: 400, height: 200 });

// Repère du texte : un cadre doublé et tourné de 90° (Ctrl+T) ; la souris y est ramenée exactement.
const quad = [{ x: 100, y: 100 }, { x: 100, y: 500 }, { x: -100, y: 500 }, { x: -100, y: 100 }]; // 200 × 100, x -> bas, y -> gauche, ×2
const axes = f.frameAxes(quad, 200, 100);
const back = f.toFrame(axes, f.fromFrame(axes, { x: 37, y: 12 }));
assert.ok(Math.abs(back.x - 37) < 1e-9 && Math.abs(back.y - 12) < 1e-9, "aller-retour document <-> texte");
assert.deepEqual(f.toFrame(axes, { x: 100, y: 500 }), { x: 200, y: 0 }, "coin haut droit du cadre");
assert.deepEqual(f.frameQuad(axes, 0, 0, 200, 100), quad, "coins recalculés");
assert.equal(f.frameAxes([{ x: 0, y: 0 }, { x: 0, y: 0 }, { x: 0, y: 0 }, { x: 0, y: 0 }], 10, 10), null, "cadre aplati refusé");
// Curseurs : bord droit d'un cadre tourné de 90° = curseur vertical ; coin = diagonale à 45°, quelle que soit la forme.
assert.equal(f.handleCursor(axes, handle(1, 0.5)), "ns-resize");
const straight = f.frameAxes([{ x: 0, y: 0 }, { x: 1000, y: 0 }, { x: 1000, y: 10 }, { x: 0, y: 10 }], 1000, 10);
assert.equal(f.handleCursor(straight, handle(1, 1)), "nwse-resize", "coin d'un cadre très large : diagonale");
assert.equal(f.handleCursor(straight, handle(0.5, 0)), "ns-resize");

// Outil Texte : moins de 4 px d'écran = simple clic (texte de point) ; sinon le cadre tracé, dans n'importe quel sens.
assert.equal(f.draggedFrame({ x: 10, y: 10 }, { x: 11, y: 11 }, 3), null);
assert.deepEqual(f.draggedFrame({ x: 120.6, y: 80.2 }, { x: 20.2, y: 30.4 }, 120), { x: 20, y: 30, width: 101, height: 50 });
console.log("OK — cadre de paragraphe : poignées, proportion, repère du texte, glissé de l'outil Texte.");

// Style de caractère et corps d'une portion : appliqués, fusionnés, puis retirés avec le style du calque.
{
  const base = { content: "Titre et corps", fontFamily: "Inter", size: 20, lineHeight: 1.2, tracking: 0, align: "left", color: [0, 0, 0], weight: 400, italic: false, runs: [] };
  const runs = r.applyStyle(base, 0, 5, { size: 40, caps: "allCaps", style: 3 });
  assert.deepEqual(runs, [{ start: 0, end: 5, size: 40, caps: "allCaps", style: 3 }]);
  const over = r.styleOver({ ...base, runs }, 0, 5);
  assert.equal(over.style, 3);
  assert.equal(over.size, 40);
  assert.equal(r.styleOver({ ...base, runs }, 0, 10).style, null, "plage à cheval : styles mélangés");
  const whole = r.applyToLayer({ ...base, runs }, { style: 4, size: 30 });
  assert.deepEqual(whole.runs, [{ start: 0, end: 5, caps: "allCaps" }], "le calque reprend style et corps ; la casse de la portion reste");
  assert.equal(whole.style, 4);
}
console.log("text-runs (styles) : ok");

