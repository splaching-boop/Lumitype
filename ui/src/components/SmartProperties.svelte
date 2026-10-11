<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { openSmartContent, relinkSmart, replaceSmartContent, smartEdit } from "../lib/actions";
  import { edit, thumbnailOf } from "../lib/engine";
  import type { LayerInfo, SmartInfo } from "../lib/ipc";
  import { displayedSize, moveMatrix, percentLabel, proportional, resizeMatrix, rotateMatrix } from "../lib/smart-geometry";
  import { app } from "../state/app.svelte";

  // Propriétés d'un objet dynamique : son contenu, et sa transformation paramétrique (taille, rotation, position)
  // réglable au pixel près. Chaque saisie est une transformation du calque (un état d'historique) : le moteur la
  // compose avec la précédente et refait le rendu depuis l'original, sans perte.
  let { layer, smart }: { layer: LayerInfo; smart: SmartInfo } = $props();

  /** Proportions liées (chaîne) : changer L change H, et inversement. Allumée par défaut, comme dans Photoshop. */
  let linkedRatio = $state(true);

  const size = $derived(displayedSize(smart));
  const round1 = (value: number) => Math.round(value * 10) / 10;
  const scaleText = $derived.by(() => {
    const [sx, sy] = [Math.abs(smart.scaleX), Math.abs(smart.scaleY)];
    return Math.abs(sx - sy) < 0.0005 ? t("{percent} de l'original", { percent: percentLabel(sx) }) : t("{x} × {y} de l'original", { x: percentLabel(sx), y: percentLabel(sy) });
  });
  const status = $derived(smart.missing ? t("introuvable") : smart.linked ? t("lié · {path}", { path: smart.path ?? "" }) : t("incorporé · {width} × {height} px", { width: smart.width, height: smart.height }));

  /** Envoie une transformation du calque, d'un seul état d'historique. */
  function transform(matrix: number[] | null): void {
    if (!matrix) return;
    void edit({ op: "transform", ids: [layer.id], matrix, finalize: true, gesture: `objet-${layer.id}-${Date.now()}` });
  }

  const read = (event: Event & { currentTarget: HTMLInputElement }): number | null => {
    const value = event.currentTarget.valueAsNumber;
    return Number.isFinite(value) ? value : null;
  };

  function setSize(changed: "width" | "height", event: Event & { currentTarget: HTMLInputElement }): void {
    const value = read(event);
    if (value === null || value < 1) {
      event.currentTarget.value = String(Math.round(size[changed])); // saisie refusée : la valeur actuelle revient
      return;
    }
    const next = linkedRatio ? proportional(smart, changed, value) : { ...size, [changed]: value };
    transform(resizeMatrix(smart, next.width, next.height));
  }

  function setRotation(event: Event & { currentTarget: HTMLInputElement }): void {
    const value = read(event);
    if (value !== null) transform(rotateMatrix(smart, value));
  }

  function setPosition(axis: "x" | "y", event: Event & { currentTarget: HTMLInputElement }): void {
    const value = read(event);
    if (value !== null) transform(moveMatrix(smart, axis === "x" ? value : smart.x, axis === "y" ? value : smart.y));
  }

  /** Miniature du calque, peinte dans le canevas dès qu'elle arrive. */
  function paint(canvas: HTMLCanvasElement, image: ImageData | undefined) {
    const render = (data: ImageData | undefined) => {
      if (!data) return;
      canvas.width = data.width;
      canvas.height = data.height;
      canvas.getContext("2d")?.putImageData(data, 0, 0);
    };
    render(image);
    return { update: render };
  }
</script>

