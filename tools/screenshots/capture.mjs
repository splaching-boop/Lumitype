// Captures d'écran de l'interface pour la notice (anglais, thème Legacy, accent rouge).
//
//   node tools/screenshots/capture.mjs                 toutes les captures
//   node tools/screenshots/capture.mjs menu-file ...   seulement celles-ci
//   OUT=/tmp/essai node tools/screenshots/capture.mjs  ailleurs que dans ui/src/assets/manual/en
//
// Lance le serveur de Vite sur `ui/`, ouvre l'interface dans Chromium (Playwright) avec un faux processus Rust
// (mock-tauri.js), met l'interface dans l'état voulu, recadre sur l'élément utile et enregistre un PNG optimisé.
// Voir README.md pour ajouter une capture.
import { createRequire } from "node:module";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "../..");
const require = createRequire(path.join(root, "package.json"));
const OUT = process.env.OUT ? path.resolve(process.env.OUT) : path.join(root, "ui/src/assets/manual/en");
const MARGIN = 16; // marge autour de l'élément capturé, en pixels CSS
const MAX_BYTES = 250 * 1024;
const VIEWPORT = { width: 1440, height: 900 };

/** `require` qui cherche aussi dans les modules globaux (Playwright et sharp y sont souvent installés). */
function load(name) {
  try {
    return require(name);
  } catch {
    const { execSync } = require("node:child_process");
    const global = execSync("npm root -g").toString().trim();
    return createRequire(path.join(global, "noop.js"))(name);
  }
}
const { chromium } = load("playwright");
const sharp = load("sharp");
const { createServer } = await import(path.join(root, "node_modules/vite/dist/node/index.js"));

// ------------------------------------------------------------------ outils communs aux captures

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Module de l'interface, chargé par le serveur de Vite : la même instance que celle de l'application. */
const mod = (page, file, body) => page.evaluate(new Function("file", `return import(file).then((m) => (${body})(m));`), file);

/** Exécute une action du catalogue (identifiants de lib/actions.ts : « file.new », « edit.preferences »…). */
async function action(page, id) {
  await page.evaluate((name) => import("/src/lib/actions.ts").then((m) => m.runAction(name)), id);
  await sleep(400);
}

/** Crée un document par Fichier › Nouveau (faux moteur : l'affiche de la scène choisie). */
async function openDocument(page) {
  await action(page, "file.new");
  const dialog = page.locator("dialog[open]");
  await dialog.waitFor();
  await dialog.locator('button[type="submit"]').click();
  await page.locator("aside.right section").waitFor();
  // Tuiles reçues et dessinées, miniatures des calques chargées.
  await sleep(1500);
}

/** Ouvre un menu de la barre par son titre affiché (anglais). */
async function openMenu(page, title) {
  const button = page.locator("header.menubar .menu > button.title", { hasText: new RegExp(`^\\s*${title}\\s*$`) });
  await button.click();
  const dropdown = page.locator("header.menubar .dropdown").first();
  await dropdown.waitFor();
  await page.mouse.move(5, 880); // pas de survol qui ouvrirait un sous-menu
  await sleep(300);
  return [button, dropdown];
}

/** Vue nette du document : outil Main et aucun calque sélectionné (ni cadre ni contour sur l'image). */
async function cleanView(page) {
  await mod(page, "/src/state/app.svelte.ts", '(m) => { m.app.tool = "hand"; if (m.app.active) m.app.active.selection = []; }');
  await sleep(300);
}

/** Clique la ligne « Calques » qui porte ce nom. */
async function selectLayer(page, name) {
  await page.locator("aside.right section").getByText(name, { exact: true }).first().click();
  await sleep(300);
}

/**
 * Chaînes restées en français dans l'interface anglaise au moment de la capture (défauts de traduction signalés,
 * à corriger dans ui/src) : remplacées dans la page juste avant la prise de vue, pour que la notice anglaise reste
 * cohérente. Retirer une ligne dès que l'interface la traduit elle-même.
 */
// Textes restés en français dans l'interface, remplacés juste avant la prise de vue :
// [français, anglais, partie d'un texte (true) ou texte entier (false)]. Vide : tout passe par t().
const FRENCH_LEFTOVERS = [];

