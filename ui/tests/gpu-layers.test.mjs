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
compile("viewport/camera.ts", ".cache-camera-for-layers.mjs");
compile("viewport/paced.ts", ".cache-paced.mjs");
const out = compile("viewport/gpu-layers.ts", ".cache-gpu-layers.mjs", (s) => s.replace(/from "\.\.\/lib\/gpu-plan"/, 'from "./.cache-gpu-plan-for-layers.mjs"').replace(/from "\.\/camera"/, 'from "./.cache-camera-for-layers.mjs"').replace(/from "\.\/paced"/, 'from "./.cache-paced.mjs"'));
const { GpuLayers, stackRuns, STACK_SIZE, previewInverse, transformedBounds, relativeMatrix } = await import(pathToFileURL(out).href);
const { homography, centroid } = await import(pathToFileURL(compile("lib/geometry.ts", ".cache-geometry-for-layers.mjs")).href);

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
  uniform1fv() {}, enable() {}, disable() {}, bindFramebuffer() {}, blendFunc() {},
  uniform4f() {},
  // Aperçu de transformation : matrices inverses (par colonnes) de la pile ; ciseaux de la dernière passe.
  uniformMatrix3fv: (name, _transpose, v) => name === "u_inv" && (inverses = Array.from(v)),
  scissor: (x, y, w, h) => (scissors = [x, y, w, h]),
  // Pile de calques : `u_count` calques placés (x, y, largeur, hauteur dans le document) dans `u_place`.
  uniform1i: (name, v) => name === "u_count" && (count = v),
  uniform4fv: (name, v) => name === "u_place" && rects.push(...Array.from({ length: count }, (_, i) => Array.from(v.slice(i * 4, i * 4 + 4)))),
};
let count = 0;
let inverses = [];
let scissors = [];
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

