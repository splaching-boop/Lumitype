<script lang="ts">
  import { CUSTOM_ICONS, ICONS, type IconName } from "../lib/icons";

  // Une icône vient de `assets/icons/<nom>.svg` si ce fichier existe (icônes dessinées par
  // l'auteur du projet), sinon du tracé intégré. Le fichier est affiché comme un masque : seule sa
  // forme compte, il prend la couleur du texte environnant comme les icônes intégrées.
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
  const custom = $derived(CUSTOM_ICONS[name]);
  // Adresse ENTRE GUILLEMETS : les petites icônes sont intégrées par Vite en `data:image/svg+xml,…`, qui
  // contient des apostrophes et des parenthèses. Sans guillemets, le `url(…)` est invalide, le masque est
  // ignoré et l'icône devient un carré plein.
  const mask = $derived(custom ? `url(${JSON.stringify(custom)})` : undefined);
</script>

{#if custom}
  <span class="custom" style:width="{size}px" style:height="{size}px" style:mask-image={mask} style:-webkit-mask-image={mask} aria-hidden="true"></span>
{:else}
  <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
    <path d={ICONS[name]} />
  </svg>
{/if}

<style>
  svg {
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .custom {
    display: inline-block;
    flex-shrink: 0;
    background: currentColor;
    mask-size: contain;
    mask-repeat: no-repeat;
    mask-position: center;
    -webkit-mask-size: contain;
    -webkit-mask-repeat: no-repeat;
    -webkit-mask-position: center;
  }
</style>
