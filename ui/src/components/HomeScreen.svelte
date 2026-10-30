<script lang="ts" module>
  import type { TemplateInfo as TemplateInfoCached } from "../lib/ipc";

  /** Modèles et aperçus déjà calculés, gardés pour toute la durée du lancement (voir `loadTemplates`). */
  const templateCache = $state<{ loaded: boolean; list: TemplateInfoCached[]; previews: Record<string, string> }>({ loaded: false, list: [], previews: {} });
</script>

<script lang="ts">
  import CategoryIcon from "./CategoryIcon.svelte";
  import logo from "../assets/logo/logo-128.png";
  import { t } from "../lib/i18n.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { onMount } from "svelte";
  import { createDocument, importFromUrl, openPaths, runAction } from "../lib/actions";
  import { refreshThumbnails } from "../lib/engine";
  import { type CatalogCategory, type Format, formatCategories, labelOf, matchesFormat, shapeOf } from "../lib/formats";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import {
    type FormatDraft,
    MAX_CATEGORIES,
    MAX_FILE_BYTES,
    addCategory,
    addFormat,
    customFormatList,
    duplicateFormat,
    editCustomFormats,
    loadCustomFormats,
    mergeCustomFormats,
    moveFormat,
    removeCategory,
    removeFormat,
    renameCategory,
    serializeCustomFormats,
    updateFormat,
  } from "../lib/custom-formats";
  import { manualOpen, searchManual } from "../lib/manual";
  import { manualNav } from "../lib/manual-nav.svelte";
  import { type QuickFormat, describe, formatHistory, fromFormat, loadFormatHistory, quickFormats } from "../lib/recent-formats";
  import { type Recent, forgetRecent, previewFor, recents, revealRecent, togglePinned, when } from "../lib/recents.svelte";
  import { api, type TemplateInfo } from "../lib/ipc";
  import { app, type OpenDocument } from "../state/app.svelte";
  import ContextMenu, { type MenuEntry } from "./ContextMenu.svelte";
  import FormatEditor from "./home/FormatEditor.svelte";
  import ManualView from "./home/ManualView.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import LanguageMenu from "./LanguageMenu.svelte";
  import MyRefView from "./myref/MyRefView.svelte";

  // Écran d'accueil (maquette validée « Lumitype — écran d'accueil ») : visible sans document, ou par Fichier > Accueil.
  // Nouveau / Ouvrir, dépôt d'une image (fichier, page web, adresse, capture d'écran), formats rapides, récents,
  // et la notice de toutes les fonctionnalités (onglet « Notice », components/home/ManualView.svelte).
  let { dropping = false }: { dropping?: boolean } = $props();

  type View = "home" | "recents" | "myref" | "templates" | "models" | "manual";
  let view = $state<View>("home");
  let query = $state("");
  let address = $state("");
  let version = $state("");
  let missing = $state<Record<string, boolean>>({});

  onMount(() => {
    getVersion().then((v) => (version = v), () => undefined);
    void loadFormatHistory();
    void loadCustomFormats();
  });

  // Notice ouverte à un endroit précis (menu Aide › Notice, résultat de recherche) : voir lib/manual-nav.svelte.ts.
  // La demande est prise ici puis vidée, pour qu'un retour ultérieur à l'accueil ne rouvre pas la notice.
  let manualRequest = $state<{ anchor: string; seq: number } | null>(null);
  $effect(() => {
    const wanted = manualNav.request;
    if (!wanted) return;
    manualNav.request = null;
    view = "manual";
    query = "";
    manualRequest = wanted;
  });

  /** Chaque clic sur l'onglet My ref réécrit son titre manuscrit (et ramène à ses collections). */
  let myrefReplay = $state(0);

  function go(next: View): void {
    if (next === "myref") myrefReplay += 1;
    view = next;
    query = "";
    if (next === "manual") {
      manualRequest = null;
      formatScroller?.scrollTo({ top: 0 }); // la notice s'ouvre à son début, pas à la hauteur de la page quittée
    }
    if (next === "models") void loadTemplates();
  }

  // Modèles : projets réalisés dans Lumitype, livrés avec l'application. Liste lue une fois, aperçus
  // calculés par le moteur à la première ouverture de l'onglet. Liste et aperçus vivent dans le bloc `module`
  // (`templateCache`) : partagés entre les montages de l'écran d'accueil, les URLs d'objets ne sont créées
  // qu'une fois par lancement au lieu d'être recréées (et jamais révoquées) à chaque retour à l'accueil.
  const templates = $derived(templateCache.list);
  const templatePreviews = $derived(templateCache.previews);
  async function loadTemplates(): Promise<void> {
    if (templateCache.loaded) return;
    templateCache.loaded = true;
    let list: TemplateInfo[];
    try {
      list = await api.listTemplates();
    } catch {
      return;
    }
    templateCache.list = list;
    for (const template of list) {
      void api.templatePreview(template.id).then(
        (png) => {
          if (png.byteLength > 0) templateCache.previews[template.id] = URL.createObjectURL(new Blob([png], { type: "image/png" }));
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

  // ---- Formats : la liste unique de lib/formats.ts ; les formats rapides suivent l'historique des créations.
  // Les formats personnels (lib/custom-formats.ts) en font partie : `$formatCategories` est lu pour que les cases
  // suivent un renommage ou une suppression.
  const quick = $derived.by(() => (void $formatCategories, quickFormats($formatHistory)));
  const used = $derived.by(() => (void $formatCategories, $formatHistory.map(describe)));

  /** Titre d'une case : le format du catalogue (« Story », « Publication portrait HD ») ou personnel, sinon la taille. */
  const quickTitle = (q: QuickFormat): string => (q.format ? `${labelOf(q.format, t)}${q.hd ? " HD" : ""}` : `${q.width} × ${q.height}`);
  /** Sous-titre : la plateforme (ou la catégorie personnelle) et la taille, ou « Personnalisé · 72 ppp » pour une taille libre. */
  function quickSub(q: QuickFormat): string {
    const id = q.format?.id;
    const category = id ? $formatCategories.find((c) => c.formats.some((f) => f.id === id)) : undefined;
    const platform = category && category.group !== "base" ? `${labelOf(category, t)} · ` : "";
    const base = q.format ? `${platform}${q.width} × ${q.height}` : t("Personnalisé · {dpi} ppp", { dpi: q.dpi });
    return q.transparent ? `${base} · ${t("fond transparent")}` : base;
  }

  /** Crée le document de cette case, tel qu'il a été fait la dernière fois (taille, résolution, fond). */
  function createQuick(q: QuickFormat): void {
    const { width, height, dpi, transparent } = q;
    void createDocument({ name: t("Sans titre"), width, height, dpi, transparent }, { width, height, dpi, transparent, ...(q.formatId ? { formatId: q.formatId } : {}), ...(q.hd ? { hd: true } : {}) });
  }
  /** Un format du catalogue (Standard ou HD, fond blanc) ou un format personnel (sur son fond). */
  function createFormat(format: Format, hd = false): void {
    const used = fromFormat(format, hd);
    void createDocument({ name: t("Sans titre"), width: used.width, height: used.height, dpi: used.dpi, transparent: used.transparent }, used);
  }

  // Page Formats : colonne de gauche (catégories), défilement jusqu'à la section, recherche par nom ou taille.
  let formatScroller = $state<HTMLElement>();
  let activeCategory = $state("essentiels");
  const formatSearch = $derived(view === "templates" ? query.trim() : "");
  // Une catégorie personnelle vide reste affichée (avec sa carte « + Ajouter un format »), sauf pendant une recherche.
  const shownCategories = $derived<CatalogCategory[]>(
    $formatCategories
      .map((c) => ({ ...c, formats: c.formats.filter((f) => matchesFormat(f, c, formatSearch, t)) }))
      .filter((c) => c.formats.length > 0 || (c.group === "custom" && !formatSearch)),
  );
  /**
   * Une seule catégorie à la fois (celle choisie dans la colonne) : la page entière était trop chargée. Pendant une
   * recherche, toutes les catégories qui ont un format correspondant sont montrées.
   */
  const visibleCategories = $derived<CatalogCategory[]>(
    formatSearch ? shownCategories : shownCategories.filter((c) => c.id === activeCategory),
  );
  const shownUsed = $derived(
    formatSearch
      ? used.filter((q) => {
          const needle = formatSearch.toLowerCase().replace(/\s*[x×*]\s*/g, "×");
          return `${quickTitle(q)} ${q.width}×${q.height}`.toLowerCase().includes(needle);
        })
      : used,
  );
  /**
   * Catégorie choisie dans la colonne : elle reste allumée pendant le défilement qui y mène, et après — les dernières
   * sections (catégories personnelles) ne peuvent pas monter tout en haut, le suivi du défilement allumerait sinon
   * une catégorie au-dessus. La main revient au suivi dès que l'utilisateur fait défiler lui-même (molette, barre,
   * clavier) : voir `releaseScroll`.
   */
  let scrollTarget: string | null = null;
  const releaseScroll = (): void => {
    scrollTarget = null;
  };
  $effect(() => {
    const scroller = formatScroller;
    if (!scroller) return;
    const events = ["wheel", "pointerdown", "keydown", "touchstart"] as const;
    for (const name of events) scroller.addEventListener(name, releaseScroll, { passive: true });
    return () => events.forEach((name) => scroller.removeEventListener(name, releaseScroll));
  });
  function scrollToCategory(id: string): void {
    activeCategory = id;
    scrollTarget = id;
    // Hors recherche, seule cette catégorie est affichée : retour en haut de la page. Pendant une recherche, la
    // colonne mène à sa section parmi les résultats.
    if (!formatSearch) formatScroller?.scrollTo({ top: 0 });
    else formatScroller?.querySelector(`[data-category="${id}"]`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }
  // Catégorie choisie disparue (catégorie personnelle supprimée, « Dernier utilisé » vidé) : la première de la liste.
  $effect(() => {
    if (activeCategory === "used" ? used.length > 0 : $formatCategories.some((c) => c.id === activeCategory)) return;
    activeCategory = $formatCategories[0]?.id ?? "essentiels";
  });
  /** En faisant défiler la page, la catégorie dont la section est en haut s'allume dans la colonne. */
  function followScroll(): void {
    if (view !== "templates" || !formatScroller) return;
    if (scrollTarget || !formatSearch) return; // défilement demandé par la colonne, ou une seule catégorie affichée
    const top = formatScroller.getBoundingClientRect().top + 120;
    let current = activeCategory;
    for (const section of formatScroller.querySelectorAll<HTMLElement>("[data-category]")) {
      if (section.getBoundingClientRect().top <= top) current = section.dataset.category ?? current;
    }
    activeCategory = current;
  }

  // ---- Catégories personnelles (lib/custom-formats.ts) : créées, renommées, supprimées ici ; leurs formats ajoutés,
  // modifiés, dupliqués, déplacés. Chaque changement est enregistré aussitôt dans les réglages.
  const customs = $derived($formatCategories.filter((c) => c.group === "custom"));
  /** Nom de la catégorie en cours de création (colonne de gauche) ; `null` : pas de création en cours. */
  let newCategory = $state<string | null>(null);
  /** Catégorie en cours de renommage (en-tête de sa section) et le nom tapé. */
  let renaming = $state<{ id: string; label: string } | null>(null);
  let confirmDelete = $state<CatalogCategory | null>(null);
  /** Panneau « Nouveau format » / « Modifier » : catégorie visée, format modifié (sinon ajout), carte d'origine. */
  let editor = $state<{ categoryId: string; formatId: string | null; initial: FormatDraft; anchor: { left: number; top: number; bottom: number } } | null>(null);
  /** Menu d'un format personnel ; `anchor` : sa carte, sous laquelle s'ouvrira le panneau « Modifier ». */
  let formatMenu = $state<{ x: number; y: number; format: Format; categoryId: string; anchor: { left: number; top: number; bottom: number } } | null>(null);

  /** Met le focus dans un champ dès qu'il apparaît (nom de catégorie tapé en place). */
  function focusNow(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  async function commitNewCategory(): Promise<void> {
    const label = newCategory?.trim() ?? "";
    newCategory = null;
    if (!label) return;
    const id = await editCustomFormats((list) => {
      const out = addCategory(list, label);
      return { list: out.list, result: out.id };
    });
    if (id) requestAnimationFrame(() => scrollToCategory(id));
  }

  function commitRename(): void {
    const current = renaming;
    renaming = null;
    if (current && current.label.trim()) void editCustomFormats((list) => renameCategory(list, current.id, current.label));
  }

  // ---- Export / import des catégories personnelles en .json (même texte que les réglages). L'import AJOUTE les
  // catégories du fichier (identifiants renouvelés au besoin, voir `mergeCustomFormats`) ; des noms déjà pris
  // demandent confirmation.
  const JSON_FILTERS = [{ name: "JSON", extensions: ["json"] }];
  /** Import en attente de confirmation : le texte du fichier et les noms déjà pris. */
  let importing = $state<{ text: string; duplicates: string[] } | null>(null);

  async function exportCustomFormats(): Promise<void> {
    try {
      await loadCustomFormats();
      const path = await save({ defaultPath: "formats-perso.json", filters: JSON_FILTERS });
      if (!path) return;
      // La commande n'écrit que des .json : l'extension est ajoutée si la fenêtre ne l'a pas mise.
      await api.writeJsonFile(/\.json$/i.test(path) ? path : `${path}.json`, serializeCustomFormats(customFormatList()));
    } catch (error) {
      app.fail(error);
    }
  }

  async function importCustomFormats(): Promise<void> {
    try {
      const path = await open({ multiple: false, directory: false, filters: JSON_FILTERS });
      if (typeof path !== "string") return;
      const bytes = await api.readFileBytes(path);
      if (bytes.byteLength > MAX_FILE_BYTES) throw new Error(t("Fichier trop gros pour des formats personnels (1 Mo au plus)."));
      const text = new TextDecoder().decode(bytes);
      await loadCustomFormats();
      const preview = mergeCustomFormats(customFormatList(), text);
      if (!preview.readable) throw new Error(t("Ce fichier n'est pas un export de formats personnels (JSON illisible)."));
      if (preview.added === 0 && preview.dropped > 0) throw new Error(t("Plus de place pour de nouvelles catégories ({n} au plus).", { n: MAX_CATEGORIES }));
      if (preview.added === 0) throw new Error(t("Aucune catégorie de formats dans ce fichier."));
      if (preview.duplicates.length > 0) importing = { text, duplicates: preview.duplicates };
      else await applyImport(text);
    } catch (error) {
      app.fail(error);
    }
  }

  /** Fusion appliquée à la liste du moment (elle a pu changer pendant la confirmation) ; défile jusqu'à la première ajoutée. */
  async function applyImport(text: string): Promise<void> {
    const first = await editCustomFormats((list) => {
      const out = mergeCustomFormats(list, text);
      return { list: out.list, result: out.added > 0 ? out.list[list.length]?.id : undefined };
    });
    if (first) requestAnimationFrame(() => scrollToCategory(first));
  }

  /** Une catégorie vide part tout de suite ; une catégorie qui contient des formats demande confirmation. */
  function askDeleteCategory(category: CatalogCategory): void {
    if (category.formats.length === 0) void editCustomFormats((list) => removeCategory(list, category.id));
    else confirmDelete = category;
  }

  /** Touches d'un champ tapé en place : Entrée valide, Échap annule (sans quitter l'accueil). */
  function inlineKeys(event: KeyboardEvent, commit: () => void, cancel: () => void): void {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      cancel();
    }
  }

  const anchorOf = (element: Element) => {
    const rect = element.getBoundingClientRect();
    return { left: rect.left, top: rect.top, bottom: rect.bottom };
  };
  function openAdd(categoryId: string, element: Element): void {
    editor = { categoryId, formatId: null, initial: { label: "", width: 1920, height: 1080, dpi: 72, transparent: false }, anchor: anchorOf(element) };
  }
  function openEdit(format: Format, categoryId: string, anchor: { left: number; top: number; bottom: number }): void {
    editor = { categoryId, formatId: format.id, initial: { label: format.label, width: format.width, height: format.height, dpi: format.dpi, transparent: !!format.transparent }, anchor };
  }
  function saveEditor(draft: FormatDraft): void {
    const current = editor;
    editor = null;
    if (!current) return;
    const formatId = current.formatId;
    if (formatId) void editCustomFormats((list) => updateFormat(list, formatId, draft));
    else void editCustomFormats((list) => addFormat(list, current.categoryId, draft).list);
  }

  function formatEntries(format: Format, categoryId: string, anchor: { left: number; top: number; bottom: number }): MenuEntry[] {
    const others = customs.filter((c) => c.id !== categoryId);
    return [
      { label: t("Modifier…"), run: () => openEdit(format, categoryId, anchor) },
      { label: t("Dupliquer"), run: () => void editCustomFormats((list) => duplicateFormat(list, format.id, t("(copie)")).list) },
      ...(others.length > 0
        ? others.map((c): MenuEntry => ({ label: t("Déplacer vers « {name} »", { name: c.label }), run: () => void editCustomFormats((list) => moveFormat(list, format.id, c.id)) }))
        : [{ label: t("Déplacer vers une autre catégorie"), disabled: true, run: () => undefined } satisfies MenuEntry]),
      "separator",
      { label: t("Supprimer"), run: () => void editCustomFormats((list) => removeFormat(list, format.id)) },
    ];
  }
  /** Menu d'un format personnel, au clic droit (sous le pointeur) ou par son bouton « ⋯ » (sous le bouton). */
  function openFormatMenu(event: MouseEvent, format: Format, categoryId: string): void {
    event.preventDefault();
    event.stopPropagation();
    const card = (event.currentTarget as HTMLElement).closest(".fcard") ?? (event.currentTarget as HTMLElement);
    const button = event.type === "contextmenu" ? null : (event.currentTarget as HTMLElement).getBoundingClientRect();
    formatMenu = { x: button ? button.left : event.clientX, y: button ? button.bottom + 2 : event.clientY, format, categoryId, anchor: anchorOf(card) };
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
      sub: `${current ? t("Document courant") : path ? t("Ouvert") : t("Ouvert, non enregistré")} · ${doc.info.width} × ${doc.info.height}`,
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
    sub: `${missing[recent.path] ? t("Introuvable") : when(recent.at)} · ${recent.width} × ${recent.height}`,
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

  // La recherche du haut cherche dans les fichiers et dans la notice ; sur la page Formats elle cherche un format.
  const search = $derived(view === "templates" ? "" : query.trim().toLowerCase());
  const foundRecents = $derived(search ? cards.filter((c) => c.name.toLowerCase().includes(search) || (c.path ?? "").toLowerCase().includes(search)) : []);
  const foundManual = $derived(search ? searchManual(search) : []);

  /** « 1 format », « 8 formats ». */
  const plural = (n: number): string => (n === 1 ? t("1 format") : t("{n} formats", { n }));
</script>

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
      {#if card.state}<span class="badge" class:now={card.state === "current"}>{card.state === "current" ? t("Reprendre") : t("Ouvert")}</span>{/if}
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

{#snippet quickCard(q: QuickFormat)}
  {@const box = shapeOf(q.width, q.height)}
  <button class="format" onclick={() => createQuick(q)} title={`${q.width} × ${q.height} px · ${q.dpi} ppp`}>
    <span class="ratio" style:width={box.width} style:height={box.height}></span>
    <span class="texts"><span class="title">{quickTitle(q)}</span><span class="sub">{quickSub(q)}</span></span>
  </button>
{/snippet}

<!-- Carte de la page Formats : un clic sur la carte crée la taille Standard, le bouton HD la taille HD. Le bouton
     qui couvre la carte (`.hit`) porte le clic ; le reste est posé dessus et laisse passer le pointeur. -->
{#snippet formatCard(format: Format)}
  {@const box = shapeOf(format.width, format.height, 22)}
  <div class="fcard">
    <button class="hit" aria-label={`${t(format.label)} · ${format.width} × ${format.height}`} onclick={() => createFormat(format)}></button>
    <span class="glyph"><span class="ratio" style:width={box.width} style:height={box.height}></span></span>
    <span class="texts">
      <span class="title">{t(format.label)}</span>
      <span class="sub">{format.width} × {format.height}{format.dpi !== 72 ? ` · ${format.dpi} ppp` : ""}{format.note ? ` · ${t(format.note)}` : ""}</span>
      <span class="sizes">
        <span class="size">{t("Standard")}</span>
        {#if format.hd}
          {@const hd = format.hd}
          <button class="size" title={t("Créer en HD : {width} × {height}", { width: hd.width, height: hd.height })} onclick={() => createFormat(format, true)}>HD {hd.width} × {hd.height}</button>
        {/if}
      </span>
    </span>
  </div>
{/snippet}

<!-- Format personnel : clic sur la carte = le document ; « ⋯ » ou clic droit = Modifier, Dupliquer, Déplacer, Supprimer. -->
{#snippet customCard(format: Format, categoryId: string)}
  {@const box = shapeOf(format.width, format.height, 22)}
  <div class="fcard custom" oncontextmenu={(event) => openFormatMenu(event, format, categoryId)} role="presentation">
    <button class="hit" aria-label={`${format.label} · ${format.width} × ${format.height}`} onclick={() => createFormat(format)}></button>
    <span class="glyph"><span class="ratio" class:clear={format.transparent} style:width={box.width} style:height={box.height}></span></span>
    <span class="texts">
      <span class="title">{format.label}</span>
      <span class="sub">{format.width} × {format.height} · {format.dpi} ppp{format.transparent ? ` · ${t("transparent")}` : ""}</span>
    </span>
    <button class="more" aria-label={t("Actions du format")} title={t("Modifier, dupliquer, déplacer, supprimer")} onclick={(event) => openFormatMenu(event, format, categoryId)}>⋯</button>
  </div>
{/snippet}

{#snippet usedCard(q: QuickFormat)}
  {@const box = shapeOf(q.width, q.height, 22)}
  <div class="fcard">
    <button class="hit" aria-label={`${quickTitle(q)} · ${q.width} × ${q.height}`} onclick={() => createQuick(q)}></button>
    <span class="glyph"><span class="ratio" style:width={box.width} style:height={box.height}></span></span>
    <span class="texts">
      <span class="title">{quickTitle(q)}</span>
      <span class="sub">{quickSub(q)}{q.format && q.dpi !== 72 ? ` · ${q.dpi} ppp` : ""}</span>
    </span>
  </div>
{/snippet}

<div class="home" role="region" aria-label={t("Accueil")}>
  <nav class="side" aria-label={t("Sections de l'accueil")}>
    <div class="brand">
      <img class="logo" src={logo} alt="" width="36" height="36" draggable="false" />
      <span class="brand-texts"><span class="app-name">{t("Lumitype")}</span><span class="brand-version">{version ? `Version ${version}` : ""}</span></span>
    </div>
    <div class="nav">
      <button class:on={view === "home"} onclick={() => go("home")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 2.5l9.5 8.2V21a1 1 0 0 1-1 1h-6v-6.5h-5V22h-6a1 1 0 0 1-1-1V10.7z" /></svg>{t("Accueil")}
      </button>
      <button class:on={view === "recents"} onclick={() => go("recents")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path fill-rule="evenodd" d="M12 2a10 10 0 1 1 0 20a10 10 0 1 1 0-20zm-1.4 5v6.1l4.7 2.8 1.1-1.8-3.6-2.1V7z" /></svg>{t("Récents")}
      </button>
      <!-- My ref : bibliothèque d'images de référence (components/myref), entre Récents et Formats. -->
      <button class:on={view === "myref"} onclick={() => go("myref")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5.5 2.5h8.2l5.8 5.8v13.2h-14zM13 3.5v5.5h5.5" fill="none" stroke="currentColor" stroke-width="1.6" /><path d="M8 17l2.6-3.2 2 2.3 1.4-1.6 2 2.5z" /></svg><i class="myref-label">My ref</i>
      </button>
      <button class:on={view === "templates"} onclick={() => go("templates")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2.5" y="2.5" width="8.5" height="8.5" rx="2.2" /><rect x="13" y="2.5" width="8.5" height="8.5" rx="2.2" /><rect x="2.5" y="13" width="8.5" height="8.5" rx="2.2" /><rect x="13" y="13" width="8.5" height="8.5" rx="2.2" /></svg>{t("Formats")}
      </button>
      <button class:on={view === "models"} onclick={() => go("models")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 5.5A2.5 2.5 0 0 1 5.5 3h13A2.5 2.5 0 0 1 21 5.5v13a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 18.5zM6 16.5l4-5 3 3.5 2-2.5 3 4z" /><circle cx="15.5" cy="8.5" r="1.8" /></svg>{t("Modèles")}
      </button>
      <!-- Notice : toutes les fonctionnalités, avec sommaire (components/home/ManualView.svelte). -->
      <button class:on={view === "manual"} onclick={() => go("manual")}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 4.5A1.5 1.5 0 0 1 4.5 3H9a3 3 0 0 1 3 3v15a2.5 2.5 0 0 0-2.5-2.5H3zM21 4.5A1.5 1.5 0 0 0 19.5 3H15a3 3 0 0 0-3 3v15a2.5 2.5 0 0 1 2.5-2.5H21z" /></svg>{t("Notice")}
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

  <main class="content" bind:this={formatScroller} onscroll={followScroll}>
    {#if view === "myref"}
      <MyRefView replay={myrefReplay} />
    {:else}
    <header>
      <div>
        <h1>{view === "home" ? t("Bon retour") : view === "recents" ? t("Récents") : view === "templates" ? t("Formats") : view === "models" ? t("Modèles") : t("Notice")}</h1>
        <p>
          {view === "home"
            ? t("Reprenez un projet, ou partez d'une image.")
            : view === "recents"
              ? t("Vos derniers fichiers ouverts ou enregistrés. Épinglez ceux que vous gardez sous la main.")
              : view === "templates"
                ? t("Un format, et c'est parti : le document s'ouvre aussitôt. « HD » double la définition pour plus de netteté à l'export.")
                : view === "models"
                  ? t("Des projets réalisés dans Lumitype, à ouvrir et à décortiquer : chacun s'ouvre comme une copie.")
                  : t("Toutes les fonctionnalités de Lumitype, expliquées simplement.")}
        </p>
      </div>
      <label class="search">
        <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="10" cy="10" r="6.6" /><path d="M15.2 15.2l5.6 5.6" /></svg>
        <input type="search" placeholder={view === "templates" ? t("Rechercher un format (ex. story, 1080)") : t("Rechercher un fichier, une fonction…")} aria-label={t("Rechercher")} bind:value={query} />
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
        <h2>{t("Notice")}</h2>
        {#if foundManual.length > 0}
          <!-- Un résultat = une ligne de la notice : un clic ouvre la notice sur elle, surlignée un instant. -->
          <div class="hits">
            {#each foundManual as hit (hit.anchor)}
              <button class="hit-row" onclick={() => manualOpen(hit.anchor)}>
                <span class="hit-title">{hit.title}{#if hit.where}<span class="hit-where">{hit.where}</span>{/if}</span>
                {#if hit.text}<span class="hit-text">{hit.text}</span>{/if}
              </button>
            {/each}
          </div>
        {:else}
          <p class="empty">{t("Aucune fonction de la notice ne correspond.")}</p>
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
            <span class="sub">{t("Depuis vos dossiers ou directement depuis une page web. PNG, JPEG, WebP, TIFF, AVIF, HEIC (iPhone), RAW (ARW, CR3, NEF, DNG…), projets .graf.")}</span>
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
        <!-- Du plus récent au plus ancien : le dernier format créé passe premier, sans doublon (voir recent-formats.ts). -->
        <div class="formats">
          {#each quick as q (q.key)}{@render quickCard(q)}{/each}
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
      <div class="formats-page">
        <nav class="categories" aria-label={t("Catégories de formats")}>
          <button class:on={activeCategory === "used"} disabled={used.length === 0} onclick={() => scrollToCategory("used")}>{t("Récents")}<em>{used.length}</em></button>
          {#each $formatCategories as category, index (category.id)}
            {#if index > 0 && category.group !== $formatCategories[index - 1]?.group}<span class="sep"></span>{/if}
            <button class:on={activeCategory === category.id} class:mine={category.group === "custom"} onclick={() => scrollToCategory(category.id)}><CategoryIcon id={category.id} size={16} /><span class="cat-label">{labelOf(category, t)}</span><em>{category.formats.length}</em></button>
          {/each}
          <!-- Nouvelle catégorie : son nom se tape en place ; Entrée la crée, Échap renonce. -->
          {#if newCategory !== null}
            {#if customs.length === 0}<span class="sep"></span>{/if}
            <input
              class="inline-name"
              type="text"
              maxlength="80"
              placeholder={t("Nom de la catégorie")}
              aria-label={t("Nom de la catégorie")}
              bind:value={newCategory}
              use:focusNow
              onkeydown={(event) => inlineKeys(event, () => void commitNewCategory(), () => (newCategory = null))}
              onblur={() => void commitNewCategory()}
            />
          {:else}
            <button class="add-category" class:spaced={customs.length === 0} onclick={() => (newCategory = "")}>{t("+ Nouvelle catégorie")}</button>
          {/if}
          <!-- Catégories personnelles en fichier .json : à garder, à passer à un autre poste. -->
          <span class="custom-io">
            <button disabled={customs.length === 0} title={t("Enregistrer vos catégories de formats dans un fichier .json")} onclick={() => void exportCustomFormats()}>{t("Exporter…")}</button>
            <span aria-hidden="true">·</span>
            <button title={t("Ajouter les catégories d'un fichier .json exporté")} onclick={() => void importCustomFormats()}>{t("Importer…")}</button>
          </span>
          <span class="sep"></span>
          <button onclick={() => runAction("file.new")}>{t("Personnalisé…")}</button>
        </nav>
        <div class="format-sections">
          {#if shownUsed.length > 0 && (formatSearch || activeCategory === "used")}
            <section data-category="used">
              <h3>{t("Récents")}<em>{plural(shownUsed.length)}</em></h3>
              <div class="fgrid">{#each shownUsed as q (q.key)}{@render usedCard(q)}{/each}</div>
            </section>
          {/if}
          {#each visibleCategories as category (category.id)}
            <section data-category={category.id}>
              {#if category.group === "custom"}
                <!-- Catégorie personnelle : son nom (renommable en place), puis « Renommer · Supprimer ». -->
                <div class="custom-head">
                  {#if renaming?.id === category.id}
                    <input
                      class="inline-name head"
                      type="text"
                      maxlength="80"
                      aria-label={t("Nom de la catégorie")}
                      bind:value={renaming.label}
                      use:focusNow
                      onkeydown={(event) => inlineKeys(event, commitRename, () => (renaming = null))}
                      onblur={commitRename}
                    />
                  {:else}
                    <h3>{category.label}<em>{plural(category.formats.length)}</em></h3>
                  {/if}
                  <span class="head-actions">
                    <button onclick={() => (renaming = { id: category.id, label: category.label })}>{t("Renommer")}</button>
                    <span aria-hidden="true">·</span>
                    <button onclick={() => askDeleteCategory(category)}>{t("Supprimer")}</button>
                  </span>
                </div>
                <div class="fgrid">
                  {#each category.formats as format (format.id)}{@render customCard(format, category.id)}{/each}
                  {#if !formatSearch}
                    <button class="fcard add" onclick={(event) => openAdd(category.id, event.currentTarget)}>
                      <span class="plus" aria-hidden="true">+</span>{t("Ajouter un format")}
                    </button>
                  {/if}
                </div>
              {:else}
                <h3>{t(category.label)}<em>{plural(category.formats.length)}</em></h3>
                <div class="fgrid">{#each category.formats as format (format.id)}{@render formatCard(format)}{/each}</div>
              {/if}
            </section>
          {/each}
          {#if shownCategories.length === 0 && shownUsed.length === 0}
            <p class="empty">{t("Aucun format ne correspond. Personnalisé… crée n'importe quelle taille.")}</p>
          {/if}
        </div>
      </div>
    {:else}
      <ManualView request={manualRequest} />
    {/if}
    {/if}
  </main>

  {#if cardMenu}
    <ContextMenu x={cardMenu.x} y={cardMenu.y} entries={cardEntries(cardMenu.card)} onclose={() => (cardMenu = null)} />
  {/if}

  {#if formatMenu}
    <ContextMenu x={formatMenu.x} y={formatMenu.y} entries={formatEntries(formatMenu.format, formatMenu.categoryId, formatMenu.anchor)} onclose={() => (formatMenu = null)} />
  {/if}

  {#if editor}
    <FormatEditor initial={editor.initial} editing={editor.formatId !== null} anchor={editor.anchor} onsave={saveEditor} oncancel={() => (editor = null)} />
  {/if}

  {#if importing}
    {@const pending = importing}
    <ConfirmDialog
      title={t("Importer des formats")}
      message={t("Des catégories portent déjà ce nom : {name}. Les importer quand même ?")}
      name={pending.duplicates.join(", ")}
      detail={t("Elles s'ajoutent à côté des vôtres : rien n'est remplacé.")}
      confirmLabel={t("Importer")}
      icon="import"
      onconfirm={() => {
        // Lu avant de fermer : `pending` suit `importing`, qui redevient `null`.
        const { text } = pending;
        importing = null;
        void applyImport(text).catch((error: unknown) => app.fail(error));
      }}
      oncancel={() => (importing = null)}
    />
  {/if}

  {#if confirmDelete}
    {@const doomed = confirmDelete}
    <ConfirmDialog
      title={t("Supprimer la catégorie")}
      message={t("Supprimer la catégorie {name} et ses formats ?")}
      name={doomed.label}
      detail={doomed.formats.length === 1 ? t("Son format sera supprimé. Les documents déjà créés ne changent pas.") : t("Ses {n} formats seront supprimés. Les documents déjà créés ne changent pas.", { n: doomed.formats.length })}
      confirmLabel={t("Supprimer")}
      onconfirm={() => {
        confirmDelete = null;
        void editCustomFormats((list) => removeCategory(list, doomed.id));
      }}
      oncancel={() => (confirmDelete = null)}
    />
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
    background: linear-gradient(180deg, color-mix(in srgb, var(--accent) 16%, transparent) 0%, color-mix(in srgb, var(--accent-shadow) 25%, transparent) 35%, transparent 70%), var(--bg-bar);
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
    flex-shrink: 0;
    display: block;
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
  .brand-version {
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
  .myref-label {
    letter-spacing: 0.01em;
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
    border: 1px solid rgb(var(--ink) / 0.08);
    background: var(--bg-panel);
    transition: transform 160ms var(--ease), background 160ms var(--ease);
  }
  .action:hover {
    background: var(--bg-card);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .action:active {
    transform: scale(0.99);
  }
  /* Carte principale : surface sombre soulignée d'un filet rouge en tête (pas de grand aplat de couleur). */
  .action.primary {
    border: 1px solid rgb(var(--ink) / 0.08);
    box-shadow: inset 0 3px 0 var(--accent);
    background: var(--bg-panel);
    color: var(--text);
  }
  .action.primary:hover {
    background: var(--bg-card);
  }
  .action .icon {
    width: 44px;
    height: 44px;
    border-radius: var(--radius-l);
    background: rgb(var(--ink) / 0.08);
    display: grid;
    place-items: center;
  }
  .action.primary .icon {
    background: var(--accent);
    color: var(--on-accent);
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
  .recent .texts {
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
    border: 1.5px dashed rgb(var(--ink) / 0.16);
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
    background: var(--bg-card);
    border: 1px solid rgb(var(--ink) / 0.1);
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
    border: 1px solid rgb(var(--ink) / 0.08);
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
  .format {
    height: 76px;
    box-sizing: border-box;
    padding: 0 16px;
    display: flex;
    align-items: center;
    gap: 14px;
    border-radius: var(--radius-l);
    border: 1px solid rgb(var(--ink) / 0.07);
    background: var(--bg-panel);
    text-align: left;
    transition: background 120ms var(--ease), border-color 120ms var(--ease);
  }
  .format:hover {
    background: var(--bg-card);
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .format.custom {
    border-style: dashed;
    border-color: rgb(var(--ink) / 0.14);
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
    border: 1px solid rgb(var(--ink) / 0.08);
    background: var(--bg-card);
    display: grid;
    place-items: center;
    transition: border-color 120ms var(--ease), transform 160ms var(--ease);
  }
  .recent .thumb:hover {
    border-color: rgb(var(--ink) / 0.22);
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
    border: 1px solid rgb(var(--ink) / 0.18);
    backdrop-filter: blur(6px);
  }
  .badge.now {
    background: var(--accent);
    border-color: transparent;
    color: var(--on-accent);
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

  /* ---- résultats de la notice (recherche) */
  .hits {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 8px;
  }
  .hit-row {
    min-width: 0;
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 3px;
    border: 1px solid var(--border);
    background: var(--bg-panel);
    text-align: left;
    transition: background 120ms var(--ease), border-color 120ms var(--ease);
  }
  .hit-row:hover {
    background: var(--bg-card);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .hit-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    font-size: 13px;
    font-weight: 600;
  }
  .hit-where {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .hit-text {
    font-size: 12px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Bouton invisible qui couvre une carte ou une ligne : il porte le clic, le contenu posé dessus laisse passer
     le pointeur (sauf ses propres boutons). Évite d'imbriquer un bouton dans un autre. */
  .hit {
    position: absolute;
    inset: 0;
    z-index: 0;
    padding: 0;
    border: 0;
    background: transparent;
  }
  .hit:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  /* ---- page Formats */
  .formats-page {
    display: grid;
    grid-template-columns: 190px minmax(0, 1fr);
    gap: 28px;
    align-items: start;
  }
  .categories {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
  }
  .categories button {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    border: 0;
    background: transparent;
    text-align: left;
    align-items: center;
    white-space: nowrap;
    padding: 8px 12px;
    font-size: 13px;
    color: var(--text-2);
    box-shadow: inset 2px 0 0 transparent;
  }
  .categories .cat-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .categories button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .categories button.on {
    color: var(--text-strong);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .categories button:disabled {
    cursor: default;
    color: var(--text-muted);
  }
  em {
    font-style: normal;
    font-size: 11.5px;
    font-weight: 400;
    color: var(--text-muted);
  }
  .sep {
    height: 1px;
    margin: 8px 0;
    background: var(--border);
  }
  /* Catégorie personnelle choisie : fond un peu plus appuyé, comme sur la maquette. */
  .categories button.mine.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .categories .add-category {
    margin-top: 6px;
    justify-content: flex-start;
    border: 1px dashed var(--border-strong);
    color: var(--text-3);
  }
  .categories .add-category.spaced {
    margin-top: 0;
  }
  /* Exporter… · Importer… : discrets, comme « Renommer · Supprimer » des en-têtes de catégories personnelles. */
  .custom-io {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px 2px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .categories .custom-io button {
    padding: 0;
    font-size: inherit;
    color: var(--text-3);
    box-shadow: none;
  }
  .categories .custom-io button:hover:not(:disabled) {
    background: transparent;
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .categories .custom-io button:disabled {
    color: var(--text-muted);
    opacity: 0.6;
  }
  .categories .add-category:hover {
    color: var(--text);
    border-color: var(--text-3);
  }
  .inline-name {
    height: 34px;
    box-sizing: border-box;
    padding: 0 12px;
    border: 1px solid var(--text-3);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    outline: none;
    user-select: text;
  }
  .categories .inline-name {
    margin-top: 6px;
  }
  .inline-name.head {
    flex: 1;
    max-width: 360px;
    height: 30px;
  }
  /* En-tête d'une catégorie personnelle : le titre (et son filet), puis « Renommer · Supprimer » à droite. */
  .custom-head {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .custom-head h3 {
    flex: 1;
    min-width: 0;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .head-actions button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-3);
    font: inherit;
  }
  .head-actions button:hover {
    color: var(--text);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .format-sections {
    display: flex;
    flex-direction: column;
    gap: 22px;
    min-width: 0;
  }
  .format-sections section {
    gap: 10px;
    scroll-margin-top: 12px;
  }
  h3 {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-2);
  }
  h3 em {
    text-transform: none;
    letter-spacing: 0;
  }
  h3::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--border);
  }
  .fgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(205px, 1fr));
    gap: 10px;
  }
  .fcard {
    position: relative;
    padding: 12px;
    display: flex;
    gap: 12px;
    align-items: flex-start;
    border: 1px solid var(--border);
    background: var(--bg-panel);
    transition: background 120ms var(--ease), border-color 120ms var(--ease);
  }
  .fcard:hover {
    background: var(--bg-card);
    border-color: var(--border-strong);
  }
  .fcard > :not(.hit) {
    position: relative;
    pointer-events: none;
  }
  .fcard .glyph {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
  }
  .fcard .ratio {
    display: block;
    box-sizing: border-box;
    border: 1.5px solid var(--text-2);
  }
  /* Format personnel transparent : le petit rectangle en pointillés. */
  .fcard .ratio.clear {
    border-style: dashed;
  }
  .fcard.custom {
    padding-right: 36px;
  }
  .fcard .more {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 26px;
    height: 24px;
    padding: 0;
    pointer-events: auto;
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 15px;
    line-height: 1;
  }
  .fcard .more:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  /* « + Ajouter un format » : même taille qu'une carte, bordure en pointillés. */
  .fcard.add {
    min-height: 64px;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border-style: dashed;
    border-color: var(--border-strong);
    background: transparent;
    color: var(--text-3);
    font-size: 12.5px;
  }
  .fcard.add:hover {
    color: var(--text);
    border-color: var(--text-3);
    background: var(--bg-hover);
  }
  .fcard.add .plus {
    font-size: 16px;
    line-height: 1;
  }
  .fcard .texts {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .fcard .title {
    font-size: 13px;
    font-weight: 600;
  }
  .fcard .sub {
    font-size: 11.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sizes {
    margin-top: 6px;
    display: flex;
    gap: 6px;
  }
  /* « Standard » et « HD » : même aspect, gris, comme les autres textes. */
  .size {
    padding: 3px 7px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text-2);
    font-size: 10.5px;
    line-height: 1.3;
  }
  button.size {
    pointer-events: auto;
  }
  button.size:hover {
    color: var(--text);
    border-color: var(--text-3);
    background: var(--bg-hover);
  }

  @media (max-width: 1180px) {
    .formats {
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }
    .search {
      width: 260px;
    }
  }
</style>
