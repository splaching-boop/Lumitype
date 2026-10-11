// Vérifie les réglages de performance (lib/performance.svelte.ts), sans navigateur : npm run test:performance
// Plafonds de mémoire vive et vidéo (pourcentages, carte intégrée, mémoire non détectée) et lecture d'un ancien fichier.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import ts from "typescript";

const here = path.dirname(fileURLToPath(import.meta.url));
// Le module importe l'état de l'application et l'IPC pour `applyPerformance`, qui n'est pas testé ici : ces imports
// sont retirés avant la transcription, le reste du module n'en dépend pas.
const source = fs
  .readFileSync(path.join(here, "../src/lib/performance.svelte.ts"), "utf8")
  .replace(/^import .*$/gm, "");
const out = path.join(here, "../../node_modules/.cache-performance.mjs");
fs.writeFileSync(out, ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText);
const { ASSUMED_VRAM_MB, DEFAULT_PERFORMANCE, gpuMemoryBudgetMb, gpuPercentOf, isIntegratedGpu, memoryPercentOf, mergeStoredPerformance, scratchQuotaOf } = await import(pathToFileURL(out).href);

// ---- Défauts et bornes des curseurs.
assert.equal(DEFAULT_PERFORMANCE.memoryPercent, 50);
assert.equal(DEFAULT_PERFORMANCE.gpuMemoryPercent, 50);
assert.equal(memoryPercentOf({}), 50, "absent : 50 %");
assert.equal(memoryPercentOf({ memoryPercent: 5 }), 10, "borné à 10 %");
assert.equal(memoryPercentOf({ memoryPercent: 95 }), 90, "borné à 90 %");
assert.equal(gpuPercentOf({ gpuMemoryPercent: 95 }), 80, "mémoire vidéo bornée à 80 %");
assert.equal(gpuPercentOf({ gpuMemoryPercent: 3 }), 10);

// ---- Mémoire vidéo : carte dédiée, la part réglée de ce qui est détecté.
const perf = { ...DEFAULT_PERFORMANCE };
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: 24576, vramShared: false, ramMb: 65536 }), 12288, "50 % de 24 Gio");
assert.equal(isIntegratedGpu({ vramMb: 24576, vramShared: false }), false);
// Carte intégrée (mémoire dédiée petite, ou signalée comme partagée) : min(part × VRAM, un quart de la mémoire vive).
assert.equal(isIntegratedGpu({ vramMb: 512 }), true, "moins d'un gibioctet dédié");
assert.equal(isIntegratedGpu({ vramMb: 16384, vramShared: true }), true, "partagée, même grande");
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: 16384, vramShared: true, ramMb: 32768 }), 8192, "50 % de 16 Gio partagés = 8 Gio, dans le quart de 32 Gio");
assert.equal(gpuMemoryBudgetMb({ ...perf, gpuMemoryPercent: 80 }, { vramMb: 16384, vramShared: true, ramMb: 32768 }), 8192, "80 % dépasserait le quart de la mémoire vive : borné");
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: 512, ramMb: 8192 }), 256, "petite carte : la part de sa mémoire dédiée");
// Mémoire non détectée : 2 Gio supposés.
assert.equal(ASSUMED_VRAM_MB, 2048);
assert.equal(isIntegratedGpu({ vramMb: null }), true);
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: null, ramMb: 16384 }), 1024, "50 % de 2 Gio supposés");
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: null, ramMb: 2048 }), 512, "… mais jamais plus d'un quart de la mémoire vive");
assert.equal(gpuMemoryBudgetMb(perf, { vramMb: null }), 1024, "mémoire vive inconnue aussi : pas de borne");

// ---- Lecture d'un fichier de réglages : les anciennes clés sont ignorées, les absentes gardent leur défaut.
const legacy = mergeStoredPerformance({ cacheMb: 4096, gpuMemoryMb: 2048, cores: 4, historyStates: 80, unknown: true });
assert.deepEqual(legacy, { ...DEFAULT_PERFORMANCE, cores: 4, historyStates: 80 });
assert.equal("cacheMb" in legacy, false, "l'ancien cache fixe ne revient pas dans le fichier réécrit");
assert.equal("gpuMemoryMb" in legacy, false);
assert.deepEqual(mergeStoredPerformance(null), DEFAULT_PERFORMANCE);
assert.deepEqual(mergeStoredPerformance("texte"), DEFAULT_PERFORMANCE);
assert.equal(mergeStoredPerformance({ memoryPercent: 70, gpuMemoryPercent: 30 }).memoryPercent, 70);
assert.equal(mergeStoredPerformance({ memoryPercent: 70, gpuMemoryPercent: 30 }).gpuMemoryPercent, 30);

// ---- Disque de travail de l'historique : actif, dossier par défaut, 20 Gio ; quota borné de 2 à 200 Gio.
assert.equal(DEFAULT_PERFORMANCE.scratchEnabled, true);
assert.equal(DEFAULT_PERFORMANCE.scratchDir, "");
assert.equal(scratchQuotaOf({}), 20, "absent : 20 Gio");
assert.equal(scratchQuotaOf({ scratchQuotaGb: 1 }), 2, "au moins 2 Gio");
assert.equal(scratchQuotaOf({ scratchQuotaGb: 500 }), 200, "au plus 200 Gio");
assert.equal(scratchQuotaOf({ scratchQuotaGb: Number.NaN }), 20, "valeur illisible : le défaut");
assert.equal(mergeStoredPerformance({ scratchEnabled: false, scratchDir: "/tmp/x" }).scratchEnabled, false);
assert.equal(mergeStoredPerformance({ cores: 2 }).scratchEnabled, true, "ancien fichier : disque de travail actif");

console.log("performance : OK");
