// Vérifie la rotation de l'affichage (outil R), sans navigateur : node ui/tests/view-rotation.test.mjs
// (lancé aussi par npm run test:geometry).
//
// L'image est composée droite dans une texture de la taille du cadre tourné, puis dessinée tournée dans le canevas
// (PRESENT_SHADER) ; les surcouches (contour du document, cadre de transformation…) sont posées dans le cadre et
// tournées par CSS. Les deux doivent tomber exactement l'un sur l'autre, et sur ce que donnent les conversions de la
// souris (screenToDoc, docToScreen), à tout angle.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, "../src/viewport/camera.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } });
const out = path.join(here, "../../node_modules/.cache-camera.mjs");
fs.writeFileSync(out, compiled.outputText);
const c = await import(pathToFileURL(out).href);

const close = (a, b, what, eps = 1e-6) => assert.ok(Math.abs(a.x - b.x) < eps && Math.abs(a.y - b.y) < eps, `${what} : ${JSON.stringify(a)} != ${JSON.stringify(b)}`);

/** Rotation CSS d'un élément posé en (left, top), de taille w × h, `transform-origin: 50% 50%` : calcul indépendant. */
const cssRotate = (p, frame, angle) => {
  const [ox, oy] = [frame.width / 2, frame.height / 2];
  const [dx, dy] = [p.x - ox, p.y - oy];
  return { x: frame.x + ox + dx * Math.cos(angle) - dy * Math.sin(angle), y: frame.y + oy + dx * Math.sin(angle) + dy * Math.cos(angle) };
};

/** PRESENT_SHADER (shaders.ts), recopié en JavaScript : pixel physique du canevas → pixel physique de la texture. */
const presentShader = (q, dpr, viewW, viewH, frame, angle) => {
  const s = { x: q.x / dpr, y: q.y / dpr };
  const [cx, cy] = [viewW / 2, viewH / 2];
  const [dx, dy] = [s.x - cx, s.y - cy];
  const [co, si] = [Math.cos(angle), Math.sin(angle)];
  const v = { x: cx + dx * co + dy * si, y: cy - dx * si + dy * co };
  return { x: (v.x - frame.x) * dpr, y: (v.y - frame.y) * dpr };
};

const shader = fs.readFileSync(path.join(here, "../src/viewport/shaders.ts"), "utf8");
assert.match(shader, /vec2 v = u_center \+ vec2\(d\.x \* u_rot\.x \+ d\.y \* u_rot\.y, -d\.x \* u_rot\.y \+ d\.y \* u_rot\.x\);/, "formule du shader recopiée ci-dessus");
assert.match(shader, /vec2 f = \(v - u_frame\) \* u_scale;/, "formule du shader recopiée ci-dessus");

