// Déplacement d'un groupe par images en cache (viewport/move-cache.ts), sans navigateur : npm run test:move-cache
//
// Un groupe de 50 calques sur un document 4K, avec une mémoire vidéo réservée réduite : la carte graphique ne peut pas
// composer le document (repli du moteur). Le déplacement doit alors passer par les images en cache : aucune demande au
// moteur pendant le glisser, un seul déplacement au relâchement, et l'aperçu bouge d'un bloc (une seule texture pour
// le groupe, décalée exactement du déplacement ; dessous et dessus immobiles).
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
compile("viewport/camera.ts", ".cache-camera-for-move.mjs");
compile("viewport/paced.ts", ".cache-paced.mjs");
compile("lib/gpu-plan.ts", ".cache-gpu-plan-for-move.mjs");
const moveCache = await import(pathToFileURL(compile("viewport/move-cache.ts", ".cache-move-cache.mjs", (s) => s.replace(/from "\.\/camera"/, 'from "./.cache-camera-for-move.mjs"').replace(/from "\.\/paced"/, 'from "./.cache-paced.mjs"'))).href);
const { GpuLayers } = await import(pathToFileURL(compile("viewport/gpu-layers.ts", ".cache-gpu-layers-for-move.mjs", (s) => s.replace(/from "\.\.\/lib\/gpu-plan"/, 'from "./.cache-gpu-plan-for-move.mjs"').replace(/from "\.\/camera"/, 'from "./.cache-camera-for-move.mjs"').replace(/from "\.\/paced"/, 'from "./.cache-paced.mjs"'))).href);
const { gpuPlan } = await import(pathToFileURL(path.join(cache, ".cache-gpu-plan-for-move.mjs")).href);
const { MoveCache, MoveSession, sliceRegion, planLevels, moveShift } = moveCache;

// ---------------------------------------------------------------- carte graphique factice
// Toute constante (MAJUSCULES) vaut un nombre, toute méthode inconnue ne fait rien. On retient les textures créées,
// les octets envoyés, et chaque fusion dessinée avec la texture source et son emplacement (`u_srcPlace`).
let textures = 0;
let uploaded = 0;
let unit = "TEXTURE0";
const units = new Map();
let place = null;
let stackPlaces = [];
let draws = [];
const known = {
  getParameter: (p) => (p === "MAX_TEXTURE_SIZE" ? 16384 : "factice"),
  getExtension: () => null,
  createTexture: () => ({ texture: ++textures }),
  createSampler: () => ({}),
  createFramebuffer: () => ({}),
  texSubImage2D: (_t, _l, _x, _y, w, h) => (uploaded += w * h),
  activeTexture: (u) => (unit = u),
  bindTexture: (_target, texture) => units.set(unit, texture),
  uniform4f: (name, ...v) => name === "u_srcPlace" && (place = v),
  uniform4fv: (name, v) => name === "u_place" && (stackPlaces = Array.from(v)),
  drawArrays: () => draws.push({ texture: units.get("TEXTURE0"), place }),
};
const gl = new Proxy(known, {
  get: (target, prop) => (prop in target ? target[prop] : typeof prop === "string" && /^[A-Z0-9_]+$/.test(prop) ? prop : () => undefined),
});
const link = () => ({ program: {}, uniform: (name) => name });

// ---------------------------------------------------------------- document : 50 calques 4K dans un groupe
const W = 3840;
const H = 2160;
const layer = (id, parent, index, extra = {}) => ({
  id, parent, index, depth: parent === null ? 0 : 1, offset: [0, 0], name: `C${id}`, isGroup: false, expanded: true, childCount: 0,
  visible: true, opacity: 1, fill: 1, blend: "normal", locks: {}, clipped: false, hasMask: false, maskEnabled: false,
  text: null, adjustment: null, shape: null, effects: [], ...extra,
});
const GROUP = 100;
const layers = [layer(1, null, 0), layer(GROUP, null, 1, { isGroup: true, blend: "pass_through", childCount: 50 })];
for (let i = 0; i < 50; i++) layers.push(layer(200 + i, GROUP, i, { offset: [(i * 37) % 1900, (i * 53) % 1000] }));
layers.push(layer(2, null, 2, { blend: "multiply" }));
const doc = { id: 7, width: W, height: H, tileSize: 256, maxLevel: 4, revision: 3, layers };