// ---- aperçu de transformation (cadre de transformation) : la matrice d'aperçu de la carte graphique et le cadre final
// du moteur donnent la même boîte. Le moteur (transform_store) donne au résultat l'image des quatre coins de la source
// par l'homographie du cadre ; la carte graphique dessine (ciseaux) la boîte englobante des coins transformés de la
// texture, et lit chaque pixel par l'homographie inverse.
{
  const apply = (m, p) => {
    const w = m[6] * p.x + m[7] * p.y + m[8];
    return { x: (m[0] * p.x + m[1] * p.y + m[2]) / w, y: (m[3] * p.x + m[4] * p.y + m[5]) / w, w };
  };
  const close = (a, b, eps = 1e-6) => assert.ok(Math.abs(a.x - b.x) < eps && Math.abs(a.y - b.y) < eps, `${JSON.stringify(a)} != ${JSON.stringify(b)}`);
  const box = (quad) => [Math.min(...quad.map((p) => p.x)), Math.min(...quad.map((p) => p.y)), Math.max(...quad.map((p) => p.x)), Math.max(...quad.map((p) => p.y))];
  // Texture de l'instance du début : calque en (30, 20), pixels (40, 40, 600, 600) de son repère → document (70, 60).
  const rect = [70, 60, 600, 600];
  const source = [{ x: 70, y: 60 }, { x: 670, y: 60 }, { x: 670, y: 660 }, { x: 70, y: 660 }];
  const turn = (a, k) => source.map((p) => ({ x: 370 + ((p.x - 370) * Math.cos(a) - (p.y - 360) * Math.sin(a)) * k, y: 360 + ((p.x - 370) * Math.sin(a) + (p.y - 360) * Math.cos(a)) * k }));
  const quads = [
    source.map((p) => ({ x: p.x * 0.25 + 3, y: p.y * 0.25 - 7 })), // réduction : niveaux de détail
    source.map((p) => ({ x: 2 * p.x - 100, y: p.y + 40 })), // agrandissement d'un seul côté
    turn(0.3, 1), // rotation
    turn(-2.1, 0.6), // rotation et réduction
    [{ x: 70, y: 60 }, { x: 900, y: 120 }, { x: 760, y: 700 }, { x: 20, y: 640 }], // perspective
    [source[1], source[0], source[3], source[2]], // miroir horizontal
    source.map((p) => ({ x: p.x + 0.4 * (p.y - 60), y: p.y })), // inclinaison
  ];
  for (const quad of quads) {
    const forward = homography(source, quad);
    // Même boîte que le moteur : l'image des quatre coins de la source.
    const engine = box(source.map((p) => apply(forward, p)));
    const gpuBox = transformedBounds(rect, forward);
    engine.forEach((v, i) => assert.ok(Math.abs(v - gpuBox[i]) < 1e-6, `boîte ${gpuBox} != ${engine}`));
    engine.forEach((v, i) => assert.ok(Math.abs(v - box(quad)[i]) < 1e-6, "boîte du cadre affiché"));
    // L'inverse ramène chaque coin du cadre sur le coin d'origine, dénominateur positif sur le cadre.
    const inverse = previewInverse(forward, centroid(quad));
    quad.forEach((p, i) => close(apply(inverse, p), source[i]));
    assert.ok(apply(inverse, centroid(quad)).w > 0, "dénominateur positif sur le calque transformé");
    // Dessin réel : la pile reçoit l'inverse (par colonnes), et les ciseaux couvrent la boîte au pixel près.
    rects = [];
    inverses = [];
    gpu.draw({ scale: 1, originX: 0, originY: 0, viewW: 2000, viewH: 1500 }, { shift: () => [0, 0], opacity: (id, opacity) => opacity, matrix: (id) => (id === 1 ? { forward, inverse } : null) });
    assert.deepEqual(rects, [rect], "texture placée comme sans transformation : seule la matrice change");
    const columns = [inverse[0], inverse[3], inverse[6], inverse[1], inverse[4], inverse[7], inverse[2], inverse[5], inverse[8]];
    columns.forEach((v, i) => assert.ok(Math.abs(inverses[i] - v) < 1e-4 * Math.max(1, Math.abs(v)), `matrice envoyée au shader ${inverses.slice(0, 9)}`));
    const [x, glY, w, h] = scissors;
    const [x0, y0, x1, y1] = [x, 1500 - glY - h, x + w, 1500 - glY];
    const [bx0, by0, bx1, by1] = [Math.max(0, engine[0]), Math.max(0, engine[1]), Math.min(2000, engine[2]), Math.min(1500, engine[3])];
    assert.ok(x0 <= bx0 && y0 <= by0 && x1 >= bx1 && y1 >= by1, `ciseaux ${[x0, y0, x1, y1]} hors de la boîte ${engine}`);
    assert.ok(bx0 - x0 <= 2 && by0 - y0 <= 2 && x1 - bx1 <= 2 && y1 - by1 <= 2, `ciseaux ${[x0, y0, x1, y1]} trop larges pour ${engine}`);
  }
  // Sans matrice : l'identité, exactement (le shader rend alors la même image qu'avant, au bit près).
  inverses = [];
  gpu.draw({ scale: 1, originX: 0, originY: 0, viewW: 2000, viewH: 1500 }, { shift: () => [0, 0], opacity: (id, opacity) => opacity });
  assert.deepEqual(inverses.slice(0, 9), [1, 0, 0, 0, 1, 0, 0, 0, 1]);
  // Perspective qui envoie un coin à l'infini : toute la vue est à redessiner (pas de boîte fausse).
  assert.equal(transformedBounds([0, 0, 10, 10], [1, 0, 0, 0, 1, 0, -0.1, 0, 1]), null);
  assert.equal(previewInverse([1, 2, 3, 2, 4, 6, 0, 0, 1], { x: 0, y: 0 }), null, "matrice non inversible");

  // Étape du moteur au relâchement : sa texture porte l'état M1 (cadre q1) ; une nouvelle poignée saisie avant ou après
  // sa réponse montre M2 (cadre q2). La texture de M1 reçoit M2 · M1⁻¹ : ses coins (ceux de q1) tombent sur ceux de q2,
  // sans saut ; et une fois l'état atteint (M2 = M1), plus aucune matrice — la texture du moteur s'affiche telle quelle.
  const [q1, q2] = [quads[2], quads[4]];
  const [m1, m2] = [homography(source, q1), homography(source, q2)];
  const relative = relativeMatrix(m2, m1, centroid(q2));
  q1.forEach((p, i) => close(apply(relative.forward, p), q2[i], 1e-5));
  q2.forEach((p, i) => close(apply(relative.inverse, p), q1[i], 1e-5));
  assert.ok(apply(relative.inverse, centroid(q2)).w > 0);
  assert.equal(relativeMatrix(m1, m1, centroid(q1)), null, "même état : la texture du moteur telle quelle");
  const fromStart = relativeMatrix(m2, null, centroid(q2));
  source.forEach((p, i) => close(apply(fromStart.forward, p), q2[i], 1e-5));
  // Retour à l'état d'avant le geste (abandon après une étape) : la texture de M1 est ramenée sur la source.
  const back = relativeMatrix([1, 0, 0, 0, 1, 0, 0, 0, 1], m1, centroid(source));
  q1.forEach((p, i) => close(apply(back.forward, p), source[i], 1e-5));
}

