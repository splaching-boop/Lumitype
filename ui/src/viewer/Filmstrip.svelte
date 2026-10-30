<script lang="ts">
  // Pellicule : les images du dossier. Molette : fait défiler la bande (l'image affichée ne change pas ; un clic la
  // choisit). Vignettes chargées à l'approche.
  import { t } from "../lib/i18n.svelte";
  import type { ViewerEntry } from "./viewer-api";
  import { requestThumb, thumbs } from "./thumb-cache.svelte";

  let {
    items,
    index,
    folder,
    compareWith,
    onpick,
  }: { items: ViewerEntry[]; index: number; folder: string; compareWith: string | null; onpick: (at: number) => void } = $props();

  let strip = $state<HTMLDivElement>();

  function want(node: HTMLElement, path: string) {
    const observer = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && requestThumb(path), { root: strip, rootMargin: "0px 400px" });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  // L'image courante reste visible dans la pellicule.
  $effect(() => {
    const at = index;
    strip?.querySelectorAll<HTMLElement>("button.cell")[at]?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  });

  /** Molette verticale ou horizontale : la bande défile (une ligne de molette ≈ une vignette). */
  function onWheel(event: WheelEvent): void {
    if (!strip) return;
    event.preventDefault();
    const delta = Math.abs(event.deltaY) > Math.abs(event.deltaX) ? event.deltaY : event.deltaX;
    strip.scrollBy({ left: event.deltaMode === 1 ? delta * 60 : delta, behavior: event.deltaMode === 1 ? "smooth" : "auto" });
  }

  const folderName = $derived(folder.split(/[\\/]/).filter(Boolean).pop() ?? "");
</script>

<div class="film">
  <span class="folder" title={folder}>{folderName} · {items.length}</span>
  <div class="strip" bind:this={strip} onwheel={onWheel}>
    {#each items as item, at (item.path)}
      <button class="cell" class:on={at === index} class:other={item.path === compareWith} title={item.name} onclick={() => onpick(at)} use:want={item.path}>
        {#if thumbs[item.path]}<img src={thumbs[item.path]} alt="" draggable="false" />{/if}
      </button>
    {/each}
  </div>
  <span class="hint" title={t("Molette sur la pellicule : faire défiler les vignettes. Molette sur l'image : zoom.")}>
    <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="3" width="10" height="18" rx="5" /><path d="M12 7v3" /></svg>
    {t("Molette : faire défiler")}
  </span>
</div>

<style>
  .film {
    height: 74px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    background: var(--bg-bar);
    border-top: 1px solid var(--border);
  }
  .folder {
    color: var(--text-muted);
    white-space: nowrap;
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .strip {
    flex: 1;
    display: flex;
    gap: 6px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    padding: 4px 2px;
  }
  .strip::-webkit-scrollbar {
    display: none;
  }
  .cell {
    flex: none;
    width: 56px;
    height: 56px;
    padding: 0;
    border: 0;
    background: var(--bg-field);
    cursor: pointer;
    opacity: 0.7;
    outline-offset: -2px;
  }
  .cell img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .cell:hover {
    opacity: 1;
  }
  .cell.on {
    opacity: 1;
    outline: 2px solid var(--accent);
  }
  .cell.other {
    opacity: 1;
    outline: 2px dashed var(--text-2);
  }
  .hint {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .hint svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
  }
</style>
