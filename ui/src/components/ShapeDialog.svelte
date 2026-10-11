<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { edit } from "../lib/engine";
  import type { ShapeData } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // « Créer une forme » : ouverte par un double-clic avec l'outil Forme. On choisit la forme, le nombre
  // de côtés d'un polygone et la taille exacte ; la forme est posée centrée sur le point cliqué.
  let dialog = $state<HTMLDialogElement>();
  let kind = $state<ShapeData["kind"]>(app.shapeKind);
  let sides = $state(app.shapeSides);
  let width = $state(400);
  let height = $state(400);

  $effect(() => {
    dialog?.showModal();
  });

  /** Sommets du polygone dans l'aperçu (même construction que le moteur : pointe en haut, étiré à la boîte). */
  const outline = $derived.by(() => {
    const n = Math.min(64, Math.max(3, Math.round(sides) || 3));
    const unit = Array.from({ length: n }, (_, i) => {
      const angle = -Math.PI / 2 + (i * 2 * Math.PI) / n;
      return [Math.cos(angle), Math.sin(angle)] as const;
    });
    const xs = unit.map((v) => v[0]);
    const ys = unit.map((v) => v[1]);
    const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
    return unit.map(([x, y]) => `${((x - x0) / (x1 - x0)) * 100},${((y - y0) / (y1 - y0)) * 100}`).join(" ");
  });

  function close(create: boolean): void {
    const at = app.shapeDialog;
    app.shapeDialog = null;
    if (!create || !at) return;
    const n = Math.min(64, Math.max(3, Math.round(sides) || 3));
    [app.shapeKind, app.shapeSides] = [kind, n];
    const [w, h] = [Math.min(16384, Math.max(2, Math.round(width) || 2)), Math.min(16384, Math.max(2, Math.round(height) || 2))];
    const { fill, ...rest } = $state.snapshot(app.shapeDefaults);
    const shape: ShapeData = { kind, sides: n, width: w, height: h, ...rest, fill: fill === undefined ? [...app.foreground, 255] : fill };
    void edit({ op: "addShape", x: Math.round(at.x - w / 2), y: Math.round(at.y - h / 2), shape, above: app.activeLayer?.id ?? null });
  }
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <h2>{t("Créer une forme")}</h2>
    <div class="body">
      <svg viewBox="-6 -6 112 112" preserveAspectRatio="xMidYMid meet" aria-hidden="true">
        {#if kind === "rect"}
          <rect width="100" height="100" />
        {:else if kind === "ellipse"}
          <ellipse cx="50" cy="50" rx="50" ry="50" />
        {:else}
          <polygon points={outline} />
        {/if}
      </svg>
      <div class="fields">
        <div class="segmented">
          <button type="button" class:on={kind === "rect"} onclick={() => (kind = "rect")}>{t("Rectangle")}</button>
          <button type="button" class:on={kind === "ellipse"} onclick={() => (kind = "ellipse")}>{t("Ellipse")}</button>
          <button type="button" class:on={kind === "polygon"} onclick={() => (kind = "polygon")}>{t("Polygone")}</button>
        </div>
        {#if kind === "polygon"}
          <label class="slider">
            <span>{t("Nombre de côtés")}</span>
            <input type="number" min="3" max="64" bind:value={sides} />
            <input type="range" min="3" max="24" step="1" bind:value={sides} />
          </label>
        {/if}
        <div class="row">
          <label>{t("Largeur (px)")} <input type="number" min="2" max="16384" bind:value={width} /></label>
          <label>{t("Hauteur (px)")} <input type="number" min="2" max="16384" bind:value={height} /></label>
        </div>
        <p>{t("3 côtés : triangle · 5 : pentagone · 6 : hexagone… La forme est centrée sur le point cliqué ; arrondi, fond et contour se règlent ensuite dans Propriétés, avec le nombre de côtés.")}</p>
      </div>
    </div>
    <footer>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill">{t("Créer")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 560px;
    box-sizing: border-box;
    padding: 22px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.5);
  }
  h2 {
    margin: 0 0 16px;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    display: flex;
    gap: 20px;
  }
  svg {
    width: 160px;
    height: 160px;
    flex-shrink: 0;
    padding: 12px;
    box-sizing: border-box;
    border-radius: var(--radius-l);
    background: var(--bg-canvas);
    fill: var(--text);
  }
  .fields {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: #0f0f11;
    border: 1px solid var(--border);
  }
  .segmented button {
    flex: 1;
    height: 28px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: #fff;
  }
  .row {
    display: flex;
    gap: 10px;
  }
  label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-3);
  }
  label.slider {
    display: grid;
    grid-template-columns: 1fr 70px;
    align-items: center;
  }
  .slider input[type="range"] {
    grid-column: 1 / -1;
    width: 100%;
  }
  input[type="number"] {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 18px;
  }
  .ghost,
  .pill {
    height: 32px;
    padding: 0 18px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
</style>
