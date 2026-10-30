// Parité du pinceau peint sur carte graphique (viewport/live-paint.ts) avec le moteur, dans un vrai WebGL2 (Chromium
// sans écran) : npm run test:live-paint
//
// Le moteur écrit des cas de référence (test ignoré `live_paint_fixture` de crates/engine/src/paint.rs) : le calque
// avant et après un trait, les points (pinceau rond) ou les empreintes (pointe importée) et les réglages. Ce test
// peint le même trait dans une texture avec LivePaint, exécuté tel quel, par petits lots comme pendant un geste, relit
// la texture et la compare au moteur, pixel par pixel ; il vérifie aussi les versions réduites refaites par zone.
//
// Demande Playwright (comme test:gpu-parity). Variables : LIVE_DIR (cas déjà écrits : on ne relance pas cargo),
// CHROMIUM (navigateur à utiliser).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, "../..");
const data = process.env.LIVE_DIR ?? path.join(root, "node_modules/.cache-live-paint");
if (!process.env.LIVE_DIR) {
  execFileSync("cargo", ["test", "-q", "-p", "engine", "--lib", "live_paint_fixture", "--", "--ignored"], { cwd: root, stdio: "inherit", env: { ...process.env, LUMI_LIVE_FIXTURE: data } });
}

const modules = {};
for (const [name, file] of [["live-paint.js", "viewport/live-paint.ts"], ["brush-path.js", "lib/brush-path.ts"], ["shaders.js", "viewport/shaders.ts"]]) {
  const source = fs.readFileSync(path.join(here, "../src", file), "utf8");
  modules[name] = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText.replace(/from "\.\.?\/(?:lib\/)?(\w[\w-]*)"/g, 'from "./$1.js"');
}
modules["harness.js"] = `
import { LivePaint } from "./live-paint.js";
import { DabPath } from "./brush-path.js";
import { VERTEX_SHADER } from "./shaders.js";
const bytes = (b64) => Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
window.paint = (c, beforeB64) => {
  const [ax, ay, w, h] = c.area;
  const canvas = document.createElement("canvas");
  const gl = canvas.getContext("webgl2", { antialias: false });
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
  const float = gl.getExtension("EXT_color_buffer_float") !== null;
  const levels = Math.floor(Math.log2(Math.max(w, h))) + 1;
  const texture = gl.createTexture();
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texStorage2D(gl.TEXTURE_2D, levels, gl.RGBA8, w, h);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, bytes(beforeB64));
  gl.generateMipmap(gl.TEXTURE_2D);
  const paint = new LivePaint(gl, link, float);
  const target = { texture, width: w, height: h, levels, dx: c.offset[0] + ax, dy: c.offset[1] + ay, docWidth: c.doc[0], docHeight: c.doc[1] };
  const base = { erase: c.erase, color: c.color, opacity: c.opacity };
  const spec = c.tip
    ? { ...base, kind: "tip", tipId: 1, tip: { width: c.tip.width, height: c.tip.height, mask: new Uint8Array(c.tip.mask) }, flow: c.flow > 0 ? Math.min(1, Math.max(0.01, c.flow)) : 1,
        dual: c.dual ? { tipId: 2, tip: { width: c.dual.tip.width, height: c.dual.tip.height, mask: new Uint8Array(c.dual.tip.mask) }, mode: c.dual.mode } : null,
        texture: c.texture ? { ...c.texture, pattern: { width: c.texture.pattern.width, height: c.texture.pattern.height, gray: new Uint8Array(c.texture.pattern.gray) } } : null }
    : { ...base, kind: "round", hardness: c.hardness, accumulate: c.hardness < 1 };
  // Deux traits de suite sur le même LivePaint : le second réutilise les textures de travail du premier, salies par un
  // trait ailleurs (celui-ci est jeté : cancel() remet la texture du calque comme avant). Le trait mesuré est le second.
  paint.begin(target, spec);
  if (c.tip) paint.addTipStamps(c.stamps.slice(0, 5).map((s) => ({ ...s, x: s.x + 40, y: s.y - 30 })), []);
  else paint.addDabs([{ ax: 30 + target.dx, ay: 40 + target.dy, ar: 20, bx: 120 + target.dx, by: 60 + target.dy, br: 20 }]);
  paint.render();
  paint.cancel();
  paint.begin(target, spec);
  // Par lots, une recomposition après chacun : comme les images successives d'un geste.
  if (c.tip) {
    for (let i = 0; i < c.stamps.length; i += 9) {
      paint.addTipStamps(c.stamps.slice(i, i + 9), i === 0 ? c.duals : []);
      paint.render();
    }
  } else {
    const spacing = c.spacing > 0 ? Math.min(5, Math.max(0.01, c.spacing)) : 0.1;
    const path = new DabPath(c.size, c.pressureSize, c.hardness < 1 ? spacing : null);
    c.points.forEach((p, i) => {
      paint.addDabs(path.add(p));
      if (i % 5 === 4) paint.render();
    });
    paint.addDabs(path.finish());
    paint.render();
  }
  paint.end();
  const framebuffer = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
  const read = (level) => {
    const [lw, lh] = [Math.max(1, w >> level), Math.max(1, h >> level)];
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, level);
    const out = new Uint8Array(lw * lh * 4);
    gl.readPixels(0, 0, lw, lh, gl.RGBA, gl.UNSIGNED_BYTE, out);
    return out;
  };
  const level0 = read(0);
  const level1 = read(1);
  // Versions réduites refaites par zone : le niveau 1 doit être la moyenne 2 × 2 du niveau 0 final.
  let mipGap = 0;
  const [w1, h1] = [w >> 1, h >> 1];
  for (let y = 0; y < h1; y++)
    for (let x = 0; x < w1; x++)
      for (let k = 0; k < 4; k++) {
        const at = (xx, yy) => level0[(yy * w + xx) * 4 + k];
        const mean = (at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1)) / 4;
        mipGap = Math.max(mipGap, Math.abs(level1[(y * w1 + x) * 4 + k] - mean));
      }
  return { pixels: Array.from(level0), mipGap, glError: gl.getError(), float };
};
window.harnessReady = true;
`;

