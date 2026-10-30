<script lang="ts">
  import { onDestroy, tick } from "svelte";
  import { activeLayerIdForNewLayer, importDroppedFiles } from "../../lib/actions";
  import { avifPathToPng, isAvif } from "../../lib/avif";
  import { edit } from "../../lib/engine";
  import { formatDate, t } from "../../lib/i18n.svelte";
  import { type RefCollectionSummary, type RefItem, type RefPatch, formatBytes, formatKind, fullImageUrl, refApi } from "../../lib/myref";
  import { previewFor } from "../../lib/recents.svelte";
  import { type OpenDocument, app } from "../../state/app.svelte";
  import MyRefConfirm from "./MyRefConfirm.svelte";
  import RefThumb from "./RefThumb.svelte";
  import CollectionPicker from "./CollectionPicker.svelte";

  // Visionneuse de My ref (maquette « image ») : l'image en grand sur fond sombre, flèches et clavier pour passer
  // d'une image à l'autre, pellicule en bas, et à droite la fiche : favori, intégration au projet ouvert (calque),
  // déplacement, infos, tags, note, suppression. Le titre se renomme d'un clic (le fichier suit, extension gardée). Couvre l'accueil seulement : la barre de menus et les boutons de la fenêtre
  // restent accessibles. `role="dialog"` : les raccourcis de l'application ne réagissent pas derrière elle.
  let {
    root,
    items,
    index = $bindable(0),
    summaries,
    onclose,
    onchange,
    onremove,
    onmembership,
  }: {
    root: string;
    items: RefItem[];
    index?: number;
    /** Collections avec leur couverture et leur nombre d'images (liste des collections de l'image). */
    summaries: RefCollectionSummary[];
    onclose: () => void;
    /** Image ajoutée à une collection ou retirée. */
    onmembership: () => void;
    /** `previousId` : identifiant d'avant un renommage (l'identifiant suit le nom du fichier). */
    onchange: (item: RefItem, previousId?: string) => void;
    onremove: (id: string) => void;
  } = $props();

  // Jamais vide : le parent ferme la visionneuse quand sa dernière image s'en va.
  const item = $derived(items[Math.min(index, items.length - 1)] as RefItem);
  let url = $state("");
  let loading = $state(true);
  let chooser = $state(false);
  let nameDraft = $state<string | null>(null);
  let nameInput = $state<HTMLInputElement | null>(null);
  let tagDraft = $state<string | null>(null);
  let tagInput = $state<HTMLInputElement | null>(null);
  let note = $state("");
  let noteFor = "";
  /** Taille lue sur l'image affichée, quand la fiche ne la connaît pas encore (avant la première vignette). */
  let natural = $state<{ w: number; h: number } | null>(null);
  /** Retrait de l'image en attente de confirmation (fenêtre de l'application). */
  let confirming = $state(false);

  // Image courante : chargée en grand, l'adresse précédente libérée.
  let shown = "";
  $effect(() => {
    const current = item;
    if (!current || current.id === shown) return;
    shown = current.id;
    loading = true;
    natural = null;
    chooser = false;
    tagDraft = null;
    nameDraft = null;
    fullImageUrl(root, current).then(
      (next) => {
        if (shown !== current.id) return URL.revokeObjectURL(next);
        if (url) URL.revokeObjectURL(url);
        url = next;
        loading = false;
      },
      (error: unknown) => {
        loading = false;
        app.fail(error);
      },
    );
  });
  // La note suit l'image affichée (et n'est pas écrasée pendant la frappe par une mise à jour de la fiche).
  $effect(() => {
    if (item && item.id !== noteFor) {
      noteFor = item.id;
      note = item.note;
    }
  });
  onDestroy(() => url && URL.revokeObjectURL(url));

  function go(step: number): void {
    if (items.length === 0) return;
    index = (index + step + items.length) % items.length;
  }

  /**
   * Clavier, écouté à la phase de CAPTURE de la fenêtre : la visionneuse passe avant les autres écouteurs (le bouton
   * Accueil ramène aux documents sur Échap, l'application a ses raccourcis), et les touches qu'elle prend s'arrêtent là.
   */
  function onKey(event: KeyboardEvent): void {
    // La confirmation ouverte a la main sur le clavier (Échap l'annule, sans fermer la visionneuse).
    if (confirming) return;
    const typing = event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (nameDraft !== null) nameDraft = null;
      else if (chooser || tagDraft !== null) [chooser, tagDraft] = [false, null];
      else if (typing) (event.target as HTMLElement).blur();
      else onclose();
      return;
    }
    if (typing) return;
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    event.stopPropagation();
    go(event.key === "ArrowLeft" ? -1 : 1);
  }

  async function update(patch: RefPatch): Promise<void> {
    try {
      onchange(await refApi.updateItem(root, item.id, patch));
    } catch (error) {
      app.fail(error);
    }
  }

  // ---- nom

  async function startRename(): Promise<void> {
    nameDraft = item.name;
    await tick();
    nameInput?.select();
  }

  /** Renomme le fichier (le moteur garde l'extension et évite les doublons) ; vide ou inchangé : rien. */
  async function commitRename(): Promise<void> {
    if (nameDraft === null) return;
    const wanted = nameDraft.trim();
    nameDraft = null;
    if (!wanted || wanted === item.name) return;
    const previous = item.id;
    try {
      const renamed = await refApi.renameItem(root, previous, wanted);
      shown = renamed.id;
      noteFor = renamed.id;
      onchange(renamed, previous);
    } catch (error) {
      app.fail(error);
    }
  }

  // ---- tags

  async function startTag(): Promise<void> {
    tagDraft = "";
    await tick();
    tagInput?.focus();
  }

  function commitTag(): void {
    const tag = (tagDraft ?? "").trim();
    tagDraft = null;
    if (tag && !item.tags.some((existing) => existing.toLowerCase() === tag.toLowerCase())) void update({ tags: [...item.tags, tag] });
  }

  function saveNote(): void {
    if (note !== item.note) void update({ note });
  }

  // ---- intégration au projet

  /** Documents ouverts, le document actif en tête. */
  const docs = $derived([...app.documents].sort((a, b) => Number(b.info.id === app.activeId) - Number(a.info.id === app.activeId)));

  async function integrate(doc: OpenDocument): Promise<void> {
    const path = item.path;
    onclose();
    app.activeId = doc.info.id;
    app.homeOpen = false;
    if (isAvif(path)) {
      // L'AVIF passe par la webview (voir lib/avif.ts), comme à l'import d'un calque.
      try {
        await importDroppedFiles([await avifPathToPng(path)]);
      } catch (error) {
        app.fail(error);
      }
      return;
    }
    app.busy = t("Import…");
    try {
      await edit({ op: "importLayer", path, above: activeLayerIdForNewLayer() });
    } finally {
      app.busy = null;
    }
  }

  function integrateClick(): void {
    if (docs.length === 1 && docs[0]) void integrate(docs[0]);
    else chooser = !chooser;
  }

  // ---- rangement

  /** Retire l'image de la collection : le fichier part dans le dossier .corbeille de la bibliothèque (récupérable). */
  async function removeFromCollection(): Promise<void> {
    confirming = false;
    const id = item.id;
    try {
      await refApi.deleteItem(root, id);
      onremove(id);
    } catch (error) {
      app.fail(error);
    }
  }

  // ---- affichage

  const WEB = ["png", "jpg", "jpeg", "jfif", "webp", "avif"];
  const size = $derived(
    item?.width && item?.height ? `${item.width} × ${item.height} px` : natural && WEB.includes(item.file.split(".").pop()?.toLowerCase() ?? "") ? `${natural.w} × ${natural.h} px` : "",
  );
  const added = $derived(item ? formatDate(new Date(item.added), { day: "numeric", month: "short", year: "numeric" }) : "");
  const source = $derived.by(() => {
    if (!item?.source) return t("Glisser-déposer");
    if (/^https?:\/\//i.test(item.source)) {
      try {
        return new URL(item.source).hostname;
      } catch {
        return item.source;
      }
    }
    return item.source.split(/[\\/]/).pop() ?? item.source;
  });
  /** Pellicule : onze images autour de la courante. */
  const film = $derived.by(() => {
    const start = Math.max(0, Math.min(index - 5, items.length - 11));
    return items.slice(start, start + 11).map((entry, offset) => ({ entry, at: start + offset }));
  });
