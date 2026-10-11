<script lang="ts">
  import { ICONS } from "../../lib/icons";
  import { demoPoster, demoSources, illustration, loadFilm, tutorialAnimation } from "../../lib/media";
  import type { Illustration } from "../../lib/news";
  import { banner } from "./banner";

  // Petit dessin d'un article, dans l'esprit des icônes : carte de l'accueil (`size="card"`), bandeau de la carte
  // mise en avant (`"feature"`), bandeau de la fenêtre de lecture (`"hero"`).
  //
  // Un guide d'outil n'a pas de dessin à lui : il affiche l'icône de son outil. Un guide de réglage affiche
  // l'illustration fabriquée par le moteur. Un guide filmé montre une **image fixe** de son film ; sur une
  // carte, le film démarre au survol (`live`) et s'y fond, puis revient à l'image fixe quand on ressort.
  // En bandeau de lecture, la vignette reste fixe : le lecteur complet est dans le corps du guide.
  let { kind, size = "card", live = false }: { kind: Illustration; size?: "card" | "feature" | "hero"; live?: boolean } = $props();
  /** Film du guide, s'il existe. */
  const film = $derived(typeof kind === "object" && "demo" in kind ? demoSources(kind.demo) : []);
  const poster = $derived(typeof kind === "object" && "demo" in kind ? demoPoster(kind.demo) : "");
  /** Le film en mémoire (voir `loadFilm`), chargé seulement quand il doit tourner. */
  let loaded = $state<{ src: string; type: string } | null>(null);
  $effect(() => {
    const article = typeof kind === "object" && "demo" in kind ? kind.demo : null;
    let current = true;
    loaded = null;
    if (article && live && size !== "hero") void loadFilm(article).then((film) => current && (loaded = film));
    return () => (current = false);
  });
  const drawn = $derived(typeof kind === "string" ? kind : film.length > 0 ? "media" : "icon" in kind ? "icon" : "media");
  const source = $derived(typeof kind === "string" ? "" : "image" in kind ? illustration(kind.image) : "gif" in kind ? tutorialAnimation(kind.gif) : "");

  let video = $state<HTMLVideoElement>();
  /** Le film a livré sa première image : la vignette fixe peut s'effacer sans trou noir. */
  let playing = $state(false);
  $effect(() => {
    const element = video;
    if (!element) return;
    if (live && size !== "hero" && loaded) {
      // `muted` posé ici et non seulement dans le balisage : Chromium n'honore pas toujours l'attribut
      // pour la lecture automatique, et un film non muet ne démarre pas sans clic.
      element.muted = true;
      element.currentTime = 0;
      void element.play().catch(() => undefined);
    } else {
      element.pause();
      playing = false;
    }
  });
</script>