const playwright = await import(process.env.PLAYWRIGHT ?? "playwright");
const chromium = playwright.chromium ?? playwright.default.chromium;
const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined, args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"] });
const page = await browser.newPage();
page.on("pageerror", (e) => console.log("erreur de page :", String(e)));
await page.route("http://live.test/**", (route) => {
  const name = route.request().url().split("/").pop();
  if (name === "index.html") return route.fulfill({ contentType: "text/html", body: `<!doctype html><script type="module" src="harness.js"></script>` });
  if (modules[name]) return route.fulfill({ contentType: "text/javascript", body: modules[name] });
  return route.fulfill({ status: 404, body: "" });
});
await page.goto("http://live.test/index.html");
await page.waitForFunction(() => window.harnessReady === true);

/**
 * Tolérance (valeurs 0-255). Pinceau rond flou : le moteur arrondit chaque dépôt avec un tramage tiré au hasard
 * (différent d'un trait à l'autre, voir `round_dab`), la carte graphique garde la valeur exacte : quelques niveaux
 * d'écart au plus, sans biais. Le reste suit les mêmes formules en entiers : arrondis des flottants près.
 */
const tolerance = (name) => (name.startsWith("rond-flou") ? 4 : 2);
const names = JSON.parse(fs.readFileSync(path.join(data, "index.json"), "utf8"));
let failures = 0;
for (const name of names) {
  const c = JSON.parse(fs.readFileSync(path.join(data, `${name}.json`), "utf8"));
  const before = fs.readFileSync(path.join(data, `${name}.before`));
  const want = fs.readFileSync(path.join(data, `${name}.after`));
  const result = await page.evaluate(([c, b64]) => window.paint(c, b64), [c, before.toString("base64")]);
  let worst = 0;
  let over = 0;
  let painted = 0;
  let bias = 0;
  for (let i = 0; i < want.length; i++) {
    const gap = result.pixels[i] - want[i];
    worst = Math.max(worst, Math.abs(gap));
    bias += gap;
    if (Math.abs(gap) > 1) over += 1;
    if (want[i] !== before[i]) painted += 1;
  }
  const ok = worst <= tolerance(name) && result.mipGap <= 1 && result.glError === 0 && painted > 0;
  if (!ok) failures += 1;
  console.log(`${ok ? "ok  " : "ÉCHEC"} ${name.padEnd(24)} écart max ${worst} (tolérance ${tolerance(name)}), > 1 : ${over} valeurs, biais moyen ${(bias / want.length).toFixed(4)}, versions réduites ${result.mipGap.toFixed(2)}, peintes ${painted}${result.glError ? `, erreur GL ${result.glError}` : ""}`);
}
await browser.close();
assert.equal(failures, 0, `${failures} cas hors tolérance`);
console.log(`${names.length} traits identiques au moteur`);
