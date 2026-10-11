<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { ACTION_LABELS } from "../lib/action-catalog";
  import { type ActionId, isActionChecked, isActionEnabled, runAction } from "../lib/actions";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";
  import HomeButton from "./HomeButton.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  type Item = ActionId | "separator";
  interface Menu {
    label: string;
    items: Item[];
    /**
     * Menu utilisable même sans document ouvert. Les autres sont grisés sur l'écran d'accueil :
     * ouvrir une liste dont toutes les entrées sont inactives ne renseigne personne.
     */
    alwaysOn?: boolean;
  }

  // Les menus sans entrée sont ceux des étapes suivantes : visibles, mais inactifs.
  const MENUS: Menu[] = [
    {
      label: "Fichier",
      items: ["file.home", "separator", "file.new", "file.open", "file.importLayer", "file.placeSmart", "file.placeLinked", "separator", "file.save", "file.saveAs", "file.exportPng", "separator", "file.close"],
      alwaysOn: true,
    },
    {
      label: "Édition",
      items: ["edit.undo", "edit.redo", "edit.toggleLastState", "separator", "edit.cut", "edit.copy", "edit.paste", "separator", "edit.fillForeground", "edit.fillBackground", "separator", "edit.freeTransform", "separator", "edit.preferences", "edit.shortcuts"],
    },
    { label: "Image", items: ["image.invert"] },
    {
      label: "Calque",
      items: ["layer.new", "layer.newGroup", "layer.group", "layer.duplicate", "layer.delete", "separator", "adjust.develop", "adjust.brightnessContrast", "adjust.hueSaturation", "adjust.levels", "adjust.gradientMap", "adjust.invert", "adjust.negative", "separator", "layer.addMask", "layer.effects", "layer.toggleClipping", "separator", "layer.convertToSmart", "layer.editSmartContent", "layer.replaceSmartContent", "layer.newSmartByCopy", "layer.exportSmartContent", "layer.embedSmartLink", "layer.relinkSmart", "layer.resetSmartTransform", "separator", "layer.rasterize", "layer.mergeGroup"],
    },
    { label: "Texte", items: [] },
    { label: t("Sélection"), items: ["select.all", "select.none", "select.invert", "separator", "select.feather", "select.modify", "select.quickMask"] },
    { label: "Filtre", items: ["filter.liquify", "separator", "filter.gaussianBlur", "filter.fieldBlur", "filter.motionBlur", "filter.lensBlur", "separator", "filter.sharpen", "filter.presence", "filter.noise", "filter.gaussianNoise", "filter.chromaticAberration", "filter.glass", "separator", "filter.dither", "filter.colorDither", "filter.ascii"] },
    { label: "Affichage", items: ["view.zoomIn", "view.zoomOut", "view.fit", "view.actualSize", "separator", "view.toggleRulers", "view.toggleGuides", "view.lockGuides", "view.toggleSnap", "separator", "view.guideLayout", "view.clearGuides", "separator", "view.proofCmyk", "view.proofFogra39", "view.proofFogra51", "view.proofFogra52", "view.proofGracol2006", "view.proofSwop2006c3"] },
    { label: t("Fenêtre"), items: [] },
    { label: "Aide", items: ["help.about"], alwaysOn: true },
    { label: "Copyright", items: ["copyright.metadata"], alwaysOn: true },
  ];

  /**
   * Un menu est utilisable s'il a des entrées, et si l'écran d'accueil n'est pas affiché — sauf
   * Fichier et Aide, qui servent aussi depuis l'accueil.
   */
  const usable = (menu: Menu): boolean => menu.items.length > 0 && (menu.alwaysOn === true || !app.homeVisible);

  let openMenu = $state<string | null>(null);

  function choose(action: ActionId): void {
    openMenu = null;
    runAction(action);
  }
  import { previewOf, specOf } from "../lib/filters";

  let hoveredFilter = $state<{ type: string; top: number; left: number } | null>(null);
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
  <div class="brand" aria-hidden="true"><span class="mark">LT</span><span class="name">Lumitype</span></div>
  <HomeButton />
  {#each MENUS as menu (menu.label)}
    <div class="menu">
      <button
        class="title"
        class:open={openMenu === menu.label}
        disabled={!usable(menu)}
        aria-haspopup="menu"
        aria-expanded={openMenu === menu.label}
        onclick={() => (openMenu = openMenu === menu.label ? null : menu.label)}
        onpointerenter={() => {
          if (openMenu && usable(menu)) openMenu = menu.label;
        }}
      >
        {t(menu.label)}
      </button>
      {#if openMenu === menu.label}
        <!-- Un menu long (Calque) défile plutôt que de sortir de la fenêtre ; pas Filtre, dont l'aperçu déborde à droite. -->
        <div class="dropdown" class:scroll={!menu.items.some((item) => item.startsWith("filter."))} role="menu">
          {#each menu.items as item, index (index)}
            {#if item === "separator"}
              <div class="separator" role="separator"></div>
            {:else}
              <button
                role="menuitem"
                disabled={!isActionEnabled(item)}
                onclick={() => choose(item)}
                onpointerenter={(event) => {
                  // Survol d'un filtre : vignette de démonstration à droite du menu, à hauteur de l'entrée.
                  const type = item.startsWith("filter.") ? item.slice(7) : "";
                  const dropdown = event.currentTarget.parentElement;
                  hoveredFilter = type && previewOf(type) && dropdown ? { type, top: Math.max(0, event.currentTarget.offsetTop - 60), left: dropdown.offsetWidth + 8 } : null;
                }}
              >
                <!-- Option à bascule : une coche dit si elle est active. -->
                <span class="label" class:checked={isActionChecked(item) === true}>{t(ACTION_LABELS[item])}</span>
                <span class="shortcut">{keymap.label(item)}</span>
              </button>
            {/if}
          {/each}
          {#if hoveredFilter}
            {@const spec = specOf(hoveredFilter.type)}
            <figure class="preview" style:top="{hoveredFilter.top}px" style:left="{hoveredFilter.left}px" aria-hidden="true">
              <div class="split">
                <img src={previewOf(hoveredFilter.type)} alt="" width="336" height="224" />
                <span class="tag left">{t("Filtré")}</span>
                <span class="tag right">{t("Original")}</span>
              </div>
              <figcaption>{spec?.hint ?? ""}</figcaption>
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
    border-right: 1px solid rgba(255, 255, 255, 0.08);
    pointer-events: none;
  }
  .brand .mark {
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    background: var(--accent);
    color: #fff;
    font-size: 10px;
    font-weight: 800;
    letter-spacing: 0.5px;
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
    z-index: 1;
    width: 336px;
    margin: 0;
    padding: 8px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    pointer-events: none;
  }
  .split {
    position: relative;
    border-radius: var(--radius-m);
    overflow: hidden;
    line-height: 0;
  }
  .split img {
    display: block;
    width: 100%;
    height: auto;
  }
  /* Trait de séparation entre la moitié filtrée et la moitié d'origine. */
  .split::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    background: rgba(255, 255, 255, 0.85);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.35);
  }
  .tag {
    position: absolute;
    bottom: 8px;
    padding: 3px 8px;
    border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    font-size: 11px;
    line-height: 1.3;
  }
  .tag.left {
    left: 8px;
  }
  .tag.right {
    right: 8px;
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
</style>
