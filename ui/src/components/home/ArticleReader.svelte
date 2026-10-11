<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { demoSources, hasTutorialAnimation, illustration, tutorialAnimation } from "../../lib/media";
  import FilmPlayer from "./FilmPlayer.svelte";
  import { type Article, KIND_LABEL, articleDate, localizedArticles, runs } from "../../lib/news";
  import ArticleArt from "./ArticleArt.svelte";
  import ArticleFigure from "./ArticleFigure.svelte";
  import { banner } from "./banner";

  // Fenêtre de lecture d'un article (maquette « Lecture d'un tutoriel ») : l'accueil flouté derrière, la fenêtre au
  // centre avec son bandeau, et à sa droite des commandes flottantes (fermer, article précédent / suivant).
  let { article, onnavigate, onclose }: { article: Article; onnavigate: (next: Article) => void; onclose: () => void } = $props();

  const ARTICLES = $derived(localizedArticles());
  const index = $derived(ARTICLES.findIndex((a) => a.id === article.id));
  const previous = $derived(index > 0 ? ARTICLES[index - 1] : undefined);
  const next = $derived(index >= 0 && index < ARTICLES.length - 1 ? ARTICLES[index + 1] : undefined);
  /** Suivant du même genre (tutoriel après tutoriel), sinon simplement le suivant. */
  const related = $derived(ARTICLES.slice(index + 1).find((a) => a.kind === article.kind) ?? next);

  let scroller = $state<HTMLElement>();
  // Un autre article s'ouvre : on revient en haut.
  $effect(() => {
    void article.id;
    scroller?.scrollTo({ top: 0 });
  });

  // Bandeau « avant / après » : le curseur se glisse (souris ou flèches du clavier).
  let split = $state(50);
  let hero = $state<HTMLElement>();
  function drag(event: PointerEvent): void {
    if (!hero) return;
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      const box = hero?.getBoundingClientRect();
      if (box) split = Math.min(98, Math.max(2, ((e.clientX - box.left) / box.width) * 100));
    };
    move(event);
    const stop = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", stop);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", stop);
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.key === "Escape") onclose();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<svelte:window onkeydowncapture={onKeyDown} />