// Moteur factice : on compte TOUT ce qui lui est demandé.
const calls = { layer: 0, layout: 0, slice: 0, adjust: 0, translate: 0 };
const fetchLayer = async (_id, _rect, _part, limit) => {
  calls.layer += 1;
  const [w, h] = [3840, 2160]; // chaque calque 4K entier
  const body = !limit || w * h <= limit[1];
  const buffer = new ArrayBuffer(16 + (body ? w * h * 4 : 0));
  const view = new DataView(buffer);
  view.setUint32(8, w, true);
  view.setUint32(12, h, true);
  return buffer;
};
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const gpu = new GpuLayers(gl, link, fetchLayer, () => Promise.reject(new Error("aucun réglage")), () => Promise.reject(new Error("aucun effet")), () => {});
gpu.budgetBytes = 512 * 1024 * 1024; // budget réduit : 50 calques 4K (44 Mo chacun) n'y tiennent pas
gpu.setDocument(doc);
for (let i = 0; i < 200 && !gpu.settled; i++) await settle();
assert.equal(gpu.ready(), false, "budget dépassé : la carte graphique ne compose pas le document");
assert.match(gpu.status().reason ?? "", /mémoire vidéo/);
const layerCallsAtRest = calls.layer;

// Découpage du moteur (voir crates/engine/src/move_preview.rs) : dessous, le groupe, puis le calque en Produit.
const slice = (kind, mode, layer = null) => ({ kind, mode, layer, adjustment: null, opacity: 1, masked: false, maskBackground: 1 });
const layout = {
  revision: 3, moving: [GROUP], exact: true, notes: [], collapsible: GROUP, bounds: [0, 0, 3820, 2140],
  slices: [slice("below", "normal"), slice("moving", "normal", GROUP), slice("layers", "multiply")],
};
const requested = [];
const source = {
  layout: async (ids) => (calls.layout++, assert.deepEqual(ids, [GROUP]), layout),
  slice: async (ids, index, level, rect, revision, own) => {
    calls.slice += 1;
    requested.push({ index, level, rect, own });
    assert.equal(revision, 3);
    const [x, y, w, h] = rect;
    const buffer = new ArrayBuffer(16 + w * h * 4);
    const view = new DataView(buffer);
    view.setInt32(0, x, true);
    view.setInt32(4, y, true);
    view.setUint32(8, w, true);
    view.setUint32(12, h, true);
    return buffer;
  },
  adjust: async () => (calls.adjust++, new ArrayBuffer(4)),
};

// Vue : le document entier tient dans une zone de travail de 1920 × 1080 pixels physiques (échelle 0,5).
const view = { scale: 0.5, originX: 0, originY: 0, viewW: 1920, viewH: 1080 };

// ---------------------------------------------------------------- zones et niveaux (modules purs)
assert.deepEqual(sliceRegion(view, doc, 1, false, null), [0, 0, 1920, 1080], "fixe : la zone visible, limitée au document");
const movingRegion = sliceRegion(view, doc, 1, true, layout.bounds);
assert.deepEqual(movingRegion, [0, 0, 1910, 1070], "déplacée : limitée à l'étendue du groupe");
assert.equal(sliceRegion(view, doc, 1, true, null), null, "rien à déplacer");
assert.deepEqual(planLevels(view, doc, layout, 1e9, "layers"), [2, 1], "basse résolution d'abord, puis celle de l'écran");
assert.deepEqual(planLevels(view, doc, layout, 1e9, "collapse"), [1], "aplat seul : directement à la résolution de l'écran");
const tight = planLevels(view, doc, layout, 12 * 1024 * 1024, "layers");
assert.ok(tight.length > 0 && tight.at(-1) > 1, `budget serré : un niveau plus grossier (${tight})`);
assert.deepEqual(planLevels(view, doc, layout, 1000, "layers"), [], "budget intenable : pas d'aperçu en couches");

