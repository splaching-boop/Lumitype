<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount, untrack } from "svelte";
  import type { LayerInfo } from "../lib/ipc";
  import { filterLabel, setFilterBlending } from "../lib/smart-filters";
  import BlendSelect from "./BlendSelect.svelte";

  // Options de fusion d'un filtre dynamique : comment son résultat se pose sur ce qu'il reçoit (mode, opacité).
  // Petit panneau posé près de la ligne du filtre ; chaque réglage est rendu par le moteur, un geste par curseur.
  let { layer, index, x, y, onclose }: { layer: LayerInfo; index: number; x: number; y: number; onclose: () => void } = $props();

  const item = $derived(layer.filters?.list[index]);
  const gesture = untrack(() => `filtre-fusion-${layer.id}-${index}-${Date.now()}`);
  let panel = $state<HTMLDivElement>();
  const left = $derived(Math.max(8, Math.min(window.innerWidth - 290, x - 280)));
  const top = $derived(Math.max(8, Math.min(window.innerHeight - 160, y - 20)));

  onMount(() => {
    const away = (event: PointerEvent) => {
      if (panel && !panel.contains(event.target as Node) && !(event.target as HTMLElement).closest?.("[role='listbox'], .blend-menu, .menu")) onclose();
    };
    const key = (event: KeyboardEvent) => event.key === "Escape" && onclose();
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", key);
    };
  });
</script>

{#if item}
  <div class="panel" bind:this={panel} role="dialog" aria-label={t("Options de fusion du filtre")} style:left="{left}px" style:top="{top}px">
    <h3>{t("Options de fusion")} · {t(filterLabel(item.filter))}</h3>
    <div class="line">
      <span>{t("Mode")}</span>
      <BlendSelect value={item.blend} exclude={["pass_through", "dissolve"]} onpreview={(mode) => setFilterBlending(layer, index, { blend: mode }, gesture)} onchoose={(mode) => setFilterBlending(layer, index, { blend: mode }, gesture)} />
    </div>
    <label class="line">
      <span>{t("Opacité")}</span>
      <input type="range" min="0" max="100" step="1" value={Math.round(item.opacity * 100)} oninput={(event) => setFilterBlending(layer, index, { opacity: event.currentTarget.valueAsNumber / 100 }, gesture)} />
      <output>{Math.round(item.opacity * 100)} %</output>
    </label>
  </div>
{/if}

<style>
  .panel {
    position: fixed;
    z-index: 60;
    width: 280px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
    color: var(--text-2);
  }
  h3 {
    margin: 0;
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .line span {
    width: 56px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  input[type="range"] {
    flex: 1;
  }
  output {
    width: 40px;
    text-align: right;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
