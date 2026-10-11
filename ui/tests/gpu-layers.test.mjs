// Vérifie le repère des textures de la composition GPU (déplacement, transformation, retouche), sans navigateur :
// npm run test:gpu-layers
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const cache = path.join(here, "../../node_modules");
const compile = (file, name, rewrite = (s) => s) => {
  const source = fs.readFileSync(path.join(here, "../src", file), "utf8");
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
  const out = path.join(cache, name);
  fs.writeFileSync(out, rewrite(compiled.outputText));
  return out;
};
compile("lib/gpu-plan.ts", ".cache-gpu-plan-for-layers.mjs");
const out = compile("viewport/gpu-layers.ts", ".cache-gpu-layers.mjs", (s) => s.replace(/from "\.\.\/lib\/gpu-plan"/, 'from "./.cache-gpu-plan-for-layers.mjs"'));
const { GpuLayers, stackRuns, STACK_SIZE } = await import(pathToFileURL(out).href);

// Carte graphique factice : on retient les envois de pixels et les rectangles dessinés.
const uploads = [];
let rects = [];
let uniformName = "";
const gl = {
  MAX_TEXTURE_SIZE: 1, RENDERER: 2,
  getParameter: (p) => (p === 1 ? 16384 : "factice"),
  getExtension: () => null,
  createSampler: () => ({}), samplerParameteri() {}, bindSampler() {},
  createTexture: () => ({}), deleteTexture() {}, bindTexture() {}, texStorage2D() {}, pixelStorei() {}, generateMipmap() {},
  texSubImage2D: (_t, _l, x, y, w, h) => uploads.push([x, y, w, h]),
  useProgram() {}, uniform1f() {}, uniform2f() {}, uniform3f() {}, activeTexture() {}, drawArrays() {},
  uniform1fv() {}, scissor() {}, enable() {}, disable() {}, bindFramebuffer() {}, blendFunc() {},
  uniform4f() {},
  // Pile de calques : `u_count` calques placés (x, y, largeur, hauteur dans le document) dans `u_place`.
  uniform1i: (name, v) => name === "u_count" && (count = v),
  uniform4fv: (name, v) => name === "u_place" && rects.push(...Array.from({ length: count }, (_, i) => Array.from(v.slice(i * 4, i * 4 + 4)))),
};
let count = 0;
const link = () => ({ program: {}, uniform: (name) => name });

// Moteur factice : on répond à la main, dans l'ordre des demandes.
const pending = [];
const fetchLayer = (id, rect, kind, limit = null) =>
  new Promise((resolve) => {
    // `body` faux : l'en-tête seul, comme le moteur répond quand la texture dépasse `limit`.
    const reply = (x, y, w, h, body = true) => {
      const buffer = new ArrayBuffer(16 + (body ? w * h * 4 : 0));
      const view = new DataView(buffer);
      view.setInt32(0, x, true); view.setInt32(4, y, true); view.setUint32(8, w, true); view.setUint32(12, h, true);
      resolve(buffer);
    };
    pending.push({ id, rect, kind, limit, answer: (x, y, w, h) => reply(x, y, w, h), refuse: (w, h) => reply(0, 0, w, h, false) });
  });
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

const layer = (offset) => ({
  id: 1, parent: null, index: 0, depth: 0, offset, name: "Photo", isGroup: false, expanded: true, childCount: 0,
  visible: true, opacity: 1, fill: 1, blend: "normal", locks: {}, clipped: false, hasMask: false, maskEnabled: false,
  text: null, adjustment: null, shape: null, effects: [],
});
const doc = (offset) => ({ id: 7, width: 2000, height: 1500, tileSize: 256, layers: [layer(offset)] });
const drawnAt = () => {
  rects = [];
  gpu.draw({ scale: 1, originX: 0, originY: 0, viewW: 2000, viewH: 1500 }, { shift: () => [0, 0], opacity: (id, opacity) => opacity });
  return rects.map((r) => [r[0], r[1]]);
};

const gpu = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});

// Chargement : pixels (0, 0) à (1618, 1080) dans le repère du calque, calque posé en (11, -505).
gpu.setDocument(doc([11, -505]));
assert.equal(pending.length, 1);
pending.shift().answer(0, 0, 1618, 1080);
await settle();
assert.deepEqual(drawnAt(), [[11, -505]]);

// Simple déplacement : aucun pixel relu, la texture suit la position.
gpu.setDocument(doc([61, -455]));
gpu.invalidate({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [] });
await settle();
assert.equal(pending.length, 0, "un déplacement ne relit rien");
assert.deepEqual(drawnAt(), [[61, -455]]);

