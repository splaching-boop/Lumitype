<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { loadFilm, toolDemoSources, toolDemoArticle } from "../lib/media";
  import { TOOL_HELP } from "../lib/tool-help";
  import type { ToolId } from "../state/app.svelte";

  // Bulle d'aide d'un outil. Sa largeur et la hauteur réservée au texte ne changent jamais d'un outil à
  // l'autre : une bulle qui grandit ou rétrécit en suivant le curseur donnerait une barre d'outils qui
  // tressaute. Seul l'aperçu peut manquer — un outil sans démonstration filmée n'affiche rien du tout
  // plutôt qu'une image de remplacement, et la bulle est alors simplement plus courte.
  let { tool, top }: { tool: ToolId; top: number } = $props();

  const help = $derived(TOOL_HELP[tool]);
  /** Démonstration filmée de l'outil ; vide tant qu'elle n'a pas été tournée. */
  const demo = $derived(toolDemoSources(tool));
  /** Le film chargé en mémoire (voir `loadFilm`) : `null` le temps du chargement. */
  let film = $state<{ src: string; type: string } | null>(null);
  $effect(() => {
    const article = toolDemoArticle(tool);
    let current = true;
    film = null;
    if (article) void loadFilm(article).then((loaded) => current && (film = loaded));
    return () => (current = false);
  });
  /** Hauteur totale, tenue par le style : elle sert à recaler la bulle si elle dépasse en bas. */
  /** 250 px de large en 16:9 : le film remplit le cadre, sans bande noire. */
  const PREVIEW = 141;
  const TEXT = 97;
  const height = $derived(demo.length > 0 ? PREVIEW + TEXT : TEXT);
  const MARGIN = 8;
  const y = $derived(Math.max(MARGIN, Math.min(top, window.innerHeight - height - MARGIN)));
</script>

<aside class="tip" style:top="{y}px" style:height="{height}px" role="tooltip">
  <!-- La bulle suit le curseur d'un outil à l'autre : `{#key}` force un nouvel élément, sinon la vidéo
       précédente resterait à l'image où elle s'était arrêtée. Sans son, en boucle, et sans contrôles :
       c'est une vignette animée, pas un lecteur. -->
  {#if demo.length > 0}
    {#key film}
      {#if film}
        <video autoplay loop muted playsinline disablepictureinpicture tabindex="-1" aria-hidden="true">
          <source src={film.src} type={film.type} />
        </video>
      {/if}
    {/key}
  {/if}
  <div class="body">
    <div class="head">
      <span class="name">{t(help.name)}</span>
      <span class="key">{t(help.keys)}</span>
    </div>
    <p class="hint">{t(help.hint)}</p>
  </div>
</aside>

<style>
  .tip {
    position: fixed;
    left: 54px;
    z-index: 35;
    width: 250px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-menu);
    overflow: hidden;
    pointer-events: none;
    animation: appear 120ms var(--ease);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateX(-4px);
    }
  }
  /* Les films sont en 16:9, comme le cadre : `cover` ne coupe rien et évite les bandes noires. */
  video {
    display: block;
    width: 100%;
    height: 141px;
    object-fit: cover;
    background: var(--bg-field);
  }
  .body {
    padding: 10px 12px 12px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .key {
    font-size: 10.5px;
    font-weight: 600;
    line-height: 15px;
    color: var(--text-2);
    padding: 1px 6px;
    background: var(--bg-field);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
  }
  .hint {
    /* Trois lignes réservées : toutes les bulles ont exactement la même taille. */
    height: 51px;
    margin: 6px 0 0;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-3);
  }
</style>
