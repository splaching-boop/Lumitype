<script lang="ts">
  import BrandIcon from "./BrandIcon.svelte";
  import { BRAND_ICONS } from "../lib/brand-icons";

  // Icône d'une catégorie de formats : le logo de la plateforme, sinon un pictogramme (Essentiels, Impression…).
  let { id, size = 18 }: { id: string; size?: number } = $props();
  const GLYPHS: Record<string, string> = {
    essentiels: "M12 3l2.6 5.5 6 .8-4.4 4.1 1.1 6L12 16.5 6.7 19.4l1.1-6L3.4 9.3l6-.8z",
    impression: "M7 9V3h10v6M7 17H4v-7h16v7h-3M7 14h10v7H7z",
    cartes: "M2 6h20v12H2zM6 10h6M6 14h9",
    signature: "M3 5h18v14H3zM3 7l9 6 9-6",
  };
</script>

{#if BRAND_ICONS[id]}
  <BrandIcon {id} {size} />
{:else if GLYPHS[id]}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><path d={GLYPHS[id]} fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round" /></svg>
{:else}
  <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3.5" fill="none" stroke="currentColor" stroke-width="1.6" /></svg>
{/if}