// ---------------------------------------------------------------- le geste
let redraws = 0;
const prepared = new MoveCache(gl, source, doc.id, [GROUP], doc, view, () => gpu.budgetBytes, "layers", () => redraws++);
assert.equal(prepared.ready, false, "tout ou rien : rien n'est montré avant que les couches soient prêtes");
assert.equal(await prepared.whenReady(), true);
for (let i = 0; i < 20 && prepared.fullMs === null; i++) await settle();
for (let i = 0; i < 20 && prepared.level !== 1; i++) await settle();
assert.equal(prepared.level, 1, "couches à la résolution de l'écran");
assert.equal(prepared.slices.length, 3);
assert.equal(calls.layout, 1);
assert.equal(calls.slice, 3, "trois couches, directement à la résolution de l'écran (elles arrivent vite)");
const preparedImages = calls.slice;
assert.ok(requested.every((r) => r.own), "opacité et masque du groupe compris dans la couche déplacée");
assert.ok(prepared.bytes < gpu.budgetBytes, "les couches tiennent dans la mémoire vidéo réservée");
// Tampons de composition à la taille de l'écran : alloués à la première image, une fois pour toutes.
gpu.drawCached(view, doc, prepared.slices, [0, 0]);
const textureCount = textures;

const snapshot = { ...calls };
const offsets = [];
let ended = false;
const session = new MoveSession(prepared, {
  offset: (dx, dy) => offsets.push([dx, dy]),
  translate: async (dx, dy) => {
    calls.translate += 1;
    assert.deepEqual([dx, dy], [-301, 177], "le déplacement final, en une fois");
  },
  settled: async () => {},
  end: () => (ended = true),
});
// Soixante pas de glisser, chacun dessiné.
const positions = [];
for (let step = 1; step <= 60; step++) {
  const [dx, dy] = step === 60 ? [-301, 177] : [Math.round(-5 * step), Math.round(3 * step)];
  session.move(dx, dy);
  draws = [];
  gpu.drawCached(view, doc, prepared.slices, [dx, dy]);
  const sources = draws.filter((d) => d.place !== null && prepared.slices.some((s) => s.texture === d.texture));
  positions.push({ dx, dy, sources: sources.map((d) => ({ texture: d.texture, place: d.place })) });
  place = null;
}
assert.deepEqual({ ...calls }, snapshot, "aucune demande au moteur pendant le glisser");
assert.equal(calls.layer, layerCallsAtRest, "aucune texture de calque relue pendant le glisser");
assert.equal(textures, textureCount, "aucune texture créée pendant le glisser (tampons de composition déjà alloués)");
assert.equal(offsets.length, 60);
const [below, moved, above] = prepared.slices;
for (const { dx, dy, sources } of positions) {
  // Une fusion par couche : le groupe de 50 calques n'est qu'UNE texture.
  assert.equal(sources.length, 3, "trois couches dessinées");
  const at = (texture) => sources.find((s) => s.texture === texture)?.place;
  assert.deepEqual(at(below.texture), below.rect, "dessous immobile");
  assert.deepEqual(at(above.texture), above.rect, "dessus immobile");
  assert.deepEqual(at(moved.texture), [moved.rect[0] + dx, moved.rect[1] + dy, moved.rect[2], moved.rect[3]], "le groupe bouge d'un bloc, exactement du déplacement");
}
// Rien qu'en mode Normal : une seule passe (pile) pour toute l'image, la couche déplacée décalée.
draws = [];
gpu.drawCached(view, doc, prepared.slices.map((s) => ({ ...s, mode: "normal" })), [10, 20]);
assert.equal(draws.length, 1, "une seule passe");
assert.deepEqual(stackPlaces.slice(4, 8), [moved.rect[0] + 10, moved.rect[1] + 20, moved.rect[2], moved.rect[3]]);
assert.deepEqual(stackPlaces.slice(0, 4), below.rect);
assert.deepEqual({ ...calls }, snapshot, "toujours aucune demande au moteur");
await session.finish();
assert.equal(calls.translate, 1, "un seul déplacement au relâchement");
assert.ok(ended, "l'aperçu se termine une fois l'image exacte affichée");
await session.finish();
assert.equal(calls.translate, 1, "un relâchement en double n'envoie rien de plus");
prepared.dispose();

