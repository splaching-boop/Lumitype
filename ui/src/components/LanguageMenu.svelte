<script lang="ts">
  import { i18n, LOCALES, type Locale, t } from "../lib/i18n.svelte";

  // Choix de la langue, en bas du menu de l'accueil : le drapeau de la langue en cours, et un clic
  // déplie la liste des autres. Le changement est immédiat, sans redémarrage, et retenu.
  let open = $state(false);
  let root = $state<HTMLDivElement>();

  const others = $derived(LOCALES.filter((l) => l.code !== i18n.locale));

  function choose(code: Locale): void {
    i18n.locale = code;
    open = false;
  }
</script>

<!-- Les drapeaux : quelques rectangles, dessinés ici pour ne dépendre d'aucune police d'emoji
     (WebKitGTK et WebView2 n'en ont pas la même, et certaines n'en ont aucune). -->
{#snippet flag(code: Locale)}
  {#if code === "fr"}
    <svg viewBox="0 0 3 2" aria-hidden="true"><rect width="1" height="2" fill="#0055a4" /><rect x="1" width="1" height="2" fill="#fff" /><rect x="2" width="1" height="2" fill="#ef4135" /></svg>
  {:else if code === "en"}
    <svg viewBox="0 0 60 40" aria-hidden="true">
      <rect width="60" height="40" fill="#012169" />
      <path d="M0 0L60 40M60 0L0 40" stroke="#fff" stroke-width="7" />
      <path d="M0 0L60 40M60 0L0 40" stroke="#c8102e" stroke-width="3" />
      <path d="M30 0v40M0 20h60" stroke="#fff" stroke-width="12" />
      <path d="M30 0v40M0 20h60" stroke="#c8102e" stroke-width="7" />
    </svg>
  {:else if code === "es"}
    <svg viewBox="0 0 3 2" aria-hidden="true"><rect width="3" height="2" fill="#aa151b" /><rect y="0.5" width="3" height="1" fill="#f1bf00" /></svg>
  {:else}
    <svg viewBox="0 0 3 2" aria-hidden="true"><rect width="3" height="2" fill="#000" /><rect y="0.667" width="3" height="0.667" fill="#d00" /><rect y="1.333" width="3" height="0.667" fill="#ffce00" /></svg>
  {/if}
{/snippet}

<svelte:window
  onpointerdown={(event) => {
    if (open && !(event.target instanceof Node && root?.contains(event.target))) open = false;
  }}
  onkeydown={(event) => {
    if (event.key === "Escape") open = false;
  }}
/>

<div class="language" bind:this={root}>
  <button class="current" aria-haspopup="listbox" aria-expanded={open} title={t("Langue de l'interface")} onclick={() => (open = !open)}>
    <span class="flag">{@render flag(i18n.locale)}</span>
    <span class="name">{i18n.info.name}</span>
    <svg class="chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" /></svg>
  </button>
  {#if open}
    <!-- Les autres langues, dépliées au-dessus du bouton : le bas du menu est déjà au bord de la fenêtre. -->
    <ul class="list" role="listbox" aria-label={t("Langue de l'interface")}>
      {#each others as locale (locale.code)}
        <li>
          <button role="option" aria-selected="false" lang={locale.code} onclick={() => choose(locale.code)}>
            <span class="flag">{@render flag(locale.code)}</span>
            <span class="name">{locale.name}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .language {
    position: relative;
  }
  .current,
  .list button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border-radius: var(--radius-m);
    color: var(--text-2);
    font-size: 12.5px;
    text-align: left;
  }
  .current:hover,
  .list button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .current[aria-expanded="true"] {
    background: var(--bg-active);
    color: var(--text);
  }
  .flag {
    width: 24px;
    height: 17px;
    flex-shrink: 0;
    border-radius: var(--radius-xs);
    overflow: hidden;
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.14);
  }
  .flag svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .name {
    flex: 1;
  }
  .chevron {
    width: 14px;
    height: 14px;
    color: var(--text-muted);
    transition: transform 120ms var(--ease);
  }
  .current[aria-expanded="true"] .chevron {
    transform: rotate(180deg);
  }
  .list {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + 6px);
    margin: 0;
    padding: 6px;
    list-style: none;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-menu);
    z-index: 5;
    animation: appear 120ms var(--ease);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