async function englishFixups(page) {
  await page.evaluate((pairs) => {
    // Pas d'anneau de focus (champ ou bouton mis au point à l'ouverture d'une boîte de dialogue) sur la capture.
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      for (const [fr, en, partial] of pairs) {
        if (partial ? node.nodeValue.includes(fr) : node.nodeValue.trim() === fr) node.nodeValue = node.nodeValue.replaceAll(fr, en);
      }
    }
    for (const input of document.querySelectorAll("input")) {
      for (const [fr, en] of pairs) if (input.value === fr) input.value = en;
    }
  }, FRENCH_LEFTOVERS);
}

// ------------------------------------------------------------------ les captures

/**
 * Chaque capture : `scene` du faux moteur (poster, print, artboards), `run(page)` qui prépare l'interface et rend
 * l'élément (ou la liste d'éléments) à cadrer ; `full: true` garde toute la fenêtre ; `margin` change la marge (0 : bord à bord).
 */
const SHOTS = {
  "home-screen": {
    full: true,
    async run(page) {
      await page.locator(".home-host").waitFor();
      await sleep(1200);
    },
  },
  // Contrôle seulement (OUT=… conseillé) : la notice rendue, ouverte sur une section illustrée.
  "check-manual": {
    full: true,
    async run(page) {
      await page.locator(".home-host").getByText("Manual", { exact: true }).first().click();
      await sleep(1500);
      await page.locator(".home-host").getByText("Edit", { exact: true }).first().click().catch(() => undefined);
      await sleep(1500);
    },
  },
  myref: {
    full: true,
    async run(page) {
      await page.locator(".home-host").getByText("My ref", { exact: true }).first().click();
      await sleep(4000);
    },
  },
  ...Object.fromEntries(
    [
      ["menu-file", "File"],
      ["menu-edit", "Edit"],
      ["menu-image", "Image"],
      ["menu-layer", "Layer"],
      ["menu-select", "Selection"],
      ["menu-filter", "Filter"],
      ["menu-view", "View"],
    ].map(([name, title]) => [
      name,
      {
        async run(page) {
          await openDocument(page);
          await selectLayer(page, "Photo");
          return openMenu(page, title);
        },
      },
    ]),
  ),
  "panel-color": {
    async run(page) {
      await openDocument(page);
      await mod(page, "/src/state/app.svelte.ts", "(m) => { m.app.foreground = [255, 0, 0]; m.app.background = [255, 255, 0]; }");
      await sleep(400);
      return page.locator("aside.left > section").first();
    },
  },
  "panel-layers": {
    async run(page) {
      await openDocument(page);
      await selectLayer(page, "Photo");
      return page.locator("aside.right section").first();
    },
  },
  toolbar: {
    async run(page) {
      await openDocument(page);
      return page.locator("nav.toolbar");
    },
  },
  transform: {
    margin: 0,
    async run(page) {
      await openDocument(page);
      await selectLayer(page, "Photo");
      await action(page, "edit.freeTransform");
      await sleep(800);
      // Zone de travail (cadre et poignées) et barre d'options de transformation au-dessus.
      return [page.locator("div.options").first(), page.locator("main")];
    },
  },
  "print-overlays": {
    scene: "print",
    margin: 0,
    async run(page) {
      await openDocument(page);
      await cleanView(page);
      return page.locator("main");
    },
  },
  artboards: {
    scene: "artboards",
    margin: 0,
    async run(page) {
      await openDocument(page);
      await cleanView(page);
      return page.locator("main");
    },
  },
  "dialog-new-document": {
    async run(page) {
      await action(page, "file.new");
      return page.locator("dialog[open]");
    },
  },
  "dialog-export": {
    async run(page) {
      await openDocument(page);
      await action(page, "file.exportPng");
      await sleep(1000);
      await page.locator("dialog[open] button", { hasText: /^\s*Fit\s*$/ }).first().click();
      await sleep(1500);
      return page.locator("dialog[open]");
    },
  },
  "dialog-print-setup": {
    async run(page) {
      await openDocument(page);
      await action(page, "image.printSetup");
      return page.locator("dialog[open]");
    },
  },
  "dialog-color-settings": {
    async run(page) {
      await openDocument(page);
      await action(page, "edit.colorSettings");
      return page.locator("dialog[open]");
    },
  },
  "dialog-preferences": {
    async run(page) {
      await openDocument(page);
      await action(page, "edit.preferences");
      return page.locator("dialog[open]");
    },
  },
  "dialog-shortcuts": {
    async run(page) {
      await openDocument(page);
      await action(page, "edit.shortcuts");
      return page.locator("dialog[open]");
    },
  },
};

// ------------------------------------------------------------------ recadrage et enregistrement