// Retouche sans déplacement : seule la zone repeinte est relue, dans le repère du calque.
gpu.invalidate({ all: false, tiles: [[1, 1]], layers: [1], selection: false, repainted: [1] });
assert.equal(pending.length, 1);
assert.deepEqual(pending[0].rect, [256 - 61, 256 + 455, 256, 256], "zone repeinte ramenée dans le repère du calque");
pending.shift().answer(195, 711, 256, 256);
await settle();
assert.deepEqual(uploads.at(-1), [195, 711, 256, 256]);

// Transformation : le calque est repeint ET son repère change (position ramenée en 0, 0). Une mise à jour partielle
// écrirait les nouveaux pixels au mauvais endroit : c'est un rechargement complet.
gpu.setDocument(doc([0, 0]));
gpu.invalidate({ all: false, tiles: [[0, 0], [1, 0], [0, 1], [1, 1]], layers: [1], selection: false, repainted: [1] });
assert.equal(pending.length, 1);
assert.equal(pending[0].rect, null, "rechargement complet après transformation");
// En attendant, l'ancienne texture reste là où ses pixels sont vrais, pas décalée vers la nouvelle position.
assert.deepEqual(drawnAt(), [[61, -455]]);
pending.shift().answer(11, -505, 1618, 1080);
await settle();
assert.deepEqual(drawnAt(), [[11, -505]], "nouvelle texture à sa place");

// Transformation pendant un chargement : les pixels reçus correspondent à l'ancien repère ; posés à l'ancienne
// position puis relus aussitôt.
gpu.setDocument(doc([100, 100]));
gpu.invalidate({ all: true, tiles: [], layers: [1], selection: false, repainted: [1] }); // rechargement complet
assert.equal(pending.length, 1);
const inFlight = pending.shift();
gpu.setDocument(doc([0, 0]));
gpu.invalidate({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [1] });
inFlight.answer(0, 0, 500, 500); // pixels lus quand le calque était en (100, 100)
await settle();
assert.equal(pending.length, 1, "relu après la transformation survenue pendant le chargement");
assert.equal(pending[0].rect, null);
assert.deepEqual(drawnAt(), [[100, 100]], "en attendant, à la position où ces pixels sont vrais");
pending.shift().answer(40, 40, 600, 600);
await settle();
assert.deepEqual(drawnAt(), [[40, 40]]);

// Déplacement pendant un chargement (sans retouche) : la texture prend la position actuelle, rien n'est relu.
gpu.invalidate({ all: true, tiles: [], layers: [1], selection: false, repainted: [1] });
const loading = pending.shift();
gpu.setDocument(doc([30, 20]));
gpu.invalidate({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [] });
loading.answer(40, 40, 600, 600);
await settle();
assert.equal(pending.length, 0);
assert.deepEqual(drawnAt(), [[70, 60]]);

// Effets de calque : le moteur dit lesquels sont fondus à part, leurs pixels sont chargés ; régler un effet relit
// les pixels stylés et les effets, les anciens restant affichés d'ici là.
{
  pending.length = 0;
  const effectsAsked = [];
  const fetchEffects = (id) => new Promise((resolve) => effectsAsked.push({ id, resolve }));
  const fx = (opacity) => [{ type: "dropShadow", enabled: true, opacity, blend: "multiply" }];
  const styled = (opacity) => ({ id: 9, width: 2000, height: 1500, tileSize: 256, layers: [{ ...layer([0, 0]), effects: fx(opacity) }] });
  const gpu2 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), fetchEffects, () => {});
  gpu2.setDocument(styled(0.5));
  assert.deepEqual(pending.map((p) => p.kind), ["pixels"]);
  assert.equal(effectsAsked.length, 1);
  effectsAsked.shift().resolve({ under: ["multiply"], over: [] });
  await settle();
  pending.shift().answer(0, 0, 100, 100);
  await settle();
  assert.equal(gpu2.ready(), false, "l'ombre n'est pas encore là");
  assert.deepEqual(pending.map((p) => p.kind), ["under:0"]);
  pending.shift().answer(-10, -10, 130, 130);
  await settle();
  assert.equal(gpu2.ready(), true);

  gpu2.setDocument(styled(0.8));
  assert.equal(effectsAsked.length, 1, "effets redemandés");
  assert.deepEqual(pending.map((p) => p.kind), ["pixels"], "pixels stylés relus");
  assert.equal(gpu2.ready(), true, "les anciennes textures restent affichées");
  effectsAsked.shift().resolve({ under: ["multiply"], over: [] });
  await settle();
  pending.shift().answer(0, 0, 100, 100);
  await settle();
  assert.deepEqual(pending.map((p) => p.kind), ["under:0"], "ombre relue");
  pending.shift().answer(-12, -12, 134, 134);
  await settle();
  assert.equal(gpu2.settled, true);

  // Effets retirés : les pixels (sans effet) sont relus, l'ombre est oubliée.
  gpu2.setDocument({ ...styled(0.8), layers: [layer([0, 0])] });
  assert.deepEqual(pending.map((p) => p.kind), ["pixels"]);
  assert.equal(effectsAsked.length, 0);
  pending.shift().answer(0, 0, 100, 100);
  await settle();
  assert.equal(gpu2.ready(), true);
}