const [viewW, viewH] = [1237, 811]; // zone de travail (pixels CSS), volontairement pas ronde
const [docW, docH] = [3000, 2000];
const degrees = [0, 17, 45, 90, 180, -33, 123.4];
let reference = null;
for (const deg of degrees) {
  const angle = c.normalizeAngle((deg * Math.PI) / 180);
  const camera = c.fitCamera(docW, docH, viewW, viewH, 32, angle);
  assert.equal(c.rotationOf(camera), angle);
  const frame = c.rotatedFrame(viewW, viewH, angle);
  const frameCam = c.frameCamera(camera, frame);

  // Cadre : la zone de travail sans rotation ; tourné, un cadre FIXE (le même à tout angle), centré, à marges entières.
  if (angle === 0) assert.deepEqual(frame, { x: 0, y: 0, width: viewW, height: viewH });
  else {
    reference ??= frame;
    assert.deepEqual(frame, reference, "même cadre à tout angle : tourner ne change jamais sa géométrie");
    assert.ok(Number.isInteger(frame.x) && Number.isInteger(frame.y) && Number.isInteger(frame.width) && Number.isInteger(frame.height));
    close({ x: frame.x + frame.width / 2, y: frame.y + frame.height / 2 }, { x: viewW / 2, y: viewH / 2 }, "cadre centré");
    assert.ok(frame.width >= Math.hypot(viewW, viewH) && frame.height >= Math.hypot(viewW, viewH), "au moins la diagonale");
  }
  // Le cadre tourné recouvre toute la zone de travail : chaque coin de l'écran tombe dans la texture.
  for (const corner of [{ x: 0, y: 0 }, { x: viewW, y: 0 }, { x: viewW, y: viewH }, { x: 0, y: viewH }]) {
    const f = c.screenToFrame(corner.x, corner.y, viewW, viewH, frame, angle);
    assert.ok(f.x >= 0 && f.y >= 0 && f.x <= frame.width && f.y <= frame.height, `${deg}° : coin ${JSON.stringify(corner)} hors du cadre`);
  }

  for (const corner of [{ x: 0, y: 0 }, { x: docW, y: 0 }, { x: docW, y: docH }, { x: 0, y: docH }]) {
    const screen = c.docToScreen(camera, corner.x, corner.y, viewW, viewH);
    // 1. Contour du document (surcouche SVG dans le cadre, tourné par CSS) : exactement sur le coin.
    const inFrame = { x: frameCam.x + corner.x * frameCam.zoom, y: frameCam.y + corner.y * frameCam.zoom };
    close(cssRotate(inFrame, frame, angle), screen, `${deg}° : contour (CSS)`);
    close(c.frameToScreen(inFrame.x, inFrame.y, viewW, viewH, frame, angle), screen, `${deg}° : frameToScreen`);
    // 2. Image (composée droite avec frameCam, puis dessinée tournée) : le pixel du coin tombe au même endroit.
    close(c.screenToFrame(screen.x, screen.y, viewW, viewH, frame, angle), inFrame, `${deg}° : image (screenToFrame)`);
    for (const dpr of [1, 1.25, 2, 0.5]) {
      const texel = presentShader({ x: screen.x * dpr, y: screen.y * dpr }, dpr, viewW, viewH, frame, angle);
      // Le rendu compose le coin du document en (frameCam.x + x · zoom) · dpr, comme renderer.ts (originX + x · scale).
      close(texel, { x: (frameCam.x + corner.x * frameCam.zoom) * dpr, y: (frameCam.y + corner.y * frameCam.zoom) * dpr }, `${deg}° à ${dpr}× : shader`, 1e-5);
    }
    // 3. Souris : la conversion inverse redonne le coin.
    close(c.screenToDoc(camera, screen.x, screen.y, viewW, viewH), corner, `${deg}° : screenToDoc`, 1e-6);
  }
  // screenToDoc ∘ docToScreen = identité (et l'inverse), partout.
  for (let i = 0; i < 50; i++) {
    const p = { x: ((i * 7919) % 4000) - 500, y: ((i * 104729) % 3000) - 500 };
    const s = c.docToScreen(camera, p.x, p.y, viewW, viewH);
    close(c.screenToDoc(camera, s.x, s.y, viewW, viewH), p, `${deg}° : aller-retour`, 1e-6);
    const q = { x: (i * 37) % viewW, y: (i * 53) % viewH };
    const d = c.screenToDoc(camera, q.x, q.y, viewW, viewH);
    close(c.docToScreen(camera, d.x, d.y, viewW, viewH), q, `${deg}° : retour-aller`, 1e-6);
    const f = c.screenToFrame(q.x, q.y, viewW, viewH, frame, angle);
    close(c.frameToScreen(f.x, f.y, viewW, viewH, frame, angle), q, `${deg}° : cadre aller-retour`, 1e-6);
  }
  // Zoom à la molette, vue tournée : le point sous la souris ne bouge pas.
  const pointer = { x: 300, y: 200 };
  const before = c.screenToDoc(camera, pointer.x, pointer.y, viewW, viewH);
  const view = c.screenToView(camera, pointer.x, pointer.y, viewW, viewH);
  const zoomed = c.zoomAt(camera, camera.zoom * 1.7, view.x, view.y);
  close(c.screenToDoc(zoomed, pointer.x, pointer.y, viewW, viewH), before, `${deg}° : zoom sous la souris`, 1e-6);
}

// Mode Bitmap : même règle de seuil pour les tuiles du moteur (niveau choisi) et la carte graphique (échelle).
for (const scale of [0.3, 0.5, 0.75, 0.99, 1, 2]) {
  const level = c.levelForScale(scale, 5, true);
  assert.equal(c.finishModeFor("bitmap", level, scale), scale >= 1 ? 2 : 1, `bitmap à ${scale}`);
  assert.equal(level === 0, scale >= 1, `niveau 0 (seuillé par le moteur) seulement à 100 % ou plus : ${scale}`);
  assert.equal(c.finishModeFor("grayscale", level, scale), 1);
  assert.equal(c.finishModeFor("rgb", level, scale), 0);
}
assert.equal(c.levelForScale(0.75, 5), 0, "hors bitmap : niveau 0 entre 50 et 100 %");
assert.equal(c.finishModeFor("bitmap", 1, 2), 1, "niveau réduit (mémoire) : gris même au-delà de 100 %");

console.log("OK — rotation de l'affichage : cadre fixe et centré, image et contour au même endroit à 0°, 17°, 45°, 90°, 180°…, conversions exactes.");
