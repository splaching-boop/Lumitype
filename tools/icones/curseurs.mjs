// Rend en PNG les curseurs préparés par generer.py. Préalable : npm i --no-save @resvg/resvg-js (ou chemin RESVG).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
const here = path.dirname(fileURLToPath(import.meta.url));
const { Resvg } = await import(process.env.RESVG ?? "@resvg/resvg-js");
const work = path.join(here, ".curseurs");
const out = path.join(here, "../../ui/src/assets/cursors");
fs.mkdirSync(out, { recursive: true });
for (const file of fs.readdirSync(work).filter((f) => f.endsWith(".svg"))) {
  const png = new Resvg(fs.readFileSync(path.join(work, file), "utf8"), { fitTo: { mode: "width", value: 32 } }).render().asPng();
  fs.writeFileSync(path.join(out, file.replace(/\.svg$/, ".png")), png);
}
fs.rmSync(work, { recursive: true, force: true });
console.log("curseurs écrits dans", out);
