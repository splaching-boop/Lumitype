<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { type RefCollectionSummary, type RefItem, refApi } from "../../lib/myref";
  import { app } from "../../state/app.svelte";
  import RefThumb from "./RefThumb.svelte";

  // Collections d'une image de My ref : un bouton « Graphisme +1 » qui ouvre la liste des collections, avec une
  // recherche, un + carré pour y ajouter l'image, un ✓ rouge pour l'en retirer, et « Créer une collection » avec elle
  // dedans. Une image rangée dans plusieurs collections y est copiée (même contenu) ; la retirer met cette copie à la
  // corbeille de la bibliothèque. Partagé par le survol des vignettes (`overlay`) et la visionneuse (`panel`).
  let {
    root,
    item,
    collections,
    variant = "overlay",
    open = $bindable(false),
    preload = false,
    onmembership,
    onremoved,
  }: {
    root: string;
    item: RefItem;
    collections: RefCollectionSummary[];
    variant?: "overlay" | "panel";
    open?: boolean;
    /** Charger tout de suite les collections de l'image (survol), pour afficher « +1 » sans attendre l'ouverture. */
    preload?: boolean;
    /** Image ajoutée à une collection ou retirée : la bibliothèque a changé. */
    onmembership: () => void;
    /** La copie affichée (celle de `item.collection`) vient d'être retirée. */
    onremoved?: (id: string) => void;
  } = $props();

  let members = $state<string[] | null>(null);
  let membersFor = "";
  let query = $state("");
  let busy = $state<string | null>(null);
  let creating = $state<string | null>(null);
  let host = $state<HTMLElement>();
  /** Liste ouverte vers le haut : l'image est trop bas dans la fenêtre pour la dérouler dessous. */
  let up = $state(false);
  /** Hauteur laissée aux collections (la place disponible du côté choisi, sans recherche ni « Créer »). */
  let rowsMax = $state(240);

  async function loadMembers(): Promise<void> {
    if (members && membersFor === item.id) return;
    const id = item.id;
    membersFor = id;
    const found = await refApi.memberships(root, id).catch(() => [item.collection]);
    if (membersFor === id) members = found;
  }
  // Autre image (visionneuse) : ses collections sont relues.
  $effect(() => {
    void item.id;
    if (membersFor !== item.id) {
      members = null;
      if (preload || open || variant === "panel") void loadMembers();
    }
  });
  $effect(() => {
    if (preload) void loadMembers();
  });

  const label = $derived.by(() => {
    const list = members ?? [item.collection];
    const first = list.includes(item.collection) ? item.collection : (list[0] ?? item.collection);
    return { first, more: Math.max(0, list.length - 1) };
  });
  const shown = $derived(collections.filter((c) => c.name.toLowerCase().includes(query.trim().toLowerCase())));

  async function toggle(name: string): Promise<void> {
    if (busy || !members) return;
    busy = name;
    try {
      if (members.includes(name)) {
        await refApi.removeFrom(root, item.id, name);
        members = members.filter((m) => m !== name);
        if (name === item.collection) onremoved?.(item.id);
      } else {
        await refApi.copyItem(root, item.id, name);
        members = [...members, name];
      }
      onmembership();
    } catch (error) {
      app.fail(error);
    } finally {
      busy = null;
    }
  }

  async function create(): Promise<void> {
    const wanted = creating?.trim();
    if (!wanted) return;
    busy = "+";
    try {
      const name = await refApi.createCollection(root, wanted);
      await refApi.copyItem(root, item.id, name);
      members = [...(members ?? [item.collection]), name];
      creating = null;
      onmembership();
    } catch (error) {
      app.fail(error);
    } finally {
      busy = null;
    }
  }

  function toggleOpen(): void {
    open = !open;
    if (open) {
      // Place dessous et dessus : la liste se déroule du côté qui a le plus de place quand elle ne tient pas dessous.
      const rect = host?.getBoundingClientRect();
      const below = rect ? window.innerHeight - rect.bottom - 12 : 1000;
      const above = rect ? rect.top - 12 : 0;
      const wanted = 400; // recherche + collections + « Créer une collection »
      up = variant === "overlay" && below < wanted && above > below;
      rowsMax = Math.max(120, Math.min(240, (up ? above : below) - 150));
      query = "";
      void loadMembers();
    }
  }

  // Clic ailleurs : la liste se referme.
  $effect(() => {
    if (!open) return;
    const away = (event: PointerEvent) => {
      if (host && !host.contains(event.target as Node)) open = false;
    };
    window.addEventListener("pointerdown", away, true);
    return () => window.removeEventListener("pointerdown", away, true);
  });
</script>

