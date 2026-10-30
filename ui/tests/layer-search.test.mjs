// Vérifie la recherche du panneau Calques (lib/layer-search.ts), sans navigateur : npm run test:layer-search
// Nom insensible à la casse et aux accents, pastilles (texte, pixels, fx, caché…), mode:<nom>, combinaisons en ET,
// groupes fermés ouverts le temps de la recherche, surlignage et passage d'un résultat à l'autre.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
const cache = path.join(here, "../../node_modules");
const compile = (file, out, rewrite = (s) => s) => {
  const source = fs.readFileSync(path.join(here, "../src/lib", file), "utf8");
  const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
  fs.writeFileSync(path.join(cache, out), rewrite(js));
};
compile("blend.ts", ".cache-layer-search-blend.mjs");
compile("layer-search.ts", ".cache-layer-search.mjs", (js) => js.replace(/from "\.\/blend"/, 'from "./.cache-layer-search-blend.mjs"'));
const s = await import(pathToFileURL(path.join(cache, ".cache-layer-search.mjs")).href);

// Calques du panneau, de haut en bas (comme `DocumentInfo.layers`) : ceux de la maquette.
const NO_LOCKS = { transparency: false, pixels: false, position: false, all: false };
let next = 1;
const layer = (name, extra = {}) => ({
  id: next++,
  parent: null,
  name,
  isGroup: false,
  expanded: false,
  visible: true,
  blend: "normal",
  locks: NO_LOCKS,
  clipped: false,
  hasMask: false,
  text: null,
  shape: null,
  smart: null,
  adjustment: null,
  fillLayer: null,
  effects: [],
  ...extra,
});
const FX = [{ type: "dropShadow", enabled: true }];
const titre = layer("Titre « Été 2026 »", { text: { content: "Été 2026" } });
const ombreTitre = layer("Ombre du titre", { text: { content: "Été" }, effects: FX });
const groupe = layer("Groupe Illustration", { isGroup: true, expanded: false, blend: "pass_through" });
const sousGroupe = layer("Détails", { isGroup: true, expanded: false, parent: groupe.id, blend: "pass_through" });
const ombreFeuille = layer("Ombre portée feuille", { parent: groupe.id, hasMask: true, effects: FX, blend: "multiply" });
const feuille = layer("Feuille", { parent: sousGroupe.id, clipped: true });
const riso = layer("Texture riso", { parent: groupe.id, visible: false, blend: "screen", locks: { ...NO_LOCKS, pixels: true } });
const courbes = layer("Courbes", { parent: groupe.id, adjustment: { type: "curves" } });
const forme = layer("Cadre", { shape: { kind: "rect" } });
const objet = layer("Photo plage", { smart: { linked: false } });
const fond = layer("Fond uni", { fillLayer: { type: "solid" }, blend: "color_burn" });
const arriere = layer("Arrière-plan", { locks: { ...NO_LOCKS, all: true } });
const LAYERS = [titre, ombreTitre, groupe, sousGroupe, ombreFeuille, feuille, riso, courbes, forme, objet, fond, arriere];

const find = (query) => s.searchLayers(LAYERS, { chips: [], text: "", ...query });
const names = (query) => find(query).ids.map((id) => LAYERS.find((l) => l.id === id).name);

// ------------------------------------------------------------------ nom, casse et accents
assert.equal(s.normalize("Été ŒUVRE Ça"), "ete oeuvre ca");
assert.deepEqual(names({ text: "ombre" }), ["Ombre du titre", "Ombre portée feuille"], "par nom, dans l'ordre du panneau");
assert.deepEqual(names({ text: "OMBRE" }), names({ text: "ombre" }), "insensible à la casse");
assert.deepEqual(names({ text: "ete" }), ["Titre « Été 2026 »"], "« ete » trouve « Été »");
assert.deepEqual(names({ text: "été" }), ["Titre « Été 2026 »"]);
assert.deepEqual(names({ text: "arriere" }), ["Arrière-plan"]);
assert.deepEqual(names({ text: "ombre feuille" }), ["Ombre portée feuille"], "plusieurs mots : tous dans le nom, en ET");
assert.deepEqual(names({ text: "introuvable" }), []);
const counted = find({ text: "ombre" });
assert.equal(counted.total, LAYERS.length, "« n sur N » : N = tous les calques");
assert.equal(find({ text: "   " }).ids.length, 0, "espaces seules : pas de recherche");
assert.equal(s.isActive({ chips: [], text: "  " }), false);

