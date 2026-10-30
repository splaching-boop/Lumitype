<script lang="ts">
  import { BRAND_ICONS } from "../lib/brand-icons";

  // Logo d'une plateforme (catégorie de formats) : carré arrondi à sa couleur, logo d'une couleur. Rien si inconnue.
  let { id, size = 18 }: { id: string; size?: number } = $props();
  const icon = $derived(BRAND_ICONS[id]);
</script>

{#if icon}
  <svg class="brand" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
    <rect width="24" height="24" rx="5.5" fill={icon.bg} />
    <g transform="translate(4.8 4.8) scale(0.6)"><path d={icon.path} fill={icon.fg} /></g>
  </svg>
{/if}

<style>
  .brand {
    display: block;
    flex-shrink: 0;
  }
</style>