// Clic sans glisser : rien n'est envoyé.
const still = new MoveSession(null, { offset() {}, translate: async () => calls.translate++, settled: async () => {}, end() {} });
await still.finish();
assert.equal(calls.translate, 1);

// Moteur lent (cache froid) : des couches plus grossières sont demandées en attendant, puis échangées d'un coup.
const levels = [];
const slow = { ...source, slice: async (ids, index, level, rect, revision, own) => {
  await new Promise((r) => setTimeout(r, level === 1 ? 300 : 10));
  return source.slice(ids, index, level, rect, revision, own);
} };
const cold = new MoveCache(gl, slow, doc.id, [GROUP], doc, view, () => 1e9, "layers", () => levels.push(cold.level));
assert.equal(await cold.whenReady(), true);
assert.equal(cold.level, 2, "d'abord la basse résolution");
for (let i = 0; i < 100 && cold.level !== 1; i++) await new Promise((r) => setTimeout(r, 10));
assert.equal(cold.level, 1, "puis celle de l'écran");
assert.ok(cold.readyMs < cold.fullMs);
cold.dispose();

// Découpage refusé par le moteur : préparation abandonnée, la voie d'avant prend le relais.
const refused = new MoveCache(gl, { ...source, layout: async () => null }, doc.id, [GROUP], doc, view, () => 1e9, "layers", () => {});
assert.equal(await refused.whenReady(), false);
assert.match(refused.describe(), /moteur/);

// ---------------------------------------------------------------- programme de la carte graphique : groupe aplati
const plain = gpuPlan(doc);
const collapsed = gpuPlan(doc, { collapsed: GROUP });
assert.equal(plain.layers.length, 52);
assert.deepEqual(collapsed.layers.map((l) => l.id), [1, GROUP, 2], "le groupe devient une seule texture");
assert.deepEqual(collapsed.program.ops.filter((op) => op.op === "blend").map((op) => [op.source.layer ?? null, op.mode]), [[1, "normal"], [GROUP, "normal"], [2, "multiply"]]);
const isolated = gpuPlan({ ...doc, layers: doc.layers.map((l) => (l.id === GROUP ? { ...l, blend: "screen", opacity: 0.5 } : l)) }, { collapsed: GROUP });
assert.ok(isolated.program.ops.some((op) => op.op === "blend" && op.source.layer === GROUP && op.mode === "screen" && op.layer === GROUP), "groupe isolé : son mode, son opacité");

// ---------------------------------------------------------------- relâchement sans double décalage
// La texture d'un calque est montrée à « départ + décalage » ; quand le moteur la replace (nouvelle position), le
// décalage restant tombe à zéro dans la même image. Une texture relue après la réponse est déjà à sa place.
const state = { moving: new Set([5]), parts: "both", dx: 40, dy: -12, start: new Map([[5, [100, 50]]]), fresh: new WeakSet() };
const texture = { offset: [100, 50] };
assert.deepEqual(moveShift(state, 5, "pixels", texture), [40, -12], "pendant le geste");
texture.offset = [140, 38]; // réponse du moteur : la texture suit la nouvelle position
assert.deepEqual(moveShift(state, 5, "pixels", texture), [0, 0], "après la réponse : plus aucun décalage (pas de saut)");
const reloaded = { offset: [100, 50] }; // masque relu, contenu décalé dans le repère du calque
state.fresh.add(reloaded);
assert.deepEqual(moveShift(state, 5, "mask", reloaded), [0, 0], "texture relue après la réponse : à sa place");
assert.deepEqual(moveShift(state, 6, "pixels", texture), [0, 0], "calque immobile");
assert.deepEqual(moveShift({ ...state, parts: "pixels" }, 5, "mask", { offset: [100, 50] }), [0, 0], "pixels seuls : le masque reste");

