<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { documentTitle, isActionEnabled, runAction } from "../lib/actions";
  import { layerDrag } from "../lib/layer-drag.svelte";
  import { proofShortName } from "../lib/proof-profiles";
  import { app } from "../state/app.svelte";
  import ContextMenu, { type MenuEntry } from "./ContextMenu.svelte";
  import Icon from "./Icon.svelte";

  // Glisser des calques vers un autre document : s'attarder sur son onglet l'ouvre (on dépose ensuite dans son
  // panneau Calques, à l'endroit voulu) ; lâcher sur l'onglet lui-même les pose en haut de sa pile. Le geste est
  // mené par lib/layer-drag.svelte.ts, qui repère l'onglet survolé par son attribut `data-doc` ; la barre ne fait
  // que le marquer.

  const percent = (zoom: number | undefined) => (zoom ? `${Math.round(zoom * 1000) / 10} %` : "");
  // Mode affiché après le zoom : « RVB/8 », ou le profil simulé quand l'épreuve est active (« CMJN FOGRA39 »).
  const mode = $derived(app.proofProfile ? `${t("CMJN")} ${proofShortName(app.proofProfile)}` : "RVB/8");

  // ---- Atteindre un document quand il y en a plus que la barre n'en montre.
  //
  // Trois moyens qui se complètent : l'onglet est borné en largeur et son nom se termine en « … » ;
  // la barre défile (molette, et elle suit l'onglet actif) ; et la liste complète s'ouvre à droite.
  let strip = $state<HTMLElement>();
  let menu = $state<{ x: number; y: number } | null>(null);

  /** L'onglet actif revient dans le champ, même quand il a été choisi au clavier ou par la liste. */
  $effect(() => {
    const id = app.activeId;
    const element = strip?.querySelector<HTMLElement>(`[data-doc="${id}"]`);
    element?.scrollIntoView({ block: "nearest", inline: "nearest" });
  });

  function onWheel(event: WheelEvent): void {
    if (!strip || (event.deltaY === 0 && event.deltaX === 0)) return;
    // La molette verticale, seule disponible sur la plupart des souris, fait défiler la barre.
    const step = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
    strip.scrollLeft += step;
    event.preventDefault();
  }

  function documentEntries(): MenuEntry[] {
    const list: MenuEntry[] = app.documents.map((doc) => ({
      // Une puce désigne le document affiché : la liste sert d'abord à savoir où l'on est.
      label: `${doc.info.id === app.activeId ? "● " : "   "}${documentTitle(doc.info)}${app.isModified(doc) ? " *" : ""}`,
      run: () => void (app.activeId = doc.info.id),
    }));
    return [
      ...list,
      "separator",
      { label: t("Document suivant"), shortcut: "Ctrl+Tab", disabled: app.documents.length < 2, run: () => runAction("doc.next") },
      { label: t("Fermer le document"), shortcut: "Ctrl+W", disabled: !isActionEnabled("file.close"), run: () => runAction("file.close") },
    ];
  }
</script>

<div class="bar">
  <div class="strip" role="tablist" bind:this={strip} onwheel={onWheel}>
    {#each app.documents as doc (doc.info.id)}
      {@const active = doc.info.id === app.activeId}
      <div
        class="tab"
        class:active
        class:smart={doc.info.smartParent !== null}
        class:hovered={layerDrag.tab === doc.info.id}
        data-doc={doc.info.id}
        role="presentation"
      >
        <button role="tab" aria-selected={active} title={doc.info.smartParent ? `${documentTitle(doc.info)} — ${t("Ctrl+S met à jour l'objet dynamique")}` : doc.info.name} onclick={() => (app.activeId = doc.info.id)}>
          <span class="name">{documentTitle(doc.info)}{app.isModified(doc) ? " *" : ""}</span>
          {#if active}<span class="about">@ {percent(doc.camera?.zoom)} ({mode})</span>{/if}
        </button>
        {#if active}
          <button class="close" aria-label={t("Fermer le document")} onclick={() => runAction("file.close")}>
            <Icon name="close" size={11} />
          </button>
        {/if}
      </div>
    {/each}
  </div>

  {#if app.documents.length > 1}
    <button
      class="more"
      aria-label={t("Tous les documents ouverts")}
      title={t("Tous les documents ouverts")}
      onclick={(event) => {
        const box = event.currentTarget.getBoundingClientRect();
        menu = { x: box.right - 260, y: box.bottom + 4 };
      }}
    >
      <Icon name="caret" size={13} />
      <span class="count">{app.documents.length}</span>
    </button>
  {/if}
</div>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} entries={documentEntries()} onclose={() => (menu = null)} />
{/if}

<style>
  .bar {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: stretch;
    background: var(--bg-bar);
    border-bottom: 1px solid var(--border);
  }
  .strip {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-end;
    gap: 2px;
    padding: 0 8px;
    overflow-x: auto;
    overflow-y: hidden;
    /* Barre de défilement masquée : la molette et la liste suffisent, et 6 px de plus
       gâcheraient une barre de 34 px de haut. */
    scrollbar-width: none;
  }
  .strip::-webkit-scrollbar {
    display: none;
  }
  .tab {
    height: 28px;
    flex-shrink: 0;
    /* Borné : dix documents tiennent à l'écran là où quatre noms longs le remplissaient. */
    max-width: 170px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px 0 12px;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: var(--radius-m) var(--radius-m) 0 0;
    color: var(--text-muted);
    white-space: nowrap;
  }
  /* Contenu d'un objet dynamique : son titre dit d'où il vient, il a droit à plus de place ; un filet rouge le distingue. */
  .tab.smart {
    max-width: 320px;
  }
  /* Onglet actif : le zoom et le mode s'y ajoutent, le titre garde sa place. */
  .tab.smart.active {
    max-width: 440px;
  }
  .tab.smart.active {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .tab.hovered {
    outline: 1px solid rgba(120, 140, 255, 0.9);
    outline-offset: -1px;
  }
  .tab.active {
    background: var(--bg-canvas);
    border-color: var(--border);
    color: var(--text);
  }
  .tab > button {
    height: 100%;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .about {
    flex-shrink: 0;
    color: var(--text-muted);
  }
  .close {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 18px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
  }
  .close:hover {
    color: var(--text);
  }
  .more {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 0 10px;
    border-left: 1px solid var(--border);
    color: var(--text-2);
  }
  .more:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .count {
    padding: 0 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-pill);
    background: var(--bg-field);
    font-size: 11px;
  }
</style>
