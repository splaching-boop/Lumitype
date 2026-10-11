// Rend en PNG le guide des icônes et un modèle 96 x 96 px par icône.
// Préalable : python3 tools/gabarit-icones.py, et `npm i --no-save @resvg/resvg-js`.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Resvg } from "@resvg/resvg-js";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const out = path.join(root, "ui/src/assets/icons");
const work = path.join(root, "tools/.icones");
fs.mkdirSync(path.join(out, "_modeles-png"), { recursive: true });
for (const name of JSON.parse(fs.readFileSync(path.join(work, "names.json"), "utf8")).names) {
  const png = new Resvg(fs.readFileSync(path.join(work, `${name}.svg`), "utf8"), { fitTo: { mode: "width", value: 96 } }).render().asPng();
  fs.writeFileSync(path.join(out, "_modeles-png", `${name}.png`), png);
}
const sheet = new Resvg(fs.readFileSync(path.join(out, "_gabarit-icones.svg"), "utf8"), { font: { loadSystemFonts: true, defaultFontFamily: "DejaVu Sans" } }).render().asPng();
fs.writeFileSync(path.join(out, "_gabarit-icones.png"), sheet);
console.log("PNG écrits dans", out);
