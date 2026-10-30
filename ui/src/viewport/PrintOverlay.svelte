<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { api, type DocumentInfo } from "../lib/ipc";
  import { exportSpace } from "../lib/color-mode.svelte";
  import { analyzeInk, ink, inkLimitOf, printFrames, printPrefs } from "../lib/print.svelte";
  import { app } from "../state/app.svelte";
  import type { Camera } from "./camera";

  // Surcouches d'impression (Affichage › Impression) : fonds perdus (pointillés rouges, bord de la zone de travail),
  // format fini (trait fin), zone de sécurité (pointillés bleus), traits de coupe hors de la zone de travail, et
  // l'alerte d'encrage : les zones au-delà de la limite hachurées en orange, avec l'encrage sous le pointeur.
  let { info, camera }: { info: DocumentInfo; camera: Camera } = $props();

  const frames = $derived(printFrames(info));
  const sx = (x: number) => camera.x + x * camera.zoom;
  const sy = (y: number) => camera.y + y * camera.zoom;
  const rect = (r: [number, number, number, number]) => ({ x: sx(r[0]), y: sy(r[1]), width: r[2] * camera.zoom, height: r[3] * camera.zoom });

  // ---- Alerte d'encrage : analyse relancée (après une courte pause) quand le document change.
  const limit = $derived(inkLimitOf(info));
  $effect(() => {
    if (!printPrefs.inkAlert) return;
    const target = info;
    void target.revision;
    const space = exportSpace(target);
    const timer = setTimeout(() => void analyzeInk(target, space, limit), 600);
    return () => clearTimeout(timer);
  });
  const report = $derived(printPrefs.inkAlert && ink.docId === info.id ? ink.report : null);

  /** Masque des cases qui dépassent, en image (une case = un pixel), posé sur la zone hachurée. */
  const maskUrl = $derived.by(() => {
    if (!report || report.overPixels === 0) return null;
    const canvas = document.createElement("canvas");
    canvas.width = report.cols;
    canvas.height = report.rows;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    const image = ctx.createImageData(report.cols, report.rows);
    report.mask.forEach((on, i) => {
      if (on) image.data.set([255, 255, 255, 255], i * 4);
    });
    ctx.putImageData(image, 0, 0);
    return canvas.toDataURL();
  });

  // ---- Encrage sous le pointeur (pipette d'encrage), échantillonné au plus toutes les 80 ms.
  let probe = $state<{ x: number; y: number; inks: [number, number, number, number] } | null>(null);
  let probing = false;
  $effect(() => {
    const at = app.pointer;
    if (!printPrefs.inkAlert || !at || at.x < 0 || at.y < 0 || at.x >= info.width || at.y >= info.height) {
      probe = null;
      return;
    }
    if (probing) return;
    probing = true;
    const space = exportSpace(info);
    const [x, y] = [Math.floor(at.x), Math.floor(at.y)];
    void api
      .sampleColor(info.id, x, y)
      .then((rgb) => (rgb ? api.cmykOf(space, rgb) : null))
      .then((inks) => (probe = inks ? { x: at.x, y: at.y, inks } : null))
      .catch(() => (probe = null))
      .finally(() => setTimeout(() => (probing = false), 80));
  });
  const percent = (v: number) => Math.round((v * 100) / 255);
  const total = $derived(probe ? probe.inks.reduce((a, v) => a + percent(v), 0) : 0);

  /** Traits de coupe : à chaque coin du format fini, deux traits dans le prolongement des bords, hors des fonds perdus. */
  const marks = $derived.by(() => {
    if (!frames) return [];
    const [x, y, w, h] = frames.trim;
    const gap = frames.bleed * camera.zoom + 6;
    const len = 18;
    const out: { x1: number; y1: number; x2: number; y2: number }[] = [];
    for (const [cx, cy, dx, dy] of [
      [x, y, -1, -1],
      [x + w, y, 1, -1],
      [x, y + h, -1, 1],
      [x + w, y + h, 1, 1],
    ] as const) {
      const [px, py] = [sx(cx), sy(cy)];
      out.push({ x1: px + dx * gap, y1: py, x2: px + dx * (gap + len), y2: py });
      out.push({ x1: px, y1: py + dy * gap, x2: px, y2: py + dy * (gap + len) });
    }
    return out;
  });
