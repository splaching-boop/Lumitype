<script lang="ts">
  import { onMount } from "svelte";
  import { type RefItem, thumbnailUrl } from "../../lib/myref";

  // Vignette d'une image de la bibliothèque, demandée seulement quand elle approche de l'écran (une collection de
  // plusieurs centaines d'images ne décode pas tout d'un coup). `fit` : « natural » garde les proportions de
  // l'image (grille en maçonnerie), « cover » remplit le cadre donné par le parent (bandeaux, collages).
  let { root, item, fit = "natural" }: { root: string; item: RefItem; fit?: "natural" | "cover" } = $props();

  let host: HTMLElement;
  let url = $state("");
  let failed = $state(false);
  /** Proportions connues (fiche) ou découvertes au chargement : la case garde sa hauteur avant l'image. */
  let natural = $state<number | null>(null);
  const ratio = $derived(natural ?? (item.width && item.height ? item.width / item.height : 4 / 3));

  /** Nombre de rechargements après une adresse révoquée (cache borné, lib/myref.ts) : un seul, pour ne pas boucler. */
  let reloads = 0;
  function load(): void {
    thumbnailUrl(root, item).then(
      (next) => (url = next),
      () => (failed = true),
    );
  }

  onMount(() => {
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        load();
      },
      { rootMargin: "600px" },
    );
    observer.observe(host);
    return () => observer.disconnect();
  });
</script>

<span class="thumb" class:cover={fit === "cover"} bind:this={host} style:aspect-ratio={fit === "natural" ? String(ratio) : undefined}>
  {#if url}
    <img
      src={url}
      alt=""
      draggable="false"
      onload={(event) => {
        const img = event.currentTarget as HTMLImageElement;
        if (!item.width && img.naturalWidth > 0) natural = img.naturalWidth / img.naturalHeight;
      }}
      onerror={() => {
        if (reloads++ === 0) load();
      }}
    />
  {:else}
    <svg class:failed viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="4" width="18" height="16" /><path d="M3 16l5-5 4 4 3-3 6 6" /><circle cx="16" cy="9" r="1.6" /></svg>
  {/if}
</span>

<style>
  .thumb {
    position: relative;
    display: grid;
    place-items: center;
    width: 100%;
    overflow: hidden;
    background: var(--bg-panel);
  }
  .thumb.cover {
    height: 100%;
  }
  img {
    width: 100%;
    height: 100%;
    display: block;
    object-fit: cover;
    animation: appear 220ms var(--ease);
  }
  svg {
    width: 22px;
    height: 22px;
    fill: none;
    stroke: #fff;
    stroke-width: 1.4;
    opacity: 0.18;
  }
  svg.failed {
    stroke: var(--accent);
    opacity: 0.5;
  }
  @keyframes appear {
    from {
      opacity: 0;
    }
  }
</style>
