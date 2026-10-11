<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { app } from "../state/app.svelte";

  // Bouton maison, tout à gauche de la barre de menus. Un seul bouton, deux états : sur un document
  // il ouvre l'accueil, sur l'accueil il reprend le travail. La pastille compte les documents ouverts,
  // et la bulle de survol dit ce que le clic va faire, en listant ce qui est ouvert.
  const count = $derived(app.documents.length);
  const resuming = $derived(app.homeVisible && count > 0);
  const current = $derived(app.documents.find((doc) => doc.info.id === app.activeId) ?? app.documents[0]);

  /** Sans document ouvert, l'accueil est déjà là et le restera : le bouton n'a rien à faire. */
  const usable = $derived(count > 0);

  function toggle(): void {
    if (!usable) return;
    app.homeOpen = !resuming;
  }

  const DELAY = 400;
  let hovered = $state(false);
  let timer: number | undefined;

  function enter(): void {
    clearTimeout(timer);
    timer = window.setTimeout(() => (hovered = true), DELAY);
  }
  function leave(): void {
    clearTimeout(timer);
    hovered = false;
  }

  // Échap depuis l'accueil revient au travail, comme le bouton.
  function onKeyDown(event: KeyboardEvent): void {
    if (event.key !== "Escape" || !resuming || app.preferencesOpen) return;
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement) return;
    app.homeOpen = false;
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<div class="host" onmouseenter={enter} onmouseleave={leave} role="presentation">
  <button
    class="home"
    class:lit={resuming}
    disabled={!usable}
    aria-label={resuming ? t("Reprendre le travail") : t("Écran d'accueil")}
    onclick={() => {
      leave();
      toggle();
    }}
  >
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 11.5L12 5l8 6.5M6.5 10v9h11v-9M10 19v-5h4v5" /></svg>
    <!-- Un point, pas un compte : le nombre exact se lit dans la bulle et dans les onglets. -->
    {#if count > 0}<span class="dot" aria-hidden="true"></span>{/if}
  </button>

  {#if hovered}
    <!-- Bulle d'information : elle dit ce que le clic fera et ce qui est ouvert. On ne clique pas
         dedans (la liste complète est dans le bouton des onglets), donc elle ne gêne jamais le pointeur. -->
    <aside class="bubble" role="tooltip">
      <div class="head">
        <b>{resuming ? t("Reprendre") : t("Écran d'accueil")}</b>
        {#if resuming}<span class="key">{t("Échap")}</span>{/if}
      </div>
      {#if count > 0}
        <p class="what">
          {resuming ? `Retour à ${current?.info.name ?? t("votre document")}.` : t("Vos documents restent ouverts.")}
        </p>
        <ul>
          {#each app.documents.slice(0, 6) as doc (doc.info.id)}
            <li class:on={doc.info.id === app.activeId}>{doc.info.name}{app.isModified(doc) ? " *" : ""}</li>
          {/each}
          {#if count > 6}<li class="more">et {count - 6} autres…</li>{/if}
        </ul>
      {:else}
        <p class="what">{t("Ouvrez ou créez un document pour commencer.")}</p>
      {/if}
    </aside>
  {/if}
</div>

<style>
  .host {
    position: relative;
    display: flex;
    margin-right: 8px;
  }
  .home {
    position: relative;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-s);
    color: var(--text-2);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  .home:not(:disabled):hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .home.lit {
    background: var(--bg-active);
    color: var(--text);
  }
  .home:disabled {
    color: var(--text-disabled);
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
    width: 15px;
    height: 15px;
  }
  .dot {
    position: absolute;
    right: -1px;
    top: -1px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 2px solid var(--bg-bar);
    background: var(--accent);
  }
  .bubble {
    position: absolute;
    left: 0;
    top: calc(100% + 6px);
    z-index: 60;
    width: 230px;
    padding: 9px 11px 10px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    pointer-events: none;
    animation: appear 120ms var(--ease);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  .head b {
    font-size: 12.5px;
    font-weight: 600;
  }
  .key {
    margin-left: auto;
    color: var(--text-muted);
    font-size: 10.5px;
  }
  .what {
    margin: 3px 0 0;
    color: var(--text-3);
    font-size: 11.5px;
    line-height: 1.4;
  }
  ul {
    margin: 7px 0 0;
    padding: 7px 0 0;
    border-top: 1px solid var(--border);
    list-style: none;
    display: grid;
    gap: 2px;
  }
  li {
    color: var(--text-muted);
    font-size: 11.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  li.on {
    color: var(--text);
  }
  li.on::before {
    content: "● ";
    color: var(--accent);
    font-size: 9px;
  }
  li.more {
    color: var(--text-disabled);
  }
</style>