<span class="art {size}" data-kind={drawn} aria-hidden="true">
  {#if typeof kind === "object"}
    {#if film.length > 0}
      <!-- Image fixe du film, et le film par-dessus, révélé en fondu dès qu'il tourne. -->
      {#if poster}<img class="poster" src={poster} alt="" />{/if}
      {#if size !== "hero" && loaded}
        {#key loaded}
          <video bind:this={video} class:playing loop muted playsinline disablepictureinpicture tabindex="-1" onplaying={() => (playing = true)} ontimeupdate={() => (playing = live)}>
            <source src={loaded.src} type={loaded.type} />
          </video>
        {/key}
      {/if}
    {:else if "icon" in kind}
      <svg class="line" viewBox="0 0 24 24"><path d={ICONS[kind.icon]} /></svg>
    {:else}
      <img src={source} alt="" />
    {/if}
  {:else if kind === "banner"}
    {#if banner}<img src={banner} alt="" />{:else}<span class="tone"></span>{/if}
  {:else if kind === "clip"}
    <span class="sheet back"></span><span class="sheet front"></span>
  {:else if kind === "softBrush"}
    <span class="dab d1"></span><span class="dab d2"></span><span class="dab d3"></span>
  {:else if kind === "multiSelect"}
    <span class="frame"><span></span><span></span></span>
  {:else if kind === "adjust"}
    <span class="half"></span>
  {:else}
    <svg viewBox="0 0 24 24"><path d="M12 1.5l3.4 4.4H8.6zM12 22.5l-3.4-4.4h6.8zM1.5 12l4.4-3.4v6.8zM22.5 12l-4.4 3.4V8.6z" /><rect x="10.9" y="5" width="2.2" height="14" rx="1.1" /><rect x="5" y="10.9" width="14" height="2.2" rx="1.1" /><circle cx="12" cy="12" r="3" /></svg>
  {/if}
</span>

<style>
  .art {
    --k: 1; /* échelle des dessins */
    width: 84px;
    height: 84px;
    flex-shrink: 0;
    position: relative;
    border-radius: var(--radius-l);
    background: #221f20;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .art.feature {
    width: 100%;
    height: 170px;
    border-radius: 0;
    --k: 1.6;
  }
  .art.hero {
    width: 100%;
    height: 300px;
    border-radius: 0;
    --k: 2.6;
  }
  img,
  video,
  .tone {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 40%;
  }
  /* Film et image fixe occupent la même place ; le film est transparent tant qu'il ne tourne pas. */
  .poster,
  video {
    position: absolute;
    inset: 0;
    object-position: center;
  }
  video {
    opacity: 0;
    transition: opacity 260ms var(--ease);
  }
  video.playing {
    opacity: 1;
  }
  .tone {
    background: #1a1726;
  }
  svg {
    width: calc(40px * var(--k));
    height: calc(40px * var(--k));
    fill: #efeadf;
  }
  /* Icône d'outil : trait, comme dans la barre d'outils, et non forme pleine. */
  svg.line {
    fill: none;
    stroke: #efeadf;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .art[data-kind="icon"] {
    background: #26262c;
  }
  .art[data-kind="media"] {
    background: #000;
  }
  /* En bandeau, l'illustration est montrée en entier : c'est une comparaison avant / après,
     la recadrer reviendrait à n'en montrer qu'une moitié. */
  .art.hero[data-kind="media"] img {
    object-fit: contain;
  }
  /* La vignette d'un film, elle, se recadre sans dommage : c'est une image d'accroche, le film entier est en dessous. */
  .art.hero .poster {
    object-fit: cover;
  }
  .sheet {
    position: absolute;
    width: calc(40px * var(--k));
    height: calc(30px * var(--k));
    border-radius: calc(var(--radius-s) * var(--k));
  }
  .sheet.back {
    background: #efeadf;
    transform: translate(calc(-6px * var(--k)), calc(8px * var(--k)));
  }
  .sheet.front {
    background: var(--accent);
    transform: translate(calc(8px * var(--k)), calc(-6px * var(--k)));
  }
  .art[data-kind="softBrush"] {
    background: #f4f4f5;
    display: flex;
    gap: calc(6px * var(--k));
    justify-content: center;
    align-items: center;
  }
  .dab {
    border-radius: 50%;
  }
  .d1 {
    width: calc(16px * var(--k));
    height: calc(16px * var(--k));
    background: radial-gradient(circle, rgba(0, 0, 0, 0.45), rgba(0, 0, 0, 0) 70%);
  }
  .d2 {
    width: calc(20px * var(--k));
    height: calc(20px * var(--k));
    background: radial-gradient(circle, rgba(0, 0, 0, 0.75), rgba(0, 0, 0, 0) 70%);
  }
  .d3 {
    width: calc(24px * var(--k));
    height: calc(24px * var(--k));
    background: radial-gradient(circle, #000 30%, rgba(0, 0, 0, 0) 72%);
  }
  .frame {
    width: calc(56px * var(--k));
    height: calc(56px * var(--k));
    box-sizing: border-box;
    border: 1.5px solid var(--accent);
    padding: calc(5px * var(--k));
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: calc(4px * var(--k));
  }
  .frame span {
    border-radius: var(--radius-xs);
    background: #efeadf;
  }
  .frame span + span {
    background: #6f6f77;
  }
  .half {
    width: calc(44px * var(--k));
    height: calc(44px * var(--k));
    border-radius: 50%;
    border: calc(4px * var(--k)) solid #efeadf;
    box-sizing: border-box;
    background: linear-gradient(90deg, #efeadf 50%, transparent 50%);
  }
</style>
