// Géométrie de la plume, sans navigateur : npm run test:path
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/lib/path.ts"), "utf8");
const out = path.join(here, "../../node_modules/.cache-path.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const g = await import(pathToFileURL(out).href);

const arc = {
  closed: false,
  anchors: [
    { point: [0, 0], handleIn: [0, 0], handleOut: [0, -66] },
    { point: [100, 0], handleIn: [100, -66], handleOut: [100, 0] },
  ],
};
const controls = [arc.anchors[0].point, arc.anchors[0].handleOut, arc.anchors[1].handleIn, arc.anchors[1].point];
const close = (a, b, eps = 1e-6) => assert.ok(g.distance(a, b) < eps, `${a} != ${b}`);

close(g.cubicAt(controls, 0), [0, 0]);
close(g.cubicAt(controls, 1), [100, 0]);
close(g.cubicAt(controls, 0.5), [50, -49.5]);

// Insérer un point ne doit pas déformer la courbe : les deux moitiés repassent par les mêmes points.
const split = g.insertAnchor(arc, 0, 0.4);
assert.equal(split.anchors.length, 3);
close(split.anchors[1].point, g.cubicAt(controls, 0.4));
const left = [split.anchors[0].point, split.anchors[0].handleOut, split.anchors[1].handleIn, split.anchors[1].point];
const right = [split.anchors[1].point, split.anchors[1].handleOut, split.anchors[2].handleIn, split.anchors[2].point];
close(g.cubicAt(left, 0.5), g.cubicAt(controls, 0.2));
close(g.cubicAt(right, 0.5), g.cubicAt(controls, 0.7));

// Le point le plus proche d'un point situé sur la courbe est ce point.
const nearest = g.nearestOnPath(arc, g.cubicAt(controls, 0.25));
assert.ok(nearest.segment === 0 && Math.abs(nearest.t - 0.25) < 0.02 && nearest.distance < 1);

// Angle <-> lisse : un angle gagne deux poignées alignées, un point lisse les perd.
const triangle = { closed: true, anchors: [[0, 0], [100, 0], [50, 80]].map(g.corner) };
const smooth = g.toggleSmooth(triangle, 1);
assert.ok(!g.isCorner(smooth.anchors[1]));
const [hin, pt, hout] = [smooth.anchors[1].handleIn, smooth.anchors[1].point, smooth.anchors[1].handleOut];
assert.ok(Math.abs((hin[0] - pt[0]) * (hout[1] - pt[1]) - (hin[1] - pt[1]) * (hout[0] - pt[0])) < 1e-6, "poignées alignées");
assert.ok(g.isCorner(g.toggleSmooth(smooth, 1).anchors[1]));

assert.equal(g.segmentCount(triangle), 3);
assert.equal(g.segmentCount({ ...triangle, closed: false }), 2);
assert.ok(g.toSvg(triangle, (p) => p).endsWith("Z") && g.toSvg(arc, (p) => p).startsWith("M0 0C0 -66"));
console.log("OK — plume : courbes, insertion sans déformation, point le plus proche, conversion de points.");
