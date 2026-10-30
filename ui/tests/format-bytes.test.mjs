// Vérifie l'affichage des tailles de fichier (lib/format-bytes.ts), sans navigateur : node ui/tests/format-bytes.test.mjs
// L'unité passe par le dictionnaire (« Mo » → « MB »), la décimale suit la langue (virgule en français).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
// Le module de traduction est remplacé par un stub : seules les clés de taille sont connues, en anglais.
const dictionary = { "{n} o": "{n} B", "{n} Ko": "{n} KB", "{n} Mo": "{n} MB" };
const stub = `
  export const i18n = { locale: "fr", info: { tag: "fr-FR" } };
  const dict = ${JSON.stringify(dictionary)};
  export function t(text, vars) {
    const out = i18n.locale === "fr" ? text : (dict[text] ?? text);
    return vars ? out.replace(/\\{(\\w+)\\}/g, (whole, name) => (name in vars ? String(vars[name]) : whole)) : out;
  }
`;
const stubPath = path.join(here, "../../node_modules/.cache-format-bytes-i18n.mjs");
fs.writeFileSync(stubPath, stub);
const source = fs.readFileSync(path.join(here, "../src/lib/format-bytes.ts"), "utf8").replace('from "./i18n.svelte"', `from ${JSON.stringify(pathToFileURL(stubPath).href)}`);
const out = path.join(here, "../../node_modules/.cache-format-bytes.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { formatBytes } = await import(pathToFileURL(out).href);
const { i18n } = await import(pathToFileURL(stubPath).href);

// ---- Français : octets, kilo-octets (au moins 1 Ko dès 1024), mébioctets à une décimale, virgule.
assert.equal(formatBytes(0), "0 o");
assert.equal(formatBytes(512), "512 o");
assert.equal(formatBytes(1024), "1 Ko");
assert.equal(formatBytes(216 * 1024), "216 Ko");
assert.equal(formatBytes(1048576), "1,0 Mo");
assert.equal(formatBytes(4.25 * 1048576), "4,3 Mo");
assert.equal(formatBytes(-5), "0 o");

// ---- Anglais : unité traduite, point décimal.
i18n.locale = "en";
i18n.info.tag = "en-GB";
assert.equal(formatBytes(512), "512 B");
assert.equal(formatBytes(216 * 1024), "216 KB");
assert.equal(formatBytes(4.25 * 1048576), "4.3 MB");

console.log("format-bytes : ok");