// Calque qui déborde du document (photo plus grande, rotation) : les tuiles signalées s'arrêtent aux bords du
// document, mais la zone repeinte que donne le moteur, dans le repère du calque, ne s'y arrête pas. Toute la zone est
// relue : rien de l'ancienne image ne subsiste hors du document (il réapparaîtrait en déplaçant le calque).
{
  pending.length = 0;
  const small = { id: 11, width: 1000, height: 800, tileSize: 256, layers: [layer([0, 0])] };
  const gpu3 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  gpu3.setDocument(small);
  pending.shift().answer(-300, -200, 1600, 1200);
  await settle();
  gpu3.invalidate({ all: false, tiles: [[0, 0], [3, 2]], layers: [1], selection: false, repainted: [1], areas: [{ layer: 1, mask: false, rect: [-256, 0, 1536, 768] }] });
  assert.equal(pending.length, 1);
  assert.deepEqual(pending[0].rect, [-256, 0, 1536, 768], "toute la zone repeinte, hors du document compris");
  pending.shift().answer(-256, 0, 1536, 768);
  await settle();
}

// Calque à masque déplacé SANS son masque (masque délié) : le moteur ne réécrit que le masque (il reste en place dans
// le document). La texture des pixels n'est pas relue, elle suit tout de suite la position ; seul le masque l'est.
{
  pending.length = 0;
  const masked = (offset) => ({ id: 12, width: 1000, height: 800, tileSize: 256, layers: [{ ...layer(offset), hasMask: true, maskEnabled: true }] });
  const gpu4 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  gpu4.setDocument(masked([0, 0]));
  for (let i = 0; i < 4; i++) {
    while (pending.length) pending.shift().answer(0, 0, 300, 200);
    await settle();
  }
  assert.deepEqual([...gpu4.resident.keys()].sort(), ["m1", "p1"]);
  gpu4.setDocument(masked([50, 40]));
  gpu4.invalidate({ all: false, tiles: [[0, 0], [1, 0]], layers: [1], selection: false, repainted: [1], areas: [{ layer: 1, mask: true, rect: [-50, -40, 350, 240] }] });
  assert.deepEqual(pending.map((p) => p.kind), ["mask"], "seul le masque est relu");
  assert.deepEqual(gpu4.resident.get("p1").offset, [50, 40], "pixels déjà à leur nouvelle place");
  assert.deepEqual(gpu4.resident.get("m1").offset, [0, 0], "masque à l'ancienne, là où ses pixels sont vrais, jusqu'à sa relecture");
  pending.shift().answer(-50, -40, 350, 240);
  await settle();
}

// Pixels ET masque relus ensemble (transformation) : les deux textures sont échangées en même temps. Tant que le masque
// n'est pas arrivé, les anciens pixels restent affichés (pas de nouveaux pixels sous l'ancien masque).
{
  pending.length = 0;
  const masked = (offset) => ({ id: 13, width: 1000, height: 800, tileSize: 256, layers: [{ ...layer(offset), hasMask: true, maskEnabled: true }] });
  const gpu5 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  gpu5.setDocument(masked([0, 0]));
  for (let i = 0; i < 4; i++) {
    while (pending.length) pending.shift().answer(0, 0, 300, 200);
    await settle();
  }
  const before = { p: gpu5.resident.get("p1"), m: gpu5.resident.get("m1") };
  gpu5.setDocument(masked([0, 0]));
  gpu5.invalidate({ all: true, tiles: [], layers: [1], selection: false, repainted: [1] });
  assert.equal(pending.length, 1, "un chargement à la fois");
  pending.shift().answer(10, 10, 320, 220);
  await settle();
  const firstKind = pending[0]?.kind;
  assert.ok(firstKind, "le compagnon est demandé ensuite");
  const arrived = firstKind === "mask" ? "p1" : "m1";
  assert.equal(gpu5.resident.get(arrived), before[arrived[0]], "la première texture arrivée attend la seconde : l'ancienne reste affichée");
  pending.shift().answer(10, 10, 320, 220);
  await settle();
  assert.notEqual(gpu5.resident.get("p1"), before.p, "pixels échangés");
  assert.notEqual(gpu5.resident.get("m1"), before.m, "masque échangé");
  assert.equal(gpu5.resident.get("p1").w, 320);
  assert.equal(gpu5.resident.get("m1").w, 320);
}

