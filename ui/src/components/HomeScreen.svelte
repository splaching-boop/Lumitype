<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";
  import { createDocument, importFromUrl, openPaths, runAction } from "../lib/actions";
  import { refreshThumbnails } from "../lib/engine";
  import { type Article, KIND_LABEL, articleDate, articleText, localizedArticles } from "../lib/news";
  import { type Recent, forgetRecent, previewFor, recents, revealRecent, togglePinned, when } from "../lib/recents.svelte";
  import { api, type TemplateInfo } from "../lib/ipc";
  import { app, type OpenDocument } from "../state/app.svelte";
  import ContextMenu, { type MenuEntry } from "./ContextMenu.svelte";
  import ArticleArt from "./home/ArticleArt.svelte";
  import ArticleReader from "./home/ArticleReader.svelte";
  import LanguageMenu from "./LanguageMenu.svelte";

  // Écran d'accueil (maquette validée « Lumitype — écran d'accueil ») : visible sans document, ou par Fichier > Accueil.
  // Nouveau / Ouvrir, dépôt d'une image (fichier, page web, adresse, capture d'écran), formats rapides, récents,
  // et le journal des nouveautés et tutoriels livré avec l'application.
  let { dropping = false }: { dropping?: boolean } = $props();

  type View = "home" | "recents" | "templates" | "models" | "learn" | "news";
  let view = $state<View>("home");
  let query = $state("");
  let address = $state("");
  let reading = $state<Article | null>(null);
  /** Carte survolée : son film tourne, les autres restent sur leur image fixe. */
  let hovered = $state<string | null>(null);
  let version = $state("");
  let missing = $state<Record<string, boolean>>({});


  // Nouveautés pas encore vues : pastille sur « Nouveautés » (préférence de confort, gardée dans ce navigateur).
  const SEEN_KEY = "lumitype.news.seen";
  let seen = $state("");
  onMount(() => {
    getVersion().then((v) => (version = v), () => undefined);
    try {
      seen = localStorage.getItem(SEEN_KEY) ?? "";
    } catch {
      seen = "";
    }
  });
  const ARTICLES = $derived(localizedArticles());
  const unread = $derived(ARTICLES.filter((a) => a.date > seen).length);
  function markSeen(): void {
    seen = ARTICLES[0]?.date ?? "";
    try {
      localStorage.setItem(SEEN_KEY, seen);
    } catch {
      // stockage indisponible : la pastille reviendra au prochain lancement
    }
  }

  function go(next: View): void {
    view = next;
    query = "";
    if (next === "news") markSeen();
    if (next === "models") void loadTemplates();
  }

  // Modèles : projets réalisés dans Lumitype, livrés avec l'application. Liste lue une fois, aperçus
  // calculés par le moteur à la première ouverture de l'onglet (puis gardés d'un lancement à l'autre).
  let templates = $state<TemplateInfo[]>([]);
  let templatePreviews = $state<Record<string, string>>({});
  let templatesLoaded = false;
  async function loadTemplates(): Promise<void> {
    if (templatesLoaded) return;
    templatesLoaded = true;
    try {
      templates = await api.listTemplates();
    } catch {
      templates = [];
      return;
    }
    for (const template of templates) {
      void api.templatePreview(template.id).then(
        (png) => {
          if (png.byteLength > 0) templatePreviews[template.id] = URL.createObjectURL(new Blob([png], { type: "image/png" }));
        },
        () => undefined,
      );
    }
  }
  function openTemplate(template: TemplateInfo): void {
    void withBusyOpen(async () => {
      const info = await api.openTemplate(template.id);
      app.add(info);
      refreshThumbnails(info, []);
      app.homeOpen = false;
    });
  }
  /** Ouverture d'un modèle, sous le voile d'attente : un projet 4K de vingt calques met une seconde à se charger. */
  async function withBusyOpen(run: () => Promise<void>): Promise<void> {
    app.busy = t("Ouverture…");
    try {
      await run();
    } catch (error) {
      app.fail(error);
    } finally {
      app.busy = null;
    }
  }
  const templateCards = $derived<Card[]>(
    templates.map((template) => ({
      key: `modele-${template.id}`,
      name: template.name,
      sub: template.description || t("Projet réalisé dans Lumitype"),
      preview: templatePreviews[template.id] ?? "",
      state: null,
      missing: false,
      open: () => openTemplate(template),
      recent: null,
      path: null,
    })),
  );

  const FORMATS = [
    { label: t("Écran HD"), width: 1920, height: 1080, dpi: 72 },
    { label: "4K", width: 3840, height: 2160, dpi: 72 },
    { label: t("A4 · 300 ppp"), width: 2480, height: 3508, dpi: 300 },
    { label: t("Carré"), width: 1080, height: 1080, dpi: 72 },
    { label: "Story", width: 1080, height: 1920, dpi: 72 },
  ];
  const MORE_FORMATS = [
    { label: "HD", width: 1280, height: 720, dpi: 72 },
    { label: "QHD", width: 2560, height: 1440, dpi: 72 },
    { label: t("Ultra large"), width: 3440, height: 1440, dpi: 72 },
    { label: t("A3 · 300 ppp"), width: 3508, height: 4961, dpi: 300 },
    { label: t("A5 · 300 ppp"), width: 1748, height: 2480, dpi: 300 },
    { label: t("Publication 4:5"), width: 1080, height: 1350, dpi: 72 },
    { label: t("Bannière"), width: 1500, height: 500, dpi: 72 },
    { label: t("Miniature vidéo"), width: 1280, height: 720, dpi: 72 },
  ];
  /** Petit rectangle aux proportions du format (côté long : 26 px). */
  const shape = (w: number, h: number) => ({ width: `${Math.round((26 * w) / Math.max(w, h))}px`, height: `${Math.round((26 * h) / Math.max(w, h))}px` });

  function create(format: { label: string; width: number; height: number; dpi: number }): void {
    void createDocument({ name: t("Sans titre"), width: format.width, height: format.height, dpi: format.dpi, transparent: false });
  }

  async function openRecent(recent: Recent): Promise<void> {
    if (!(await api.fileExists(recent.path).catch(() => false))) {
      missing[recent.path] = true;
      app.error = `Fichier introuvable : ${recent.path}. Il a peut-être été déplacé ou supprimé.`;
      return;
    }
    await openPaths([recent.path]);
  }

  // ---- Menu contextuel d'une carte (clic droit), qu'elle soit ouverte ou simplement récente.
  let cardMenu = $state<{ x: number; y: number; card: Card } | null>(null);

  function cardEntries(card: Card): MenuEntry[] {
    const recent = card.recent;
    const path = card.path;
    return [
      { label: card.state ? "Reprendre" : "Ouvrir", run: card.open },
      ...(recent ? [{ label: recent.pinned ? t("Désépingler") : t("Épingler"), run: () => togglePinned(recent.path) } satisfies MenuEntry] : []),
      // Un document ouvert ne se cache pas : il reviendrait à la prochaine image affichée.
      ...(recent && !card.state ? [{ label: t("Cacher de la liste"), run: () => forgetRecent(recent.path) } satisfies MenuEntry] : []),
      ...(path
        ? ([
            "separator",
            { label: t("Ouvrir l'emplacement"), run: () => void revealRecent(path).catch((error: unknown) => app.fail(error)) },
            { label: t("Copier le chemin"), run: () => void navigator.clipboard.writeText(path).catch(() => (app.error = path)) },
          ] satisfies MenuEntry[])
        : []),
    ];
  }

  function importAddress(): void {
    const url = address.trim();
    if (!/^https?:\/\//i.test(url)) {
      app.error = t("Collez une adresse d'image qui commence par https://");
      return;
    }
    address = "";
    void importFromUrl(url).catch((error: unknown) => app.fail(error));
  }

  // Épinglés d'abord, puis du plus récent au plus ancien.
  const ordered = $derived([...recents.list].sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.at - a.at));
  const pinned = $derived(recents.list.filter((r) => r.pinned));

  /**
   * Une seule liste : ce qui est ouvert en ce moment passe devant, le document courant en tête,
   * puis les récents. Un document ouvert n'apparaît pas deux fois — sa carte remplace la ligne des
   * récents qui porte le même chemin.
   */
  interface Card {
    key: string;
    name: string;
    sub: string;
    preview: string;
    /** Étiquette posée sur la vignette : le document courant se reprend, les autres sont ouverts. */
    state: "current" | "open" | null;
    missing: boolean;
    open: () => void;
    /** Entrée des récents correspondante, quand il y en a une (épingler, retirer, chemin). */
    recent: Recent | null;
    path: string | null;
  }

  function openCard(doc: OpenDocument): Card {
    const path = doc.info.projectPath;
    const current = doc.info.id === app.activeId;
    const modified = app.isModified(doc);
    return {
      key: `doc-${doc.info.id}`,
      name: `${doc.info.name}${modified ? " *" : ""}`,
      sub: current
        ? `Document courant · ${doc.info.width} × ${doc.info.height}`
        : `${path ? "Ouvert" : "Ouvert, non enregistré"} · ${doc.info.width} × ${doc.info.height}`,
      preview: previewFor(path),
      state: current ? "current" : "open",
      missing: false,
      open: () => {
        app.activeId = doc.info.id;
        app.homeOpen = false;
      },
      recent: path ? (recents.list.find((r) => r.path === path) ?? null) : null,
      path,
    };
  }

  const recentCard = (recent: Recent): Card => ({
    key: `recent-${recent.path}`,
    name: recent.name,
    sub: `${missing[recent.path] ? "Introuvable" : when(recent.at)} · ${recent.width} × ${recent.height}`,
    preview: recents.previews[recent.key] ?? "",
    state: null,
    missing: !!missing[recent.path],
    open: () => void openRecent(recent),
    recent,
    path: recent.path,
  });

  /** Chemins déjà représentés par une carte de document ouvert : ils ne repassent pas dans les récents. */
  const shownPaths = $derived(new Set(app.documents.map((d) => d.info.projectPath).filter((p): p is string => !!p)));
  const openCards = $derived(
    [...app.documents].sort((a, b) => Number(b.info.id === app.activeId) - Number(a.info.id === app.activeId)).map(openCard),
  );
  const cards = $derived([...openCards, ...ordered.filter((r) => !shownPaths.has(r.path)).map(recentCard)]);

  const search = $derived(query.trim().toLowerCase());
  const foundRecents = $derived(search ? cards.filter((c) => c.name.toLowerCase().includes(search) || (c.path ?? "").toLowerCase().includes(search)) : []);
  const foundArticles = $derived(search ? ARTICLES.filter((a) => articleText(a).toLowerCase().includes(search)) : []);

  const featured = $derived(ARTICLES[0]);
  const others = $derived(ARTICLES.slice(1, 5));
  const learning = $derived(ARTICLES.filter((a) => a.kind !== "update"));