// ---------------------------------------------------------------- groupe en Transfert optimisé par paquets
// Le moteur découpe son contenu (voir move_preview.rs, `MAX_PACKS`) : plusieurs couches suivent le groupe, chacune
// dans son mode, et le masque d'un réglage du groupe aussi. Toutes sont décalées, et transformées (Ctrl+T).
const { movesWith } = moveCache;
const packed = {
  ...layout,
  collapsible: null,
  slices: [
    slice("below", "normal"),
    { ...slice("moving", "normal"), moving: true },
    { ...slice("moving", "multiply"), moving: true },
    { ...slice("adjust", "normal"), adjustment: { kind: "invert" }, masked: true, moving: true },
    { ...slice("layers", "screen"), moving: false },
  ],
};
assert.deepEqual(packed.slices.map(movesWith), [false, true, true, true, false], "couches qui suivent le groupe");
assert.equal(movesWith(slice("moving", "normal")), true, "ancien découpage (sans le champ) : le « déplacé » suit");
const packSource = { ...source, layout: async () => packed };
const packs = new MoveCache(gl, packSource, doc.id, [GROUP], doc, view, () => gpu.budgetBytes, "layers", () => {});
assert.equal(await packs.whenReady(), true);
assert.deepEqual(packs.slices.map((s) => s.moving), [false, true, true, true, false]);
const turn = { forward: [0.9, -0.2, 120, 0.2, 0.9, -40, 0, 0, 1], inverse: [0.9, 0.2, -100, -0.2, 0.9, 60, 0, 0, 1] };
gpu.drawCached(view, doc, packs.slices, [15, -8], 1, turn);
gpu.drawCached(view, doc, packs.slices, [15, -8], 1, null);
packs.dispose();

// ---------------------------------------------------------------- couches fixes gardées d'un geste à l'autre
// Même empreinte (contenu inchangé), même vue : le geste suivant reprend « dessous » et « dessus » sans rien
// redemander au moteur ; seule la couche qui bouge est recomposée.
const printed = {
  ...layout,
  slices: layout.slices.map((s, i) => (s.kind === "moving" ? { ...s, moving: true } : { ...s, fingerprint: `f${i}` })),
};
const asked = [];
const keepSource = { ...source, layout: async () => printed, slice: async (ids, index, ...rest) => (asked.push(index), source.slice(ids, index, ...rest)) };
const first = new MoveCache(gl, keepSource, doc.id, [GROUP], doc, view, () => 1e9, "layers", () => {});
assert.equal(await first.whenReady(), true);
for (let i = 0; i < 100 && first.level !== 1; i++) await new Promise((r) => setTimeout(r, 10));
const firstAsked = asked.length;
const keptTextures = first.slices.filter((s) => !s.moving && s.texture).map((s) => s.texture);
first.dispose();
asked.length = 0;
const second = new MoveCache(gl, keepSource, doc.id, [GROUP], doc, view, () => 1e9, "layers", () => {});
assert.equal(await second.whenReady(), true);
for (let i = 0; i < 100 && second.level !== 1; i++) await new Promise((r) => setTimeout(r, 10));
const fixedIndexes = printed.slices.map((s, i) => (s.moving ? -1 : i)).filter((i) => i >= 0);
assert.ok(firstAsked > 0);
assert.ok(second.slices.filter((s) => !s.moving && s.texture).every((s) => keptTextures.includes(s.texture)), "mêmes textures reprises");
assert.ok(!asked.some((i) => fixedIndexes.includes(i) && second.level === 1 && asked.filter((j) => j === i).length > 1), "pas de double demande");
assert.ok(asked.filter((i) => fixedIndexes.includes(i)).length < firstAsked, "les couches fixes ne sont pas redemandées à la résolution de l'écran");
second.dispose();
moveCache.forgetRetained();

console.log(`OK — groupe de 50 calques 4K : ${preparedImages} images préparées (${prepared.readyMs ?? "?"} ms), 60 pas sans appel au moteur, 1 déplacement au relâchement.`);
