<script lang="ts">
  import { tick } from "svelte";
  import { ACTION_LABELS } from "../../lib/action-catalog";
  import type { ActionId } from "../../lib/actions";
  import { t } from "../../lib/i18n.svelte";
  import { MANUAL_SECTIONS, type ManualBlock, blockAnchor, manualImage, tm } from "../../lib/manual";
  import { runs } from "../../lib/rich-text";
  import { keymap } from "../../state/keymap.svelte";
  import ToolDemoCard from "./ToolDemoCard.svelte";

  // Onglet « Notice » de l'accueil (maquette validée « Notice Lumitype ») : sommaire fixe à gauche, texte à droite
  // (une colonne d'environ 66 caractères). La section lue s'allume dans le sommaire pendant le défilement ; un clic
  // dans le sommaire y mène. `request` : ancre où se placer (menu Aide, recherche de l'accueil), surlignée un instant.
  let { request = null }: { request?: { anchor: string; seq: number } | null } = $props();

  let root = $state<HTMLElement>();
  /** Section et sous-section allumées dans le sommaire. */
  let current = $state<{ section: string; sub: string | null }>({ section: MANUAL_SECTIONS[0]?.id ?? "", sub: null });
  /** Ancre surlignée un instant après une recherche. */
  let flashing = $state<string | null>(null);

  const domId = (anchor: string): string => `notice-${anchor}`;

  // ---- Raccourcis : une action affiche le raccourci réel de l'utilisateur ; `=Ctrl+V`, une touche fixe.
  const KEY_NAMES: Record<string, string> = { Ctrl: "Ctrl", Alt: "Alt", Shift: "Maj", Del: "Suppr", Enter: "Entrée", Esc: "Échap", Space: "Espace" };
  function keyLabel(key: string): string {
    if (key.startsWith("=")) return key.slice(1).split("+").map((part) => (KEY_NAMES[part] ? t(KEY_NAMES[part]) : part)).join("+");
    return key in ACTION_LABELS ? keymap.label(key as ActionId) : "";
  }
  /** Raccourcis à afficher, sans doublon ni vide (une action sans raccourci ne laisse pas de case vide). */
  const labels = (keys: string[] | undefined): string[] => [...new Set((keys ?? []).map(keyLabel).filter(Boolean))];

  /** Ligne « fonction » décrivant l'outil d'une carte : la première, après les cartes, au même raccourci. */
  function toolLine(owner: string, blocks: ManualBlock[], from: number, keys: string[] | undefined): string | null {
    const first = keys?.[0];
    const index = blocks.findIndex((block, i) => i > from && block.type === "fn" && block.keys?.[0] === first);
    return index >= 0 ? blockAnchor(owner, index) : null;
  }

  // ---- Défilement : le conteneur qui défile est le contenu de l'accueil (un ancêtre).
  let scroller: HTMLElement | null = null;
  /** Section choisie dans le sommaire : elle reste allumée jusqu'à ce que l'utilisateur fasse défiler lui-même. */
  let pinned = false;

  function findScroller(node: HTMLElement): HTMLElement | null {
    for (let parent = node.parentElement; parent; parent = parent.parentElement) {
      const overflow = getComputedStyle(parent).overflowY;
      if (overflow === "auto" || overflow === "scroll") return parent;
    }
    return null;
  }

  function follow(): void {
    if (pinned || !root || !scroller) return;
    const limit = scroller.getBoundingClientRect().top + 96;
    let section = current.section;
    let sub: string | null = null;
    for (const heading of root.querySelectorAll<HTMLElement>("[data-toc]")) {
      if (heading.getBoundingClientRect().top > limit) break;
      if (heading.dataset.level === "section") {
        section = heading.dataset.toc ?? section;
        sub = null;
      } else sub = heading.dataset.toc ?? null;
    }
    if (section !== current.section || sub !== current.sub) current = { section, sub };
  }

  $effect(() => {
    const node = root;
    if (!node) return;
    const found = findScroller(node);
    scroller = found;
    if (!found) return;
    const release = (): void => {
      pinned = false;
    };
    const events = ["wheel", "pointerdown", "keydown", "touchstart"] as const;
    for (const name of events) found.addEventListener(name, release, { passive: true });
    found.addEventListener("scroll", follow, { passive: true });
    return () => {
      for (const name of events) found.removeEventListener(name, release);
      found.removeEventListener("scroll", follow);
    };
  });

  /** Fait défiler jusqu'à une ancre ; `flash` la surligne un instant. Ancre vide : le début de la notice. */
  function goTo(anchor: string, flash: boolean): void {
    const target = anchor ? root?.querySelector<HTMLElement>(`#${CSS.escape(domId(anchor))}`) : null;
    if (!target) {
      scroller?.scrollTo({ top: 0 });
      return;
    }
    const section = MANUAL_SECTIONS.find((s) => s.id === anchor || anchor.startsWith(`${s.id}--`) || s.subsections?.some((sub) => sub.id === anchor || anchor.startsWith(`${sub.id}--`)));
    const sub = section?.subsections?.find((s) => s.id === anchor || anchor.startsWith(`${s.id}--`));
    if (section) {
      current = { section: section.id, sub: sub?.id ?? null };
      pinned = true;
    }
    target.scrollIntoView({ behavior: flash ? "auto" : "smooth", block: flash ? "center" : "start" });
    if (flash) {
      flashing = null;
      void tick().then(() => (flashing = anchor));
    }
  }

  $effect(() => {
    const wanted = request;
    if (!wanted) return;
    void tick().then(() => goTo(wanted.anchor, wanted.anchor !== ""));
  });