// Texture refusée (budget dépassé) : le moteur reçoit la limite et ne répond que l'en-tête ; le refus est retenu. Une
// nouvelle description du document (chaque pas d'un déplacement) ne remet pas la texture en file : le document reste
// au moteur sans relecture. Le calque repeint, ou le budget relevé : on réessaie.
{
  pending.length = 0;
  const big = (offset) => ({ id: 14, width: 1000, height: 800, tileSize: 256, layers: [layer(offset)] });
  const gpu6 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  gpu6.budgetBytes = 1000 * 1000;
  gpu6.setDocument(big([0, 0]));
  assert.equal(pending.length, 1);
  assert.deepEqual(pending[0].limit, [gpu6.maxTexture, Math.floor((1000 * 1000) / (4 * (4 / 3)))], "limite transmise au moteur : côté maximal, pixels logeables");
  pending.shift().refuse(2000, 1500);
  await settle();
  assert.equal(gpu6.ready(), false);
  assert.equal(gpu6.status().reason, "mémoire vidéo réservée dépassée");
  for (const offset of [[10, 0], [20, 0], [30, 0]]) gpu6.setDocument(big(offset));
  assert.equal(pending.length, 0, "déplacé : la texture refusée n'est pas relue");
  assert.equal(gpu6.status().reason, "mémoire vidéo réservée dépassée", "le document reste au moteur");
  // Repeint : une nouvelle lecture (le contenu a pu rapetisser), refusée à nouveau, puis retenue.
  gpu6.invalidate({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [1], areas: [{ layer: 1, mask: false, rect: [0, 0, 256, 256] }] });
  assert.equal(pending.length, 0, "rien en file : la texture n'était pas posée");
  gpu6.setDocument(big([30, 0]));
  assert.equal(pending.length, 1, "calque repeint : on réessaie");
  pending.shift().refuse(2000, 1500);
  await settle();
  gpu6.setDocument(big([40, 0]));
  assert.equal(pending.length, 0, "refus retenu à nouveau");
  // Budget relevé : on réessaie, et cette fois la texture tient.
  gpu6.budgetBytes = 1024 * 1024 * 1024;
  gpu6.setDocument(big([40, 0]));
  assert.equal(pending.length, 1, "budget relevé : on réessaie");
  pending.shift().answer(0, 0, 2000, 1500);
  await settle();
  assert.equal(gpu6.ready(), true, "texture posée, composition par la carte graphique");
}

// ---- piles : les suites de calques Normal sans masque deviennent une opération par STACK_SIZE calques.
{
  const blend = (layer, mode = "normal", target = 0) => ({ op: "blend", target, source: { layer }, mode, layer });
  const masked = new Set([5]);
  const ops = [blend(1), blend(2), blend(3, "multiply"), blend(4), blend(5), blend(6), { op: "clear", target: 1 }, blend(7, "normal", 1), blend(8, "normal", 1), blend(9)];
  const runs = stackRuns(ops, (id) => masked.has(id));
  assert.deepEqual(
    runs.map((o) => (o.op === "stack" ? `pile(${o.target}):${o.layers.join(",")}` : o.op === "blend" ? `${o.mode}:${o.layer}` : o.op)),
    ["pile(0):1,2", "multiply:3", "normal:4", "normal:5", "normal:6", "clear", "pile(1):7,8", "normal:9"],
    "un masque, un autre mode, une autre cible ou un calque seul coupent la pile",
  );
  const many = Array.from({ length: STACK_SIZE * 2 + 1 }, (_, i) => blend(i + 1));
  assert.deepEqual(stackRuns(many, () => false).map((o) => (o.op === "stack" ? o.layers.length : 1)), [STACK_SIZE, STACK_SIZE, 1], "au plus STACK_SIZE calques par pile");
}

console.log("OK — composition GPU : repère des textures après déplacement, transformation et retouche ; effets de calque ; calque qui déborde ; piles.");