</script>

<svelte:window onkeydowncapture={onKey} />

{#if item}
  <div class="lightbox" role="dialog" aria-modal="true" aria-label={item.name}>
    <div class="stage">
      <div class="top">
        <span class="crumb"><i>My ref</i> › {item.collection} › <b>{item.name}</b></span>
        <span>{index + 1} / {items.length} · {t("Échap pour fermer")}</span>
        <span class="spacer"></span>
      </div>
      {#if url}
        <img
          class:dim={loading}
          src={url}
          alt={item.name}
          draggable="false"
          onload={(event) => {
            const img = event.currentTarget as HTMLImageElement;
            natural = { w: img.naturalWidth, h: img.naturalHeight };
          }}
        />
      {/if}
      {#if items.length > 1}
        <button class="nav left" aria-label={t("Image précédente")} onclick={() => go(-1)}>‹</button>
        <button class="nav right" aria-label={t("Image suivante")} onclick={() => go(1)}>›</button>
      {/if}
      <div class="film">
        {#each film as { entry, at } (entry.id)}
          <button class:on={at === index} aria-label={entry.name} onclick={() => (index = at)}><RefThumb {root} item={entry} fit="cover" /></button>
        {/each}
      </div>
    </div>
    <button class="close" aria-label={t("Fermer")} onclick={onclose}>✕</button>

    <aside>
      <div>
        {#if nameDraft !== null}
          <input
            class="name-input"
            bind:this={nameInput}
            bind:value={nameDraft}
            aria-label={t("Nom du fichier")}
            spellcheck="false"
            onblur={commitRename}
            onkeydown={(event) => event.key === "Enter" && (event.preventDefault(), (event.currentTarget as HTMLInputElement).blur())}
          />
        {:else}
          <button class="title" title={t("Renommer")} onclick={startRename}>
            <h3>{item.name}</h3>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4" /></svg>
          </button>
        {/if}
        <div class="muted">{[size, formatKind(item.file), formatBytes(item.size)].filter(Boolean).join(" · ")}</div>
      </div>

      <button class="big" class:fav={item.favorite} aria-pressed={item.favorite} onclick={() => update({ favorite: !item.favorite })}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z" fill={item.favorite ? "currentColor" : "none"} /></svg>
        {item.favorite ? t("Dans les favoris") : t("Ajouter aux favoris")}
      </button>

      <CollectionPicker {root} {item} collections={summaries} variant="panel" onmembership={onmembership} onremoved={(id) => onremove(id)} />

      {#if docs.length > 0}
      <div class="integrate">
        <button class="big" aria-expanded={chooser} onclick={integrateClick}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 4h16v16H4zM4 9h16M9 9v11" /></svg>
          <span class="label">
            {t("Intégrer au projet")}
            {#if docs.length === 1}<small>{docs[0]?.info.name}</small>{/if}
          </span>
          {#if docs.length > 1}<span class="caret" aria-hidden="true">▾</span>{/if}
        </button>
        {#if chooser && docs.length > 1}
          <div class="menu" role="menu">
            <div class="head">{t("{n} projets ouverts — choisir lequel", { n: docs.length })}</div>
            {#each docs as doc (doc.info.id)}
              {@const preview = previewFor(doc.info.projectPath)}
              <button class="it" class:on={doc.info.id === app.activeId} role="menuitem" onclick={() => integrate(doc)}>
                <span class="mini">{#if preview}<img src={preview} alt="" />{/if}</span>
                <span class="name">{doc.info.name}</span>
                {#if doc.info.id === app.activeId}<em>{t("actif")}</em>{/if}
              </button>
            {/each}
          </div>
        {/if}
      </div>
      {/if}

      <div class="sep"></div>

      <div class="lbl">{t("Infos")}</div>
      <dl>
        <dt>{t("Collection")}</dt>
        <dd>{item.collection}</dd>
        <dt>{t("Ajoutée le")}</dt>
        <dd>{added}</dd>
        <dt>{t("Source")}</dt>
        <dd class="ellipsis" title={item.source}>{source}</dd>
        {#if item.colors.length > 0}
          <dt>{t("Couleurs")}</dt>
          <dd class="swatches">{#each item.colors as color (color)}<i style:background={color} title={color}></i>{/each}</dd>
        {/if}
      </dl>

      <div class="lbl">{t("Tags")}</div>
      <div class="chips">
        {#each item.tags as tag (tag)}
          <button class="chip" title={t("Retirer")} onclick={() => update({ tags: item.tags.filter((x) => x !== tag) })}>{tag}<span aria-hidden="true">×</span></button>
        {/each}
        {#if tagDraft !== null}
          <input class="chip-input" bind:this={tagInput} bind:value={tagDraft} aria-label={t("Nouveau tag")} onblur={commitTag} onkeydown={(event) => event.key === "Enter" && (event.preventDefault(), commitTag(), void startTag())} />
        {:else}
          <button class="chip add" onclick={startTag}>+ {t("tag")}</button>
        {/if}
      </div>

      <div class="lbl">{t("Note")}</div>
      <textarea bind:value={note} onblur={saveNote} placeholder={t("Pourquoi cette image, ce qu'il faut en retenir…")}></textarea>

      <div class="grow"></div>
      <button class="remove" onclick={() => (confirming = true)}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13" /></svg>
        {t("Supprimer de la collection")}
      </button>
    </aside>
  </div>
  {#if confirming}
    <MyRefConfirm
      title={t("Supprimer de la collection ?")}
      message={t("Êtes-vous bien sûr de vouloir retirer l'image {name} de la collection « {collection} » ?", { collection: item.collection })}
      name={item.name}
      detail={t("Elle est déplacée dans le dossier .corbeille de votre bibliothèque My ref.")}
      confirmLabel={t("Supprimer")}
      onconfirm={removeFromCollection}
      oncancel={() => (confirming = false)}
    />
  {/if}
{/if}

<style>
  .lightbox {
    position: absolute;
    inset: 0;
    z-index: 60;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 330px;
    background: rgba(8, 8, 8, 0.95);
    animation: fade 160ms var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  button {
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  .stage {
    position: relative;
    min-width: 0;
    display: grid;
    place-items: center;
    padding: 56px 72px 96px;
    overflow: hidden;
  }
  .stage img {
    max-width: 100%;
    max-height: calc(100vh - 200px);
    object-fit: contain;
    box-shadow: 0 20px 80px rgba(0, 0, 0, 0.6);
    transition: opacity 160ms var(--ease);
  }
  .stage img.dim {
    opacity: 0.4;
  }
  .top {
    position: absolute;
    left: 24px;
    top: 18px;
    right: 24px;
    display: flex;
    justify-content: space-between;
    gap: 16px;
    color: var(--text-3);
    font-size: 12px;
  }
  .crumb {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .crumb b {
    color: var(--text);
  }
  .spacer {
    width: 30px;
  }
  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 40px;
    height: 56px;
    border: 0;
    display: grid;
    place-items: center;
    color: var(--text-2);
    font-size: 26px;
    background: rgba(255, 255, 255, 0.04);
  }
  .nav:hover {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text);
  }
  .nav.left {
    left: 16px;
  }
  .nav.right {
    right: 16px;
  }
  .film {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 16px;
    display: flex;
    justify-content: center;
    gap: 6px;
  }
  .film button {
    width: 46px;
    height: 46px;
    padding: 0;
    border: 1px solid var(--border);
    background: var(--bg-panel);
    opacity: 0.7;
  }
  .film button:hover {
    opacity: 1;
  }
  .film button.on {
    outline: 2px solid var(--accent);
    opacity: 1;
  }
  .close {
    position: absolute;
    right: 342px;
    top: 10px;
    width: 32px;
    height: 32px;
    border: 0;
    background: transparent;
    display: grid;
    place-items: center;
    color: var(--text-2);
    font-size: 18px;
    z-index: 2;
  }
  .close:hover {
    color: var(--text);
  }

  /* ---- fiche */
  aside {
    min-height: 0;
    overflow-y: auto;
    box-sizing: border-box;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
    padding: 22px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    color: var(--text);
  }
  h3 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--text-muted);
    font-size: 12px;
    margin-top: 3px;
  }
  .big {
    width: 100%;
    min-height: 40px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    border: 1px solid var(--border-strong);
    background: transparent;
    font-size: 13px;
    text-align: left;
  }
  .big:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .big:disabled {
    color: var(--text-disabled);
    cursor: default;
  }
  .big.fav {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .big.fav:hover {
    background: var(--accent-strong);
  }
  .big svg {
    width: 17px;
    height: 17px;
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .label small {
    color: var(--text-muted);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .caret {
    margin-left: auto;
    color: var(--text-muted);
  }
  .menu {
    border: 1px solid var(--border-strong);
    background: var(--bg-menu);
    box-shadow: var(--shadow-menu);
    display: flex;
    flex-direction: column;
  }
  .menu .head {
    padding: 8px 12px;
    color: var(--text-muted);
    font-size: 11px;
    border-bottom: 1px solid var(--border);
  }
  .it {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    border: 0;
    background: transparent;
    font-size: 12.5px;
    text-align: left;
  }
  .it:hover {
    background: var(--bg-hover);
  }
  .it.on {
    background: var(--bg-hover);
  }
  .it.on .name {
    color: var(--accent);
    font-weight: 600;
  }
  .it .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .it em {
    margin-left: auto;
    font-style: normal;
    color: var(--text-muted);
    font-size: 11px;
  }
  .mini {
    width: 30px;
    height: 22px;
    flex: none;
    background: #2a2727;
    overflow: hidden;
  }
  .mini img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .sep {
    border-top: 1px solid var(--border);
  }
  .lbl {
    font-size: 11px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    margin-bottom: -8px;
  }
  dl {
    display: grid;
    grid-template-columns: 90px minmax(0, 1fr);
    gap: 7px 10px;
    margin: 0;
    font-size: 12px;
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    margin: 0;
    color: var(--text-2);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .swatches {
    display: flex;
    gap: 3px;
  }
  .swatches i {
    width: 14px;
    height: 14px;
    display: block;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid var(--border-strong);
    background: transparent;
    padding: 3px 8px;
    font-size: 11.5px;
    color: var(--text-2);
  }
  .chip span {
    display: none;
    color: var(--text-muted);
  }
  .chip:hover span {
    display: inline;
  }
  .chip.add {
    border-style: dashed;
    color: var(--text-muted);
  }
  .chip-input {
    width: 110px;
    height: 24px;
    box-sizing: border-box;
    padding: 0 8px;
    border: 1px solid var(--accent);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    font-size: 11.5px;
    outline: none;
    user-select: text;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    min-height: 70px;
    background: var(--bg-field);
    border: 1px solid var(--border);
    color: var(--text-2);
    padding: 8px;
    font: inherit;
    font-size: 12px;
    resize: vertical;
    outline: none;
    user-select: text;
  }
  textarea:focus {
    border-color: var(--border-strong);
  }
  .grow {
    flex-grow: 1;
  }
  .integrate {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .title {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
  }
  .title svg {
    width: 14px;
    height: 14px;
    margin-top: 3px;
    flex-shrink: 0;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 1.7;
    opacity: 0;
    transition: opacity 120ms var(--ease);
  }
  .title:hover svg,
  .title:focus-visible svg {
    opacity: 1;
  }
  .name-input {
    width: 100%;
    box-sizing: border-box;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--accent);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    font-size: 15px;
    font-weight: 600;
    outline: none;
    user-select: text;
  }
  .remove {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 34px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text-2);
    font-size: 12.5px;
  }
  .remove svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
  }
  .remove:hover {
    color: var(--danger);
    border-color: var(--danger);
  }
</style>
