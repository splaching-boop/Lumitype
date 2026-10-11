<script lang="ts">
  import { loadFilm } from "../../lib/media";

  // Démonstration filmée d'un guide : lecteur complet, avec contrôles. On peut mettre en pause, revenir en arrière,
  // passer en plein écran — ce qu'une animation en boucle ne permet pas. Elle démarre seule et sans son, comme une
  // animation, pour que le geste soit visible sans rien cliquer. Le film est lu depuis la mémoire (voir `loadFilm`).
  let { article }: { article: string } = $props();
  let film = $state<{ src: string; type: string } | null>(null);
  $effect(() => {
    let current = true;
    film = null;
    void loadFilm(article).then((loaded) => current && (film = loaded));
    return () => (current = false);
  });
</script>

{#if film}
  {#key film}
    <!-- svelte-ignore a11y_media_has_caption -->
    <video class="shot" controls autoplay loop muted playsinline>
      <source src={film.src} type={film.type} />
    </video>
  {/key}
{:else}
  <div class="shot loading" aria-hidden="true"></div>
{/if}

<style>
  .shot {
    display: block;
    width: 100%;
    border-radius: var(--radius-l);
    background: #000;
    border: 1px solid var(--border);
  }
  .loading {
    aspect-ratio: 16 / 9;
    background: #111;
  }
</style>