</script>

<svg class="print-overlay" aria-hidden="true">
  {#if frames}
    {@const doc = rect([0, 0, info.width, info.height])}
    {@const trim = rect(frames.trim)}
    {#if printPrefs.showBleed && frames.bleed > 0}
      <rect class="bleed" x={doc.x} y={doc.y} width={doc.width} height={doc.height} />
      <rect class="trim" x={trim.x} y={trim.y} width={trim.width} height={trim.height} />
    {/if}
    {#if printPrefs.showSafe && frames.safe}
      {@const safe = rect(frames.safe)}
      <rect class="safe" x={safe.x} y={safe.y} width={safe.width} height={safe.height} />
    {/if}
    {#if printPrefs.showMarks}
      {#each marks as m, i (i)}<line class="mark" x1={m.x1} y1={m.y1} x2={m.x2} y2={m.y2} />{/each}
    {/if}
  {/if}
  {#if report}
    {#each report.regions.slice(0, 60) as r, i (i)}
      {@const box = rect(r)}
      <rect class="over" x={box.x - 2} y={box.y - 2} width={box.width + 4} height={box.height + 4} />
    {/each}
  {/if}
</svg>

{#if maskUrl && report}
  <!-- La grille couvre cols × rows cases de `cell` pixels (un peu plus que le document à droite et en bas) : posée
       à sa vraie taille dans un cadre à la taille du document, qui coupe le dépassement. -->
  {@const doc = rect([0, 0, info.width, info.height])}
  <div class="clip" style:left="{doc.x}px" style:top="{doc.y}px" style:width="{doc.width}px" style:height="{doc.height}px">
    <div class="hatch" style:width="{report.cols * report.cell * camera.zoom}px" style:height="{report.rows * report.cell * camera.zoom}px" style:mask-image="url({maskUrl})" style:-webkit-mask-image="url({maskUrl})"></div>
  </div>
{/if}

{#if probe}
  <div class="probe" style:left="{sx(probe.x) + 18}px" style:top="{sy(probe.y) + 18}px">
    <b>{t("Encrage")}</b>
    <span>C {percent(probe.inks[0])} · M {percent(probe.inks[1])} · J {percent(probe.inks[2])} · N {percent(probe.inks[3])}</span>
    <span class:over={total > limit}>{t("Total {total} %", { total })}{total > limit ? ` — ${t("limite {limit} %", { limit })}` : ""}</span>
    <i style:width="{Math.min(100, (total / 400) * 100)}%" class:over={total > limit}></i>
  </div>
{/if}

<style>
  .print-overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    overflow: visible;
  }
  rect {
    fill: none;
    vector-effect: non-scaling-stroke;
  }
  .bleed {
    stroke: #ff4d5e;
    stroke-width: 1;
    stroke-dasharray: 5 4;
  }
  .trim {
    stroke: rgba(255, 255, 255, 0.75);
    stroke-width: 1;
    mix-blend-mode: difference;
  }
  .safe {
    stroke: #4aa3ff;
    stroke-width: 1;
    stroke-dasharray: 5 4;
  }
  .mark {
    stroke: var(--text, #ddd);
    stroke-width: 1;
  }
  .over {
    stroke: #ffb020;
    stroke-width: 1.5;
  }
  .clip {
    position: absolute;
    overflow: hidden;
    pointer-events: none;
  }
  .hatch {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
    background: repeating-linear-gradient(45deg, rgba(255, 159, 26, 0.85) 0 3px, transparent 3px 9px);
    mask-size: 100% 100%;
    -webkit-mask-size: 100% 100%;
    mask-repeat: no-repeat;
    image-rendering: pixelated;
  }
  .probe {
    position: absolute;
    pointer-events: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 190px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    font-size: 11px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .probe b {
    color: var(--text-strong, var(--text));
  }
  .probe .over {
    color: #ffc04d;
    font-weight: 600;
  }
  .probe i {
    display: block;
    height: 4px;
    margin-top: 4px;
    border-radius: 2px;
    background: var(--accent);
  }
  .probe i.over {
    background: #ffb020;
  }
</style>
