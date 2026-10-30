<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount, tick, untrack } from "svelte";
  import { formatDate, t } from "../../lib/i18n.svelte";
  import { dialogExtensions, IMAGE_EXTENSIONS } from "../../lib/ipc";
  import {
    type Dropped,
    type RefCollection,
    type RefCollectionSummary,
    type RefItem,
    type RefOverview,
    type RefSort,
    carriesDrop,
    importDrop,
    loadRefRoot,
    readDrop,
    readWindowDrop,
    refApi,
    saveRefRoot,
    setWindowDropTarget,
    shortPath,
  } from "../../lib/myref";
  import { app } from "../../state/app.svelte";
  import MyRefConfirm from "./MyRefConfirm.svelte";
  import MyRefCoverPicker from "./MyRefCoverPicker.svelte";
  import MyRefLightbox from "./MyRefLightbox.svelte";
  import MyRefMenu, { type RefMenuEntry } from "./MyRefMenu.svelte";
  import MyRefTitle from "./MyRefTitle.svelte";
  import RefHover from "./RefHover.svelte";
  import RefThumb from "./RefThumb.svelte";

  // Vue « My ref » de l'accueil (maquettes validées : choix du dossier, collections, collection, visionneuse).
  // Bibliothèque d'images de référence rangée en fichiers ordinaires dans un dossier choisi (voir lib/myref.ts et
  // src-tauri/src/myref.rs). `replay` change à chaque clic sur l'onglet : le titre se réécrit, et l'on revient
  // aux collections.
  let { replay = 0 }: { replay?: number } = $props();

  let root = $state<string | null>(null);
  let ready = $state(false);
  let failure = $state<string | null>(null);
  let overview = $state<RefOverview | null>(null);
  let current = $state<RefCollection | null>(null);
  let query = $state("");
  let found = $state<RefItem[] | null>(null);
  let notice = $state("");
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  /** Nouvelle collection en cours de nommage (carte « Nouvelle collection »), avec les images déposées dessus. */
  let creating = $state<{ name: string; pending: Dropped | null } | null>(null);
  let createInput = $state<HTMLInputElement | null>(null);
  /** Cible de dépôt survolée : « new », « col:<nom> » ou « open » (collection affichée). */
  let dropTarget = $state<string | null>(null);
  let lightbox = $state<{ items: RefItem[]; index: number } | null>(null);
  let menu = $state<"import" | "sort" | "more" | null>(null);
  /** Bouton qui a ouvert le menu « Trier » ou « Plus » : le menu se pose dessous. */
  let menuAnchor = $state<HTMLElement | null>(null);
  /** Clic droit sur une collection de la vue d'ensemble : menu au point du clic. */
  let context = $state<{ summary: RefCollectionSummary; x: number; y: number } | null>(null);
  /** Suppression de collection en attente de confirmation. */
  let confirming = $state<{ name: string; count: number } | null>(null);
  /** Fenêtre « Changer la couverture… » ouverte pour cette collection. */
  let coverFor = $state<string | null>(null);
  let organizing = $state(false);
  /** Nom de la collection ouverte en cours de modification (Organiser, Plus › Renommer, F2), sinon `null`. */
  let renaming = $state<string | null>(null);
  let renameInput = $state<HTMLInputElement | null>(null);
  /** Renommage en place d'une collection de la vue d'ensemble (clic droit › Renommer, F2). */
  let renamingCard = $state<{ name: string; draft: string } | null>(null);
  let cardInput = $state<HTMLInputElement | null>(null);
  /** Glisser d'une image en mode Organiser : fichier tiré. */
  let dragging = $state<string | null>(null);

  const message = (error: unknown) => (typeof error === "string" ? error : ((error as { message?: string })?.message ?? String(error)));

  function say(text: string): void {
    notice = text;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = ""), 4000);
  }

  onMount(() => {
    void (async () => {
      root = await loadRefRoot();
      ready = true;
      if (root) await refresh();
    })();
    // Dépôts reçus par la fenêtre (Linux avec Chromium) : App.svelte nous les propose d'abord.
    setWindowDropTarget({
      hover: (position) => (dropTarget = position ? targetAt(position) : null),
      drop: (paths, position) => {
        const target = targetAt(position);
        dropTarget = null;
        if (!target) return false;
        // Le même glisser déjà reçu par la page : pris, mais pas importé deux fois.
        if (performance.now() - lastPageDrop < 1500) return true;
        void receive(target, readWindowDrop(paths));
        return true;
      },
    });
    return () => {
      setWindowDropTarget(null);
      clearTimeout(noticeTimer);
    };
  });

  // Clic sur l'onglet : retour aux collections (le titre, lui, est recréé par le bloc {#key}).
  let lastReplay = untrack(() => replay);
  /** Dernier clic sur l'onglet dont le titre a été écrit (variable ordinaire : lue à la création du titre). */
  const title = { played: -1 };
  $effect(() => {
    if (replay === lastReplay) return;
    lastReplay = replay;
    current = null;
    organizing = false;
    renaming = null;
    lightbox = null;
    context = null;
    query = "";
    if (root) void refresh();
  });

  async function refresh(): Promise<void> {
    if (!root) return;
    try {
      overview = await refApi.overview(root);
      if (current) current = await refApi.collection(root, current.name);
      failure = null;
    } catch (error) {
      failure = message(error);
    }
  }

  // ---- dossier

  async function chooseFolder(): Promise<void> {
    const chosen = await open({ directory: true, title: t("Dossier de My ref") }).catch(() => null);
    if (typeof chosen !== "string" || !chosen) return;
    root = chosen;
    current = null;
    await saveRefRoot(chosen).catch((error: unknown) => app.fail(error));
    await refresh();
  }

  // ---- recherche (nom, tag, note)

  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const text = query.trim();
    clearTimeout(searchTimer);
    if (!text || !root) {
      found = null;
      return;
    }
    const library = root;
    searchTimer = setTimeout(() => {
      refApi.search(library, text).then(
        (items) => (found = query.trim() === text ? items : found),
        (error: unknown) => app.fail(error),
      );
    }, 180);
  });

  // ---- collections

  async function openCollection(name: string): Promise<void> {
    if (!root) return;
    try {
      current = await refApi.collection(root, name);
      organizing = false;
      renaming = null;
      menu = null;
      context = null;
      query = "";
    } catch (error) {
      app.fail(error);
    }
  }

  async function startCreating(pending: Dropped | null = null): Promise<void> {
    creating = { name: t("Nouvelle collection"), pending };
    await tick();
    createInput?.focus();
    createInput?.select();
  }

  async function confirmCreate(): Promise<void> {
    if (!root || !creating) return;
    const { name, pending } = creating;
    creating = null;
    try {
      const made = await refApi.createCollection(root, name.trim() || t("Nouvelle collection"));
      if (pending) await add(made, pending);
      else await refresh();
    } catch (error) {
      app.fail(error);
    }
  }

  /** Revient à la vue d'ensemble des collections. */
  function backToOverview(): void {
    current = null;
    organizing = false;
    renaming = null;
    menu = null;
    void refresh();
  }

  /** Titre de la collection ouverte transformé en champ (Plus › Renommer, F2). */
  async function startRenameCurrent(): Promise<void> {
    if (!current) return;
    menu = null;
    renaming = current.name;
    await tick();
    renameInput?.focus();
    renameInput?.select();
  }

  /** Échap dans le champ : le nom d'origine revient, rien n'est renommé. */
  let renameCancelled = false;

  async function renameCurrent(): Promise<void> {
    if (!root || !current || renaming === null) return;
    const wanted = renaming.trim();
    if (renameCancelled || !wanted || wanted === current.name) {
      renameCancelled = false;
      renaming = organizing ? current.name : null;
      return;
    }
    try {
      const name = await refApi.renameCollection(root, current.name, wanted);
      current = await refApi.collection(root, name);
      renaming = organizing ? name : null;
      await refresh();
    } catch (error) {
      app.fail(error);
      renaming = organizing ? current.name : null;
    }
  }

  function renameKey(event: KeyboardEvent): void {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") input.blur();
    else if (event.key === "Escape") {
      // Pris ici : Échap n'a pas à quitter l'accueil.
      event.preventDefault();
      event.stopPropagation();
      renameCancelled = true;
      input.blur();
    }
  }

  // ---- renommage en place d'une collection de la vue d'ensemble

  async function startRenameCard(name: string): Promise<void> {
    context = null;
    renamingCard = { name, draft: name };
    await tick();
    cardInput?.focus();
    cardInput?.select();
  }

  let cardCancelled = false;

  async function commitRenameCard(): Promise<void> {
    if (!renamingCard || !root) return;
    const { name, draft } = renamingCard;
    renamingCard = null;
    const wanted = draft.trim();
    if (cardCancelled || !wanted || wanted === name) {
      cardCancelled = false;
      return;
    }
    try {
      await refApi.renameCollection(root, name, wanted);
    } catch (error) {
      app.fail(error);
    }
    await refresh();
  }

  function cardKey(event: KeyboardEvent): void {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") input.blur();
    else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cardCancelled = true;
      input.blur();
    }
  }

  // ---- dossier, suppression, couverture

  function openFolder(name: string): void {
    if (root) void refApi.openCollection(root, name).catch((error: unknown) => app.fail(error));
  }

  function askDelete(name: string, count: number): void {
    menu = null;
    context = null;
    confirming = { name, count };
  }

  /** Confirmée dans la fenêtre de l'application : la collection part dans le dossier .corbeille de la bibliothèque. */
  async function deleteConfirmed(): Promise<void> {
    if (!root || !confirming) return;
    const { name } = confirming;
    confirming = null;
    try {
      await refApi.deleteCollection(root, name);
      if (current?.name === name) {
        current = null;
        organizing = false;
        renaming = null;
      }
      await refresh();
    } catch (error) {
      app.fail(error);
    }
  }

  function deleteMessage(count: number): string {
    if (count === 0) return t("Êtes-vous bien sûr de vouloir supprimer la collection {name} ?");
    if (count === 1) return t("Êtes-vous bien sûr de vouloir supprimer la collection {name} et son image ?");
    return t("Êtes-vous bien sûr de vouloir supprimer la collection {name} et ses {n} images ?", { n: count });
  }

  // ---- tri (mémorisé par collection dans myref.json)

  async function sortCurrent(by: RefSort): Promise<void> {
    menu = null;
    if (!root || !current) return;
    try {
      current = await refApi.sortItems(root, current.name, by);
    } catch (error) {
      app.fail(error);
    }
  }

  // ---- menus

  function toggleMenu(which: "sort" | "more", event: MouseEvent): void {
    menuAnchor = event.currentTarget as HTMLElement;
    menu = menu === which ? null : which;
  }

  /** Position d'un menu ouvert sous un bouton : centré dessous, 6 px plus bas. */
  function below(anchor: HTMLElement | null): { x: number; y: number } {
    const rect = anchor?.querySelector("span")?.getBoundingClientRect() ?? anchor?.getBoundingClientRect();
    return rect ? { x: rect.left + rect.width / 2, y: (anchor?.getBoundingClientRect().bottom ?? rect.bottom) + 8 } : { x: 0, y: 0 };
  }

  function sortEntries(collection: RefCollection): RefMenuEntry[] {
    const sorts: [RefSort, string][] = [
      ["recent", t("Plus récentes d'abord")],
      ["oldest", t("Plus anciennes d'abord")],
      ["name", t("Nom (A → Z)")],
    ];
    return sorts.map(([by, label]) => ({ label, checked: collection.sort === by, run: () => void sortCurrent(by) }));
  }

  function moreEntries(collection: RefCollection): RefMenuEntry[] {
    return [
      { label: t("Renommer"), icon: "rename", shortcut: "F2", run: () => void startRenameCurrent() },
      { label: t("Ouvrir le dossier"), icon: "folder", run: () => openFolder(collection.name) },
      "separator",
      { label: t("Supprimer la collection…"), icon: "trash", danger: true, run: () => askDelete(collection.name, collection.items.length) },
    ];
  }

  function contextEntries(summary: RefCollectionSummary): RefMenuEntry[] {
    return [
      { label: t("Renommer"), icon: "rename", shortcut: "F2", run: () => void startRenameCard(summary.name) },
      { label: t("Ouvrir le dossier"), icon: "folder", run: () => openFolder(summary.name) },
      { label: t("Changer la couverture…"), icon: "cover", run: () => (coverFor = summary.name) },
      "separator",
      { label: t("Supprimer la collection…"), icon: "trash", danger: true, run: () => askDelete(summary.name, summary.count) },
    ];
  }

  function openContext(event: MouseEvent, summary: RefCollectionSummary): void {
    event.preventDefault();
    menu = null;
    context = { summary, x: event.clientX, y: event.clientY };
  }

  /**
   * F2 : renomme la collection ouverte, ou, dans la vue d'ensemble, celle du menu contextuel ou de la carte qui a le
   * focus clavier. Rien pendant une saisie, la visionneuse ou une fenêtre ouverte.
   */
  function onKey(event: KeyboardEvent): void {
    if (event.key !== "F2" || event.repeat || lightbox || confirming || coverFor) return;
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
    if (current) {
      event.preventDefault();
      void startRenameCurrent();
      return;
    }
    const name = context?.summary.name ?? (document.activeElement as HTMLElement | null)?.closest<HTMLElement>("[data-collection]")?.dataset.collection;
    if (!name || !overview) return;
    event.preventDefault();
    void startRenameCard(name);
  }

  // ---- import (bouton, dépôt)

  const IMAGE_FILTER = { name: "Images", extensions: dialogExtensions(IMAGE_EXTENSIONS) };

  async function pickFiles(collection: string): Promise<void> {
    menu = null;
    const paths = await open({ multiple: true, filters: [IMAGE_FILTER] }).catch(() => null);
    if (!paths || paths.length === 0) return;
    await add(collection, { files: [], paths, urls: [] });
  }

  async function add(collection: string, drop: Dropped): Promise<void> {
    if (!root) return;
    app.busy = t("Import…");
    try {
      const count = await importDrop(root, collection, drop);
      say(count === 1 ? t("1 image ajoutée à « {name} »", { name: collection }) : t("{count} images ajoutées à « {name} »", { count, name: collection }));
    } catch (error) {
      app.fail(error);
    } finally {
      app.busy = null;
    }
    await refresh();
  }

  /** Une cible de dépôt sous un point de la fenêtre (dépôts reçus par la fenêtre, sans événement de la page). */
  function targetAt(position: { x: number; y: number }): string | null {
    const element = document.elementFromPoint(position.x, position.y);
    return element?.closest<HTMLElement>("[data-myref-drop]")?.dataset.myrefDrop ?? null;
  }

  let lastPageDrop = 0;

  function receive(target: string, drop: Dropped): Promise<void> | void {
    if (drop.files.length === 0 && drop.paths.length === 0 && drop.urls.length === 0) {
      app.error = t("Rien d'exploitable dans ce qui a été déposé : glissez l'image elle-même, pas la page.");
      return;
    }
    if (target === "new") return void startCreating(drop);
    if (target === "open" && current) return add(current.name, drop);
    if (target.startsWith("col:")) return add(target.slice(4), drop);
  }

  /** Gestionnaires du glisser de la page pour une cible. `preventDefault` au dépôt : App.svelte ne l'importe pas. */
  function droppable(target: string) {
    return {
      "data-myref-drop": target,
      ondragenter: (event: DragEvent) => {
        if (!carriesDrop(event.dataTransfer)) return;
        event.preventDefault();
        dropTarget = target;
      },
      ondragover: (event: DragEvent) => {
        if (!carriesDrop(event.dataTransfer)) return;
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
        dropTarget = target;
      },
      ondragleave: (event: DragEvent) => {
        const into = event.relatedTarget as Node | null;
        if (!into || !(event.currentTarget as HTMLElement).contains(into)) dropTarget = dropTarget === target ? null : dropTarget;
      },
      ondrop: (event: DragEvent) => {
        const data = event.dataTransfer;
        dropTarget = null;
        // Déjà pris par une cible intérieure (carte), qui l'emporte sur la vue entière.
        if (!data || !carriesDrop(data) || event.defaultPrevented) return;
        // Marque le dépôt comme traité : App.svelte, qui le reçoit ensuite (fenêtre), ne l'ouvre pas comme document.
        event.preventDefault();
        lastPageDrop = performance.now();
        void receive(target, readDrop(data));
      },
    };
  }

  // ---- organiser : renommer, réordonner au glisser (pointeur : le glisser natif est coupé dans l'application)

  function toggleOrganize(): void {
    organizing = !organizing;
    menu = null;
    renaming = organizing && current ? current.name : null;
  }

  function startReorder(event: PointerEvent, file: string): void {
    if (!organizing || event.button !== 0) return;
    event.preventDefault();
    dragging = file;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function moveReorder(event: PointerEvent): void {
    if (!dragging || !current) return;
    const over = document.elementFromPoint(event.clientX, event.clientY)?.closest<HTMLElement>("[data-file]");
    const target = over?.dataset.file;
    if (!over || !target || target === dragging) return;
    // L'image tirée prend la place de celle survolée (après elle en avançant, avant elle en reculant).
    const items = [...current.items];
    const from = items.findIndex((i) => i.file === dragging);
    const to = items.findIndex((i) => i.file === target);
    const moved = items[from];
    if (from < 0 || to < 0 || !moved) return;
    items.splice(from, 1);
    items.splice(to, 0, moved);
    current = { ...current, items };
  }

  function endReorder(): void {
    if (!dragging || !root || !current) return;
    dragging = null;
    // Ordre fait à la main : plus aucun tri n'est coché (le processus Rust l'oublie aussi).
    current = { ...current, sort: null };
    void refApi.reorderItems(root, current.name, current.items.map((i) => i.file)).catch((error: unknown) => app.fail(error));
  }

  // ---- visionneuse

  function show(items: RefItem[], item: RefItem): void {
    lightbox = { items, index: Math.max(0, items.findIndex((i) => i.id === item.id)) };
  }

  /** Une fiche modifiée dans la visionneuse : reportée partout où l'image est affichée. */
  /** Fiche modifiée dans la visionneuse ; après un renommage, `previousId` retrouve l'ancienne entrée. */
  function changed(item: RefItem, previousId?: string): void {
    const was = previousId ?? item.id;
    const patch = (list: RefItem[]) => list.map((i) => (i.id === was ? item : i));
    if (current) current = { ...current, items: patch(current.items) };
    if (found) found = patch(found);
    if (lightbox) lightbox = { ...lightbox, items: patch(lightbox.items) };
    if (overview) {
      const favorites = item.favorite ? (overview.favorites.some((i) => i.id === was) ? patch(overview.favorites) : [item, ...overview.favorites]) : overview.favorites.filter((i) => i.id !== was);
      overview = { ...overview, favorites, recent: patch(overview.recent) };
    }
  }

  /** Une image déplacée ou supprimée de sa collection depuis la visionneuse : retirée des listes, la visionneuse passe à la suivante. */
  async function removed(id: string): Promise<void> {
    if (lightbox) {
      const items = lightbox.items.filter((i) => i.id !== id);
      lightbox = items.length > 0 ? { items, index: Math.min(lightbox.index, items.length - 1) } : null;
    }
    if (found) found = found.filter((i) => i.id !== id);
    await refresh();
  }

  // ---- affichage

  const count = (n: number) => (n === 1 ? t("1 élément") : t("{n} éléments", { n }));

  function modified(ms: number): string {
    if (!ms) return t("vide");
    const day = 86_400_000;
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    if (ms >= start.getTime()) return t("modifiée aujourd'hui");
    if (ms >= start.getTime() - day) return t("modifiée hier");
    return t("modifiée le {date}", { date: formatDate(new Date(ms), { day: "numeric", month: "short", year: "numeric" }) });
  }

  const collectionNames = $derived(overview?.collections.map((c) => c.name) ?? []);
</script>

{#snippet strip(title: string, items: RefItem[], empty: string)}
  <section class="row">
    <div class="row-title"><h2>{title}</h2>{#if items.length > 8}<button class="link" onclick={() => items[0] && show(items, items[0])}>{t("Tout voir")}</button>{/if}</div>
    {#if items.length > 0}
      <div class="strip">
        {#each items.slice(0, 8) as item (item.id)}
          <div class="strip-card">
            <button class="open" title={item.name} onclick={() => show(items, item)}>
              {#if root}<RefThumb {root} {item} fit="cover" />{/if}
            </button>
            {#if item.favorite}<span class="star" aria-hidden="true"><svg class="heart-mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z" /></svg></span>{/if}
            {#if root && overview}<RefHover {root} {item} collections={overview.collections} onchanged={(next) => changed(next)} onmembership={() => void refresh()} />{/if}
          </div>
        {/each}
      </div>
    {:else}
      <p class="empty">{empty}</p>
    {/if}
  </section>
{/snippet}

{#snippet masonry(items: RefItem[])}
  <div class="masonry">
    {#each items as item (item.id)}
      <div class="card">
        <button class="open" title={item.name} onclick={() => show(items, item)}>
          {#if root}<RefThumb {root} {item} />{/if}
        </button>
        {#if item.favorite}<span class="fav" aria-label={t("Favori")}><svg class="heart-mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z" /></svg></span>{/if}
        {#if root && overview}<RefHover {root} {item} collections={overview.collections} onchanged={(next) => changed(next)} onmembership={() => void refresh()} />{/if}
      </div>
    {/each}
  </div>
{/snippet}

<div class="myref">
  {#if !current}
    <header>
      <div class="heading">
        {#key replay}<MyRefTitle size={58} animate={title.played !== replay} onplayed={() => (title.played = replay)} />{/key}
        {#if root}
          <span class="path">{t("Dossier :")} <code title={root}>{shortPath(root)}</code> <button class="underline" onclick={chooseFolder}>{t("Changer")}</button></span>
        {/if}
      </div>
      {#if root && !failure}
        <div class="tools">
          <label class="search">
            <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="10" r="6.6" /><path d="M15.2 15.2l5.6 5.6" /></svg>
            <input type="search" placeholder={t("Rechercher une image, un tag…")} aria-label={t("Rechercher dans My ref")} bind:value={query} />
          </label>
          <div class="menu-host">
            <button class="btn" onclick={() => (menu = menu === "import" ? null : "import")} aria-expanded={menu === "import"}>
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v12M6 10l6 6 6-6M4 20h16" /></svg>{t("Importer")}
            </button>
            {#if menu === "import"}
              <div class="menu" role="menu">
                <span class="menu-head">{t("Importer dans…")}</span>
                {#each collectionNames as name (name)}
                  <button role="menuitem" onclick={() => pickFiles(name)}>{name}</button>
                {/each}
                <button role="menuitem" class="accent" onclick={() => ((menu = null), void startCreating())}>{t("Nouvelle collection…")}</button>
              </div>
            {/if}
          </div>
        </div>
      {/if}
    </header>
  {/if}

  {#if notice}<p class="notice" role="status">{notice}</p>{/if}

  {#if !ready}
    <!-- lecture des réglages : rien à montrer pendant une fraction de seconde -->
  {:else if !root}
    <!-- Premier usage : choisir le dossier de la bibliothèque. -->
    <div class="setup">
      <div class="setup-icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2 6.6A2.6 2.6 0 0 1 4.6 4h5l2.3 2.6h7.5A2.6 2.6 0 0 1 22 9.2v9.2A2.6 2.6 0 0 1 19.4 21H4.6A2.6 2.6 0 0 1 2 18.4z" /></svg></div>
      <h2>{t("Où ranger vos références ?")}</h2>
      <p>{t("Choisissez un dossier : vos images, vos collections, leurs noms, l'ordre, les favoris et les notes y seront enregistrés en fichiers ordinaires. Une nouvelle version de Lumitype, un autre ordinateur ou une sauvegarde : il suffit de pointer vers ce dossier pour tout retrouver.")}</p>
      <button class="btn primary big" onclick={chooseFolder}>{t("Choisir un dossier…")}</button>
      <pre class="tree"><b>My ref/</b>
├─ myref.json <em>— {t("collections, ordre, couvertures (format ouvert)")}</em>
├─ Dessin/
│  ├─ ville-brume.jpg
│  ├─ ville-brume.jpg.json <em>— {t("nom, source, tags, note, favori")}</em>
│  └─ …
├─ Graphisme/
└─ .apercus/ <em>— {t("vignettes, recréées si effacées")}</em></pre>
      <p class="note">{t("Rien n'est envoyé en ligne. Le dossier peut être sur un disque externe ou un cloud synchronisé.")}</p>
    </div>
  {:else if failure}
    <div class="setup">
      <h2>{t("Bibliothèque inaccessible")}</h2>
      <p>{failure}</p>
      <div class="row-buttons">
        <button class="btn" onclick={refresh}>{t("Réessayer")}</button>
        <button class="btn primary" onclick={chooseFolder}>{t("Choisir un autre dossier…")}</button>
      </div>
    </div>
  {:else if found}
    <section class="row">
      <div class="row-title"><h2>{found.length === 0 ? t("Aucune image ne correspond.") : t("Résultats")}</h2><span class="muted">{found.length > 0 ? count(found.length) : ""}</span></div>
      {@render masonry(found)}
    </section>
  {:else if current}
    {@const collection = current}
    <!-- Une collection : toute la vue accepte les images déposées. -->
    <div class="collection" class:drop-on={dropTarget === "open"} {...droppable("open")}>
      <div class="topline">
        <!-- Pastille retour (maquette « collection ») : bien visible, avant le fil d'Ariane. -->
        <button class="back" onclick={backToOverview} title={t("Retour aux collections")}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>My ref
        </button>
        <nav class="crumb" aria-label={t("Chemin")}>
          <button onclick={backToOverview}><i>My ref</i></button>
          <span aria-hidden="true">›</span>
          <b>{collection.name}</b>
        </nav>
      </div>
      <div class="ctop">
        {#if renaming !== null}
          <input class="rename" aria-label={t("Nom de la collection")} bind:this={renameInput} bind:value={renaming} onblur={renameCurrent} onkeydown={renameKey} />
        {:else}
          <h2 class="ctitle">{collection.name}</h2>
        {/if}
        <div class="meta">{count(collection.items.length)} · {modified(collection.modified)}</div>
        <div class="acts">
          <button onclick={() => pickFiles(collection.name)}><span><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg></span>{t("Ajouter")}</button>
          <button class:on={organizing} onclick={toggleOrganize}><span><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6h16M4 12h10M4 18h6" /></svg></span>{organizing ? t("Terminé") : t("Organiser")}</button>
          <button class:on={menu === "sort"} aria-haspopup="menu" aria-expanded={menu === "sort"} onclick={(event) => toggleMenu("sort", event)}><span><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7h18M7 3v4M17 3v4M4 7v13h16V7" /></svg></span>{t("Trier")}</button>
          <button class:on={menu === "more"} aria-haspopup="menu" aria-expanded={menu === "more"} onclick={(event) => toggleMenu("more", event)}><span><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="5" cy="12" r="1.3" /><circle cx="12" cy="12" r="1.3" /><circle cx="19" cy="12" r="1.3" /></svg></span>{t("Plus")}</button>
          {#if menu === "sort" || menu === "more"}
            {@const at = below(menuAnchor)}
            <MyRefMenu
              x={at.x}
              y={at.y}
              align="center"
              anchor={menuAnchor}
              head={menu === "sort" ? t("Trier par") : ""}
              headStyle="caps"
              label={menu === "more" ? t("Plus") : ""}
              entries={menu === "sort" ? sortEntries(collection) : moreEntries(collection)}
              onclose={() => (menu = null)}
            />
          {/if}
        </div>
      </div>

      {#if collection.items.length === 0}
        <div class="empty-drop">
          <p>{t("Collection vide. Déposez des images ici (depuis vos dossiers ou un navigateur), ou cliquez sur Ajouter.")}</p>
        </div>
      {:else if organizing}
        <p class="hint">{t("Glissez les images pour changer leur ordre.")}</p>
        <div class="organize" onpointermove={moveReorder} onpointerup={endReorder} onpointercancel={endReorder} role="list">
          {#each collection.items as item (item.id)}
            <div class="tile" class:dragging={dragging === item.file} data-file={item.file} role="listitem" onpointerdown={(event) => startReorder(event, item.file)}>
              {#if root}<RefThumb {root} {item} fit="cover" />{/if}
            </div>
          {/each}
        </div>
      {:else}
        {@render masonry(collection.items)}
      {/if}
    </div>
  {:else if overview}
    <div class="cols">
      <div class="col" class:drop-on={dropTarget === "new"} {...droppable("new")}>
        {#if creating}
          <div class="cover new">
            <form onsubmit={(event) => (event.preventDefault(), void confirmCreate())}>
              <input
                bind:this={createInput}
                bind:value={creating.name}
                aria-label={t("Nom de la nouvelle collection")}
                onkeydown={(event) => event.key === "Escape" && (creating = null)}
              />
            </form>
            <span class="hint">{creating.pending ? t("Entrée : créer et y ranger les images") : t("Entrée pour créer")}</span>
          </div>
        {:else}
          <button class="cover new" onclick={() => startCreating()} aria-label={t("Nouvelle collection")}><span aria-hidden="true">+</span></button>
        {/if}
        <h3>{t("Nouvelle collection")}</h3>
        <p>{dropTarget === "new" ? t("Relâchez pour créer une collection") : t("Regrouper des éléments associés")}</p>
      </div>
      {#each overview.collections as summary (summary.name)}
        {@const target = `col:${summary.name}`}
        <div class="col" class:drop-on={dropTarget === target} class:menu-on={context?.summary.name === summary.name} {...droppable(target)}>
          <button
            class="cover"
            class:single={!!summary.cover}
            class:n1={!summary.cover && summary.covers.length === 1}
            class:n2={!summary.cover && summary.covers.length === 2}
            class:n3={!summary.cover && summary.covers.length === 3}
            data-collection={summary.name}
            onclick={() => openCollection(summary.name)}
            oncontextmenu={(event) => openContext(event, summary)}
            aria-label={summary.name}
          >
            {#if summary.cover}
              <!-- Couverture choisie : l'image seule remplit la case. -->
              {#key summary.cover.id}<span class="piece">{#if root}<RefThumb {root} item={summary.cover} fit="cover" />{/if}</span>{/key}
            {:else if summary.covers.length > 0}
              <!-- Mosaïque automatique : les quatre derniers ajouts. -->
              {#each summary.covers as cover (cover.id)}
                <span class="piece">{#if root}<RefThumb {root} item={cover} fit="cover" />{/if}</span>
              {/each}
            {/if}
            {#if summary.favorites > 0}<span class="tag"><svg class="heart-mark" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z" /></svg> {summary.favorites}</span>{/if}
          </button>
          {#if renamingCard?.name === summary.name}
            <input class="card-rename" aria-label={t("Nom de la collection")} bind:this={cardInput} bind:value={renamingCard.draft} onblur={commitRenameCard} onkeydown={cardKey} />
          {:else}
            <h3>{summary.name}</h3>
          {/if}
          <p>{dropTarget === target ? t("Relâchez pour ajouter") : count(summary.count)}</p>
        </div>
      {/each}
    </div>
    {#if overview.collections.length === 0}
      <p class="empty">{t("Pas encore de collection : créez-en une, ou déposez des images sur « Nouvelle collection ».")}</p>
    {/if}
    {@render strip(t("Favoris"), overview.favorites, t("Aucun favori pour l'instant : le cœur d'une image en ajoute."))}
    {@render strip(t("Ajoutées récemment"), overview.recent, t("Les images que vous ajoutez apparaîtront ici."))}
  {/if}
</div>

{#if menu === "import"}
  <!-- Clic ailleurs : le menu ouvert se referme. -->
  <button class="scrim" aria-label={t("Fermer le menu")} onclick={() => (menu = null)}></button>
{/if}

{#if context}
  <MyRefMenu
    x={context.x}
    y={context.y}
    head={`${context.summary.name} · ${count(context.summary.count)}`}
    entries={contextEntries(context.summary)}
    onclose={() => (context = null)}
  />
{/if}

{#if confirming}
  <MyRefConfirm
    title={t("Supprimer la collection ?")}
    message={deleteMessage(confirming.count)}
    name={confirming.name}
    detail={confirming.count === 0 ? t("Son dossier est déplacé dans le dossier .corbeille de votre bibliothèque My ref.") : confirming.count === 1 ? t("Elle est déplacée dans le dossier .corbeille de votre bibliothèque My ref.") : t("Elles sont déplacées dans le dossier .corbeille de votre bibliothèque My ref.")}
    confirmLabel={t("Supprimer")}
    onconfirm={deleteConfirmed}
    oncancel={() => (confirming = null)}
  />
{/if}

{#if coverFor && root}
  <MyRefCoverPicker
    {root}
    name={coverFor}
    oncancel={() => (coverFor = null)}
    ondone={() => {
      coverFor = null;
      void refresh();
    }}
  />
{/if}

<svelte:window onkeydown={onKey} />

{#if lightbox && root}
  <MyRefLightbox
    {root}
    items={lightbox.items}
    bind:index={lightbox.index}
    summaries={overview?.collections ?? []}
    onmembership={() => void refresh()}
    onclose={() => (lightbox = null)}
    onchange={changed}
    onremove={removed}
  />
{/if}

<style>
  .myref {
    display: flex;
    flex-direction: column;
    gap: 0;
    min-height: 100%;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
  }
  button {
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  /* ---- en-tête */
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 24px;
    margin-bottom: 26px;
  }
  .heading {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .path {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    color: var(--text-muted);
    font-size: 12px;
  }
  .path code {
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
    font-family: inherit;
    background: var(--bg-field);
    padding: 2px 7px;
    border: 1px solid var(--border);
  }
  .underline {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-3);
    text-decoration: underline;
    font-size: 12px;
  }
  .underline:hover {
    color: var(--text);
  }
  .tools {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .search {
    height: 34px;
    width: 250px;
    box-sizing: border-box;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    display: flex;
    align-items: center;
    padding: 0 12px;
    gap: 8px;
  }
  .search svg {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    stroke: var(--text-muted);
    stroke-width: 2.6;
    stroke-linecap: round;
  }
  .search input {
    flex-grow: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    user-select: text;
  }
  .btn {
    height: 34px;
    padding: 0 14px;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
  }
  .btn:hover {
    background: var(--bg-hover);
  }
  .btn svg {
    width: 15px;
    height: 15px;
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .btn.primary:hover {
    background: var(--accent-strong);
  }
  .btn.big {
    height: 40px;
    padding: 0 20px;
  }
  .notice {
    position: sticky;
    top: 0;
    z-index: 3;
    align-self: center;
    margin: 0 0 16px;
    padding: 8px 14px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    border-left: 3px solid var(--accent);
    font-size: 12.5px;
    animation: rise 200ms var(--ease);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  /* ---- menus */
  .menu-host {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 6px);
    z-index: 5;
    min-width: 220px;
    max-height: 360px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 4px 0;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .menu button {
    padding: 8px 14px;
    border: 0;
    background: transparent;
    text-align: left;
    font-size: 12.5px;
    white-space: nowrap;
  }
  .menu button:hover {
    background: var(--bg-hover);
  }
  .menu .accent {
    color: var(--accent);
  }
  .menu-head {
    padding: 6px 14px 8px;
    color: var(--text-muted);
    font-size: 11px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 2;
    border: 0;
    background: transparent;
    cursor: default;
  }

  /* ---- premier usage */
  .setup {
    max-width: 760px;
    width: 100%;
    box-sizing: border-box;
    margin: 30px auto 0;
    border: 1px dashed var(--border-strong);
    padding: 44px 48px;
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  .setup-icon {
    width: 64px;
    height: 64px;
    margin-bottom: 18px;
    background: var(--accent);
    display: grid;
    place-items: center;
  }
  .setup-icon svg {
    width: 30px;
    height: 30px;
    fill: #fff;
    stroke: none;
  }
  .setup h2 {
    margin: 0 0 10px;
    font-size: 22px;
  }
  .setup p {
    color: var(--text-3);
    line-height: 1.6;
    margin: 0 auto 22px;
    max-width: 560px;
    font-size: 13.5px;
  }
  .row-buttons {
    display: flex;
    gap: 10px;
  }
  .tree {
    margin: 26px auto 0;
    width: 100%;
    max-width: 520px;
    box-sizing: border-box;
    text-align: left;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    padding: 16px 20px;
    font: 12px/1.75 ui-monospace, monospace;
    color: var(--text-2);
    white-space: pre;
    overflow-x: auto;
  }
  .tree em {
    color: var(--text-muted);
    font-style: normal;
  }
  .setup .note {
    margin: 16px 0 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  /* ---- collections */
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 22px;
  }
  .col {
    min-width: 0;
  }
  .col .cover {
    width: 100%;
    aspect-ratio: 1;
    box-sizing: border-box;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    gap: 3px;
    padding: 3px;
    position: relative;
    overflow: hidden;
    transition: border-color 120ms var(--ease), transform 160ms var(--ease);
  }
  .col .cover:hover {
    border-color: rgb(var(--ink) / 0.22);
  }
  .piece {
    background: var(--bg-card);
    display: block;
    min-height: 0;
    overflow: hidden;
  }
  /* Mosaïque automatique : 2 × 2 avec quatre images ; une, deux ou trois remplissent la case autrement. */
  .cover.n1 .piece,
  .cover.single .piece {
    grid-area: 1 / 1 / 3 / 3;
  }
  .cover.n2 .piece {
    grid-row: 1 / 3;
  }
  .cover.n3 .piece:first-child {
    grid-row: 1 / 3;
  }
  /* Couverture choisie : une seule image, sans marge intérieure. */
  .cover.single {
    padding: 0;
  }
  .col.menu-on .cover {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .card-rename {
    width: 100%;
    box-sizing: border-box;
    height: 24px;
    margin: 9px 0 0;
    padding: 0 8px;
    background: var(--bg-field);
    border: 1px solid var(--accent);
    outline: none;
    color: var(--text);
    font: inherit;
    font-size: 15px;
    font-weight: 600;
    user-select: text;
  }
  .cover.new {
    display: grid;
    place-items: center;
    grid-template-columns: 1fr;
    grid-template-rows: 1fr;
    border: 1px dashed var(--border-strong);
    background: transparent;
  }
  .cover.new span {
    font-size: 42px;
    font-weight: 200;
    color: var(--text-3);
  }
  div.cover.new {
    align-content: center;
    gap: 10px;
    padding: 16px;
  }
  div.cover.new form {
    width: 100%;
  }
  div.cover.new input {
    width: 100%;
    box-sizing: border-box;
    height: 34px;
    padding: 0 10px;
    background: var(--bg-field);
    border: 1px solid var(--accent);
    outline: none;
    color: var(--text);
    font: inherit;
    font-size: 13px;
    user-select: text;
  }
  div.cover.new .hint {
    font-size: 11px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .col h3 {
    margin: 12px 0 2px;
    font-size: 15px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .col p {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }
  /* Cible de dépôt survolée : cadre rouge, comme sur la maquette. */
  .col.drop-on .cover {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    transform: scale(1.015);
  }
  .col.drop-on p {
    color: var(--accent);
  }
  .tag {
    position: absolute;
    right: 8px;
    top: 8px;
    background: rgba(0, 0, 0, 0.6);
    color: var(--text-strong);
    font-size: 11px;
    padding: 2px 7px;
  }
  .row {
    display: flex;
    flex-direction: column;
  }
  .row-title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    margin: 34px 0 12px;
  }
  .row-title h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    font-size: 12px;
  }
  .link:hover {
    color: var(--text);
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
  }
  .strip {
    display: grid;
    grid-template-columns: repeat(8, minmax(0, 1fr));
    gap: 10px;
  }
  .strip-card {
    aspect-ratio: 3 / 4;
    padding: 0;
    position: relative;
    border: 1px solid var(--border);
    background: var(--bg-panel);
    overflow: hidden;
  }
  .strip-card:hover,
  .card:hover {
    border-color: rgb(var(--ink) / 0.24);
  }
  .star {
    position: absolute;
    left: 6px;
    top: 4px;
    color: var(--text-strong);
    font-size: 12px;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.7);
  }
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .cols + .empty {
    margin-top: 18px;
  }

  /* ---- collection */
  .collection {
    position: relative;
    flex-grow: 1;
    outline: 2px dashed transparent;
    outline-offset: 8px;
    transition: outline-color 120ms var(--ease);
  }
  .collection.drop-on {
    outline-color: var(--accent);
  }
  .topline {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 6px;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 12px 0 8px;
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    color: var(--text-2);
    font-size: 12.5px;
    transition:
      background 120ms var(--ease),
      border-color 120ms var(--ease),
      color 120ms var(--ease);
  }
  .back svg {
    width: 16px;
    height: 16px;
    stroke-width: 1.8;
  }
  .back:hover {
    background: var(--bg-card);
    border-color: rgb(var(--ink) / 0.2);
    color: var(--text);
  }
  .crumb {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-muted);
    font-size: 12.5px;
  }
  .crumb button {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
  }
  .crumb button:hover {
    color: var(--text);
  }
  .crumb b {
    color: var(--text-2);
    font-weight: 500;
  }
  .ctop {
    text-align: center;
    margin-bottom: 22px;
  }
  .ctitle {
    margin: 0;
    font-size: 26px;
    font-weight: 600;
  }
  .rename {
    width: min(420px, 100%);
    height: 40px;
    box-sizing: border-box;
    text-align: center;
    font: inherit;
    font-size: 22px;
    font-weight: 600;
    color: var(--text);
    background: var(--bg-field);
    border: 1px solid var(--accent);
    outline: none;
    user-select: text;
  }
  .meta {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 4px;
  }
  .acts {
    display: flex;
    justify-content: center;
    gap: 22px;
    margin-top: 16px;
  }
  .acts button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-3);
    font-size: 11px;
  }
  .acts button > span {
    width: 34px;
    height: 34px;
    box-sizing: border-box;
    border: 1px solid var(--border-strong);
    display: grid;
    place-items: center;
    transition: background 120ms var(--ease);
  }
  .acts button:hover > span {
    background: var(--bg-hover);
  }
  .acts button.on > span {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .acts svg {
    width: 16px;
    height: 16px;
    stroke: var(--text);
    stroke-width: 1.6;
  }
  .acts svg circle {
    fill: var(--text);
  }
  .masonry {
    columns: 6 170px;
    column-gap: 12px;
  }
  .card {
    break-inside: avoid;
    width: 100%;
    margin-bottom: 12px;
    padding: 0;
    background: var(--bg-panel);
    border: 1px solid var(--border);
    position: relative;
    display: block;
  }
  .strip-card .open {
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
  }
  .strip-card:hover .star {
    opacity: 0;
  }
  .strip-card:has(:global(.hover.open)) {
    z-index: 5;
    overflow: visible;
  }
  .card .open {
    display: block;
    width: 100%;
    padding: 0;
  }
  .card:hover .fav {
    opacity: 0;
  }
  /* Liste des collections ouverte : la carte passe devant ses voisines. */
  .card:has(:global(.hover.open)) {
    z-index: 5;
  }
  .heart-mark {
    width: 12px;
    height: 12px;
    vertical-align: -1px;
    fill: currentColor;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }
  .fav {
    position: absolute;
    left: 7px;
    top: 5px;
    color: var(--text-strong);
    font-size: 12px;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.7);
  }
  .empty-drop {
    margin-top: 10px;
    border: 1px dashed var(--border-strong);
    padding: 70px 24px;
    text-align: center;
    color: var(--text-muted);
  }
  .empty-drop p {
    margin: 0;
  }
  .hint {
    margin: 0 0 12px;
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
  }
  .organize {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 10px;
    touch-action: none;
  }
  .tile {
    aspect-ratio: 1;
    border: 1px solid var(--border);
    cursor: grab;
    user-select: none;
    transition: transform 140ms var(--ease);
  }
  .tile.dragging {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
    opacity: 0.75;
    cursor: grabbing;
    transform: scale(0.96);
  }

  @media (max-width: 1180px) {
    .strip {
      grid-template-columns: repeat(5, minmax(0, 1fr));
    }
    .search {
      width: 190px;
    }
  }
</style>