</script>

{#snippet illustration(article: Article, big: boolean)}
  <ArticleArt kind={article.illustration} size={big ? "feature" : "card"} live={hovered === article.id} />
{/snippet}

{#snippet meta(article: Article)}
  <span class="meta"><span class="tag" data-kind={article.kind}>{t(KIND_LABEL[article.kind])}{article.version ? ` ${article.version}` : ""}</span>{articleDate(article.date)} · {article.minutes} min</span>
{/snippet}

{#snippet fileCard(card: Card)}
  <div
    class="recent"
    class:missing={card.missing}
    class:current={card.state === "current"}
    oncontextmenu={(event) => {
      event.preventDefault();
      cardMenu = { x: event.clientX, y: event.clientY, card };
    }}
    role="presentation"
  >
    <button class="thumb" onclick={card.open} title={card.path ?? card.name}>
      {#if card.preview}<img src={card.preview} alt="" />{:else}<span class="blank">{card.name.replace(/\.[^.]+$/, "")}</span>{/if}
      {#if card.state}<span class="badge" class:now={card.state === "current"}>{card.state === "current" ? "Reprendre" : "Ouvert"}</span>{/if}
    </button>
    <div class="line">
      <span class="texts">
        <span class="name">{card.name}</span>
        <span class="sub" class:now={card.state === "current"}>{card.sub}</span>
      </span>
      {#if card.recent}
        {@const recent = card.recent}
        <button class="mini" class:on={recent.pinned} aria-label={recent.pinned ? t("Désépingler") : t("Épingler")} title={recent.pinned ? t("Désépingler") : t("Épingler")} onclick={() => togglePinned(recent.path)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 2.5h8l-1.2 7 3.7 3.8v2H5.5v-2l3.7-3.8zM11 15.3h2V22h-2z" /></svg>
        </button>
        {#if !card.state}
          <button class="mini" aria-label={t("Retirer des récents")} title={t("Retirer des récents")} onclick={() => forgetRecent(recent.path)}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
          </button>
        {/if}
      {/if}
    </div>
  </div>
{/snippet}

{#snippet articleCard(article: Article)}
  <button class="card" onclick={() => (reading = article)} onpointerenter={() => (hovered = article.id)} onpointerleave={() => (hovered = null)}>
    {@render illustration(article, false)}
    <span class="texts">
      {@render meta(article)}
      <span class="title">{t(article.title)}</span>
      <span class="summary">{t(article.summary)}</span>
    </span>
  </button>
{/snippet}

<div class="home" role="region" aria-label={t("Accueil")}>
  <nav class="side" class:behind={reading !== null} aria-label={t("Sections de l'accueil")}>
    <div class="brand">
      <span class="logo">LT</span>
      <span class="brand-texts"><span class="app-name">{t("Lumitype")}</span><span class="version">{version ? `Version ${version}` : ""}</span></span>
    </div>
    <div class="nav">
      <button class:on={view === "home"} onclick={() => go("home")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.5l9.5 8.2V21a1 1 0 0 1-1 1h-6v-6.5h-5V22h-6a1 1 0 0 1-1-1V10.7z" /></svg>{t("Accueil")}
      </button>
      <button class:on={view === "recents"} onclick={() => go("recents")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path fill-rule="evenodd" d="M12 2a10 10 0 1 1 0 20a10 10 0 1 1 0-20zm-1.4 5v6.1l4.7 2.8 1.1-1.8-3.6-2.1V7z" /></svg>{t("Récents")}
      </button>
      <button class:on={view === "templates"} onclick={() => go("templates")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.5" y="2.5" width="8.5" height="8.5" rx="2.2" /><rect x="13" y="2.5" width="8.5" height="8.5" rx="2.2" /><rect x="2.5" y="13" width="8.5" height="8.5" rx="2.2" /><rect x="13" y="13" width="8.5" height="8.5" rx="2.2" /></svg>{t("Formats")}
      </button>
      <button class:on={view === "models"} onclick={() => go("models")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5.5A2.5 2.5 0 0 1 5.5 3h13A2.5 2.5 0 0 1 21 5.5v13a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 18.5zM6 16.5l4-5 3 3.5 2-2.5 3 4z" /><circle cx="15.5" cy="8.5" r="1.8" /></svg>{t("Modèles")}
      </button>
      <button class:on={view === "learn"} onclick={() => go("learn")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3L1.5 8.5 12 14l8.5-4.4V16h2V8.5z" /><path d="M5.5 12.4v4.4c0 1.8 3 3.7 6.5 3.7s6.5-1.9 6.5-3.7v-4.4L12 15.8z" /></svg>{t("Apprendre")}
      </button>
      <button class:on={view === "news"} onclick={() => go("news")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 1.5q1.2 7.3 8.5 8.5q-7.3 1.2-8.5 8.5q-1.2-7.3-8.5-8.5q7.3-1.2 8.5-8.5z" /><circle cx="19" cy="19.5" r="2.5" /></svg>{t("Nouveautés")}
        {#if unread > 0}<span class="badge" aria-label={t("{unread} nouveautés", { unread })}></span>{/if}
      </button>
    </div>
    {#if pinned.length > 0}
      <div class="nav pinned">
        <span class="caption">{t("Épinglés")}</span>
        {#each pinned as recent (recent.path)}
          <button onclick={() => openRecent(recent)} title={recent.path}>
            {#if recents.previews[recent.key]}<img class="chip" src={recents.previews[recent.key]} alt="" />{:else}<span class="chip"></span>{/if}
            <span class="ellipsis">{recent.name}</span>
          </button>
        {/each}
      </div>
    {/if}
    <div class="nav bottom">
      {#if app.documents.length > 0}
        <button onclick={() => (app.homeOpen = false)}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M14.5 4.8L7.3 12l7.2 7.2" fill="none" stroke="currentColor" stroke-width="3.4" stroke-linecap="round" stroke-linejoin="round" /></svg>{t("Retour aux documents")}
        </button>
      {/if}
      <button onclick={() => (app.preferencesOpen = true)}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path fill-rule="evenodd" d="M10.3 1.8h3.4l.6 2.9 1.9.8 2.5-1.6 2.4 2.4-1.6 2.5.8 1.9 2.9.6v3.4l-2.9.6-.8 1.9 1.6 2.5-2.4 2.4-2.5-1.6-1.9.8-.6 2.9h-3.4l-.6-2.9-1.9-.8-2.5 1.6-2.4-2.4 1.6-2.5-.8-1.9-2.9-.6v-3.4l2.9-.6.8-1.9-1.6-2.5 2.4-2.4 2.5 1.6 1.9-.8zM12 8.4a3.6 3.6 0 1 0 0 7.2a3.6 3.6 0 1 0 0-7.2z" /></svg>{t("Préférences")}
      </button>
      <LanguageMenu />
    </div>
  </nav>

  <main class="content" class:behind={reading !== null}>
    <header>
      <div>
        <h1>{view === "home" ? t("Bon retour") : view === "recents" ? t("Récents") : view === "templates" ? t("Formats") : view === "models" ? t("Modèles") : view === "learn" ? t("Apprendre") : t("Nouveautés")}</h1>
        <p>
          {view === "home"
            ? t("Reprenez un projet, ou partez d'une image.")
            : view === "recents"
              ? t("Vos derniers fichiers ouverts ou enregistrés. Épinglez ceux que vous gardez sous la main.")
              : view === "templates"
                ? t("Un format, et c'est parti : le document s'ouvre aussitôt.")
                : view === "models"
                  ? t("Des projets réalisés dans Lumitype, à ouvrir et à décortiquer : chacun s'ouvre comme une copie.")
                  : view === "learn"
                  ? t("Tutoriels et astuces pour aller plus vite.")
                  : t("Ce qui change dans Lumitype, version après version.")}
        </p>
      </div>
      <label class="search">
        <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="10" r="6.6" /><path d="M15.2 15.2l5.6 5.6" /></svg>
        <input type="search" placeholder={t("Rechercher un fichier, un tutoriel…")} aria-label={t("Rechercher")} bind:value={query} />
      </label>
    </header>

    {#if search}
      <section>
        <h2>{t("Fichiers")}</h2>
        {#if foundRecents.length > 0}
          <div class="recents">{#each foundRecents as card (card.key)}{@render fileCard(card)}{/each}</div>
        {:else}
          <p class="empty">{t("Aucun fichier récent ne correspond.")}</p>
        {/if}
      </section>
      <section>
        <h2>{t("Articles")}</h2>
        {#if foundArticles.length > 0}
          <div class="list">{#each foundArticles as article (article.id)}{@render articleCard(article)}{/each}</div>
        {:else}
          <p class="empty">{t("Aucun article ne correspond.")}</p>
        {/if}
      </section>
    {:else if view === "home"}
      <section class="start">
        <div class="actions">
          <button class="action primary" onclick={() => runAction("file.new")}>
            <span class="icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v16M4 12h16" /></svg></span>
            <span class="texts"><span class="title">{t("Nouveau")}</span><span class="sub"><span>{t("Document vierge, à vos mesures")}</span><span>{t("Ctrl N")}</span></span></span>
          </button>
          <button class="action" onclick={() => runAction("file.open")}>
            <span class="icon"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2 6.6A2.6 2.6 0 0 1 4.6 4h5l2.3 2.6h7.5A2.6 2.6 0 0 1 22 9.2v9.2A2.6 2.6 0 0 1 19.4 21H4.6A2.6 2.6 0 0 1 2 18.4z" /></svg></span>
            <span class="texts"><span class="title">{t("Ouvrir…")}</span><span class="sub"><span>{t("Projet .graf ou image")}</span><span>{t("Ctrl O")}</span></span></span>
          </button>
        </div>
        <div class="drop" class:dropping>
          <div class="stack" aria-hidden="true">
            <span class="card-back left"></span>
            <span class="card-front"><svg viewBox="0 0 72 40"><path d="M0 40l18-20 12 12 14-18 28 26z" /><circle cx="54" cy="10" r="5" /></svg></span>
            <span class="card-back right"></span>
          </div>
          <div class="drop-texts">
            <span class="title">{dropping ? t("Relâchez pour ouvrir") : t("Déposez une image pour commencer")}</span>
            <span class="sub">{t("Depuis vos dossiers ou directement depuis une page web. PNG, JPEG, WebP, TIFF, AVIF, HEIC (iPhone), DNG, projets .graf.")}</span>
          </div>
          <form class="address" onsubmit={(event) => (event.preventDefault(), importAddress())}>
            <label>
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 14a4.5 4.5 0 0 0 6.4 0l3.2-3.2a4.5 4.5 0 0 0-6.4-6.4l-1 1" /><path d="M14 10a4.5 4.5 0 0 0-6.4 0l-3.2 3.2a4.5 4.5 0 0 0 6.4 6.4l1-1" /></svg>
              <input type="url" placeholder={t("Coller l'adresse d'une image (https://…)")} aria-label={t("Adresse d'une image")} bind:value={address} />
            </label>
            <button class="pill" type="submit">{t("Importer")}</button>
          </form>
          <span class="hint">{t("Ou Ctrl V pour coller une capture d'écran")}</span>
        </div>
      </section>

      <section>
        <h2>{t("Formats rapides")}</h2>
        <div class="formats">
          {#each FORMATS as format (format.label)}
            <button class="format" onclick={() => create(format)}>
              <span class="ratio" style:width={shape(format.width, format.height).width} style:height={shape(format.width, format.height).height}></span>
              <span class="texts"><span class="title">{t(format.label)}</span><span class="sub">{format.width} × {format.height}</span></span>
            </button>
          {/each}
          <button class="format custom" onclick={() => runAction("file.new")}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
            <span class="title">{t("Personnalisé…")}</span>
          </button>
        </div>
      </section>

      {#if cards.length > 0}
        <section>
          <div class="section-head">
            <h2>{t("Récents")}{#if openCards.length > 0}<span class="open-count">{openCards.length} ouvert{openCards.length > 1 ? "s" : ""}</span>{/if}</h2>
            <button class="link" onclick={() => go("recents")}>{t("Tout afficher")}</button>
          </div>
          <div class="recents">{#each cards.slice(0, 4) as card (card.key)}{@render fileCard(card)}{/each}</div>
        </section>
      {/if}

      {#if featured}
        <section>
          <div class="section-head"><h2>{t("Nouveautés et tutoriels")}</h2><button class="link" onclick={() => go("news")}>{t("Voir le journal")}</button></div>
          <div class="blog">
            <button class="feature" onclick={() => (reading = featured)} onpointerenter={() => (hovered = featured.id)} onpointerleave={() => (hovered = null)}>
              {@render illustration(featured, true)}
              <span class="texts">
                {@render meta(featured)}
                <span class="title">{t(featured.title)}</span>
                <span class="summary">{t(featured.summary)}</span>
              </span>
            </button>
            <div class="column">{#each others.slice(0, 2) as article (article.id)}{@render articleCard(article)}{/each}</div>
            <div class="column">{#each others.slice(2, 4) as article (article.id)}{@render articleCard(article)}{/each}</div>
          </div>
        </section>
      {/if}
    {:else if view === "recents"}
      <section>
        {#if cards.length > 0}
          <div class="recents">{#each cards as card (card.key)}{@render fileCard(card)}{/each}</div>
        {:else}
          <p class="empty">{t("Aucun fichier récent pour l'instant : ouvrez ou enregistrez un projet, il apparaîtra ici.")}</p>
        {/if}
      </section>
    {:else if view === "models"}
      <section>
        {#if templateCards.length > 0}
          <div class="recents">{#each templateCards as card (card.key)}{@render fileCard(card)}{/each}</div>
        {:else}
          <p class="empty">{t("Aucun modèle livré avec cette version.")}</p>
        {/if}
      </section>
    {:else if view === "templates"}
      <section>
        <div class="formats wide">
          {#each [...FORMATS, ...MORE_FORMATS] as format (format.label)}
            <button class="format" onclick={() => create(format)}>
              <span class="ratio" style:width={shape(format.width, format.height).width} style:height={shape(format.width, format.height).height}></span>
              <span class="texts"><span class="title">{t(format.label)}</span><span class="sub">{format.width} × {format.height}{format.dpi !== 72 ? ` · ${format.dpi} ppp` : ""}</span></span>
            </button>
          {/each}
          <button class="format custom" onclick={() => runAction("file.new")}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
            <span class="title">{t("Personnalisé…")}</span>
          </button>
        </div>
      </section>
    {:else}
      <section>
        <div class="list">{#each view === "learn" ? learning : ARTICLES as article (article.id)}{@render articleCard(article)}{/each}</div>
      </section>
    {/if}
  </main>

  {#if reading}
    <ArticleReader article={reading} onnavigate={(next) => (reading = next)} onclose={() => (reading = null)} />
  {/if}

  {#if cardMenu}
    <ContextMenu x={cardMenu.x} y={cardMenu.y} entries={cardEntries(cardMenu.card)} onclose={() => (cardMenu = null)} />
  {/if}
</div>

<style>
  .home {
    position: absolute;
    inset: 0;
    z-index: 40;
    display: flex;
    background: var(--bg-app);
    color: var(--text);
    font-size: 14px;
    overflow: hidden;
  }
  svg {
    fill: currentColor;
  }
  button {
    font: inherit;
    color: inherit;
    cursor: pointer;
  }

  /* Un article est ouvert : l'accueil passe derrière, flouté et assombri (verre dépoli). Le flou est posé sur
     l'accueil lui-même plutôt qu'en backdrop-filter, que WebKitGTK n'applique pas toujours. */
  .side,
  .content {
    transition: filter 200ms var(--ease);
  }
  .behind {
    filter: blur(12px) brightness(0.55) saturate(0.9);
    pointer-events: none;
  }

  /* ---- barre latérale */
  .side {
    width: 248px;
    flex-shrink: 0;
    box-sizing: border-box;
    padding: 20px 14px;
    display: flex;
    flex-direction: column;
    gap: 22px;
    /* Bande rouge qui s'éteint vers le noir, comme les bandes de l'affiche : signature de Lumitype. */
    background: linear-gradient(180deg, rgba(236, 42, 60, 0.16) 0%, rgba(57, 27, 30, 0.25) 35%, transparent 70%), var(--bg-bar);
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 4px 8px;
  }
  .logo {
    width: 36px;
    height: 36px;
    border-radius: var(--radius-m);
    background: var(--accent);
    color: #fff;
    display: grid;
    place-items: center;
    font-size: 16px;
    font-weight: 700;
    letter-spacing: -0.5px;
  }
  .brand-texts {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .app-name {
    font-size: 15px;
    font-weight: 600;
  }
  .version {
    font-size: 12px;
    color: var(--text-muted);
    min-height: 15px;
  }
  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .nav button {
    height: 36px;
    padding: 0 10px;
    display: flex;
    align-items: center;
    gap: 12px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    transition: background 120ms var(--ease);
  }
  .nav button svg {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  .nav button:hover {
    background: var(--bg-hover);
  }
  .nav button.on {
    background: var(--bg-active);
    color: var(--text);
    font-weight: 500;
    box-shadow: inset 3px 0 0 var(--accent); /* filet rouge de l'onglet actif */
  }
  .nav button.on svg {
    color: var(--text);
  }
  /* Pastille de nouveautés : un point, sans chiffre — l'accueil ne compte rien à l'écran. */
  .badge {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }
  .caption {
    padding: 0 10px 6px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.6px;
    text-transform: uppercase;
    color: #6f6f77;
  }
  .pinned button {
    height: 34px;
    font-size: 13px;
  }
  .chip {
    width: 20px;
    height: 13px;
    flex-shrink: 0;
    border-radius: var(--radius-xs);
    background: #2a2a2f;
    object-fit: cover;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bottom {
    margin-top: auto;
  }

  /* ---- contenu */
  .content {
    flex-grow: 1;
    min-width: 0;
    box-sizing: border-box;
    padding: 40px 56px 56px;
    display: flex;
    flex-direction: column;
    gap: 36px;
    overflow-y: auto;
  }
  header {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 24px;
  }
  h1 {
    margin: 0 0 6px;
    font-size: 34px;
    font-weight: 600;
    letter-spacing: -0.6px;
  }
  header p {
    margin: 0;
    font-size: 15px;
    color: var(--text-muted);
  }
  .search {
    width: 380px;
    height: 40px;
    flex-shrink: 0;
    box-sizing: border-box;
    padding: 0 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: var(--radius-l);
    background: var(--bg-field);
    border: 1px solid var(--border-strong);
  }
  .search svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 3;
    stroke-linecap: round;
  }
  .search input,
  .address input {
    flex-grow: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    user-select: text;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .section-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  .link {
    border: 0;
    background: transparent;
    color: var(--text-3);
    font-size: 13px;
  }
  .link:hover {
    color: var(--text);
  }
  .empty {
    margin: 0;
    color: var(--text-muted);
  }

  /* ---- commencer */
  .start {
    display: grid;
    grid-template-columns: 300px minmax(0, 1fr);
    gap: 20px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  .action {
    flex-grow: 1;
    box-sizing: border-box;
    padding: 22px;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 24px;
    text-align: left;
    border-radius: var(--radius-l);
    border: 1px solid rgba(255, 255, 255, 0.08);
    background: var(--bg-panel);
    transition: transform 160ms var(--ease), background 160ms var(--ease);
  }
  .action:hover {
    background: #1e1b1c;
    border-color: rgba(236, 42, 60, 0.5);
  }
  .action:active {
    transform: scale(0.99);
  }
  /* Carte principale : surface sombre soulignée d'un filet rouge en tête (pas de grand aplat de couleur). */
  .action.primary {
    border: 1px solid rgba(255, 255, 255, 0.08);
    box-shadow: inset 0 3px 0 var(--accent);
    background: var(--bg-panel);
    color: var(--text);
  }
  .action.primary:hover {
    background: #1e1b1c;
  }
  .action .icon {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-l);
    background: rgba(255, 255, 255, 0.08);
    display: grid;
    place-items: center;
  }
  .action.primary .icon {
    background: var(--accent);
    color: #fff;
  }
  .action .icon svg {
    width: 20px;
    height: 20px;
  }
  .action.primary .icon svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 3.4;
    stroke-linecap: round;
  }
  .action .texts,
  .format .texts,
  .recent .texts,
  .card .texts,
  .feature .texts {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .action .title {
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.3px;
  }
  .action .sub {
    display: flex;
    justify-content: space-between;
    font-size: 13px;
    color: var(--text-muted);
  }
  .action.primary .sub {
    color: #55555c;
  }

  .drop {
    box-sizing: border-box;
    min-height: 300px;
    padding: 32px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
    border-radius: var(--radius-l);
    border: 1.5px dashed rgba(255, 255, 255, 0.16);
    background: var(--bg-bar);
    text-align: center;
    transition: border-color 160ms var(--ease), background 160ms var(--ease);
  }
  .drop.dropping {
    border-color: var(--accent);
    background: rgba(77, 141, 255, 0.07);
  }
  .stack {
    display: flex;
    align-items: flex-end;
    transition: transform 200ms var(--ease);
  }
  .drop.dropping .stack {
    transform: translateY(-4px) scale(1.04);
  }
  .card-back {
    width: 56px;
    height: 44px;
    border-radius: var(--radius-m);
    background: #221f20;
    border: 1px solid rgba(255, 255, 255, 0.1);
  }
  .card-back.left {
    transform: rotate(-8deg) translate(10px, 4px);
  }
  .card-back.right {
    transform: rotate(8deg) translate(-10px, 4px);
  }
  .card-front {
    position: relative;
    z-index: 1;
    width: 72px;
    height: 56px;
    border-radius: var(--radius-m);
    background: #efeadf;
    display: flex;
    align-items: flex-end;
    overflow: hidden;
  }
  .card-front svg {
    width: 72px;
    height: 40px;
    fill: #201e1e;
  }
  .drop-texts {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .drop-texts .title {
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.3px;
  }
  .drop-texts .sub,
  .hint {
    color: var(--text-muted);
  }
  .hint {
    font-size: 12px;
    color: #6f6f77;
  }
  .address {
    width: min(520px, 100%);
    display: flex;
    gap: 8px;
  }
  .address label {
    flex-grow: 1;
    height: 40px;
    box-sizing: border-box;
    padding: 0 14px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: var(--radius-pill);
    background: var(--bg-field);
    border: 1px solid rgba(255, 255, 255, 0.08);
  }
  .address svg {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 2.6;
    stroke-linecap: round;
  }
  .pill {
    height: 40px;
    padding: 0 20px;
    border: 0;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 600;
  }
  .pill:disabled {
    opacity: 0.45;
    cursor: default;
  }

  /* ---- formats */
  .formats {
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 12px;
  }
  .formats.wide {
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  }
  .format {
    height: 76px;
    box-sizing: border-box;
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 14px;
    border-radius: var(--radius-l);
    border: 1px solid rgba(255, 255, 255, 0.07);
    background: var(--bg-panel);
    text-align: left;
    transition: background 120ms var(--ease), border-color 120ms var(--ease);
  }
  .format:hover {
    background: #1e1b1c;
    border-color: var(--border-strong);
  }
  .format .ratio {
    flex-shrink: 0;
    box-sizing: border-box;
    border-radius: var(--radius-xs);
    border: 2px solid var(--text-3);
  }
  .format .title {
    font-size: 14px;
    font-weight: 500;
  }
  .format .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
  .format.custom {
    border-style: dashed;
    border-color: rgba(255, 255, 255, 0.14);
    background: transparent;
    color: var(--text-2);
  }
  .format.custom svg {
    width: 22px;
    height: 22px;
    fill: none;
    stroke: var(--text-3);
    stroke-width: 3;
    stroke-linecap: round;
  }

  /* ---- récents */
  .recents {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 16px;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .recent .thumb {
    position: relative; /* repère de l'étiquette « Ouvert » / « Reprendre » */
    width: 100%;
    aspect-ratio: 16 / 9;
    padding: 0;
    overflow: hidden;
    border-radius: var(--radius-l);
    border: 1px solid rgba(255, 255, 255, 0.08);
    background: #1e1b1c;
    display: grid;
    place-items: center;
    transition: border-color 120ms var(--ease), transform 160ms var(--ease);
  }
  .recent .thumb:hover {
    border-color: rgba(255, 255, 255, 0.22);
  }
  .recent .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .recent .blank {
    padding: 0 16px;
    font-size: 18px;
    font-weight: 600;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .recent.missing .thumb {
    opacity: 0.4;
  }
  .recent .line {
    display: flex;
    align-items: flex-start;
    gap: 4px;
  }
  .recent .texts {
    flex-grow: 1;
    gap: 2px;
  }
  .recent .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent .sub {
    font-size: 12px;
    color: var(--text-muted);
  }
  .mini {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-muted);
    display: grid;
    place-items: center;
    opacity: 0;
    transition: opacity 120ms var(--ease), background 120ms var(--ease);
  }
  /* Document ouvert : une étiquette sur la vignette, et un liseré bleu pour celui qu'on reprendrait. */
  .badge {
    position: absolute;
    left: 8px;
    top: 8px;
    padding: 2px 8px;
    border-radius: var(--radius-pill);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.2px;
    color: var(--text-2);
    background: rgba(12, 12, 13, 0.72);
    border: 1px solid rgba(255, 255, 255, 0.18);
    backdrop-filter: blur(6px);
  }
  .badge.now {
    background: var(--accent);
    border-color: transparent;
    color: #fff;
  }
  .recent.current {
    outline: 1.5px solid var(--accent);
    outline-offset: 3px;
    border-radius: var(--radius-l);
  }
  .sub.now {
    color: var(--accent);
  }
  .open-count {
    margin-left: 10px;
    color: var(--accent);
    font-size: 12px;
    font-weight: 500;
  }
  .recent:hover .mini,
  .mini:focus-visible,
  .mini.on {
    opacity: 1;
  }
  .mini:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .mini.on {
    color: var(--accent);
  }
  .mini svg {
    width: 15px;
    height: 15px;
  }
  .mini svg path[d^="M6 6"] {
    fill: none;
    stroke: currentColor;
    stroke-width: 3;
    stroke-linecap: round;
  }

  /* ---- journal */
  .blog {
    display: grid;
    grid-template-columns: minmax(0, 1.35fr) minmax(0, 1fr) minmax(0, 1fr);
    gap: 16px;
  }
  .column {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 16px;
  }
  .feature,
  .card {
    box-sizing: border-box;
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    text-align: left;
    transition: background 120ms var(--ease), border-color 120ms var(--ease);
  }
  .feature:hover,
  .card:hover {
    background: #1e1b1c;
    border-color: rgba(236, 42, 60, 0.5);
  }
  .feature {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .feature .texts {
    padding: 18px 20px 20px;
    gap: 8px;
  }
  .feature .title {
    font-size: 19px;
    font-weight: 600;
    line-height: 1.3;
    letter-spacing: -0.2px;
  }
  .feature .summary {
    font-size: 14px;
    line-height: 1.5;
    color: var(--text-3);
  }
  .card {
    flex-grow: 1;
    padding: 16px;
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .card .texts {
    gap: 6px;
  }
  .card .title {
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
  }
  .card .summary {
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .tag {
    font-weight: 600;
  }
  .tag[data-kind="update"] {
    color: #ff8f9a;
  }
  .tag[data-kind="tutorial"] {
    color: #7fd6a4;
  }
  .tag[data-kind="tip"] {
    color: #f0b86b;
  }
  .feature .tag {
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-pill);
    background: rgba(77, 141, 255, 0.16);
    display: grid;
    place-items: center;
  }

  @media (max-width: 1180px) {
    .formats {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .blog {
      grid-template-columns: minmax(0, 1fr);
    }
    .search {
      width: 260px;
    }
  }
</style>