// ------------------------------------------------------------------ pastilles
const chip = (kind) => ({ kind });
assert.deepEqual(names({ chips: [chip("text")] }), ["Titre « Été 2026 »", "Ombre du titre"]);
assert.deepEqual(names({ chips: [chip("pixels")] }), ["Ombre portée feuille", "Feuille", "Texture riso", "Arrière-plan"], "pixels : ni texte, forme, réglage, remplissage, objet ni groupe");
assert.deepEqual(names({ chips: [chip("adjustment")] }), ["Courbes"]);
assert.deepEqual(names({ chips: [chip("fill")] }), ["Fond uni"]);
assert.deepEqual(names({ chips: [chip("shape")] }), ["Cadre"]);
assert.deepEqual(names({ chips: [chip("smart")] }), ["Photo plage"]);
assert.deepEqual(names({ chips: [chip("group")] }), ["Groupe Illustration", "Détails"]);
assert.deepEqual(names({ chips: [chip("fx")] }), ["Ombre du titre", "Ombre portée feuille"]);
assert.deepEqual(names({ chips: [chip("mask")] }), ["Ombre portée feuille"]);
assert.deepEqual(names({ chips: [chip("hidden")] }), ["Texture riso"], "caché : œil fermé");
assert.deepEqual(names({ chips: [chip("locked")] }), ["Texture riso", "Arrière-plan"], "n'importe quel verrou");
assert.deepEqual(names({ chips: [chip("clipped")] }), ["Feuille"]);

// ------------------------------------------------------------------ combinaisons (ET)
assert.deepEqual(names({ chips: [chip("text"), chip("fx")] }), ["Ombre du titre"], "maquette : Texte + Avec effets = 1 sur N");
assert.deepEqual(names({ chips: [chip("fx")], text: "feuille" }), ["Ombre portée feuille"], "pastille + texte libre");
assert.deepEqual(names({ chips: [chip("text"), chip("pixels")] }), [], "deux types exclusifs : rien");
assert.deepEqual(names({ chips: [chip("pixels"), chip("locked")], text: "riso" }), ["Texture riso"]);

// ------------------------------------------------------------------ mode:
assert.equal(s.resolveMode("produit"), "multiply");
assert.equal(s.resolveMode("multiply"), "multiply");
assert.equal(s.resolveMode("PRODUIT"), "multiply");
assert.equal(s.resolveMode("prod"), "multiply", "début non ambigu");
assert.equal(s.resolveMode("superposition"), "screen");
assert.equal(s.resolveMode("screen"), "screen");
assert.equal(s.resolveMode("color_burn"), "color_burn");
assert.equal(s.resolveMode("colorburn"), "color_burn");
assert.equal(s.resolveMode("densite-couleur+"), "color_burn", "nom français, sans accents, signe gardé");
assert.equal(s.resolveMode("densité couleur -"), "color_dodge");
assert.equal(s.resolveMode("transfert"), "pass_through");
assert.equal(s.resolveMode("densite"), null, "ambigu : aucun");
assert.equal(s.resolveMode("rien"), null);
assert.deepEqual(names({ text: "mode:produit" }), ["Ombre portée feuille"], "mode: tapé dans le texte");
assert.deepEqual(names({ text: "mode:multiply" }), ["Ombre portée feuille"], "nom anglais");
assert.deepEqual(names({ chips: [{ kind: "mode", blend: "screen" }] }), ["Texture riso"], "pastille de mode");
assert.deepEqual(names({ text: "mode:inconnu" }), [], "mode inconnu : rien");
assert.equal(find({ text: "mode:" }).ids.length, LAYERS.length, "« mode: » en cours de frappe ne filtre pas encore");
assert.deepEqual(names({ chips: [chip("fill")], text: "mode:densite-couleur+ fond" }), ["Fond uni"], "pastille, mode et nom combinés");
assert.equal(s.chipLabel({ kind: "mode", blend: "multiply" }), "Mode : Produit");
assert.equal(s.chipLabel({ kind: "fx" }), "Avec effets");

// ------------------------------------------------------------------ groupes fermés
const deep = find({ text: "feuille" });
assert.deepEqual([...deep.open].sort(), [groupe.id, sousGroupe.id].sort(), "les groupes fermés qui contiennent un résultat s'ouvrent, jusqu'au plus profond");
assert.equal(find({ text: "titre" }).open.size, 0, "résultats hors groupe : rien à ouvrir");
assert.equal(find({}).open.size, 0, "recherche effacée : les groupes retrouvent leur état");
const opened = s.searchLayers(
  LAYERS.map((l) => (l.id === groupe.id ? { ...l, expanded: true } : l)),
  { chips: [], text: "feuille" },
);
assert.deepEqual([...opened.open], [sousGroupe.id], "un groupe déjà ouvert n'est pas compté");
// Parenté incohérente (cycle) : la remontée s'arrête.
const a = layer("A", { isGroup: true });
const b = layer("B", { isGroup: true, parent: a.id });
a.parent = b.id;
const looped = s.searchLayers([a, b, layer("C", { parent: b.id })], { chips: [], text: "c" });
assert.equal(looped.ids.length, 1);

