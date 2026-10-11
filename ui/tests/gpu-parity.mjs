// Parité de la composition WebGL avec le moteur, dans un vrai WebGL2 (Chromium sans écran) : npm run test:gpu-parity
//
// Le moteur écrit des documents de référence (crates/engine/examples/webgl_parity.rs) : description, pixels,
// masques, réglages mis à plat et image composée par le processeur. Ce test les charge dans la composition sur carte
// graphique de l'interface (viewport/gpu-layers.ts, exécutée telle quelle), dessine chaque document à 100 %, relit
// l'image et la compare au moteur, pixel par pixel.
//
// Demande Playwright (hors des dépendances du projet : `npm i -D playwright`, ou PLAYWRIGHT = chemin du paquet).
// Variables : PARITY_DIR (documents déjà écrits : on ne relance pas cargo), CHROMIUM (navigateur à utiliser).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "../..");
const data = process.env.PARITY_DIR ?? path.join(root, "node_modules/.cache-gpu-parity");
if (!process.env.PARITY_DIR) {
  execFileSync("cargo", ["run", "-q", "-p", "engine", "--release", "--example", "webgl_parity", "--", data], { cwd: root, stdio: "inherit" });
}

// Modules de l'interface, transpilés tels quels (imports relatifs réécrits en .js).
const modules = {};
for (const [name, file] of [["gpu-plan.js", "lib/gpu-plan.ts"], ["gpu-layers.js", "viewport/gpu-layers.ts"], ["shaders.js", "viewport/shaders.ts"]]) {
  const source = fs.readFileSync(path.join(here, "../src", file), "utf8");
  const out = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
  modules[name] = out.replace(/from "\.\.\/lib\/gpu-plan"/g, 'from "./gpu-plan.js"').replace(/from "\.\/(\w[\w-]*)"/g, 'from "./$1.js"');
}
modules["harness.js"] = `
import { GpuLayers } from "./gpu-layers.js";
import { VERTEX_SHADER } from "./shaders.js";
const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)).buffer;
window.compose = async (info, files, scale) => {
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(info.width * scale);
  canvas.height = Math.round(info.height * scale);
  const gl = canvas.getContext("webgl2", { alpha: true, premultipliedAlpha: true, antialias: false, preserveDrawingBuffer: true });
  const link = (fragment) => {
    const program = gl.createProgram();
    for (const [type, text] of [[gl.VERTEX_SHADER, VERTEX_SHADER], [gl.FRAGMENT_SHADER, fragment]]) {
      const shader = gl.createShader(type);
      gl.shaderSource(shader, text);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader));
      gl.attachShader(program, shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program));
    return { program, uniform: (name) => gl.getUniformLocation(program, name) };
  };
  gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  const fetchLayer = async (id, rect, part) => {
    const [kind, n] = part.split(":");
    const name = { pixels: "p", mask: "m", raw: "r", under: "u", over: "o" }[kind] + id + (n === undefined ? "" : "_" + n);
    if (!files[name]) throw new Error("absent : " + name);
    return bytes(files[name]);
  };
  const fetchEffects = async (id) => JSON.parse(new TextDecoder().decode(bytes(files["e" + id])));
  // Réglages mis à plat par le moteur, calque par calque.
  const byAdjustment = new Map(info.layers.filter((l) => l.adjustment).map((l) => [JSON.stringify(l.adjustment), files["a" + l.id]]));
  const fetchAdjust = async (adjustment) => bytes(byAdjustment.get(JSON.stringify(adjustment)));
  GpuLayers.floatBuffers = window.floatBuffers === true;
  const gpu = new GpuLayers(gl, link, fetchLayer, fetchAdjust, fetchEffects, () => {});
  gpu.setDocument(info);
  for (let i = 0; i < 500 && !(gpu.ready() && gpu.settled); i++) await new Promise((r) => setTimeout(r, 5));
  if (!gpu.ready()) return { error: "pas prêt : " + JSON.stringify(gpu.status()) };
  gl.bindFramebuffer(gl.FRAMEBUFFER, null);
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT);
  gl.enable(gl.BLEND);
  const view = { scale, originX: 0, originY: 0, viewW: canvas.width, viewH: canvas.height };
  const still = { shift: () => [0, 0], opacity: (id, opacity) => opacity };
  gpu.draw(view, still);
  const error = gl.getError();
  const pixels = new Uint8Array(canvas.width * canvas.height * 4);
  gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  // Aperçu de déplacement : un calque porteur de pixels décalé, l'image doit être celle d'un recalcul complet avec le
  // même décalage, deux fois de suite (rien de l'image précédente ne doit subsister).
  let partialGap = null;
  const moved = info.layers.find((l) => !l.isGroup && !l.adjustment && l.visible);
  if (moved && !gpu["plan"].simple) {
    const shifted = { shift: (id) => (id === moved.id ? [37, -23] : [0, 0]), opacity: (id, opacity) => opacity };
    const snap = () => { gl.clear(gl.COLOR_BUFFER_BIT); gpu.draw(view, shifted); const out = new Uint8Array(canvas.width * canvas.height * 4); gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, out); return out; };
    const first = snap();
    const second = snap();
    partialGap = 0;
    for (let i = 0; i < first.length; i++) partialGap = Math.max(partialGap, Math.abs(first[i] - second[i]));
    gl.clear(gl.COLOR_BUFFER_BIT);
  }
  // Lignes de bas en haut -> de haut en bas.
  const rows = [];
  const stride = canvas.width * 4;
  for (let y = canvas.height - 1; y >= 0; y--) rows.push(...pixels.subarray(y * stride, (y + 1) * stride));
  const floatTargets = gpu["floatTargets"];
  return { pixels: rows, glError: error, floatTargets, simple: gpu["plan"].simple, partialGap };
};
window.harnessReady = true;
`;

