<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { type GradientStop, resolveStops, toCss } from "../lib/gradient";
  import { app } from "../state/app.svelte";
  import GradientStops from "./GradientStops.svelte";

  // Éditeur de dégradé de l'outil Dégradé : la rangée d'arrêts partagée (`GradientStops`) dans une
  // fenêtre flottante, plus les deux préréglages qui suivent les couleurs de premier plan et
  // d'arrière-plan. Tout changement s'applique tout de suite : pas de bouton OK.
  let { x, y, onclose }: { x: number; y: number; onclose: () => void } = $props();

  let panel = $state<HTMLDivElement>();
  let editor = $state<ReturnType<typeof GradientStops>>();

  const stops = $derived(resolveStops(app.gradient, app.foreground, app.background));

  /** Toute modification fige le dégradé : il ne suit plus les couleurs de premier plan et d'arrière-plan. */
  function commit(next: GradientStop[]): void {
    app.gradient = { stops: next, dynamic: null };
  }

  const left = $derived(Math.max(8, Math.min(x, window.innerWidth - 388)));
</script>

<svelte:window
  onpointerdown={(event) => {
    const inside = event.target instanceof Node && panel?.contains(event.target);
    // Le sélecteur de couleur s'ouvre hors du panneau : un clic dedans ne doit pas fermer l'éditeur.
    if (!inside && !editor?.pickingColor() && !event.altKey) onclose();
  }}
  onkeydown={(event) => {
    if (event.key === "Escape" && !editor?.pickingColor()) onclose();
    if ((event.key === "Delete" || event.key === "Backspace") && !(event.target instanceof HTMLInputElement)) editor?.removeSelected();
  }}
/>

<div class="editor" role="dialog" aria-label={t("Éditeur de dégradé")} bind:this={panel} style:left="{left}px" style:top="{y}px">
  <GradientStops bind:this={editor} {stops} onchange={commit}>
    <button class="preset" class:on={app.gradient.dynamic === "foregroundToBackground"} title={t("Premier plan → arrière-plan (suit vos couleurs)")} aria-label={t("Premier plan vers arrière-plan")} onclick={() => (app.gradient = { stops: [], dynamic: "foregroundToBackground" })}>
      <span style:background={toCss(resolveStops({ stops: [], dynamic: "foregroundToBackground" }, app.foreground, app.background))}></span>
    </button>
    <button class="preset" class:on={app.gradient.dynamic === "foregroundToTransparent"} title={t("Premier plan → transparent (suit votre couleur)")} aria-label={t("Premier plan vers transparent")} onclick={() => (app.gradient = { stops: [], dynamic: "foregroundToTransparent" })}>
      <span style:background={toCss(resolveStops({ stops: [], dynamic: "foregroundToTransparent" }, app.foreground, app.background))}></span>
    </button>
  </GradientStops>
</div>

<style>
  .editor {
    position: fixed;
    z-index: 40;
    width: 380px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  /* Les deux préréglages dynamiques, rendus ici : mêmes cases que celles du catalogue. */
  .preset {
    height: 30px;
    padding: 0;
    border-radius: var(--radius-m);
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: repeating-conic-gradient(#3a3a3e 0% 25%, #232326 0% 50%) 0 0 / 10px 10px;
  }
  .preset span {
    display: block;
    width: 100%;
    height: 100%;
  }
  .preset:hover,
  .preset.on {
    outline: 1.5px solid #fff;
    outline-offset: 1px;
  }
</style>