{#snippet rich(text: string)}
  {#each runs(text) as run, i (i)}{#if run.kind === "key"}<kbd>{run.text}</kbd>{:else if run.kind === "bold"}<strong>{run.text}</strong>{:else}{run.text}{/if}{/each}
{/snippet}

<div class="backdrop" role="presentation" onclick={(event) => event.target === event.currentTarget && onclose()}>
  <div class="frame">
    <article class="window" aria-label={t(article.title)}>
      <div class="scroll" bind:this={scroller}>
        {#if article.hero === "beforeAfter" && banner}
          <!-- Avant / après : à droite du curseur, la photo corrigée. -->
          <div class="hero" bind:this={hero}>
            <img src={banner} alt="" />
            <div class="after" style:clip-path="inset(0 0 0 {split}%)"><img src={banner} alt="" /></div>
            <button
              class="divider"
              style:left="{split}%"
              aria-label={t("Glisser pour comparer avant et après")}
              onpointerdown={drag}
              onkeydown={(event) => {
                if (event.key === "ArrowLeft") split = Math.max(2, split - 4);
                else if (event.key === "ArrowRight") split = Math.min(98, split + 4);
              }}
            >
              <span class="knob"><svg viewBox="0 0 24 24"><path d="M9 6v12l-6-6zM15 6v12l6-6z" /></svg></span>
            </button>
            <span class="badge left">{t("Avant")}</span>
            <span class="badge right">{t("Après")}</span>
          </div>
        {:else}
          <ArticleArt kind={article.hero === "beforeAfter" ? "banner" : article.hero} size="hero" />
        {/if}

        <header>
          <div class="meta">
            <span class="tag" data-kind={article.kind}>{t(KIND_LABEL[article.kind])}{article.version ? ` ${article.version}` : ""}</span>
            <span>{articleDate(article.date)}</span><span>·</span><span>{t("{minutes} min de lecture", { minutes: article.minutes })}</span>
            {#if article.level}<span>·</span><span>{t("Niveau {level}", { level: t(article.level).toLowerCase() })}</span>{/if}
          </div>
          <h1>{t(article.title)}</h1>
        </header>

        <div class="body">
          <p class="intro">{@render rich(article.intro)}</p>
          {#each article.blocks as block, b (b)}
            {#if block.type === "p"}
              <p>{@render rich(block.text)}</p>
            {:else if block.type === "h"}
              <h2>{block.text}</h2>
            {:else if block.type === "list"}
              <ul>{#each block.items as item, i (i)}<li>{@render rich(item)}</li>{/each}</ul>
            {:else if block.type === "steps"}
              <ol>
                {#each block.items as item, i (i)}
                  <li><span class="number">{i + 1}</span><span><strong>{@render rich(item.title)}</strong> {@render rich(item.text)}</span></li>
                {/each}
              </ol>
            {:else if block.type === "tip"}
              <div class="tip">
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 1.5q1.2 7.3 8.5 8.5q-7.3 1.2-8.5 8.5q-1.2-7.3-8.5-8.5q7.3-1.2 8.5-8.5z" /></svg>
                <span><strong>{t("Astuce")}</strong> · {@render rich(block.text)}</span>
              </div>
            {:else if block.type === "image"}
              <figure>
                <img class="shot" src={illustration(block.name)} alt="" />
                <figcaption>{@render rich(block.caption)}</figcaption>
              </figure>
            {:else if block.type === "gif"}
              {@const film = demoSources(block.name)}
              <figure>
                <!-- Démonstration filmée : lecteur complet, avec contrôles. On peut mettre en pause, revenir en
                     arrière, passer en plein écran — ce qu'une animation en boucle ne permet pas. Elle démarre
                     seule et sans son, comme une animation, pour que le geste soit visible sans rien cliquer. -->
                {#if film.length > 0}
                  <FilmPlayer article={block.name} />
                {:else}
                  <img class="shot" src={tutorialAnimation(block.name)} alt="" />
                {/if}
                <figcaption>
                  {#if !hasTutorialAnimation(block.name)}<span class="todo">{t("Démonstration à venir")}</span>{/if}{@render rich(block.caption)}
                </figcaption>
              </figure>
            {:else}
              <figure>
                <ArticleFigure figure={block.figure} />
                <figcaption>{@render rich(block.caption)}</figcaption>
              </figure>
            {/if}
          {/each}

          {#if related}
            <button class="next" onclick={() => onnavigate(related)}>
              <ArticleArt kind={related.illustration} />
              <span class="next-texts">
                <span class="next-label">{related.kind === article.kind ? t("{kind} suivant", { kind: t(KIND_LABEL[related.kind]) }) : t("Article suivant")}</span>
                <span class="next-title">{t(related.title)}</span>
              </span>
              <svg class="chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 4.8l7.2 7.2L9 19.2" /></svg>
            </button>
          {/if}
        </div>
      </div>
    </article>

    <!-- Commandes flottantes, à droite de la fenêtre -->
    <div class="controls">
      <button class="round" aria-label={t("Fermer (Échap)")} title={t("Fermer (Échap)")} onclick={onclose}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
      <div class="pair">
        <button aria-label={t("Article précédent")} title={t("Article précédent")} disabled={!previous} onclick={() => previous && onnavigate(previous)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4.8 15.4L12 8.2l7.2 7.2" /></svg>
        </button>
        <span class="rule"></span>
        <button aria-label={t("Article suivant")} title={t("Article suivant")} disabled={!next} onclick={() => next && onnavigate(next)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4.8 8.6l7.2 7.2 7.2-7.2" /></svg>
        </button>
      </div>
      <span class="position">{index + 1} / {ARTICLES.length}</span>
    </div>
  </div>
</div>

<style>
  /* L'accueil derrière : flouté et assombri, façon verre dépoli. */
  .backdrop {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: flex;
    justify-content: center;
    padding-top: 40px;
    background: rgba(8, 8, 10, 0.35); /* le flou de l'accueil est posé par HomeScreen */
    animation: fade 180ms var(--ease);
  }
  .frame {
    position: relative;
    width: min(880px, calc(100% - 160px));
    height: 100%;
  }
  .window {
    height: 100%;
    overflow: hidden;
    border-radius: var(--radius-l) var(--radius-l) 0 0;
    background: var(--bg-panel);
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-bottom: 0;
    box-shadow: 0 40px 120px rgba(0, 0, 0, 0.7);
    animation: rise 260ms var(--ease);
  }
  .scroll {
    height: 100%;
    overflow-y: auto;
    scrollbar-width: thin;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(24px);
    }
  }

  /* bandeau avant / après */
  .hero {
    position: relative;
    height: 300px;
    overflow: hidden;
    user-select: none;
  }
  .hero img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 45%;
    display: block;
    pointer-events: none;
  }
  .after {
    position: absolute;
    inset: 0;
  }
  .after img {
    filter: saturate(1.7) sepia(0.25) hue-rotate(-12deg) contrast(1.08) brightness(1.04);
  }
  .divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 32px;
    margin-left: -16px;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: ew-resize;
    display: grid;
    place-items: center;
  }
  .divider::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 15px;
    width: 2px;
    background: #fff;
  }
  .knob {
    position: relative;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: #fff;
    display: grid;
    place-items: center;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
  }
  .knob svg {
    width: 16px;
    height: 16px;
    fill: #121111;
  }
  .badge {
    position: absolute;
    bottom: 18px;
    height: 28px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: rgba(12, 12, 13, 0.72);
    font-size: 12px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .badge.left {
    left: 20px;
  }
  .badge.right {
    right: 20px;
  }

  /* en-tête */
  header {
    padding: 26px 48px 24px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.07);
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 10px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .tag {
    height: 24px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .tag[data-kind="tutorial"] {
    background: rgba(127, 214, 164, 0.14);
    color: #7fd6a4;
  }
  .tag[data-kind="tip"] {
    background: rgba(240, 184, 107, 0.14);
    color: #f0b86b;
  }
  .tag[data-kind="update"] {
    background: rgba(77, 141, 255, 0.16);
    color: #8fb6ff;
  }
  h1 {
    margin: 0;
    font-size: 32px;
    font-weight: 600;
    line-height: 1.2;
    letter-spacing: -0.6px;
  }

  /* corps */
  .body {
    padding: 28px 48px 48px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    font-size: 15px;
    line-height: 1.65;
    color: var(--text-2);
    user-select: text;
  }
  .body p {
    margin: 0;
  }
  .intro {
    font-size: 16px;
    font-weight: 500;
    color: var(--text);
  }
  h2 {
    margin: 8px 0 0;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.2px;
    color: var(--text);
  }
  strong {
    color: var(--text);
    font-weight: 600;
  }
  kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 7px;
    box-sizing: border-box;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-bottom-width: 2px;
    font: 500 12px "Geist Mono", ui-monospace, monospace;
    color: var(--text);
    vertical-align: 1px;
  }
  ul {
    margin: 0;
    padding: 0 0 0 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  ol li {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .number {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--text);
    color: var(--bg-app);
    font-size: 13px;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .tip {
    margin-top: 4px;
    padding: 16px 18px;
    border-radius: var(--radius-l);
    background: rgba(240, 184, 107, 0.1);
    display: flex;
    gap: 14px;
    align-items: flex-start;
    font-size: 14px;
    line-height: 1.55;
    color: #e8d6b8;
  }
  .tip svg {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    margin-top: 2px;
    fill: #f0b86b;
  }
  .tip strong {
    color: #f0b86b;
  }
  figure {
    margin: 4px 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  figcaption {
    font-size: 13px;
    color: var(--text-muted);
  }
  /* Emplacement d'animation non encore tourné : la légende le dit, pour ne pas laisser croire
     que l'animation de remplacement illustre vraiment l'outil. */
  .todo {
    margin-right: 8px;
    padding: 1px 7px;
    border-radius: var(--radius-pill);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
    background: var(--bg-field);
    border: 1px solid var(--border-strong);
  }

  /* Illustration ou animation : pleine largeur du texte, fond noir tant que l'image charge. */
  .shot {
    display: block;
    width: 100%;
    border-radius: var(--radius-l);
    background: #000;
    border: 1px solid var(--border);
  }

  /* article suivant */
  .next {
    margin-top: 12px;
    padding: 14px;
    border-radius: var(--radius-l);
    border: 1px solid rgba(255, 255, 255, 0.07);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 16px;
    transition: background 120ms var(--ease);
  }
  .next:hover {
    background: #202024;
  }
  .next-texts {
    flex-grow: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
    line-height: 1.3;
  }
  .next-label {
    font-size: 12px;
    color: var(--text-muted);
  }
  .next-title {
    font-size: 15px;
    font-weight: 600;
  }
  .chevron {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: var(--text-3);
    stroke-width: 3;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* commandes flottantes */
  .controls {
    position: absolute;
    top: 0;
    left: calc(100% + 16px);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    animation: fade 260ms var(--ease);
  }
  .round,
  .pair {
    border: 1px solid rgba(255, 255, 255, 0.14);
    background: rgba(30, 30, 34, 0.72);
    backdrop-filter: blur(12px);
    -webkit-backdrop-filter: blur(12px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .round {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--text);
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .pair {
    width: 44px;
    border-radius: var(--radius-l);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .pair button {
    height: 44px;
    border: 0;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    display: grid;
    place-items: center;
  }
  .pair button:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .round:hover,
  .pair button:not(:disabled):hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .rule {
    height: 1px;
    margin: 0 10px;
    background: rgba(255, 255, 255, 0.1);
  }
  .controls svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 3.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .position {
    font-size: 11px;
    color: var(--text-3);
  }
</style>