async function boxOf(targets, margin = MARGIN) {
  const list = Array.isArray(targets) ? targets : [targets];
  const boxes = [];
  for (const target of list) {
    const box = await target.boundingBox();
    if (!box) throw new Error("élément à capturer introuvable ou invisible");
    boxes.push(box);
  }
  const x0 = Math.min(...boxes.map((b) => b.x)) - margin;
  const y0 = Math.min(...boxes.map((b) => b.y)) - margin;
  const x1 = Math.max(...boxes.map((b) => b.x + b.width)) + margin;
  const y1 = Math.max(...boxes.map((b) => b.y + b.height)) + margin;
  const x = Math.max(0, Math.floor(x0));
  const y = Math.max(0, Math.floor(y0));
  return { x, y, width: Math.min(VIEWPORT.width, Math.ceil(x1)) - x, height: Math.min(VIEWPORT.height, Math.ceil(y1)) - y };
}

/** PNG sans perte s'il tient sous 250 Ko ; sinon palette de plus en plus réduite (toujours du PNG). */
async function optimize(buffer) {
  let best = await sharp(buffer).png({ compressionLevel: 9, effort: 10 }).toBuffer();
  for (const colours of [256, 192, 128, 96, 64]) {
    if (best.length <= MAX_BYTES) break;
    const candidate = await sharp(buffer).png({ palette: true, colours, quality: 95, effort: 10, dither: 0.8, compressionLevel: 9 }).toBuffer();
    if (candidate.length < best.length) best = candidate;
  }
  return best;
}

// ------------------------------------------------------------------ déroulement

const wanted = process.argv.slice(2);
const names = wanted.length ? wanted : Object.keys(SHOTS).filter((n) => !n.startsWith("check-")); // « check-… » : contrôle, sur demande seulement
for (const name of names) if (!SHOTS[name]) throw new Error(`capture inconnue : ${name} (connues : ${Object.keys(SHOTS).join(", ")})`);
fs.mkdirSync(OUT, { recursive: true });

// La configuration de Vite donne ses pages relativement à la racine du projet.
process.chdir(root);
const server = await createServer({
  configFile: path.join(root, "vite.config.ts"),
  root: path.join(root, "ui"),
  server: { port: 5199, strictPort: false, hmr: false },
  logLevel: "error",
});
await server.listen();
const url = server.resolvedUrls.local[0];
// WebGL2 sans carte graphique : SwiftShader (rendu logiciel d'ANGLE).
const browser = await chromium.launch({ args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"] });
const unknown = new Set();
const report = [];
let failures = 0;

for (const name of names) {
  const shot = SHOTS[name];
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, locale: "en-GB" });
  const page = await context.newPage();
  const problems = [];
  page.on("console", (message) => {
    const text = message.text();
    if (text.startsWith("[mock] inconnue")) unknown.add(text.replace(/^\[mock\] inconnue : /, ""));
    else if (message.type() === "error") problems.push(text.slice(0, 300));
  });
  page.on("pageerror", (error) => problems.push(error.message));
  await page.addInitScript({ content: `window.__MOCK_SCENE__ = ${JSON.stringify(shot.scene ?? "poster")};` });
  await page.addInitScript({ path: path.join(here, "mock-tauri.js") });
  try {
    await page.goto(url);
    await page.locator("header.menubar").waitFor();
    await sleep(1500);
    const target = await shot.run(page);
    await sleep(500);
    await englishFixups(page);
    const clip = shot.full || !target ? { x: 0, y: 0, ...VIEWPORT } : await boxOf(target, shot.margin ?? MARGIN);
    const raw = await page.screenshot({ clip, animations: "disabled" });
    const png = await optimize(raw);
    fs.writeFileSync(path.join(OUT, `${name}.png`), png);
    report.push(`${name}.png  ${Math.round(png.length / 1024)} Ko  ${clip.width}×${clip.height}`);
    if (problems.length) console.warn(`  ${name} : messages d'erreur de la page :\n    ${problems.join("\n    ")}`);
  } catch (error) {
    failures++;
    console.error(`ÉCHEC ${name} : ${error.message}`);
    await page.screenshot({ path: path.join(OUT, `${name}.failed.png`) }).catch(() => undefined);
  }
  await context.close();
}

await browser.close();
await server.close();
console.log(report.join("\n"));
if (unknown.size) console.log(`\nCommandes non simulées (réponse null) :\n  ${[...unknown].join("\n  ")}`);
process.exit(failures ? 1 : 0);
