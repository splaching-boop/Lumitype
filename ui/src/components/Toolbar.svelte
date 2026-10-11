<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { type ActionId, runAction } from "../lib/actions";
  import type { IconName } from "../lib/icons";
  import { preloadAnimations } from "../lib/media";
  import { app, type ToolId } from "../state/app.svelte";
  import Icon from "./Icon.svelte";
  import ToolTip from "./ToolTip.svelte";

  interface Tool {
    icon: IconName;
    label: string;
    /** Absent : outil d'une étape ultérieure, affiché mais inactif. */
    id?: ToolId;
    /** Double-clic, comme dans Photoshop : Main = taille écran, Zoom = 100 %. */
    doubleClick?: ActionId;
  }

  const TOOLS: Tool[] = [
    { icon: "move", label: t("Déplacement (V)"), id: "move" },
    { icon: "marquee", label: t("Rectangle / ellipse de sélection (M, Maj+M)"), id: "marquee" },
    { icon: "lasso", label: t("Lasso libre / polygonal (L, Maj+L)"), id: "lasso" },
    { icon: "wand", label: t("Baguette magique (W)"), id: "wand" },
    { icon: "quickSelect", label: t("Sélection rapide (Maj+W)"), id: "quickSelect" },
    { icon: "crop", label: t("Recadrage (C)"), id: "crop" },
    { icon: "pipette", label: t("Pipette (I)"), id: "eyedropper" },
    { icon: "heal", label: t("Correcteur localisé (J)"), id: "heal" },
    { icon: "brush", label: t("Pinceau (B)"), id: "brush" },
    { icon: "eraser", label: t("Gomme (E)"), id: "eraser" },
    { icon: "bucket", label: t("Dégradé / pot de peinture (G, Maj+G)"), id: "fill" },
    { icon: "pen", label: t("Plume (P)"), id: "pen" },
    { icon: "type", label: t("Texte (T)"), id: "text" },
    { icon: "pathSelect", label: t("Sélection directe (A)"), id: "pathSelect" },
    { icon: "shape", label: t("Forme (U, Maj+U)"), id: "shape" },
    { icon: "hand", label: "Main (H)", id: "hand", doubleClick: "view.fit" },
    { icon: "zoom", label: t("Zoom (Z)"), id: "zoom", doubleClick: "view.actualSize" },
  ];

  /** Temps de survol avant l'apparition de la bulle d'aide. */
  const DELAY = 1000;
  /** Court sursis à la sortie : passer d'un outil au suivant ne referme pas la bulle. */
  const GRACE = 120;

  let shown: { tool: ToolId; top: number } | null = $state(null);
  let open = false;
  let appear: number | undefined;
  let vanish: number | undefined;

  function enter(tool: Tool, event: MouseEvent): void {
    if (!tool.id) return;
    clearTimeout(vanish);
    const top = (event.currentTarget as HTMLElement).getBoundingClientRect().top;
    const id = tool.id;
    // Bulle déjà ouverte : elle suit le curseur d'un outil à l'autre sans réclamer le délai,
    // comme dans Photoshop. Sinon, on attend la seconde complète.
    if (open) {
      shown = { tool: id, top };
      return;
    }
    clearTimeout(appear);
    appear = window.setTimeout(() => {
      open = true;
      shown = { tool: id, top };
    }, DELAY);
  }

  function leave(): void {
    clearTimeout(appear);
    clearTimeout(vanish);
    vanish = window.setTimeout(() => {
      open = false;
      shown = null;
    }, GRACE);
  }

  /** Au clic, l'aide n'a plus lieu d'être : l'outil est pris en main. */
  function dismiss(): void {
    clearTimeout(appear);
    clearTimeout(vanish);
    open = false;
    shown = null;
  }

  // Les animations sont mises en cache une fois l'application ouverte, jamais pendant son démarrage.
  $effect(() => {
    const later = window.setTimeout(preloadAnimations, 2000);
    return () => clearTimeout(later);
  });
</script>

<nav class="toolbar" aria-label={t("Outils")}>
  {#each TOOLS as tool (tool.icon)}
    <button
      class:active={tool.id === app.tool}
      disabled={!tool.id}
      title={tool.id ? undefined : `${t(tool.label)} — à venir`}
      aria-label={t(tool.label)}
      aria-pressed={tool.id === app.tool}
      onclick={() => {
        dismiss();
        if (tool.id) app.tool = tool.id;
      }}
      ondblclick={() => tool.doubleClick && runAction(tool.doubleClick)}
      onmouseenter={(event) => enter(tool, event)}
      onmouseleave={leave}
    >
      <Icon name={tool.icon} size={17} />
    </button>
  {/each}
</nav>

{#if shown}
  <ToolTip tool={shown.tool} top={shown.top} />
{/if}

<style>
  .toolbar {
    width: 48px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 0;
    background: var(--bg-bar);
    border-right: 1px solid var(--border);
  }
  button {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-3);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  button:disabled {
    color: var(--text-disabled);
  }
  button:not(:disabled):hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  button.active {
    background: var(--bg-active);
    color: #fff;
  }
</style>