</script>

{#snippet rich(text: string)}
  {#each runs(text) as run, i (i)}{#if run.kind === "key"}<kbd>{run.text}</kbd>{:else if run.kind === "bold"}<b>{run.text}</b>{:else}{run.text}{/if}{/each}
{/snippet}

{#snippet keysOf(keys: string[] | undefined)}
  {@const shown = labels(keys)}
  {#if shown.length > 0}<span class="keys">{#each shown as label (label)}<kbd>{label}</kbd>{/each}</span>{/if}
{/snippet}

{#snippet blocks(owner: string, list: ManualBlock[])}
  {#each list as block, index (index)}
    {#if block.type === "p"}
      <p>{@render rich(tm(block.text))}</p>
    {:else if block.type === "h"}
      <h4 id={domId(block.id)}>{tm(block.text)}</h4>
    {:else if block.type === "fn"}
      {@const anchor = blockAnchor(owner, index)}
      <div class="fn" class:flash={flashing === anchor} id={domId(anchor)}>
        <b class="name">{tm(block.name)}</b>
        {@render keysOf(block.keys)}
        {#if block.text}
          <span class="text">{@render rich(tm(block.text))}</span>
        {:else if block.items}
          <span class="text">{block.items.map((item) => tm(item)).join(", ")}</span>
        {/if}
      </div>
    {:else if block.type === "note"}
      <p class="note">{@render rich(tm(block.text))}</p>
    {:else if block.type === "list"}
      <ul>{#each block.items as item, i (i)}<li>{@render rich(tm(item))}</li>{/each}</ul>
    {:else if block.type === "image"}
      {@const url = manualImage(block.src)}
      {#if url}
        <figure>
          <img src={url} alt={tm(block.alt)} loading="lazy" />
          {#if block.caption}<figcaption>{@render rich(tm(block.caption))}</figcaption>{/if}
        </figure>
      {/if}
    {:else if block.type === "tools"}
      <div class="tools">
        {#each block.tools as card (card.tool)}
          {@const line = toolLine(owner, list, index, card.keys)}
          <ToolDemoCard tool={card.tool} name={tm(card.name)} shortcut={labels(card.keys)[0] ?? ""} onselect={() => line && goTo(line, true)} />
        {/each}
      </div>
    {:else if block.type === "table"}
      <table>
        <thead><tr><th>{t("Action")}</th><th>{t("Raccourci")}</th></tr></thead>
        <tbody>
          {#each block.rows as row (row.label)}
            <tr><td>{tm(row.label)}</td><td>{@render keysOf(row.keys)}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
  {/each}
{/snippet}

<div class="manual" bind:this={root}>
  <nav class="toc" aria-label={t("Sommaire")}>
    <h4>{t("Sommaire")}</h4>
    {#each MANUAL_SECTIONS as section (section.id)}
      <button class:on={current.section === section.id && current.sub === null} class:within={current.section === section.id} onclick={() => goTo(section.id, false)}>{tm(section.short ?? section.title)}</button>
      {#if current.section === section.id || (section.subsections?.length ?? 0) <= 3}
        {#each section.subsections ?? [] as sub (sub.id)}
          <button class="sub" class:on={current.sub === sub.id} onclick={() => goTo(sub.id, false)}>{tm(sub.title)}</button>
        {/each}
      {/if}
    {/each}
  </nav>

  <article class="body">
    {#each MANUAL_SECTIONS as section (section.id)}
      <section>
        <h2 id={domId(section.id)} data-toc={section.id} data-level="section" class:flash={flashing === section.id}>{tm(section.title)}</h2>
        {@render blocks(section.id, section.blocks)}
        {#each section.subsections ?? [] as sub (sub.id)}
          <h3 id={domId(sub.id)} data-toc={sub.id} data-level="sub" class:flash={flashing === sub.id}>{tm(sub.title)}</h3>
          {@render blocks(sub.id, sub.blocks)}
        {/each}
      </section>
    {/each}
  </article>
</div>

<style>
  .manual {
    display: grid;
    grid-template-columns: 220px minmax(0, 1fr);
    gap: 40px;
    align-items: start;
    max-width: 1080px;
    font-size: 13px;
    line-height: 1.55;
  }

  /* ---- sommaire */
  .toc {
    position: sticky;
    top: 0;
    max-height: calc(100vh - 140px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }
  .toc h4 {
    margin: 0 0 8px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .toc button {
    padding: 5px 10px;
    border: 0;
    border-left: 2px solid transparent;
    background: transparent;
    color: var(--text-3);
    font: inherit;
    font-size: 13px;
    text-align: left;
    cursor: pointer;
  }
  .toc button.sub {
    padding-left: 22px;
    font-size: 12px;
  }
  .toc button:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .toc button.within {
    color: var(--text);
  }
  .toc button.on {
    color: var(--text-strong);
    border-left-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .toc button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  /* ---- texte */
  .body {
    min-width: 0;
    max-width: 66ch;
    padding-bottom: 40vh; /* la dernière section peut monter en haut de la fenêtre */
  }
  section + section h2 {
    margin-top: 40px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }
  h2 {
    margin: 0 0 10px;
    font-size: 19px;
    font-weight: 600;
    letter-spacing: -0.01em;
    text-wrap: balance;
  }
  h3 {
    margin: 26px 0 6px;
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
  }
  h4 {
    margin: 22px 0 8px;
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }
  h2,
  h3,
  h4,
  .fn {
    scroll-margin-top: 12px;
  }
  p {
    margin: 0 0 10px;
    color: var(--text-2);
  }
  b {
    font-weight: 600;
    color: var(--text);
  }
  ul {
    margin: 4px 0 12px;
    padding-left: 18px;
    color: var(--text-2);
  }
  li {
    margin: 3px 0;
  }
  li::marker {
    color: var(--accent);
  }

  /* Ligne « fonction » : nom en gras, raccourci aligné à droite, phrase grise dessous. */
  .fn {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 2px 16px;
    padding: 9px 0;
    border-top: 1px solid var(--border);
  }
  .fn .name {
    font-weight: 500;
  }
  .fn .text {
    grid-column: 1 / -1;
    color: var(--text-3);
  }
  .fn .text b {
    color: var(--text-2);
    font-weight: 600;
  }
  .keys {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-self: start;
    gap: 4px;
  }
  kbd {
    display: inline-block;
    padding: 1px 6px;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text-2);
    font: 11px/1.5 ui-monospace, monospace;
    white-space: nowrap;
  }
  .fn .text kbd,
  p kbd,
  li kbd {
    padding: 0 4px;
    font-size: 10.5px;
  }

  /* Encadré « À savoir » : filet d'accent à gauche. */
  .note {
    margin: 12px 0;
    padding: 8px 12px;
    border-left: 2px solid var(--accent);
    background: var(--bg-card);
  }

  figure {
    margin: 14px 0 16px;
  }
  figure img {
    display: block;
    max-width: 100%;
    height: auto;
    border: 1px solid var(--border);
  }
  figcaption {
    margin-top: 6px;
    font-size: 12px;
    color: var(--text-muted);
  }

  .tools {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 12px;
    margin: 12px 0 18px;
  }

  table {
    width: 100%;
    margin: 8px 0 12px;
    border-collapse: collapse;
  }
  th {
    padding: 6px 0;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    text-align: left;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border-strong);
  }
  th + th {
    text-align: right;
  }
  td {
    padding: 7px 0;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    vertical-align: top;
  }
  td + td {
    padding-left: 16px;
  }

  /* Surlignage bref de la ligne trouvée par la recherche. */
  .flash {
    animation: flash 1.6s var(--ease);
  }
  @keyframes flash {
    0%,
    35% {
      background: color-mix(in srgb, var(--accent) 22%, transparent);
      box-shadow: -8px 0 0 color-mix(in srgb, var(--accent) 22%, transparent), 8px 0 0 color-mix(in srgb, var(--accent) 22%, transparent);
    }
    100% {
      background: transparent;
      box-shadow: none;
    }
  }

  @media (max-width: 900px) {
    .manual {
      grid-template-columns: minmax(0, 1fr);
    }
    .toc {
      position: static;
      max-height: none;
      flex-direction: row;
      flex-wrap: wrap;
      gap: 4px;
    }
    .toc h4 {
      width: 100%;
    }
    .toc button.sub {
      display: none;
    }
  }
</style>
