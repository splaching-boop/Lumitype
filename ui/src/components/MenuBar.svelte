<script lang="ts">
  import logo from "../assets/logo/logo-64.png";
  import { t } from "../lib/i18n.svelte";
  import { ACTION_LABELS } from "../lib/action-catalog";
  import { type ActionId, isActionChecked, isActionEnabled, runAction } from "../lib/actions";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";
  import HomeButton from "./HomeButton.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  /** Sous-menu : un libellé, et ses entrées calculées à l'ouverture. */
  interface Submenu {
    submenu: string;
    entries: () => SubEntry[];
  }
  interface SubEntry {
    label: string;
    run: () => void;
    /** Entrée grisée (action indisponible). */
    disabled?: boolean;
    /** Trait de séparation (le libellé est ignoré). */
    separator?: boolean;
    shortcut?: string;
    /** Vignette montrée au survol (filtres). */
    preview?: string;
    hint?: string;
    isNew?: boolean;
    /** Option cochée (`true`) ou cochable non cochée (`false`) : une colonne de coche aligne les libellés. */
    checked?: boolean;
  }
  type Item = ActionId | "separator" | Submenu;
  interface Menu {
    label: string;
    items: Item[];
    /**
     * Menu utilisable même sans document ouvert. Les autres sont grisés sur l'écran d'accueil :
     * ouvrir une liste dont toutes les entrées sont inactives ne renseigne personne.
     */
    alwaysOn?: boolean;
    /** Menu sans liste : un clic lance directement cette action (Plans de travail). */
    direct?: ActionId;
  }

  // Menu Filtre : un sous-menu par catégorie de la galerie. Chaque filtre ouvre la galerie sur lui (la
  // Fluidité, sa propre fenêtre) ; au survol, sa vignette et sa description s'affichent à côté.
  const filterEntry = (type: GalleryKind): SubEntry => {
    const spec = specOf(type);
    return {
      label: `${t(spec?.label ?? type)}…`,
      shortcut: (FILTER_ACTIONS as readonly string[]).includes(`filter.${type}`) ? keymap.label(`filter.${type}` as ActionId) : "",
      preview: previewOf(type),
      hint: spec ? t(spec.hint) : "",
      isNew: NEW_FILTERS.has(type),
      disabled: !isActionEnabled("filter.gallery"),
      run: () => (type === "liquify" ? runAction("filter.liquify") : (app.filterDialog = type)),
    };
  };
  const FILTER_SUBMENUS: Submenu[] = CATEGORIES.map((category) => ({ submenu: category.label, entries: () => category.types.map(filterEntry) }));
  const FAVORITE_FILTERS: Submenu = { submenu: "Favoris", entries: () => favorites().map(filterEntry) };
  /** Filtres qui ont aussi leur propre action (raccourci possible). */
  const FILTER_ACTIONS = Object.keys(ACTION_LABELS).filter((id) => id.startsWith("filter."));

  /** Sous-menu fait d'actions ordinaires (Calque › Nouveau calque de réglage…). */
  const actionSubmenu = (label: string, ids: (ActionId | "separator")[]): Submenu => ({
    submenu: label,
    entries: () =>
      ids.map((id, n) =>
        id === "separator"
          ? { label: `—${n}`, separator: true, run: () => undefined }
          : { label: t(ACTION_LABELS[id]).replace(/^(Réglage|Calque de remplissage) /, ""), shortcut: keymap.label(id), disabled: !isActionEnabled(id), run: () => runAction(id) },
      ),
  });

  // Image › Mode, comme Photoshop : le mode du document coché ; « 8 bits/couche », seule profondeur, coché et grisé.
  const modeEntry = (id: ActionId, label: string): SubEntry => ({
    label: t(label),
    shortcut: keymap.label(id),
    disabled: !isActionEnabled(id),
    checked: isActionChecked(id) === true,
    run: () => runAction(id),
  });
  const MODE_SUBMENU: Submenu = {
    submenu: "Mode",
    entries: () => [
      modeEntry("image.modeBitmap", "Bitmap…"),
      modeEntry("image.modeGrayscale", "Niveaux de gris"),
      { label: "—1", separator: true, run: () => undefined },
      modeEntry("image.modeRgb", "Couleurs RVB"),
      modeEntry("image.modeCmyk", "Couleurs CMJN…"),
      { label: "—2", separator: true, run: () => undefined },
      modeEntry("image.mode8Bits", "8 bits/couche"),
    ],
  };

  // Affichage › Impression : cadres, traits de coupe, alerte d'encrage et séparations, cochables ; puis la vérification.
  const PRINT_SUBMENU: Submenu = {
    submenu: "Impression",
    entries: () => [
      modeEntry("view.printBleed", "Fonds perdus"),
      modeEntry("view.printSafe", "Zone de sécurité"),
      modeEntry("view.printMarks", "Traits de coupe"),
      { label: "—1", separator: true, run: () => undefined },
      modeEntry("view.inkAlert", "Alerte d'encrage"),
      modeEntry("view.separations", "Séparations (C/M/J/N)"),
      { label: "—2", separator: true, run: () => undefined },
      { label: t("Vérifier avant impression…"), shortcut: keymap.label("file.preflight"), disabled: !isActionEnabled("file.preflight"), run: () => runAction("file.preflight") },
    ],
  };

  // Les menus sans entrée sont ceux des étapes suivantes : visibles, mais inactifs.
  const MENUS: Menu[] = [
    {
      label: "Fichier",
      items: ["file.home", "separator", "file.new", "file.open", "file.importLayer", "file.placeSmart", "file.placeLinked", "separator", "file.save", "file.saveAs", "file.exportPng", "separator", "file.close"],
      alwaysOn: true,
    },
    {
      label: "Édition",
      items: ["edit.undo", "edit.redo", "edit.toggleLastState", "separator", "edit.cut", "edit.copy", "edit.paste", "separator", "edit.fillForeground", "edit.fillBackground", "separator", "edit.freeTransform", "separator", "edit.colorSettings", "edit.preferences", "edit.shortcuts"],
    },
    { label: "Image", items: [MODE_SUBMENU, "separator", "image.printSetup", "image.correctInk", "separator", "image.invert"] },
    {
      label: "Calque",
      items: [
        "layer.new",
        "layer.newGroup",
        "layer.group",
        "layer.duplicate",
        "layer.delete",
        "separator",
        // Comme Photoshop : réglages, remplissages et style de calque en sous-menus (le menu tient à l'écran).
        actionSubmenu("Nouveau calque de réglage", ["adjust.develop", "separator", "adjust.brightnessContrast", "adjust.levels", "adjust.curves", "adjust.exposure", "separator", "adjust.hueSaturation", "adjust.colorBalance", "adjust.blackWhite", "adjust.channelMixer", "adjust.selectiveColor", "separator", "adjust.invert", "adjust.negative", "adjust.posterize", "adjust.threshold", "adjust.gradientMap"]),
        actionSubmenu("Nouveau calque de remplissage", ["layer.fillSolidColor", "layer.fillGradient", "layer.fillPattern"]),
        actionSubmenu("Style de calque", ["layer.effects", "separator", "layer.copyStyle", "layer.pasteStyle", "layer.clearStyle"]),
        "separator",
        "layer.addMask",
        "layer.toggleClipping",
        "separator",
        "layer.convertToSmart",
        "layer.editSmartContent",
        "layer.replaceSmartContent",
        "layer.newSmartByCopy",
        "layer.exportSmartContent",
        "layer.embedSmartLink",
        "layer.relinkSmart",
        "layer.resetSmartTransform",
        "separator",
        "layer.rasterize",
        "layer.mergeGroup",
      ],
    },
    { label: "Texte", items: [] },
    { label: t("Sélection"), items: ["select.all", "select.none", "select.invert", "separator", "select.advanced", "select.feather", "select.modify", "select.quickMask"] },
    // Comme Photoshop : Filtre Camera Raw et Fluidité, hors galerie, sous elle.
    { label: "Filtre", items: ["filter.lastFilter", "filter.gallery", "separator", "filter.cameraRaw", "filter.liquify", "separator", ...FILTER_SUBMENUS, "separator", FAVORITE_FILTERS] },
    { label: "Affichage", items: ["view.zoomIn", "view.zoomOut", "view.fit", "view.actualSize", "view.resetRotation", "view.mirror", "separator", "view.toggleRulers", "view.toggleGuides", "view.lockGuides", "view.toggleSnap", "separator", "view.guideLayout", "view.clearGuides", "separator", PRINT_SUBMENU, "view.proofCmyk"] },
    { label: t("Fenêtre"), items: [] },
    { label: "Plans de travail", items: ["artboard.open"], direct: "artboard.open" },
    // Toujours les deux derniers, dans cet ordre : Copyright, puis Aide.
    { label: "Copyright", items: ["copyright.metadata"], alwaysOn: true },
    { label: "Aide", items: ["help.manual", "separator", "help.reportBug", "help.openLogs", "separator", "help.about"], alwaysOn: true },
  ];

  /**
   * Un menu est utilisable s'il a des entrées, et si l'écran d'accueil n'est pas affiché — sauf
   * Fichier et Aide, qui servent aussi depuis l'accueil.
   */
  const usable = (menu: Menu): boolean => menu.items.length > 0 && (menu.alwaysOn === true || !app.homeVisible);
  const isSubmenu = (item: Item): item is Submenu => typeof item === "object";
  /** Sous-menu ouvert (son libellé), position verticale de son entrée dans le menu. */
  let openSub = $state<{ label: string; top: number } | null>(null);
  $effect(() => {
    if (openMenu === null) openSub = null;
  });

  let openMenu = $state<string | null>(null);

  function choose(action: ActionId): void {
    openMenu = null;
    runAction(action);
  }
  import { CATEGORIES, NEW_FILTERS, type GalleryKind, previewOf, specOf } from "../lib/filters";
  import { favorites, lastApplied } from "../lib/filter-memory";

  let hoveredFilter = $state<{ image: string; hint: string; label: string; top: number; left: number } | null>(null);
  $effect(() => {
    if (openMenu === null) hoveredFilter = null;
  });

  // La barre de menus tient lieu de barre de titre (la fenêtre n'a pas de décoration du système) :
  // la saisir dans le vide déplace la fenêtre, un double-clic l'agrandit ou la restaure.
  // Seuls les espaces vides comptent : pas les menus, le bouton Exporter ni les boutons de fenêtre.
  const isFrame = (event: Event): boolean => event.target instanceof Element && event.target.closest("button, .menu") === null;

  function onFramePointerDown(event: PointerEvent): void {
    if (event.button !== 0 || !isFrame(event)) return;
    openMenu = null;
    void getCurrentWindow().startDragging();
  }

  function onFrameDoubleClick(event: MouseEvent): void {
    if (isFrame(event)) void getCurrentWindow().toggleMaximize();
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (openMenu && !(event.target instanceof Element && event.target.closest(".menubar"))) openMenu = null;
  }}
  onkeydown={(event) => {
    if (event.key === "Escape") openMenu = null;
  }}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="menubar" onpointerdown={onFramePointerDown} ondblclick={onFrameDoubleClick}>
  <div class="brand" aria-hidden="true"><img class="mark" src={logo} alt="" width="20" height="20" draggable="false" /><span class="name">Lumitype</span></div>
  <HomeButton />
  {#each MENUS as menu (menu.label)}
    <div class="menu">
      <button
        class="title"
        class:open={openMenu === menu.label}
        disabled={!usable(menu)}
        aria-haspopup="menu"
        aria-expanded={openMenu === menu.label}
        onclick={() => (menu.direct ? ((openMenu = null), choose(menu.direct)) : (openMenu = openMenu === menu.label ? null : menu.label))}
        onpointerenter={() => {
          if (openMenu && usable(menu) && !menu.direct) openMenu = menu.label;
        }}
      >
        {t(menu.label)}
      </button>
      {#if openMenu === menu.label && !menu.direct}
        <!-- Un menu long (Calque) défile plutôt que de sortir de la fenêtre ; pas Filtre, dont l'aperçu déborde à droite. -->
        <div class="dropdown" class:scroll={!menu.items.some(isSubmenu)} role="menu">
          {#each menu.items as item, index (index)}
            {#if item === "separator"}
              <div class="separator" role="separator"></div>
            {:else if isSubmenu(item)}
              {@const entries = item.entries()}
              <button
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={openSub?.label === item.submenu}
                class:active={openSub?.label === item.submenu}
                disabled={entries.every((entry) => entry.disabled || entry.separator)}
                onpointerenter={(event) => {
                  hoveredFilter = null;
                  openSub = { label: item.submenu, top: event.currentTarget.offsetTop - 7 };
                }}
                onclick={(event) => (openSub = { label: item.submenu, top: event.currentTarget.offsetTop - 7 })}
              >
                <span class="label">{t(item.submenu)}</span>
                <span class="shortcut arrow">›</span>
              </button>
              {#if openSub?.label === item.submenu}
                <div class="dropdown sub" role="menu" style:top="{openSub.top}px">
                  {#each entries as entry (entry.label)}
                    {#if entry.separator}
                      <div class="separator" role="separator"></div>
                    {:else}
                    <button
                      disabled={entry.disabled}
                      role="menuitem"
                      onclick={() => {
                        openMenu = null;
                        entry.run();
                      }}
                      onpointerenter={(event) => {
                        const box = event.currentTarget.parentElement;
                        hoveredFilter = entry.preview && box ? { image: entry.preview, hint: entry.hint ?? "", label: entry.label, top: (openSub?.top ?? 0) + event.currentTarget.offsetTop - 40, left: box.offsetLeft + box.offsetWidth + 8 } : null;
                      }}
                    >
                      <span class="label" class:checked={entry.checked === true} class:checkable={entry.checked === false}>{#if entry.isNew}<i class="new" aria-label={t("Nouveau")}></i>{/if}{entry.label}</span>
                      <span class="shortcut">{entry.shortcut ?? ""}</span>
                    </button>
                    {/if}
                  {/each}
                </div>
              {/if}
            {:else}
              <button
                role="menuitem"
                disabled={!isActionEnabled(item)}
                onclick={() => choose(item)}
                onpointerenter={() => {
                  openSub = null;
                  hoveredFilter = null;
                }}
              >
                <!-- Option à bascule : une coche dit si elle est active. -->
                <span class="label" class:checked={isActionChecked(item) === true}>{t(ACTION_LABELS[item])}{#if item === "filter.lastFilter" && lastApplied()}{` : ${t(specOf(lastApplied()?.type ?? "")?.label ?? "")}`}{/if}</span>
                <span class="shortcut">{keymap.label(item)}</span>
              </button>
            {/if}
          {/each}
          {#if hoveredFilter}
            <figure class="preview" style:top="{Math.max(0, hoveredFilter.top)}px" style:left="{hoveredFilter.left}px" aria-hidden="true">
              <img src={hoveredFilter.image} alt="" width="320" height="213" />
              <b>{hoveredFilter.label.replace(/…$/, "")}</b>
              <figcaption>{hoveredFilter.hint}</figcaption>
            </figure>
          {/if}
        </div>
      {/if}
    </div>
  {/each}
  <div class="spacer"></div>
  <!-- L'accueil n'a rien à exporter : le bouton disparaît plutôt que de rester grisé. -->
  {#if !app.homeVisible}
    <button class="pill" disabled={!isActionEnabled("file.exportPng")} onclick={() => runAction("file.exportPng")}>
      {t("Exporter")}
    </button>
  {/if}
  <WindowControls />
</header>

<style>
  .menubar {
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 0 0 12px;
    background: var(--bg-bar);
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 22px;
    margin-right: 10px;
    padding-right: 14px;
    border-right: 1px solid rgb(var(--ink) / 0.08);
    pointer-events: none;
  }
  .brand .mark {
    width: 20px;
    height: 20px;
    display: block;
  }
  .brand .name {
    font-weight: 600;
    letter-spacing: 0.2px;
    color: var(--text);
  }
  .menu {
    position: relative;
  }
  .title {
    padding: 6px 8px;
    border-radius: var(--radius-s);
    color: var(--text-2);
    transition: background 120ms var(--ease);
  }
  .title:disabled {
    color: var(--text-disabled);
  }
  .title:not(:disabled):hover,
  .title.open {
    background: var(--bg-active);
    color: var(--text);
  }
  .dropdown {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    /* Au-dessus de l'écran d'accueil (40) : sans cela, Fichier et Aide s'ouvraient derrière lui. */
    z-index: 60;
    min-width: 300px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .dropdown.scroll {
    max-height: calc(100vh - 56px);
    overflow-y: auto;
  }
  .dropdown.scroll > * {
    flex-shrink: 0;
  }
  .dropdown button {
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    color: var(--text);
    text-align: left;
  }
  .dropdown button:disabled {
    color: var(--text-disabled);
  }
  .dropdown button:not(:disabled):hover {
    background: var(--bg-active);
  }
  .shortcut {
    color: var(--text-muted);
  }
  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border-strong);
  }
  .spacer {
    flex: 1;
  }
  .pill {
    height: 24px;
    margin-right: 8px;
    padding: 0 14px;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
    transition: opacity 120ms var(--ease);
  }
  .pill:disabled {
    opacity: 0.35;
  }
  .preview {
    position: absolute;
    z-index: 2;
    width: 336px;
    margin: 0;
    padding: 8px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    pointer-events: none;
  }
  .preview img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: var(--radius-m);
  }
  .preview b {
    display: block;
    padding: 8px 4px 0;
    font-weight: 600;
  }
  /* Sous-menu : à droite du menu, à hauteur de son entrée. */
  .dropdown.sub {
    top: 0;
    left: calc(100% - 4px);
    min-width: 260px;
  }
  .dropdown button.active {
    background: var(--bg-active);
  }
  .arrow {
    font-size: 15px;
    line-height: 1;
  }
  .new {
    display: inline-block;
    width: 6px;
    height: 6px;
    margin: 0 7px 1px -2px;
    border-radius: 50%;
    background: var(--accent);
  }
  figcaption {
    padding: 8px 4px 2px;
    color: var(--text-3);
    line-height: 1.4;
  }
  .label.checked::before {
    content: "✓";
    display: inline-block;
    width: 16px;
    margin-left: -2px;
  }
  .label.checkable::before {
    content: "";
    display: inline-block;
    width: 16px;
    margin-left: -2px;
  }
</style>