// ------------------------------------------------------------------ surlignage
assert.deepEqual(s.highlight("Ombre du titre", { chips: [], text: "ombre" }), [
  { text: "Ombre", mark: true },
  { text: " du titre", mark: false },
]);
assert.deepEqual(
  s.highlight("Titre « Été 2026 »", { chips: [], text: "ete" }).filter((p) => p.mark).map((p) => p.text),
  ["Été"],
  "le surlignage garde les accents d'origine",
);
assert.deepEqual(s.highlight("Feuille", { chips: [chip("fx")], text: "" }), [{ text: "Feuille", mark: false }]);
assert.deepEqual(
  s.highlight("Ombre portée feuille", { chips: [], text: "feuille ombre mode:produit" }).filter((p) => p.mark).map((p) => p.text),
  ["Ombre", "feuille"],
);

// ------------------------------------------------------------------ suggestions et pastilles
const labels = (word, chips = []) => s.suggestions(word, chips).map((x) => x.label);
assert.ok(labels("").length >= s.FILTERS.length, "champ vide : toute la liste");
assert.deepEqual(labels("tex"), ["Type : Texte"]);
assert.deepEqual(labels("TEXT"), ["Type : Texte"], "anglais et casse");
assert.deepEqual(labels("regl"), ["Type : Réglage"], "sans accent");
assert.deepEqual(labels("verr"), ["Verrouillés"]);
assert.deepEqual(labels("cach"), ["Masqués (œil fermé)"]);
assert.deepEqual(labels("masq"), ["Avec masque", "Masqués (œil fermé)"]);
assert.deepEqual(labels("mo"), ["Mode : Produit…"]);
assert.ok(labels("o").includes("Type : Objet dynamique"));
assert.deepEqual(labels("ombre"), [], "un nom : pas de suggestion");
assert.ok(!labels("", [chip("text")]).includes("Type : Texte"), "pastille déjà posée : plus proposée");
assert.deepEqual(labels("mode:prod"), ["Mode : Produit"]);
assert.deepEqual(labels("mode:mult"), ["Mode : Produit"], "nom anglais");
assert.ok(labels("mode:").length > 20, "mode: seul : tous les modes");
assert.equal(s.currentWord("ombre tex"), "tex");
assert.equal(s.currentWord("ombre "), "");

// Tab sur une suggestion : le mot en cours devient pastille, le reste du texte est gardé.
let query = { chips: [], text: "ombre tex" };
query = s.acceptSuggestion(query, s.suggestions(s.currentWord(query.text), query.chips)[0]);
assert.deepEqual(query, { chips: [{ kind: "text" }], text: "ombre " });
query = s.acceptSuggestion({ ...query, text: "fx" }, s.suggestions("fx", query.chips)[0]);
assert.deepEqual(query, { chips: [{ kind: "text" }, { kind: "fx" }], text: "" });
assert.deepEqual(names(query), ["Ombre du titre"]);
// « Mode : Produit… » prépare « mode: », puis le mode choisi devient pastille.
query = s.acceptSuggestion({ chips: [], text: "mo" }, s.suggestions("mo", [])[0]);
assert.deepEqual(query, { chips: [], text: "mode:" });
query = s.acceptSuggestion({ chips: [], text: "mode:prod" }, s.suggestions("mode:prod", [])[0]);
assert.deepEqual(query, { chips: [{ kind: "mode", blend: "multiply" }], text: "" });

// ------------------------------------------------------------------ ↑ / ↓
assert.equal(s.stepResult([4, 7, 9], null, 1), 4);
assert.equal(s.stepResult([4, 7, 9], null, -1), 9);
assert.equal(s.stepResult([4, 7, 9], 7, 1), 9);
assert.equal(s.stepResult([4, 7, 9], 9, 1), 4, "boucle");
assert.equal(s.stepResult([4, 7, 9], 4, -1), 9);
assert.equal(s.stepResult([4, 7, 9], 100, 1), 4, "calque actif hors résultats : le premier");
assert.equal(s.stepResult([], 3, 1), null);

// ------------------------------------------------------------------ chevron d'un groupe pendant la recherche
{
  const open = new Set([5]);
  const none = new Set();
  // Groupe fermé dans le document, ouvert par la recherche : affiché ouvert ; un clic le referme sans rien envoyer.
  assert.equal(s.groupShownOpen(5, false, open, none), true);
  assert.deepEqual(s.toggleGroup(5, false, open, none), { send: null, closeTemporary: true, forgetClosed: false });
  const closed = new Set([5]);
  assert.equal(s.groupShownOpen(5, false, open, closed), false, "refermé : affiché fermé");
  // Le rouvrir l'ouvre pour de bon (il le reste après la recherche).
  assert.deepEqual(s.toggleGroup(5, false, open, closed), { send: true, closeTemporary: false, forgetClosed: true });
  // Groupe ouvert dans le document : le fermer l'envoie au moteur.
  assert.deepEqual(s.toggleGroup(6, true, open, none), { send: false, closeTemporary: false, forgetClosed: false });
  assert.deepEqual(s.toggleGroup(7, false, open, none), { send: true, closeTemporary: false, forgetClosed: true }, "fermé : on ouvre");
}

console.log("layer-search : recherche par nom, pastilles, mode:, groupes et suggestions vérifiés");