// Chaque texture posée, complète ou retouchée, est signalée (nouvel objet) : l'aperçu d'une transformation sait ainsi
// de quelle étape du moteur elle vient.
{
  pending.length = 0;
  const committed = [];
  const gpu7 = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  gpu7.onTextureCommitted = (id, texture) => committed.push([id, texture]);
  gpu7.setDocument({ id: 21, width: 2000, height: 1500, tileSize: 256, layers: [layer([0, 0])] });
  pending.shift().answer(0, 0, 600, 600);
  await settle();
  assert.equal(committed.length, 1);
  gpu7.invalidate({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [1], areas: [{ layer: 1, mask: false, rect: [0, 0, 256, 256] }] });
  pending.shift().answer(0, 0, 256, 256);
  await settle();
  assert.equal(committed.length, 2, "retouche partielle signalée");
  assert.notEqual(committed[1][1], committed[0][1], "nouvel objet pour la texture retouchée");
}

// Aperçu de transformation lié au document de son geste : un autre onglet a souvent les mêmes identifiants de calques
// (1, 2…). L'aperçu du document A ne s'applique pas aux calques de B, et il est encore là au retour sur A.
{
  const previewsOut = compile("viewport/transform-previews.ts", ".cache-transform-previews.mjs", (s) => s.replace(/from "\.\/gpu-layers"/, 'from "./.cache-gpu-layers.mjs"'));
  const { TransformPreviews } = await import(pathToFileURL(previewsOut).href);
  const docA = { id: 1, layers: [{ id: 1, parent: null }, { id: 2, parent: null }, { id: 3, parent: 2 }] };
  const docB = { id: 2, layers: [{ id: 1, parent: null }] };
  const previews = new TransformPreviews();
  const shifted = [1, 0, 40, 0, 1, 25, 0, 0, 1];
  const texture = {};
  previews.setDocument(docA);
  assert.equal(previews.show("geste", docB.id, [1], shifted, { x: 0, y: 0 }, true), false, "pas d'aperçu pour un document non affiché");
  assert.equal(previews.show("geste", docA.id, [2], shifted, { x: 0, y: 0 }, false), false, "composition indisponible : le moteur rend chaque pas");
  assert.ok(previews.show("geste", docA.id, [2], shifted, { x: 0, y: 0 }, true));
  assert.ok(previews.active);
  assert.deepEqual(previews.matrix(3, texture)?.forward, shifted, "le groupe emporte ses calques");
  assert.equal(previews.matrix(1, texture), null, "calque non transformé");

  // Onglet B (qui a aussi un calque 1, mais ni 2 ni 3) : rien ne s'y applique, aucune texture de B ne reçoit d'état.
  previews.setDocument(docB);
  assert.equal(previews.active, false);
  previews.step("geste", shifted);
  const textureB = {};
  previews.committed(1, textureB);
  assert.equal(previews.matrix(1, textureB), null, "l'aperçu de A ne s'applique pas à B");
  assert.ok(previews.show("geste", docA.id, [2], [1, 0, 0, 0, 1, 0, 0, 0, 1], { x: 0, y: 0 }, false), "abandon depuis B : l'aperçu de A est ramené à l'identité");
  assert.equal(previews.show("geste", docB.id, [1], shifted, { x: 0, y: 0 }, true), false, "même geste, autre document : refusé");
  previews.show("geste", docA.id, [2], shifted, { x: 0, y: 0 }, false);

  // Retour sur A : l'aperçu est intact (calques 2 et 3 compris), les textures arrivées depuis portent l'étape du moteur.
  previews.setDocument(docA);
  assert.ok(previews.active);
  assert.deepEqual(previews.matrix(3, texture)?.forward, shifted, "aperçu encore là au retour");
  const fresh = {};
  previews.committed(3, fresh);
  assert.equal(previews.matrix(3, fresh), null, "texture de l'étape du moteur : affichée telle quelle");
  assert.ok(previews.end("geste"));
  assert.equal(previews.active, false);
  assert.equal(previews.matrix(3, texture), null, "rendu final arrivé : plus d'aperçu");
}

