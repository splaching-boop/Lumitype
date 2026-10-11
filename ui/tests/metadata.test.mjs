// Vérifie la construction des métadonnées d'export (lib/metadata.ts), sans navigateur : npm run test:metadata
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
// Les imports de l'état et de l'IPC ne servent qu'à la lecture/enregistrement, hors de ce test.
const source = fs
  .readFileSync(path.join(here, "../src/lib/metadata.ts"), "utf8")
  .replace(/^import .*$/gm, "");
const out = path.join(here, "../../node_modules/.cache-metadata.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
globalThis.__APP_VERSION__ = "9.9.9";
const { DEFAULT_METADATA, buildMetadata, splitKeywords, stampNow, suggestedCopyright } = await import(pathToFileURL(out).href);

// Mots-clés : virgules ou points-virgules, blancs retirés, doublons (sans casse) ignorés.
assert.deepEqual(splitKeywords(" essai, Lumitype ;lumitype,, ,photo "), ["essai", "Lumitype", "photo"]);
assert.deepEqual(splitKeywords(""), []);

// Date : forme EXIF et ISO 8601 avec le fuseau local.
const stamp = stampNow(new Date(2026, 9, 3, 2, 5, 9));
assert.equal(stamp.created, "2026:10:03 02:05:09");
assert.match(stamp.createdIso, /^2026-10-03T02:05:09[+-]\d\d:\d\d$/);

// Rien de coché ou rien de rempli : rien d'inscrit.
assert.equal(buildMetadata({ ...DEFAULT_METADATA, software: false }), null);
assert.equal(buildMetadata({ ...DEFAULT_METADATA, firstName: "Antoine", embed: false }), null);

// Champs remplis : auteur assemblé, logiciel versionné, surcharge par image, date seulement si demandée.
const settings = { ...DEFAULT_METADATA, firstName: " Antoine ", lastName: "Degardin", copyright: "© 2026 Antoine Degardin", contact: "https://exemple.fr", keywords: "a, b", description: "Défaut" };
const meta = buildMetadata(settings);
assert.deepEqual(meta, { author: "Antoine Degardin", copyright: "© 2026 Antoine Degardin", description: "Défaut", keywords: ["a", "b"], contact: "https://exemple.fr", software: "Lumitype 9.9.9", created: "", createdIso: "" });
const image = buildMetadata({ ...settings, date: true }, { description: "Celle-ci", keywords: "x" }, new Date(2026, 0, 2, 3, 4, 5));
assert.equal(image.description, "Celle-ci");
assert.deepEqual(image.keywords, ["x"]);
assert.equal(image.created, "2026:01:02 03:04:05");
assert.ok(image.createdIso.startsWith("2026-01-02T03:04:05"));

// Mention proposée quand le champ est vide.
assert.equal(suggestedCopyright({ ...DEFAULT_METADATA, firstName: "A", lastName: "B" }), `© ${new Date().getFullYear()} A B`);
assert.equal(suggestedCopyright(DEFAULT_METADATA), "");

console.log("OK — métadonnées d'export : mots-clés, dates, assemblage, surcharge par image.");