<div class="picker {variant}" bind:this={host}>
  <button type="button" class="select" aria-haspopup="listbox" aria-expanded={open} onclick={toggleOpen}>
    {#if variant === "panel"}<svg class="folder" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7h7l2 2h9v10H3z" /></svg>{/if}
    <span class="name">{label.first}</span>
    {#if label.more}<span class="more">+{label.more}</span>{/if}
    <svg viewBox="0 0 12 12" aria-hidden="true"><path d={open ? "M2 8l4-4 4 4" : "M2 4l4 4 4-4"} /></svg>
  </button>
  {#if open}
    <div class="list" class:up role="listbox" aria-label={t("Collections")}>
      <!-- svelte-ignore a11y_autofocus -->
      <input type="search" placeholder={t("Rechercher une collection…")} bind:value={query} autofocus />
      <div class="rows" style:max-height="{rowsMax}px">
        {#each shown as c (c.name)}
          {@const inside = members?.includes(c.name) ?? false}
          {@const cover = c.cover ?? c.covers[0] ?? null}
          <div class="row">
            <span class="swatch">{#if cover}<RefThumb {root} item={cover} fit="cover" />{/if}</span>
            <span class="text"><b>{c.name}</b><small>{c.count === 1 ? t("1 élément") : t("{count} éléments", { count: c.count })}</small></span>
            <button
              type="button"
              class="square"
              class:in={inside}
              class:busy={busy === c.name}
              disabled={!members || busy !== null}
              aria-label={inside ? t("Retirer de {name}", { name: c.name }) : t("Ajouter à {name}", { name: c.name })}
              title={inside ? t("Retirer de {name}", { name: c.name }) : t("Ajouter à {name}", { name: c.name })}
              onclick={() => void toggle(c.name)}
            >
              {#if inside}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>{:else}<svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg>{/if}
            </button>
          </div>
        {/each}
      </div>
      {#if creating === null}
        <button type="button" class="create" onclick={() => (creating = "")}>
          <span class="dashed"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3v10M3 8h10" /></svg></span>
          <span class="text"><b>{t("Créer une collection")}</b><small>{t("Avec cette image dedans")}</small></span>
        </button>
      {:else}
        <form class="create" onsubmit={(event) => (event.preventDefault(), void create())}>
          <!-- svelte-ignore a11y_autofocus -->
          <input bind:value={creating} placeholder={t("Nom de la collection")} autofocus onkeydown={(e) => e.key === "Escape" && (creating = null)} />
          <button type="submit" class="ok" disabled={!creating.trim() || busy !== null}>{t("Créer")}</button>
        </form>
      {/if}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }
  .select {
    width: 100%;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: var(--radius-m);
    background: rgba(20, 20, 22, 0.88);
    color: #fff;
    backdrop-filter: blur(6px);
    text-align: left;
  }
  .select .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .select .more {
    color: rgba(255, 255, 255, 0.6);
    font-size: 11px;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .select svg {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
  }
  .list {
    position: absolute;
    left: 0;
    right: 0;
    top: 38px;
    width: max(100%, 270px);
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--bg-menu);
    box-shadow: var(--shadow-menu);
    overflow: hidden;
  }
  /* Vers le haut : la liste s'ouvre au-dessus du bouton, la recherche reste en tête. */
  .list.up {
    top: auto;
    bottom: 38px;
  }
  .list input {
    margin: 8px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
  }
  .rows {
    max-height: 240px;
    overflow-y: auto;
  }
  .row,
  .create {
    display: grid;
    grid-template-columns: 34px 1fr 30px;
    gap: 10px;
    align-items: center;
    width: 100%;
    padding: 6px 8px;
    text-align: left;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .swatch {
    width: 34px;
    height: 34px;
    overflow: hidden;
    border-radius: var(--radius-s);
    background: var(--bg-panel);
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .text b {
    font-weight: 600;
    color: var(--text-strong, var(--text));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text small {
    color: var(--text-muted);
    font-size: 11px;
  }
  /* + carré : visible au survol de la ligne ; ✓ rouge : déjà dans la collection (clic : retirer). */
  .square {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    border: 1px solid var(--border-strong);
    color: var(--text);
    opacity: 0;
    transition: opacity 100ms;
  }
  .row:hover .square,
  .square:focus-visible,
  .square.in {
    opacity: 1;
  }
  .square svg {
    width: 15px;
    height: 15px;
  }
  .square.in {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .square.busy {
    opacity: 0.5;
  }
  .create {
    border-top: 1px solid var(--border);
    padding: 8px;
  }
  form.create {
    grid-template-columns: 1fr auto;
  }
  form.create input {
    margin: 0;
  }
  .create .ok {
    height: 30px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
  }
  .create .ok:disabled {
    opacity: 0.4;
  }
  .dashed {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .dashed svg {
    width: 14px;
    height: 14px;
  }
    /* Visionneuse : bouton pleine largeur comme les autres de la fiche, liste dans le flux sous lui. */
  .panel .select {
    height: 44px;
    gap: 10px;
    padding: 0 14px;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    backdrop-filter: none;
  }
  .panel .select .more {
    color: var(--text-muted);
  }
  .panel .select svg.folder {
    width: 18px;
    height: 18px;
  }
  .panel .list {
    position: static;
    width: 100%;
    margin-top: 4px;
  }
  .panel .square {
    opacity: 1;
  }
</style>
