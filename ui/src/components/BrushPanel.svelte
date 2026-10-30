<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { type BrushEntry, type BrushFolder, brushLibrary, previewParams, strokePreview } from "../lib/brush-library.svelte";
  import { api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Menu des pinceaux (clic droit sur l'image) : petit, pour laisser voir le plan de travail. Grille dense des pointes,
  // dossiers repliables (fournis, packs importés, dossiers personnels), recherche, et en tête l'aperçu d'un coup bref
  // en courbe — pression de 0 à 100 % — du pinceau survolé (sinon du pinceau courant). Déplaçable, redimensionnable ;
  // il se referme dès qu'on peint, sur Échap ou ✕. On glisse un pinceau sur un dossier personnel pour l'y ranger.
  const MIN = { width: 300, height: 280 };
  const lib = brushLibrary;
  const settings = $derived(app.brushes[app.paintTool ?? "brush"]);
  const panelSize = $derived(lib.settings.panel);
  const position = $derived(app.brushPanel ?? { x: 80, y: 80 });
  const left = $derived(Math.max(4, Math.min(position.x, window.innerWidth - Math.min(panelSize.width, window.innerWidth - 8) - 4)));
  const top = $derived(Math.max(4, Math.min(position.y, window.innerHeight - Math.min(panelSize.height, window.innerHeight - 8) - 4)));

  let search = $state("");
  let importing = $state(false);
  let renaming = $state<string | null>(null);
  let renameValue = $state("");
  let menu = $state<{ x: number; y: number; folder: BrushFolder; entry?: BrushEntry } | null>(null);
  let drag = $state<{ entry: BrushEntry; x: number; y: number; over: string | null } | null>(null);
  /** Pinceau survolé : l'aperçu en tête le montre. */
  let hovered = $state<BrushEntry | null>(null);

  onMount(() => {
    lib.load().catch((error: unknown) => app.fail(error));
  });

  const query = $derived(search.trim().toLowerCase());
  const shown = $derived(
    lib.folders
      .map((folder) => ({ folder, entries: query ? folder.entries.filter((e) => t(e.name).toLowerCase().includes(query)) : folder.entries }))
      .filter(({ folder, entries }) => !query || entries.length > 0 || folder.name.toLowerCase().includes(query)),
  );
  const userFolders = $derived(lib.folders.filter((f) => f.source === "user"));
  const thumb = $derived(lib.settings.thumb);
  const grid = $derived(lib.settings.view === "grid");
  const strokeWidth = $derived(Math.max(90, Math.min(260, panelSize.width - 190)));

  // ------------------------------------------------------------ déplacement et taille du panneau

  function moveStart(event: PointerEvent): void {
    if (event.button !== 0 || (event.target as HTMLElement).closest("input, button")) return;
    const start = { x: event.clientX, y: event.clientY, left, top };
    const move = (e: PointerEvent) => (app.brushPanel = { x: start.left + e.clientX - start.x, y: start.top + e.clientY - start.y });
    const up = () => (window.removeEventListener("pointermove", move), window.removeEventListener("pointerup", up));
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function resizeStart(event: PointerEvent, axes: "both" | "x"): void {
    event.preventDefault();
    const start = { x: event.clientX, y: event.clientY, ...panelSize };
    const move = (e: PointerEvent) => {
      lib.settings.panel = {
        width: Math.max(MIN.width, Math.min(window.innerWidth - left - 4, start.width + e.clientX - start.x)),
        height: axes === "x" ? start.height : Math.max(MIN.height, Math.min(window.innerHeight - top - 4, start.height + e.clientY - start.y)),
      };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      lib.saveSettings();
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // ------------------------------------------------------------ pinceaux : choix, glisser vers un dossier

  function entryDown(event: PointerEvent, entry: BrushEntry): void {
    if (event.button !== 0) return;
    const start = { x: event.clientX, y: event.clientY };
    let moved = false;
    const move = (e: PointerEvent) => {
      if (!moved && Math.hypot(e.clientX - start.x, e.clientY - start.y) < 5) return;
      moved = true;
      const target = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-folder]");
      drag = { entry, x: e.clientX, y: e.clientY, over: target?.dataset.folder ?? null };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (!moved) lib.apply(entry);
      else if (drag?.over && lib.isUserFolder(drag.over)) lib.addTo(drag.over, entry.key);
      drag = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  async function importFile(): Promise<void> {
    try {
      const path = await open({ multiple: false, filters: [{ name: t("Pinceaux"), extensions: ["abr", "gbr", "png", "jpg", "jpeg", "webp"] }] });
      if (typeof path !== "string") return;
      importing = true;
      await api.importBrushes(path);
      await lib.refresh();
      const stem = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "").replaceAll("_", " ");
      const folder = lib.folders.find((f) => f.source === "pack" && f.name === stem);
      if (folder?.entries[0]) lib.apply(folder.entries[0]);
    } catch (error) {
      app.fail(error);
    } finally {
      importing = false;
    }
  }

  function newFolder(): void {
    const id = lib.createFolder(t("Nouveau dossier"));
    startRename(id, t("Nouveau dossier"));
  }

  function startRename(id: string, name: string): void {
    renaming = id;
    renameValue = name;
    menu = null;
  }

  function commitRename(): void {
    if (renaming) lib.renameFolder(renaming, renameValue);
    renaming = null;
  }

  function openMenu(event: MouseEvent, folder: BrushFolder, entry?: BrushEntry): void {
    event.preventDefault();
    menu = { x: Math.min(event.clientX, window.innerWidth - 220), y: Math.min(event.clientY, window.innerHeight - 220), folder, entry };
  }

  /** Enregistre le pinceau courant dans « Mes pinceaux », sous son nom et sa taille. */
  function savePreset(): void {
    const base = app.brushKey ? (lib.all.find((e) => e.key === app.brushKey)?.name ?? "Pinceau") : "Pinceau";
    lib.saveCurrent(`${t(base)} ${Math.round(settings.size)} px`);
  }

  // ------------------------------------------------------------ aperçus

  /** Aperçu de trait chargé quand la ligne devient visible. */
  function preview(node: HTMLImageElement, args: { entry: BrushEntry; width: number; height: number }) {
    let current = args;
    const load = () => strokePreview(previewParams(current.entry), current.width, current.height).then((url) => (node.src = url), () => undefined);
    const observer = new IntersectionObserver((items) => {
      if (items.some((i) => i.isIntersecting)) {
        observer.disconnect();
        void load();
      }
    });
    observer.observe(node);
    // Adresse révoquée entre-temps (cache des aperçus borné) : l'aperçu est redemandé, une fois.
    let retried = false;
    const onError = () => {
      if (retried) return;
      retried = true;
      void load();
    };
    node.addEventListener("error", onError);
    return {
      update(next: typeof args) {
        const changed = next.width !== current.width || next.height !== current.height || next.entry.key !== current.entry.key;
        current = next;
        retried = false;
        if (changed) void load();
      },
      destroy: () => {
        observer.disconnect();
        node.removeEventListener("error", onError);
      },
    };
  }

  // Aperçu en tête : coup bref en courbe, pression de 0 à 100 %, du pinceau survolé, sinon du pinceau courant (il suit
  // chaque réglage).
  let headPreview = $state<string | null>(null);
  const liveParams = $derived({
    size: settings.size,
    hardness: settings.hardness,
    opacity: 1,
    color: [255, 255, 255] as [number, number, number],
    erase: false,
    pressureSize: settings.pressureSize,
    pressureOpacity: settings.pressureOpacity,
    kind: $state.snapshot(app.brushKind),
    dynamics: $state.snapshot(app.brushDynamics),
    dual: $state.snapshot(app.brushDual),
    texture: $state.snapshot(app.brushTexture),
  });
  const headParams = $derived(hovered ? { ...previewParams(hovered), pressureSize: true } : liveParams);
  $effect(() => {
    const params = headParams;
    const width = Math.max(200, panelSize.width - 24);
    const timer = setTimeout(() => strokePreview(params, width, 46, true).then((url) => (headPreview = url), () => undefined), hovered ? 40 : 120);
    return () => clearTimeout(timer);
  });

  const isOn = (entry: BrushEntry) => app.brushKey === entry.key;
  // Taille sur une échelle exponentielle (même échelle que le panneau Propriétés) : position 0..1000.
  const MAX_SIZE = 2500;
  const sizeToPosition = (size: number) => Math.round((Math.log(Math.max(1, size)) / Math.log(MAX_SIZE)) * 1000);
  const positionToSize = (position: number) => Math.max(1, Math.round(Math.exp((position / 1000) * Math.log(MAX_SIZE))));
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key !== "Escape") return;
    if (menu) menu = null;
    else if (renaming) renaming = null;
    else app.brushPanel = null;
  }}
  onpointerdown={(event) => menu && !(event.target as HTMLElement).closest(".ctx") && (menu = null)}
/>

<div class="panel" role="dialog" aria-label={t("Pinceaux")} tabindex="-1" style:left="{left}px" style:top="{top}px" style:width="{panelSize.width}px" style:height="{panelSize.height}px" oncontextmenu={(event) => event.preventDefault()}>
  <header role="toolbar" tabindex="-1" onpointerdown={moveStart}>
    <input class="search" type="search" placeholder={t("Rechercher un pinceau…")} bind:value={search} onkeydown={(event) => (event.stopPropagation(), event.key === "Escape" && (search ? (search = "") : (app.brushPanel = null)))} />
    <!-- Vrais boutons : icône et nom. Disquette = enregistrer le pinceau courant. -->
    <div class="actions">
      <button type="button" class="action" title={t("Enregistrer le pinceau courant")} onclick={savePreset}>
        <svg viewBox="0 0 24 24"><path d="M4 4h13l3 3v13H4z" /><path d="M7 4h8v5H7zM7 13h10v7H7z" /></svg>{t("Enregistrer")}
      </button>
      <button type="button" class="action" title={t("Nouveau dossier")} onclick={newFolder}>
        <svg viewBox="0 0 24 24"><path d="M3 6h7l2 2h9v11H3zM12 11v5M9.5 13.5h5" /></svg>{t("Dossier")}
      </button>
      <button type="button" class="action" title={t("Importer (.abr, .gbr, image)")} disabled={importing} onclick={importFile}>
        <svg viewBox="0 0 24 24"><path d="M12 4v11M7.5 10.5 12 15l4.5-4.5M4 16v4h16v-4" /></svg>{t("Importer")}
      </button>
      <button type="button" class="action close" aria-label={t("Fermer")} title={t("Fermer")} onclick={() => (app.brushPanel = null)}>
        <svg viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </div>
    <div class="view">
      <div class="segmented" role="group" aria-label={t("Affichage")}>
        <button type="button" class:on={grid} onclick={() => ((lib.settings.view = "grid"), lib.saveSettings())}>{t("Vignettes")}</button>
        <button type="button" class:on={!grid} onclick={() => ((lib.settings.view = "list"), lib.saveSettings())}>{t("Liste")}</button>
      </div>
      <span>{t("Taille des aperçus")}</span>
      <input class="thumb" type="range" min="24" max="56" step="2" value={thumb} aria-label={t("Taille des aperçus")} oninput={(event) => (lib.settings.thumb = event.currentTarget.valueAsNumber)} onchange={() => lib.saveSettings()} />
    </div>
  </header>

  <div class="head-preview" title={t("Coup bref, pression de 0 à 100 %")}>
    {#if headPreview}<img src={headPreview} alt="" />{/if}
    <span>{hovered ? t(hovered.name) : t(lib.all.find((e) => e.key === app.brushKey)?.name ?? "")}</span>
  </div>
  <!-- Taille seule, sous l'aperçu, sur une échelle exponentielle (détail au début, montée rapide à la fin). -->
  <label class="size">
    <span>{t("Taille")}</span>
    <input type="range" min="0" max="1000" step="1" value={sizeToPosition(settings.size)} oninput={(event) => (settings.size = positionToSize(event.currentTarget.valueAsNumber))} />
    <strong>{Math.round(settings.size)} px</strong>
  </label>

  <div class="list" role="presentation" onpointerleave={() => (hovered = null)}>
    {#if !lib.loaded}
      <p class="empty">{t("Chargement…")}</p>
    {/if}
    {#each shown as { folder, entries } (folder.id)}
      {@const collapsed = !query && lib.isCollapsed(folder.id)}
      <section class="folder" class:target={drag && drag.over === folder.id && folder.source === "user"} class:droppable={drag && folder.source === "user"}>
        <div class="fh" role="button" tabindex="0" data-folder={folder.id} onclick={() => lib.toggle(folder.id)} onkeydown={(event) => event.key === "Enter" && lib.toggle(folder.id)} oncontextmenu={(event) => openMenu(event, folder)}>
          <span class="chev">{collapsed ? "▸" : "▾"}</span>
          {#if renaming === folder.id}
            <!-- svelte-ignore a11y_autofocus -->
            <input class="rename" autofocus bind:value={renameValue} onclick={(event) => event.stopPropagation()} onkeydown={(event) => (event.stopPropagation(), event.key === "Enter" && commitRename(), event.key === "Escape" && (renaming = null))} onblur={commitRename} />
          {:else}
            <b>{t(folder.name)}</b>
          {/if}
          {#if folder.source === "pack"}<span class="badge">ABR</span>{/if}
          <em>{folder.entries.length}</em>
        </div>
        {#if !collapsed}
          <div class:rows={!grid} class:tiles={grid} style:--h="{thumb}px">
            {#each entries as entry (entry.key)}
              <button
                type="button"
                class="entry"
                class:on={isOn(entry)}
                title={t(entry.name)}
                onpointerenter={() => (hovered = entry)}
                onpointerdown={(event) => entryDown(event, entry)}
                oncontextmenu={(event) => openMenu(event, folder, entry)}
              >
                <span class="tip">
                  {#if entry.kind.type === "tip" && app.brushThumbnails[entry.kind.id]}
                    <img src={app.brushThumbnails[entry.kind.id]} alt="" />
                  {:else}
                    <i class="round" class:soft={entry.hardness < 0.5}></i>
                  {/if}
                </span>
                {#if !grid}
                  <img class="stroke" alt="" style:width="{strokeWidth}px" use:preview={{ entry, width: strokeWidth, height: Math.round(thumb * 0.8) }} />
                {/if}
                <span class="name">{t(entry.name)}</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {/each}
  </div>

  <button type="button" class="grip" aria-label={t("Redimensionner")} onpointerdown={(event) => resizeStart(event, "both")}>
    <svg viewBox="0 0 16 16"><path d="M15 5L5 15M15 10l-5 5" /></svg>
  </button>
</div>

{#if drag}
  <div class="ghost" style:left="{drag.x + 12}px" style:top="{drag.y + 8}px">{t(drag.entry.name)}</div>
{/if}

{#if menu}
  {@const m = menu}
  <div class="ctx" role="menu" style:left="{m.x}px" style:top="{m.y}px">
    {#if m.entry}
      {#each userFolders.filter((f) => f.id !== m.folder.id) as folder (folder.id)}
        <button type="button" role="menuitem" onclick={() => (lib.addTo(folder.id, m.entry!.key), (menu = null))}>{t("Ajouter à « {name} »", { name: folder.name })}</button>
      {/each}
      {#if m.folder.source === "user"}
        <button type="button" role="menuitem" onclick={() => (lib.removeFrom(m.folder.id, m.entry!.key), (menu = null))}>{t("Retirer du dossier")}</button>
      {/if}
      {#if m.folder.source === "mine"}
        <button type="button" role="menuitem" onclick={() => (lib.removeMine(m.entry!.name), (menu = null))}>{t("Supprimer")}</button>
      {/if}
      <hr />
    {/if}
    <button type="button" role="menuitem" onclick={() => ((menu = null), newFolder())}>{t("Nouveau dossier")}</button>
    {#if m.folder.source === "user"}
      <button type="button" role="menuitem" onclick={() => startRename(m.folder.id, m.folder.name)}>{t("Renommer")}</button>
    {/if}
    {#if m.folder.source !== "builtin"}
      <button type="button" role="menuitem" class="danger" onclick={() => (lib.deleteFolder(m.folder.id).catch((error: unknown) => app.fail(error)), (menu = null))}>{m.folder.source === "pack" ? t("Retirer ce pack") : t("Supprimer le dossier")}</button>
    {/if}
  </div>
{/if}

<style>
  /* Stylet : glisser un pinceau vers un dossier n'est pas pris pour un défilement (voir LayersPanel). */
  .list button {
    touch-action: none;
  }
  .panel {
    position: fixed;
    z-index: 58;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow-menu);
    overflow: hidden;
    font-size: 11px;
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 7px 8px;
    border-bottom: 1px solid var(--border);
    cursor: move;
  }
  header .search {
    height: 26px;
  }
  .actions {
    display: flex;
    gap: 5px;
  }
  .action {
    flex: 1;
    min-width: 0;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text);
    white-space: nowrap;
  }
  .action:hover {
    background: var(--bg-hover);
  }
  .action:disabled {
    opacity: 0.5;
  }
  .action.close {
    flex: 0 0 26px;
  }
  .action svg {
    width: 14px;
    height: 14px;
  }
  .view {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
  }
  .view .thumb {
    flex: 1;
    width: auto;
  }
  .segmented {
    display: flex;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .segmented button {
    padding: 3px 9px;
    color: var(--text-3);
  }
  .segmented button.on {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text-strong);
  }
  .size {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 8px;
    padding: 6px 10px 8px;
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
    font-size: 12px;
  }
  .size input {
    min-width: 0;
    height: 16px;
  }
  .size strong {
    color: var(--text-strong);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    min-width: 54px;
    text-align: right;
  }
  .search {
    flex: 1;
    min-width: 50px;
    height: 22px;
    padding: 0 7px;
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 11px;
    user-select: text;
    cursor: text;
  }
  svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .thumb {
    width: 48px;
    flex-shrink: 0;
  }
  .head-preview {
    position: relative;
    height: 50px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    background: var(--bg-well);
    border-bottom: 1px solid var(--border);
  }
  .head-preview img {
    max-width: 100%;
    height: 46px;
    filter: invert(var(--ink-invert));
  }
  .head-preview span {
    position: absolute;
    left: 7px;
    bottom: 3px;
    color: var(--text-muted);
    font-size: 10px;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    padding: 2px 4px 4px;
  }
  .empty {
    margin: 4px 18px;
    color: var(--text-muted);
  }
  .folder.droppable .fh {
    outline: 1px dashed var(--border-strong);
  }
  .folder.target .fh {
    outline: 1.5px dashed var(--accent);
    background: var(--bg-hover);
  }
  .fh {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 4px;
    color: var(--text-2);
    cursor: pointer;
  }
  .fh:hover {
    background: var(--bg-hover);
  }
  .fh b {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fh em {
    margin-left: auto;
    font-style: normal;
    color: var(--text-muted);
  }
  .chev {
    width: 9px;
    color: var(--text-muted);
    font-size: 9px;
  }
  .badge {
    font-size: 9px;
    padding: 0 4px;
    color: var(--accent-strong);
    border: 1px solid var(--accent-deep);
  }
  .rename {
    height: 20px;
    padding: 0 5px;
    border: 1px solid var(--accent);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(calc(var(--h) + 18px), 1fr));
    gap: 2px;
    padding: 1px 0 4px 12px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    padding-left: 12px;
  }
  .entry {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 1px 4px;
    border: 1px solid transparent;
    color: var(--text-2);
    text-align: left;
  }
  .tiles .entry {
    flex-direction: column;
    gap: 1px;
    padding: 3px 1px 2px;
  }
  .entry:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .entry.on {
    background: var(--bg-active);
    border-color: var(--accent);
    color: var(--text-strong);
  }
  .tip {
    width: var(--h);
    height: var(--h);
    flex-shrink: 0;
    display: grid;
    place-items: center;
  }
  .rows .tip {
    width: calc(var(--h) * 0.7);
    height: calc(var(--h) * 0.7);
  }
  .tip img {
    max-width: 100%;
    max-height: 100%;
    filter: invert(var(--ink-invert));
  }
  .round {
    width: 62%;
    height: 62%;
    border-radius: 50%;
    background: var(--text);
  }
  .round.soft {
    filter: blur(2px);
  }
  .stroke {
    flex-shrink: 0;
    height: calc(var(--h) * 0.8);
    object-fit: contain;
    filter: invert(var(--ink-invert));
  }
  .stroke:not([src]) {
    visibility: hidden;
  }
  .name {
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tiles .name {
    font-size: 9.5px;
    text-align: center;
  }
  .grip {
    position: absolute;
    right: 1px;
    bottom: 1px;
    width: 12px;
    height: 12px;
    cursor: nwse-resize;
    color: var(--text-muted);
  }
  .grip svg {
    width: 11px;
    height: 11px;
    stroke-width: 1.5;
  }
  .ghost {
    position: fixed;
    z-index: 70;
    padding: 4px 8px;
    background: var(--bg-active);
    border: 1px solid var(--accent);
    color: var(--text);
    pointer-events: none;
    box-shadow: var(--shadow-menu);
  }
  .ctx {
    position: fixed;
    z-index: 71;
    min-width: 180px;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .ctx button {
    padding: 5px 9px;
    text-align: left;
    color: var(--text);
  }
  .ctx button:hover {
    background: var(--accent);
    color: var(--on-accent);
  }
  .ctx .danger {
    color: var(--danger);
  }
  .ctx hr {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--border);
    margin: 4px 0;
  }
</style>
