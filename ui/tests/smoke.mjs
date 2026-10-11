// Test de fumée : monte l'interface compilée dans un DOM simulé.
// Il attrape les erreurs d'exécution au démarrage (écran vide) que ni la compilation
// ni la vérification de types ne voient. Lancer avec : npm run smoke
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";

const assets = path.join(path.dirname(fileURLToPath(import.meta.url)), "../../dist-smoke/assets");
// L'application compte deux pages (l'interface et la bannière de lancement) : on teste l'interface.
const bundle = fs.readdirSync(assets).find((file) => file.startsWith("main") && file.endsWith(".js"));

/**
 * Les deux pages partagent un petit morceau de code, que le paquet principal importe. `eval` ne sait pas
 * importer : chaque morceau est donc inséré sur place, enfermé dans une fonction qui rend ses exportations.
 */
function inlineChunks(code) {
  return code.replace(/import\s*\{([^}]*)\}\s*from\s*"\.\/([^"]+)";?/g, (_, names, file) => {
    const chunk = fs.readFileSync(path.join(assets, file), "utf8");
    const exported = /export\s*\{([^}]*)\};?\s*$/.exec(chunk);
    const pairs = (exported?.[1] ?? "").split(",").map((entry) => entry.trim().split(/\s+as\s+/)).filter((pair) => pair[0]);
    const body = inlineChunks(chunk.replace(/export\s*\{[^}]*\};?\s*$/, ""));
    const table = pairs.map(([local, alias]) => `${JSON.stringify(alias ?? local)}: ${local}`).join(", ");
    const wanted = names.split(",").map((entry) => entry.trim().split(/\s+as\s+/)).filter((pair) => pair[0]);
    const picks = wanted.map(([name, alias]) => `${JSON.stringify(name)}: ${alias ?? name}`).join(", ");
    return `const { ${picks} } = (() => { ${body}\n return { ${table} }; })();`;
  });
}
if (!bundle) throw new Error("Bundle introuvable : la compilation a-t-elle réussi ?");

const dom = new JSDOM('<!doctype html><html><body><div id="app"></div></body></html>', {
  runScripts: "outside-only",
  pretendToBeVisual: true,
  url: "http://tauri.localhost/",
});
const { window } = dom;
const errors = [];
process.on("uncaughtException", (error) => errors.push(error.stack ?? String(error)));
process.on("unhandledRejection", (reason) => errors.push(reason?.stack ?? String(reason)));
window.addEventListener("error", (event) => errors.push(event.error?.stack ?? event.message));
window.console.error = (...args) => errors.push(args.map(String).join(" "));

// Ce que jsdom ne fournit pas, et un moteur Rust factice.
window.ResizeObserver = class {
  observe() {}
  disconnect() {}
};
window.HTMLDialogElement.prototype.showModal ??= function showModal() {};
window.HTMLDialogElement.prototype.close ??= function close() {};
window.HTMLCanvasElement.prototype.getContext = () => null;
// Masques des icônes, relevés au moment où l'interface les pose (jsdom ne les conserve pas).
const masks = [];
const setProperty = window.CSSStyleDeclaration.prototype.setProperty;
window.CSSStyleDeclaration.prototype.setProperty = function (name, value, priority) {
  if (/mask-image$/.test(name) && value) masks.push(String(value));
  return setProperty.call(this, name, value, priority);
};
// Svelte pose souvent tout le style d'un coup (`style.cssText`) : on y relève aussi chaque `mask-image: …`.
const cssText = Object.getOwnPropertyDescriptor(window.CSSStyleDeclaration.prototype, "cssText");
Object.defineProperty(window.CSSStyleDeclaration.prototype, "cssText", {
  ...cssText,
  set(text) {
    for (const [, value] of String(text).matchAll(/(?:^|;)\s*(?:-webkit-)?mask-image:\s*(url\(.*?\))(?=\s*;\s*[a-z-]+\s*:|\s*;?\s*$)/gs)) masks.push(value);
    cssText.set.call(this, text);
  },
});
window.__TAURI_INTERNALS__ = {
  // Préférences absentes, abonnements aux événements acceptés, tout le reste refusé.
  invoke: async (command) => (command === "load_settings" ? null : command.startsWith("plugin:event|") ? 0 : Promise.reject(`commande factice : ${command}`)),
  transformCallback: () => 0,
  metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
};

const source = inlineChunks(fs.readFileSync(path.join(assets, bundle), "utf8"))
  .replace(/import\.meta/g, "({})")
  .replace(/^export\s+default\s+/m, "")
  .replace(/export\s*\{[^}]*\};?/g, "");
window.eval(`(async () => {${source}})()`);
await new Promise((resolve) => setTimeout(resolve, 800));

// Icônes dessinées (assets/icons) : leur adresse doit être entre guillemets dans le masque CSS. Sans guillemets, une
// icône intégrée en `data:image/svg+xml,…` (apostrophes, parenthèses) rend le `url(…)` invalide : carré plein.
// jsdom ignore la propriété `mask-image` : on juge donc les valeurs que l'interface lui confie (relevées plus haut).
const icons = window.document.querySelectorAll("span.custom").length;
const unquoted = masks.filter((value) => !/^url\(".*"\)$/s.test(value));
if (icons === 0 || masks.length === 0 || unquoted.length > 0) {
  console.error(`ÉCHEC — icônes : ${icons}, masques : ${masks.length}, dont ${unquoted.length} sans guillemets (ex. ${unquoted[0]?.slice(0, 60)})`);
  process.exit(1);
}
// Fenêtre native OpenGL : le sélecteur qui la retire ne doit désigner que des éléments flottants. S'il attrape un conteneur
// permanent de l'interface (un `.menu` de la barre de menus, par exemple), la fenêtre se croit retirée pour toujours
// et n'est jamais créée — sans la moindre erreur, puisque c'est le chemin nominal.
// Le message d'erreur WebGL attendu ici affiche un bandeau flottant, qui doit bien retirer la fenêtre : on le ferme
// d'abord pour juger l'interface au repos.
for (const node of window.document.querySelectorAll(".toast button")) node.click();
await new Promise((resolve) => setTimeout(resolve, 50));
const mounted = window.document.getElementById("app").children.length;
// WebGL n'existe pas dans jsdom : ce message est attendu et prouve que l'erreur est bien affichée, pas avalée.
const unexpected = errors.filter((message) => !message.includes("WebGL2 indisponible"));
if (mounted === 0 || unexpected.length > 0) {
  console.error(`ÉCHEC — éléments montés : ${mounted}\n${unexpected.join("\n---\n")}`);
  process.exit(1);
}
console.log(`OK — interface montée (${window.document.querySelectorAll("button").length} boutons, ${icons} icônes), aucune erreur au démarrage.`);
process.exit(0);