// Pinceau peint sur carte graphique (live-paint.ts) et annulations avec pixels : rien n'est relu de ce qui est déjà juste.
{
  Object.assign(gl, { createFramebuffer: () => ({}), deleteFramebuffer() {}, framebufferTexture2D() {}, clearColor() {}, clear() {}, blitFramebuffer() {}, isEnabled: () => false });
  const live = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
  const painted = { ...doc([0, 0]), id: 9 };
  live.setDocument(painted);
  pending.shift().answer(100, 100, 300, 200);
  await settle();
  const area = (rect) => ({ all: false, tiles: [[0, 0]], layers: [1], selection: false, repainted: [1], areas: [{ layer: 1, mask: false, rect }] });

  // Texture agrandie au document entier pour pouvoir y peindre partout ; contenu d'avant recopié.
  const target = live.paintTarget(1);
  assert.ok(target, "texture prête à peindre");
  assert.deepEqual([target.width, target.height, target.dx, target.dy, target.docWidth], [2000, 1500, 0, 0, 2000]);

  // Pendant le trait : les modifications signalées par le moteur pour ce trait ne relisent rien.
  live.holdPaint(1);
  live.markFresh(1, "all");
  live.invalidate(area([120, 130, 40, 40]));
  live.clearFresh();
  assert.equal(pending.length, 0, "trait peint ici : rien n'est relu");
  // Une autre modification pendant le trait attend sa fin (elle lirait l'état d'avant le trait).
  live.invalidate(area([500, 500, 10, 10]));
  await settle();
  assert.equal(pending.length, 0, "relecture suspendue pendant le trait");
  live.releasePaint(1);
  await settle();
  assert.equal(pending.length, 1, "relue une fois le trait fini par le moteur");
  assert.deepEqual(pending[0].rect, [500, 500, 10, 10]);
  pending.shift().answer(500, 500, 10, 10);
  await settle();

  // Trait refusé par le moteur : la texture est relue entièrement.
  live.holdPaint(1);
  live.reloadPixels(1);
  live.releasePaint(1);
  await settle();
  assert.equal(pending.length, 1);
  assert.equal(pending[0].rect, null, "relecture complète après un trait refusé");
  pending.shift().answer(0, 0, 2000, 1500);
  await settle();

  // Annulation : les pixels arrivent avec la réponse, posés tels quels ; la modification signalée ne relit que ce qui
  // n'a pas été envoyé.
  const before = uploads.length;
  const applied = live.applyPatches([{ layer: 1, rect: [40, 50, 8, 6], pixels: new Uint8Array(8 * 6 * 4) }]);
  assert.deepEqual(uploads.slice(before), [[40, 50, 8, 6]], "zone posée directement");
  assert.deepEqual([...applied.entries()], [[1, [[40, 50, 8, 6]]]]);
  live.markFresh(1, applied.get(1));
  live.invalidate(area([40, 50, 8, 6]));
  live.clearFresh();
  assert.equal(pending.length, 0, "zone envoyée avec l'annulation : pas relue");
  live.markFresh(1, applied.get(1));
  live.invalidate(area([40, 50, 30, 6]));
  live.clearFresh();
  assert.equal(pending.length, 1, "zone plus grande que celle envoyée : relue");
  pending.shift().answer(40, 50, 30, 6);
  await settle();
  // Hors de la texture : refusé (relu normalement).
  assert.equal(live.applyPatches([{ layer: 1, rect: [1990, 1490, 20, 20], pixels: new Uint8Array(1600) }]).size, 0);
  // Document changé (textures relâchées) : plus de trait en cours.
  live.holdPaint(1);
  live.setDocument({ ...painted, id: 10 });
  assert.equal(live.paintHeld(1), false);
}

// Mode Gris ou Bitmap : la règle du rendu final des aperçus en couches (shaders.ts, FINISH_SHADER) est mot pour mot
// celle de la composition sur carte graphique (finishColor de gpu-layers.ts).
{
  const finishColor = (file) => fs.readFileSync(path.join(here, "../src", file), "utf8").match(/vec4 finishColor\(vec4 c\) \{[\s\S]*?\n\}\n/)?.[0];
  const reference = finishColor("viewport/gpu-layers.ts");
  assert.ok(reference && reference.includes("77 * q.r + 150 * q.g + 29 * q.b + 128"), "finishColor de gpu-layers.ts introuvable");
  assert.equal(finishColor("viewport/shaders.ts"), reference, "FINISH_SHADER : même finishColor que gpu-layers.ts");
}

console.log("OK — composition GPU : repère des textures après déplacement, transformation et retouche ; effets de calque ; calque qui déborde ; piles ; aperçu de transformation ; pinceau peint sur carte graphique et annulations avec pixels.");
