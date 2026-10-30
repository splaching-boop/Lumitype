// Catégories de formats personnelles (lib/custom-formats.ts) et leur place dans l'historique des formats
// (lib/recent-formats.ts), sans navigateur : npm run test:formats
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";
const here = path.dirname(fileURLToPath(import.meta.url));
const lib = path.join(here, "../src/lib");
const out = path.join(here, "../../node_modules/.cache-custom-formats");
fs.mkdirSync(out, { recursive: true });
const js = (file) => ts.transpileModule(fs.readFileSync(path.join(lib, file), "utf8"), { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
// L'IPC est remplacé par des réglages en mémoire : `globalThis.__settings` (nom → texte).
const ipc = "const api = { loadSettings: async (n) => globalThis.__settings[n] ?? null, saveSettings: async (n, c) => { globalThis.__settings[n] = c; } };";
globalThis.__settings = {};
fs.writeFileSync(path.join(out, "formats.mjs"), js("formats.ts"));
for (const file of ["custom-formats", "recent-formats"]) {
  fs.writeFileSync(
    path.join(out, `${file}.mjs`),
    js(`${file}.ts`)
      .replace(/from "\.\/formats"/, 'from "./formats.mjs"')
      .replace(/import \{ api \} from "\.\/ipc";/, ipc),
  );
}
const F = await import(pathToFileURL(path.join(out, "formats.mjs")).href);
const C = await import(pathToFileURL(path.join(out, "custom-formats.mjs")).href);
const R = await import(pathToFileURL(path.join(out, "recent-formats.mjs")).href);
const { get } = await import("svelte/store");

// ---- Lecture tolérante.
assert.deepEqual(C.parseCustomFormats(null), []);
assert.deepEqual(C.parseCustomFormats("pas du json {"), []);
assert.deepEqual(C.parseCustomFormats('"texte"'), []);
assert.deepEqual(C.parseCustomFormats('{"version":1,"categories":"non"}'), []);
const read = C.parseCustomFormats(
  JSON.stringify({
    version: 1,
    categories: [
      {
        id: "perso-cat-a",
        label: "  Client   Dupont ",
        formats: [
          { id: "perso-x", label: "Bandeau site", width: 1920, height: 600, dpi: 72 },
          { id: "perso-x", label: "Doublon", width: 10, height: 10, dpi: 72, transparent: true }, // identifiant repris
          { id: "perso-y", label: "Géant", width: 99999, height: 0.4, dpi: -3 }, // ramené dans les bornes
          { id: "perso-z", label: "", width: 10, height: 10 }, // sans nom : ignoré
          { id: "perso-w", label: "Cassé", width: "1920", height: 10 }, // largeur illisible : ignoré
          null,
          { id: "ecran-hd", label: "Collision", width: 5, height: 5, dpi: 72 }, // identifiant du catalogue : remplacé
        ],
      },
      { id: "perso-cat-a", label: "Même identifiant", formats: "rien" },
      { label: "" },
      "texte",
      { id: 4, label: "Sans identifiant" },
    ],
  }),
);
assert.deepEqual(
  read.map((c) => c.label),
  ["Client Dupont", "Même identifiant", "Sans identifiant"],
);
const [dupont] = read;
assert.deepEqual(
  dupont.formats.map((f) => [f.label, f.width, f.height, f.dpi, f.transparent]),
  [
    ["Bandeau site", 1920, 600, 72, false],
    ["Doublon", 10, 10, 72, true],
    ["Géant", 30000, 1, 1, false],
    ["Collision", 5, 5, 72, false],
  ],
);
const ids = read.flatMap((c) => [c.id, ...c.formats.map((f) => f.id)]);
assert.equal(new Set(ids).size, ids.length, "identifiants uniques");
assert.equal(dupont.id, "perso-cat-a");
assert.equal(dupont.formats[0].id, "perso-x");
assert.ok(ids.every((id) => id.startsWith("perso-")));
assert.ok(!ids.includes("ecran-hd"));
// Une liste nue (sans enveloppe) est acceptée ; le texte enregistré se relit à l'identique.
assert.deepEqual(C.parseCustomFormats(JSON.stringify(read)), read);
assert.deepEqual(C.parseCustomFormats(C.serializeCustomFormats(read)), read);

// ---- Ajout, renommage, modification, duplication, déplacement, suppression.
let list = [];
let r = C.addCategory(list, "  ");
assert.equal(r.id, null, "nom vide : rien n'est créé");
r = C.addCategory(list, "Client Dupont");
list = r.list;
const a = r.id;
r = C.addCategory(list, "Mes affiches");
list = r.list;
const b = r.id;
assert.deepEqual(
  list.map((c) => c.label),
  ["Client Dupont", "Mes affiches"],
);
const draft = { label: "Bandeau site", width: 1920, height: 600, dpi: 72, transparent: false };
assert.equal(C.addFormat(list, a, { ...draft, width: 0 }).id, null, "largeur hors bornes refusée");
assert.equal(C.addFormat(list, a, { ...draft, width: 30001 }).id, null);
assert.equal(C.addFormat(list, a, { ...draft, dpi: 1.5 }).id, null);
assert.equal(C.addFormat(list, "inconnue", draft).id, null);
r = C.addFormat(list, a, draft);
const before = list;
list = r.list;
const banner = r.id;
assert.equal(before[0].formats.length, 0, "fonctions pures : l'ancienne liste n'a pas bougé");
r = C.addFormat(list, a, { label: "Flyer A5", width: 1748, height: 2480, dpi: 300, transparent: false });
list = r.list;
const flyer = r.id;
list = C.renameCategory(list, a, "Client Durand");
assert.equal(list[0].label, "Client Durand");
assert.equal(C.renameCategory(list, a, "   "), list, "nom vide : inchangé");
list = C.updateFormat(list, banner, { ...draft, label: "Bandeau large", width: 2400, transparent: true });
assert.deepEqual(list[0].formats[0], { id: banner, label: "Bandeau large", width: 2400, height: 600, dpi: 72, transparent: true });
r = C.duplicateFormat(list, banner, "(copie)");
list = r.list;
assert.deepEqual(
  list[0].formats.map((f) => f.label),
  ["Bandeau large", "Bandeau large (copie)", "Flyer A5"],
);
assert.notEqual(r.id, banner);
list = C.moveFormat(list, flyer, b);
assert.deepEqual(
  list.map((c) => c.formats.map((f) => f.label)),
  [["Bandeau large", "Bandeau large (copie)"], ["Flyer A5"]],
);
assert.equal(list[1].formats[0].id, flyer, "déplacé : même identifiant");
assert.equal(C.moveFormat(list, flyer, b), list, "déjà dans cette catégorie : inchangé");
list = C.removeFormat(list, r.id);
assert.deepEqual(
  list[0].formats.map((f) => f.label),
  ["Bandeau large"],
);
list = C.removeCategory(list, b);
assert.deepEqual(
  list.map((c) => c.label),
  ["Client Durand"],
);

// ---- Réglages : enregistrés sous « formats-perso », publiés dans le catalogue (après les plateformes).
await C.loadCustomFormats();
let made = await C.editCustomFormats((l) => {
  const out = C.addCategory(l, "Client Dupont");
  return { list: out.list, result: out.id };
});
const cat = made;
const fmt = await C.editCustomFormats((l) => {
  const out = C.addFormat(l, cat, { label: "Story", width: 1200, height: 628, dpi: 72, transparent: false });
  return { list: out.list, result: out.id };
});
assert.equal(C.parseCustomFormats(globalThis.__settings["formats-perso"])[0].formats[0].id, fmt);
const all = get(F.formatCategories);
assert.equal(all.length, F.FORMAT_CATEGORIES.length + 1);
assert.equal(all.at(-1).group, "custom");
assert.equal(all.at(-1).formats[0].custom, true);
assert.equal(F.formatById(fmt)?.label, "Story");
assert.equal(F.labelOf(F.formatById(fmt), () => "traduit"), "Story", "nom personnel : jamais traduit");
assert.equal(F.labelOf(F.formatById("story"), () => "traduit"), "traduit");
assert.equal(F.categoryOf(fmt)?.label, "Client Dupont");
// Recherche : par nom et par taille, comme le catalogue.
assert.ok(F.matchesFormat(F.formatById(fmt), all.at(-1), "1200x628"));
assert.ok(F.matchesFormat(F.formatById(fmt), all.at(-1), "dupont"));

// ---- Historique : reconnu par son identifiant, jamais par sa seule taille.
const used = R.fromFormat(F.formatById(fmt));
assert.deepEqual(used, { width: 1200, height: 628, dpi: 72, transparent: false, formatId: fmt });
assert.equal(R.describe(used).format?.label, "Story");
assert.equal(R.describe({ width: 1200, height: 628, dpi: 72, transparent: false }).format, null, "taille libre identique : reste une taille libre");
// Format personnel transparent : le document est créé transparent.
await C.editCustomFormats((l) => C.updateFormat(l, fmt, { label: "Vignette", width: 800, height: 800, dpi: 72, transparent: true }));
assert.equal(R.fromFormat(F.formatById(fmt)).transparent, true);
// Redimensionné ou supprimé : l'ancienne entrée de l'historique redevient une taille libre.
assert.equal(R.describe(used).format, null);
const used2 = R.fromFormat(F.formatById(fmt));
assert.equal(R.describe(used2).format?.label, "Vignette");
await C.editCustomFormats((l) => C.removeFormat(l, fmt));
assert.equal(R.describe(used2).format, null);
assert.equal(F.formatById(fmt), undefined);
// Une nouvelle lecture des réglages enregistrés redonne la même liste.
assert.deepEqual(C.parseCustomFormats(globalThis.__settings["formats-perso"]), C.customFormatList());

// ---- Lisibilité : un texte qui n'est pas du JSON (ou pas une liste de catégories) est « illisible ».
assert.deepEqual(C.readCustomFormats(null), { list: [], readable: true }, "pas encore de réglages : lisible, vide");
assert.equal(C.readCustomFormats("pas du json {").readable, false);
assert.equal(C.readCustomFormats('"texte"').readable, false);
assert.equal(C.readCustomFormats('{"version":1,"categories":"non"}').readable, false);
assert.equal(C.readCustomFormats('{"version":1,"categories":[]}').readable, true);

// ---- Copie de sauvegarde : chaque écriture qui remplace des catégories garde l'ancien texte dans « formats-perso-bak ».
{
  const previous = globalThis.__settings["formats-perso"];
  assert.ok(C.parseCustomFormats(previous).length > 0);
  await C.editCustomFormats((l) => C.addCategory(l, "Sauvegardée").list);
  assert.equal(globalThis.__settings[C.BACKUP_SETTINGS_NAME], previous, "l'ancien contenu est copié avant d'être remplacé");
  assert.notEqual(globalThis.__settings["formats-perso"], previous);
}

// ---- Réglages abîmés (pas du JSON) : jamais écrasés, la modification reste en mémoire. (Module neuf : lu une seule
// fois par lancement.)
{
  const broken = "{ abîmé à la main";
  globalThis.__settings = { "formats-perso": broken };
  const fresh = await import(pathToFileURL(path.join(out, "custom-formats.mjs")).href + "?abime");
  await fresh.loadCustomFormats();
  assert.equal(fresh.customFormatsUnreadable(), true);
  await fresh.editCustomFormats((l) => fresh.addCategory(l, "Nouvelle").list);
  assert.equal(globalThis.__settings["formats-perso"], broken, "le fichier abîmé n'est pas écrasé au premier ajout");
  assert.deepEqual(fresh.customFormatList().map((c) => c.label), ["Nouvelle"], "le changement reste en mémoire");
  // Un échec de lecture du disque aussi.
  const failing = await import(pathToFileURL(path.join(out, "custom-formats.mjs")).href + "?echec");
  const load = globalThis.__settings;
  globalThis.__settings = new Proxy({}, { get: () => { throw new Error("disque"); } });
  await failing.loadCustomFormats();
  globalThis.__settings = load;
  assert.equal(failing.customFormatsUnreadable(), true);
}

// ---- Import (fusion) : catégories ajoutées après les siennes, identifiants renouvelés en cas de doublon, noms en
// double signalés (l'interface demande confirmation), fichier illisible refusé sans rien changer.
{
  const mine = C.parseCustomFormats(JSON.stringify([{ id: "perso-cat-a", label: "Client Dupont", formats: [{ id: "perso-x", label: "Bandeau", width: 100, height: 50, dpi: 72 }] }]));
  const file = C.serializeCustomFormats([
    { id: "perso-cat-a", label: "client dupont", formats: [{ id: "perso-x", label: "Bandeau", width: 100, height: 50, dpi: 72, transparent: false }] },
    { id: "perso-cat-b", label: "Affiches", formats: [{ id: "perso-y", label: "A3", width: 3508, height: 4961, dpi: 300, transparent: false }] },
  ]);
  const merged = C.mergeCustomFormats(mine, file);
  assert.equal(merged.readable, true);
  assert.equal(merged.added, 2);
  assert.deepEqual(merged.duplicates, ["client dupont"], "nom déjà pris (casse ignorée)");
  assert.deepEqual(merged.list.map((c) => c.label), ["Client Dupont", "client dupont", "Affiches"]);
  assert.equal(merged.list[0], mine[0], "ses catégories ne bougent pas");
  const all = merged.list.flatMap((c) => [c.id, ...c.formats.map((f) => f.id)]);
  assert.equal(new Set(all).size, all.length, "identifiants renouvelés : tous uniques");
  assert.notEqual(merged.list[1].id, "perso-cat-a");
  assert.notEqual(merged.list[1].formats[0].id, "perso-x");
  assert.equal(merged.list[2].id, "perso-cat-b", "un identifiant libre est gardé");
  const bad = C.mergeCustomFormats(mine, "pas du json");
  assert.equal(bad.readable, false);
  assert.equal(bad.list, mine, "illisible : rien ne change");
  assert.equal(C.mergeCustomFormats(mine, "[]").added, 0);
  // Liste pleine : le reste du fichier est laissé de côté, et compté.
  let full = [];
  for (let i = 0; i < C.MAX_CATEGORIES - 1; i++) full = C.addCategory(full, `C${i}`).list;
  const capped = C.mergeCustomFormats(full, file);
  assert.equal(capped.added, 1);
  assert.equal(capped.dropped, 1);
  assert.equal(capped.list.length, C.MAX_CATEGORIES);
  // Un export se relit à l'identique par l'import dans une liste vide.
  assert.deepEqual(C.mergeCustomFormats([], C.serializeCustomFormats(mine)).list, mine);
}

console.log("formats personnels : ok");
