<script lang="ts">
  import { ICONS } from "../../lib/icons";
  import { demoPoster, loadFilm, toolDemoArticle } from "../../lib/media";
  import { TOOL_GROUPS } from "../../lib/tool-groups.svelte";

  // Carte d'outil de la notice : l'image fixe du film au repos, le film muet en boucle au survol (ou au focus
  // clavier), fondu par-dessus dès sa première image. Sans film tourné, l'icône de l'outil.
  let { tool, name, shortcut, onselect }: { tool: string; name: string; shortcut: string; onselect: () => void } = $props();

  const article = $derived(toolDemoArticle(tool));
  const poster = $derived(article ? demoPoster(article) : "");
  const icon = $derived(TOOL_GROUPS.flatMap((group) => group.variants).find((variant) => variant.tool === tool)?.icon ?? null);

  let live = $state(false);
  /** Le film en mémoire (voir `loadFilm`), chargé seulement quand il doit tourner. */
  let loaded = $state<{ src: string; type: string } | null>(null);
  let video = $state<HTMLVideoElement>();
  /** Le film a livré sa première image : l'image fixe peut s'effacer sans trou noir. */
  let playing = $state(false);

  $effect(() => {
    const current = article;
    if (!current || !live || loaded) return;
    let alive = true;
    void loadFilm(current).then((film) => alive && (loaded = film));
    return () => (alive = false);
  });

  $effect(() => {
    const element = video;
    if (!element) return;
    if (live) {
      // `muted` posé ici et non seulement dans le balisage : Chromium ne lance pas toujours un film non muet sans clic.
      element.muted = true;
      element.currentTime = 0;
      void element.play().catch(() => undefined);
    } else {
      element.pause();
      playing = false;
    }
  });
</script>

<button
  class="tool"
  onclick={onselect}
  onpointerenter={() => (live = true)}
  onpointerleave={() => (live = false)}
  onfocus={() => (live = true)}
  onblur={() => (live = false)}
>
  <span class="film" aria-hidden="true">
    {#if poster}
      <img src={poster} alt="" />
    {:else if icon}
      <svg viewBox="0 0 24 24"><path d={ICONS[icon]} /></svg>
    {/if}
    {#if loaded}
      <video bind:this={video} class:playing loop muted playsinline disablepictureinpicture tabindex="-1" onplaying={() => (playing = live)}>
        <source src={loaded.src} type={loaded.type} />
      </video>
    {/if}
  </span>
  <span class="t"><b>{name}</b>{#if shortcut}<kbd>{shortcut}</kbd>{/if}</span>
</button>

<style>
  .tool {
    padding: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    background: var(--bg-card);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color 120ms var(--ease);
  }
  .tool:hover,
  .tool:focus-visible {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .film {
    position: relative;
    aspect-ratio: 16 / 9;
    display: grid;
    place-items: center;
    background: var(--bg-well);
    overflow: hidden;
  }
  img,
  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  video {
    opacity: 0;
    transition: opacity 260ms var(--ease);
  }
  video.playing {
    opacity: 1;
  }
  svg {
    width: 34px;
    height: 34px;
    fill: none;
    stroke: var(--text-3);
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .t {
    padding: 8px 10px;
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 8px;
    font-size: 12.5px;
  }
  .t b {
    min-width: 0;
    font-weight: 500;
  }
  kbd {
    flex-shrink: 0;
    font: 11px ui-monospace, monospace;
    color: var(--text-muted);
  }
</style>
