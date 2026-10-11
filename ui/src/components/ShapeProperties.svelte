<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { rgbToHex } from "../lib/color";
  import { editLatest } from "../lib/engine";
  import type { LayerInfo, Rgb, ShapeData } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import ColorPopover from "./ColorPopover.svelte";

  // Réglages d'un calque de forme : taille, arrondi des coins, fond, contour. Tout reste
  // modifiable à tout moment ; une série de réglages ne fait qu'un état d'historique.
  let { layer, shape }: { layer: LayerInfo; shape: ShapeData } = $props();

  let picking = $state<{ target: "fill" | "stroke"; x: number; y: number } | null>(null);
  const maxRadius = $derived(Math.floor(Math.min(shape.width, shape.height) / 2));

  function change(patch: Partial<ShapeData>): void {
    const next = { ...shape, ...patch };
    // Les choix faits ici servent de point de départ à la prochaine forme.
    const { width: _w, height: _h, kind: _k, sides: _s, ...defaults } = next;
    app.shapeDefaults = defaults;
    editLatest(`forme-${layer.id}`, { op: "setShape", id: layer.id, shape: next });
  }

  const number = (event: Event & { currentTarget: HTMLInputElement }, apply: (value: number) => void) => {
    const value = event.currentTarget.valueAsNumber;
    if (Number.isFinite(value)) apply(value);
  };
  const rgb = (color: number[]): Rgb => [color[0] ?? 0, color[1] ?? 0, color[2] ?? 0];

  function toggle(target: "fill" | "stroke", on: boolean): void {
    if (!on) change({ [target]: null });
    else if (target === "fill") change({ fill: [...app.foreground, 255] });
    else change({ stroke: [...app.background, 255], strokeWidth: Math.max(1, shape.strokeWidth) });
  }

  function openPicker(event: MouseEvent, target: "fill" | "stroke"): void {
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    picking = picking?.target === target ? null : { target, x: rect.right + 12, y: rect.top - 120 };
  }
</script>

<div class="shape">
  <div class="row">
    <label>{t("Largeur (px)")} <input type="number" min="1" max="16384" value={Math.round(shape.width)} onchange={(e) => number(e, (width) => change({ width: Math.min(16384, Math.max(1, width)) }))} /></label>
    <label>{t("Hauteur (px)")} <input type="number" min="1" max="16384" value={Math.round(shape.height)} onchange={(e) => number(e, (height) => change({ height: Math.min(16384, Math.max(1, height)) }))} /></label>
  </div>

  {#if shape.kind === "polygon"}
    <label class="slider">
      <span>{t("Côtés")}</span>
      <output>{shape.sides ?? 3}</output>
      <input type="range" min="3" max="24" step="1" value={shape.sides ?? 3} oninput={(e) => change({ sides: e.currentTarget.valueAsNumber })} />
    </label>
  {/if}
  {#if shape.kind !== "ellipse"}
    <label class="slider">
      <span>{t("Arrondi des coins")}</span>
      <output>{Math.round(Math.min(shape.radius, maxRadius))} px</output>
      <input type="range" min="0" max={maxRadius} step="1" value={Math.min(shape.radius, maxRadius)} oninput={(e) => change({ radius: e.currentTarget.valueAsNumber })} />
    </label>
  {/if}

  {#each [{ target: "fill", label: "Fond", color: shape.fill }, { target: "stroke", label: "Contour", color: shape.stroke }] as const as entry (entry.target)}
    <div class="row paint">
      <label class="check"><input type="checkbox" checked={entry.color !== null} onchange={(e) => toggle(entry.target, e.currentTarget.checked)} /> {t(entry.label)}</label>
      {#if entry.color}
        <button class="color" title={t("Couleur")} onclick={(event) => openPicker(event, entry.target)}>
          <span style:background={rgbToHex(rgb(entry.color))}></span>
          {rgbToHex(rgb(entry.color))}
        </button>
      {/if}
    </div>
  {/each}

  {#if shape.stroke}
    <label class="slider">
      <span>{t("Épaisseur du contour")}</span>
      <output>{Math.round(shape.strokeWidth)} px</output>
      <input type="range" min="1" max={Math.max(1, maxRadius)} step="1" value={shape.strokeWidth} oninput={(e) => change({ strokeWidth: e.currentTarget.valueAsNumber })} />
    </label>
  {/if}
  <p>{t("Le contour est tracé vers l'intérieur : la forme garde sa taille. Ctrl+T la redimensionne sans la pixelliser ; pour la faire tourner ou la peindre, clic droit sur le calque &gt; Pixelliser.")}</p>
</div>

{#if picking}
  {@const current = picking.target === "fill" ? shape.fill : shape.stroke}
  {#if current}
    <ColorPopover color={rgb(current)} x={picking.x} y={picking.y} onchange={(color) => picking && change({ [picking.target]: [...color, current[3] ?? 255] })} onclose={() => (picking = null)} />
  {/if}
{/if}

<style>
  .shape {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 14px;
  }
  .row {
    display: flex;
    align-items: flex-end;
    gap: 10px;
  }
  .row.paint {
    align-items: center;
    justify-content: space-between;
    min-height: 26px;
  }
  label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-3);
  }
  label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
  }
  label.slider {
    display: grid;
    grid-template-columns: 1fr auto;
  }
  .slider input {
    grid-column: 1 / -1;
    width: 100%;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  input[type="number"] {
    height: 26px;
    min-width: 0;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .color {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .color:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .color span {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.35);
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
</style>