<section class="smart" aria-label={t("Objet dynamique")}>
  <h3>{t("Objet dynamique")}</h3>
  <div class="grid">
    <span class="label">{t("Contenu")}</span>
    <div class="content" title={smart.path ?? undefined}>
      <canvas use:paint={app.activeId === null ? undefined : thumbnailOf(app.activeId, layer.id, false)}></canvas>
      <div class="about">
        <span class="file">{smart.exportName}</span>
        <span class="status" class:missing={smart.missing}>{status}</span>
      </div>
    </div>

    <span class="label">{t("L / H")}</span>
    <div class="pair">
      <label class="unit"><input type="number" min="1" step="1" aria-label={t("Largeur (px)")} value={Math.round(size.width)} onchange={(e) => setSize("width", e)} /><i>px</i></label>
      <button class="chain" class:on={linkedRatio} aria-pressed={linkedRatio} title={linkedRatio ? t("Proportions liées : cliquez pour les délier") : t("Proportions libres : cliquez pour les lier")} aria-label={t("Conserver les proportions")} onclick={() => (linkedRatio = !linkedRatio)}>
        <svg viewBox="0 0 10 10" width="11" height="11" aria-hidden="true">
          {#if linkedRatio}
            <path d="M4.2 5.8 5.8 4.2M3.6 4.6 2.4 5.8a1.5 1.5 0 0 0 2.1 2.1l1.2-1.2M6.4 5.4l1.2-1.2a1.5 1.5 0 0 0-2.1-2.1L4.3 3.3" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
          {:else}
            <path d="M3.6 4.6 2.4 5.8a1.5 1.5 0 0 0 2.1 2.1l1.2-1.2M6.4 5.4l1.2-1.2a1.5 1.5 0 0 0-2.1-2.1L4.3 3.3" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
          {/if}
        </svg>
      </button>
      <label class="unit"><input type="number" min="1" step="1" aria-label={t("Hauteur (px)")} value={Math.round(size.height)} onchange={(e) => setSize("height", e)} /><i>px</i></label>
    </div>

    <span class="label">{t("Rotation")}</span>
    <div class="pair">
      <label class="unit" title={t("Rotation autour du centre, en degrés (sens horaire)")}><input type="number" step="0.1" aria-label={t("Rotation")} value={round1(smart.rotationDeg)} onchange={setRotation} /><i>°</i></label>
      <span class="gap"></span>
      <span class="rest"></span>
    </div>

    <span class="label">{t("Position")}</span>
    <div class="pair" title={t("Coin haut gauche du contenu, en pixels du document")}>
      <label class="unit"><b>X</b><input type="number" step="1" aria-label={t("Position X")} value={Math.round(smart.x)} onchange={(e) => setPosition("x", e)} /></label>
      <span class="gap"></span>
      <label class="unit"><b>Y</b><input type="number" step="1" aria-label={t("Position Y")} value={Math.round(smart.y)} onchange={(e) => setPosition("y", e)} /></label>
    </div>

    <span class="label">{t("Échelle")}</span>
    <output class="scale">{scaleText}</output>
  </div>

  <div class="actions">
    <button class="primary" onclick={() => void openSmartContent(layer.id)}>{t("Modifier le contenu")}</button>
    {#if smart.missing}
      <button onclick={() => void relinkSmart(layer.id)}>{t("Relier…")}</button>
    {:else}
      <button onclick={() => void replaceSmartContent(layer.id)}>{t("Remplacer…")}</button>
    {/if}
    {#if smart.linked && !smart.missing}
      <button onclick={() => void smartEdit("embedSmartLink", layer.id)}>{t("Incorporer")}</button>
    {/if}
  </div>
  {#if smart.instances > 1}
    <p class="note">{t("Partagé par {n} calques : modifier le contenu les met tous à jour.", { n: smart.instances })}</p>
  {/if}
</section>

<style>
  .smart {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 4px 14px 14px;
  }
  h3 {
    margin: 0;
    color: var(--text-3);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  .grid {
    display: grid;
    grid-template-columns: 52px minmax(0, 1fr);
    align-items: center;
    gap: 8px 8px;
  }
  .label {
    color: var(--text-3);
  }
  .content {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .content canvas {
    width: 38px;
    height: 38px;
    flex-shrink: 0;
    object-fit: contain;
    border: 1px solid var(--border-strong);
    background: repeating-conic-gradient(#bdbdbd 0% 25%, #ffffff 0% 50%) 0 0 / 6px 6px;
  }
  .about {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .file,
  .status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    color: var(--text-muted);
  }
  .status.missing {
    color: var(--accent);
  }
  .pair {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  /* Même largeur que le bouton chaîne : les colonnes des trois lignes s'alignent. */
  .gap {
    width: 22px;
    flex-shrink: 0;
  }
  .rest {
    flex: 1;
  }
  .unit {
    flex: 1;
    min-width: 0;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 6px;
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text-muted);
  }
  .unit:focus-within {
    border-color: var(--border-strong);
  }
  .unit b {
    font-weight: 500;
    color: var(--text-3);
  }
  .unit i {
    font-style: normal;
  }
  .unit input {
    flex: 1;
    min-width: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    user-select: text;
    font-variant-numeric: tabular-nums;
  }
  .unit input:focus {
    outline: none;
  }
  .chain {
    width: 22px;
    height: 26px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
  }
  .chain.on {
    color: var(--text);
  }
  .chain:hover {
    background: var(--bg-hover);
  }
  .scale {
    height: 26px;
    display: flex;
    align-items: center;
    padding: 0 8px;
    border: 1px solid var(--border);
    background: var(--bg-field);
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .actions button {
    height: 28px;
    padding: 0 12px;
    border: 1px solid var(--border-strong);
    color: var(--text);
  }
  .actions button:hover {
    background: var(--bg-hover);
  }
  .actions .primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }
  .actions .primary:hover {
    background: var(--accent-strong);
  }
  .note {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
</style>