const playwright = await import(process.env.PLAYWRIGHT ?? "playwright");
const chromium = playwright.chromium ?? playwright.default.chromium;
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
});
const page = await browser.newPage();
page.on("pageerror", (e) => console.log("erreur de page :", String(e)));
await page.route("http://parity.test/**", (route) => {
  const name = route.request().url().split("/").pop();
  if (name === "index.html") return route.fulfill({ contentType: "text/html", body: `<!doctype html><script type="module" src="harness.js"></script>` });
  if (modules[name]) return route.fulfill({ contentType: "text/javascript", body: modules[name] });
  return route.fulfill({ status: 404, body: "" });
});
// PARITY_FLOAT=0 : tampons RGBA8 au lieu des demi-flottants, pour mesurer (le mode Saturation y dépasse la tolérance).
await page.addInitScript((floatBuffers) => { window.floatBuffers = floatBuffers; }, process.env.PARITY_FLOAT !== "0");
await page.goto("http://parity.test/index.html");
await page.waitForFunction(() => window.harnessReady === true);

/** Tolérance (valeurs 0-255) : arrondis des demi-flottants ; puissances et exponentielles des réglages. */
const tolerance = (name) => (name.startsWith("reglage") ? 3 : 2);
const names = JSON.parse(fs.readFileSync(path.join(data, "index.json"), "utf8"));
let failures = 0;
let worstOverall = 0;
for (const name of names) {
  const dir = path.join(data, name);
  const info = JSON.parse(fs.readFileSync(path.join(dir, "doc.json"), "utf8"));
  const files = {};
  for (const file of fs.readdirSync(dir)) if (file.endsWith(".bin") && file !== "expected.bin") files[file.slice(0, -4)] = fs.readFileSync(path.join(dir, file)).toString("base64");
  const result = await page.evaluate(([info, files]) => window.compose(info, files, 1), [info, files]);
  if (result.error) {
    console.log(`${name} : ${result.error}`);
    failures += 1;
    continue;
  }
  const got = Uint8Array.from(result.pixels);
  const want = fs.readFileSync(path.join(dir, "expected.bin"));
  assert.equal(got.length, want.length, name);
  let worst = 0;
  let over = 0;
  let at = -1;
  for (let i = 0; i < want.length; i++) {
    const gap = Math.abs(want[i] - got[i]);
    if (gap > worst) [worst, at] = [gap, i];
    if (gap > tolerance(name)) over += 1;
  }
  worstOverall = Math.max(worstOverall, worst);
  const where = at >= 0 ? ` (pire : pixel ${(at >> 2) % info.width},${Math.floor((at >> 2) / info.width)} canal ${at % 4} : moteur ${want[at]}, carte ${got[at]})` : "";
  const verdict = over === 0 ? "ok" : `ÉCHEC, ${over} valeurs hors tolérance`;
  // Aperçu de déplacement : deux images de suite identiques, au pixel près.
  const partial = result.partialGap === null ? "" : result.partialGap === 0 ? "  déplacement : stable" : `  déplacement : ÉCART ${result.partialGap}`;
  console.log(`${name.padEnd(28)} écart max ${String(worst).padStart(3)}  ${result.simple ? "direct  " : "programme"}  ${verdict}${over ? where : ""}${result.glError ? ` erreur GL ${result.glError}` : ""}${partial}`);
  if (over > 0 || result.glError || (result.partialGap !== null && result.partialGap !== 0)) failures += 1;
}
// Zoom à 200 % : chaque pixel du document couvre 2 × 2 pixels d'écran (pixels nets) ; même image, agrandie. Vérifie
// le placement des calques, des masques et du bruit de Dissoudre à une autre échelle que 1.
for (const name of ["masques", "groupes", "effets", "mode-dissolve", "reglage-9-normal"].filter((n) => names.includes(n))) {
  const dir = path.join(data, name);
  const info = JSON.parse(fs.readFileSync(path.join(dir, "doc.json"), "utf8"));
  const files = {};
  for (const file of fs.readdirSync(dir)) if (file.endsWith(".bin") && file !== "expected.bin") files[file.slice(0, -4)] = fs.readFileSync(path.join(dir, file)).toString("base64");
  const result = await page.evaluate(([info, files]) => window.compose(info, files, 2), [info, files]);
  const want = fs.readFileSync(path.join(dir, "expected.bin"));
  const got = Uint8Array.from(result.pixels);
  const width = info.width * 2;
  let worst = 0;
  let over = 0;
  for (let y = 0; y < info.height * 2; y++) {
    for (let x = 0; x < width; x++) {
      for (let c = 0; c < 4; c++) {
        const gap = Math.abs(got[(y * width + x) * 4 + c] - want[((y >> 1) * info.width + (x >> 1)) * 4 + c]);
        worst = Math.max(worst, gap);
        if (gap > tolerance(name)) over += 1;
      }
    }
  }
  console.log(`${(name + " à 200 %").padEnd(28)} écart max ${String(worst).padStart(3)}  ${over === 0 ? "ok" : `ÉCHEC, ${over} valeurs hors tolérance`}`);
  if (over > 0 || result.glError) failures += 1;
}
await browser.close();
assert.equal(failures, 0, `${failures} document(s) différent(s) du moteur`);
console.log(`OK — composition WebGL identique au moteur sur ${names.length} documents (écart max ${worstOverall}).`);
