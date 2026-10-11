<script lang="ts">
  import { t } from "./lib/i18n.svelte";
  import ColorPanel from "./components/ColorPanel.svelte";
  import DocTabs from "./components/DocTabs.svelte";
  import ErrorToast from "./components/ErrorToast.svelte";
  import FeatherDialog from "./components/FeatherDialog.svelte";
  import HistoryPanel from "./components/HistoryPanel.svelte";
  import LayersPanel from "./components/LayersPanel.svelte";
  import MenuBar from "./components/MenuBar.svelte";
  import NewDocumentDialog from "./components/NewDocumentDialog.svelte";
  import OptionsBar from "./components/OptionsBar.svelte";
  import Panel from "./components/Panel.svelte";
  import AboutDialog from "./components/AboutDialog.svelte";
  import BrushPicker from "./components/BrushPicker.svelte";
  import BrushQuickMenu from "./components/BrushQuickMenu.svelte";
  import ExportProgress from "./components/ExportProgress.svelte";
  import RecoveryDialog from "./components/RecoveryDialog.svelte";
  import HomeScreen from "./components/HomeScreen.svelte";
  import { api } from "./lib/ipc";
  import AdjustmentProperties from "./components/AdjustmentProperties.svelte";
  import AdjustmentsPanel from "./components/AdjustmentsPanel.svelte";
  import EffectsDialog from "./components/EffectsDialog.svelte";
  import ExportDialog from "./components/ExportDialog.svelte";
  import FilterDialog from "./components/FilterDialog.svelte";
  import GuideLayoutDialog from "./components/GuideLayoutDialog.svelte";
  import LiquifyDialog from "./components/LiquifyDialog.svelte";
  import ModifySelectionDialog from "./components/ModifySelectionDialog.svelte";
  import PreferencesDialog from "./components/PreferencesDialog.svelte";
  import CopyrightDialog from "./components/CopyrightDialog.svelte";
  import SavePrompt from "./components/SavePrompt.svelte";
  import SplashScreen from "./components/SplashScreen.svelte";
  import ShapeDialog from "./components/ShapeDialog.svelte";
  import ShapeProperties from "./components/ShapeProperties.svelte";
  import SmartProperties from "./components/SmartProperties.svelte";
  import StatusBar from "./components/StatusBar.svelte";
  import TextProperties from "./components/TextProperties.svelte";
  import WindowEdges from "./components/WindowEdges.svelte";
  import Toolbar from "./components/Toolbar.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, untrack } from "svelte";
  import ShortcutsDialog from "./components/ShortcutsDialog.svelte";
  import { checkSmartLinks, importDroppedFiles, importFromUrl, importLocalPaths, pasteFromClipboard, runAction, setOpacityFromDigit, settleUnsaved } from "./lib/actions";
  import { floatingPanelOpen, resolve } from "./lib/keymap";
  import { commitTransform, transformIsModal, transformIsPending } from "./lib/transform-session";
  import { rgbToHex } from "./lib/color";
  import { app } from "./state/app.svelte";
  import { keymap } from "./state/keymap.svelte";
  import Viewport from "./viewport/Viewport.svelte";

  /** Actions qui se répètent quand la touche reste enfoncée. */
  const REPEATABLE = /^(view\.zoom|brush\.|edit\.undo|edit\.redo)/;

  /**
   * Chiffre sans modificateur, lu par POSITION de touche : en AZERTY la rangée
   * du haut donne & é " … sans Maj, mais `event.code` vaut bien Digit1, Digit2…
   */
  function plainDigit(event: KeyboardEvent): number | null {
    const typing = event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement;
    if (typing || event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return null;
    const match = /^(?:Digit|Numpad)(\d)$/.exec(event.code);
    return match?.[1] === undefined ? null : Number(match[1]);
  }

  const LEFT_TABS = [
    { id: "properties", label: t("Propriétés") },
    { id: "adjustments", label: t("Réglages") },
    { id: "history", label: t("Historique") },
  ];

  /**
   * Pinceau ou gomme en main : Propriétés montre les pinceaux — SAUF sur un calque de réglage, dont
   * on ne peut pas peindre les pixels. Là, ce sont ses curseurs qui servent ; et si l'on vise son
   * masque (l'autre vignette), les pinceaux reviennent, puisqu'on peut bel et bien y peindre.
   */
  const brushInProperties = $derived(!!app.paintTool && !(app.activeLayer?.adjustment && app.active?.target !== "mask"));

  // L'onglet de gauche ne change jamais tout seul : créer ou sélectionner un calque de réglage, un texte
  // ou une forme laisse l'onglet où l'utilisateur l'a mis (Réglages, Historique…). Ses curseurs sont dans
  // Propriétés, qu'il ouvre quand il le veut.

  onMount(() => void keymap.load());

  // Fermer la fenêtre demande confirmation dès qu'un document est ouvert.
  onMount(() => {
    // Fermer la fenêtre : chaque document modifié propose d'être enregistré (Oui / Non / Annuler).
    // Annuler, à n'importe quelle étape, garde l'application ouverte.
    // Une fermeture normale ne laisse aucune copie de secours : elles ne servent qu'après une fermeture imprévue.
    const clearRecoveries = () => Promise.all(app.documents.map((doc) => api.recoveryDelete(doc.recoveryKey).catch(() => undefined)));
    const unlisten = getCurrentWindow().onCloseRequested(async (event) => {
      if (!app.documents.some((doc) => app.isModified(doc))) {
        event.preventDefault();
        await clearRecoveries();
        await getCurrentWindow().destroy();
        return;
      }
      event.preventDefault(); // la réponse est asynchrone : on ferme nous-mêmes une fois tout réglé
      for (const doc of [...app.documents]) {
        if (!(await settleUnsaved(doc))) return;
      }
      await clearRecoveries();
      await getCurrentWindow().destroy();
    });
    return () => void unlisten.then((stop) => stop());
  });

  // Un calque de texte ou de réglage qui devient actif (créé depuis l'onglet Réglages ou le menu Calque,
  // cliqué dans la liste des calques ou sur le plan de travail) ouvre l'onglet Propriétés : c'est là que
  // se règlent police et style, curseurs du réglage. Sans cela, créer un réglage laissait l'onglet Réglages
  // affiché et ses curseurs restaient invisibles.
  let lastPropertiesLayer: number | null = null;
  $effect(() => {
    const layer = app.activeLayer;
    const id = layer && (layer.text || layer.adjustment || layer.smart) ? layer.id : null;
    if (id !== null && id !== lastPropertiesLayer) app.leftTab = "properties";
    lastPropertiesLayer = id;
  });

  // ------------------------------------------------------ objets liés
  // Un fichier lié a pu changer pendant qu'on était dans un autre logiciel : on vérifie au retour dans la fenêtre,
  // et quand un document qui en contient devient actif (au plus une fois toutes les 2 s par document).
  $effect(() => {
    const id = app.activeId; // seul le changement de document compte, pas chaque modification du document
    untrack(() => id !== null && void checkSmartLinks(app.active));
  });

  // ------------------------------------------------------ dépôt de fichiers
  // Seuls les glissers venant de l'extérieur (fichiers, images d'un navigateur) passent par le glisser natif. Les
  // glissers internes (calques, masques) sont menés par l'application, sans lui : voir lib/layer-drag.svelte.ts.
  let fileHover = $state(false);
  /**
   * Alt tenu pendant le glisser : l'image déposée devient un objet dynamique (incorporé) plutôt qu'un calque de
   * pixels. Lu sur les événements du glisser de la page, et sur le clavier pour les dépôts reçus par la fenêtre.
   */
  let dropAsSmart = $state(false);
  // Un fichier venu de l'explorateur, ou une image glissée depuis un navigateur. Selon le navigateur,
  // celle-ci arrive comme fichier, ou seulement comme adresse (liste d'URI, ou balise <img> en HTML).
  const carriesFiles = (event: DragEvent) => event.dataTransfer?.types.includes("Files") ?? false;
  const carriesLink = (event: DragEvent) => (event.dataTransfer?.types.includes("text/uri-list") || event.dataTransfer?.types.includes("text/html")) ?? false;

  /**
   * Fin d'un glisser venu de l'extérieur. `dragleave` ou `drop` ne nous parviennent pas toujours (glisser abandonné
   * hors de la fenêtre) : le premier clic qui suit remet aussi l'indicateur de dépôt en ordre.
   */
  function endDragState(): void {
    fileHover = false;
    app.externalDrag = false;
    [app.layerDrag, app.maskDrag] = [null, null];
  }

  /**
   * Début d'un glisser dans la fenêtre. Les images, films, dessins et liens de l'interface (aide, bulles, vignettes de
   * l'accueil) sont glissables par défaut dans Chromium, qui les joint alors comme FICHIER ou comme adresse : Lumitype
   * les prenait pour une image venue de l'extérieur et l'importait. Plus rien ne se glisse nativement : les calques et
   * les masques du panneau Calques ont leur propre glisser, mené aux événements de pointeur (lib/layer-drag.svelte.ts).
   */
  function onDragStart(event: DragEvent): void {
    // Le texte sélectionné ne se glisse pas non plus (le nom d'une police dans son champ, par
    // exemple) : sous Linux, Chromium mène un tel glisser dans une boucle imbriquée qui bloque la fenêtre, et GNOME
    // finissait par afficher « Lumitype ne répond pas ». C'est aussi ce qui faisait « coller » un calque au pointeur :
    // aucun glisser natif ne naît plus dans la fenêtre.
    event.preventDefault();
  }

  function onDragOver(event: DragEvent): void {
    // Aucun glisser natif ne naît dans la fenêtre (voir onDragStart) : tout ce qui arrive ici vient de l'extérieur.
    if (!carriesFiles(event) && !carriesLink(event)) return;
    event.preventDefault();
    fileHover = true;
    dropAsSmart = event.altKey;
  }

  /** Chemins des fichiers locaux d'un glisser (adresses file:// de la liste text/uri-list), sous Linux comme sous Windows. */
  function droppedLocalPaths(data: DataTransfer): string[] {
    return data
      .getData("text/uri-list")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line.toLowerCase().startsWith("file://"))
      .map((uri) => {
        try {
          const path = decodeURIComponent(new URL(uri).pathname);
          return /^\/[A-Za-z]:\//.test(path) ? path.slice(1) : path; // file:///C:/… sous Windows
        } catch {
          return "";
        }
      })
      .filter((path) => path !== "");
  }

  /** Adresse de l'image d'un glisser venu d'un navigateur : la balise <img> d'abord, sinon le lien. */
  function droppedImageUrl(data: DataTransfer): string | null {
    const html = data.getData("text/html");
    const fromTag = /<img[^>]+src=["']([^"']+)["']/i.exec(html)?.[1]?.replaceAll("&amp;", "&");
    const candidate = fromTag ?? data.getData("text/uri-list").split("\n").find((line) => line.trim() !== "" && !line.startsWith("#"))?.trim();
    return candidate && /^https?:\/\//i.test(candidate) ? candidate : null;
  }

  function onDrop(event: DragEvent): void {
    const data = event.dataTransfer;
    app.externalDrag = false;
    if (!data || (!carriesFiles(event) && !carriesLink(event))) return;
    event.preventDefault();
    fileHover = false;
    lastPageDrop = performance.now();
    // Sous Linux, un fichier glissé depuis le gestionnaire de fichiers n'arrive souvent que par son adresse
    // file:// (la liste des fichiers reste vide, ou ne contient que des fichiers vides) : on lit alors le fichier
    // lui-même, par son chemin.
    const files = [...data.files].filter((file) => file.size > 0);
    const paths = droppedLocalPaths(data);
    const url = droppedImageUrl(data);
    const asSmart = event.altKey;
    dropAsSmart = false;
    if (files.length > 0) void importDroppedFiles(files, asSmart);
    else if (paths.length > 0) void importLocalPaths(paths, asSmart);
    else if (url) void importFromUrl(url).catch((error: unknown) => app.fail(error));
    else app.error = t("Rien d'exploitable dans ce qui a été déposé : glissez l'image elle-même, pas la page.");
  }

  /** Instant du dernier dépôt reçu par la page : le même glisser, s'il arrive aussi par la fenêtre, n'est pas importé deux fois. */
  let lastPageDrop = 0;

  /**
   * Dépôt reçu par la FENÊTRE (événements Tauri `tauri://drag-*`). Sous Linux avec Chromium, le bureau (Mutter) remet
   * le glisser à la fenêtre de premier niveau et non à la page : les fichiers d'un dossier et les images d'un site
   * arrivent par ici, en chemins locaux ou en adresses web (l'image d'abord, la page ensuite — voir
   * crates/winit-x11/PORTAGE-LUMITYPE.md). Avec la webview du système (Windows), ces événements sont coupés
   * (`dragDropEnabled: false`) et c'est le dépôt de la page, plus haut, qui sert.
   */
  function importWindowDrop(paths: readonly string[]): void {
    const web = paths.filter((path) => /^https?:\/\//i.test(path));
    const local = paths.filter((path) => !web.includes(path));
    if (local.length > 0) void importLocalPaths(local, dropAsSmart);
    else if (web[0]) void importFromUrl(web[0]).catch((error: unknown) => app.fail(error));
    else app.error = t("Rien d'exploitable dans ce qui a été déposé : glissez l'image elle-même, pas la page.");
  }

  onMount(() => {
    const unlisten = getCurrentWindow().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") {
        [fileHover, app.externalDrag] = [true, true];
        return;
      }
      [fileHover, app.externalDrag] = [false, false];
      if (payload.type !== "drop" || performance.now() - lastPageDrop < 1500) return void (dropAsSmart = false);
      importWindowDrop(payload.paths);
      dropAsSmart = false;
    });
    return () => void unlisten.then((stop) => stop());
  });

  /**
   * Ctrl+V. Une image dans le presse-papiers du système (capture d'écran, image copiée depuis un
   * navigateur ou un autre logiciel) est importée ; sinon c'est le collage interne du dernier copier.
   * On passe par l'événement « coller » : c'est le seul accès au presse-papiers qui ne demande aucune permission.
   */
  // Sous Linux, un clic du milieu colle la « sélection primaire » du système : c'est un geste de navigation
  // ici (déplacer la vue), pas un collage. On ignore donc tout collage qui suit de près un clic du milieu.
  let middleClickAt = 0;
  function onMiddle(event: MouseEvent): void {
    if (event.button !== 1) return;
    middleClickAt = performance.now();
    event.preventDefault(); // empêche aussi le collage lui-même, là où c'est possible
  }

  function onPaste(event: ClipboardEvent): void {
    if (performance.now() - middleClickAt < 1500) {
      event.preventDefault();
      return;
    }
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return; // collage de texte dans un champ
    event.preventDefault();
    // Le presse-papiers est lu nativement (pasteFromClipboard) : le plus récent, d'ici ou d'ailleurs, est collé.
    // Les images de l'événement ne servent qu'en dernier recours.
    const images = [...(event.clipboardData?.files ?? [])].filter((file) => file.type.startsWith("image/"));
    void pasteFromClipboard(images.map((file, i) => new File([file], file.name && file.name !== "image.png" ? file.name : `Image collée${i ? ` ${i + 1}` : ""}.png`, { type: file.type })));
  }

  function onKeyDown(event: KeyboardEvent): void {
    // Une fenêtre modale ouverte (filtre, export, préférences, effets…) garde ses frappes : les raccourcis de
    // l'application n'agissent pas derrière elle (Suppr n'y supprime pas de calque, un chiffre n'y règle pas
    // l'opacité, une lettre n'y change pas d'outil). Elle gère elle-même Entrée et Échap.
    if (document.querySelector("dialog[open]")) return;
    // Les panneaux flottants non modaux (Effets de calque, éditeur de dégradé, sélecteur de couleur, menu
    // du pinceau) sont des `div[role=dialog]` : l'image reste utilisable à la souris, mais le clavier leur
    // revient aussi. Sans ce filtre, Suppr y supprimait le calque et une lettre changeait d'outil pendant
    // le réglage. Seul l'affichage (zoom, règles, repères) reste au clavier, comme pendant Ctrl+T.
    if (floatingPanelOpen()) {
      const action = resolve(event, keymap.bindings);
      if (action?.startsWith("view.")) {
        event.preventDefault();
        if (!event.repeat || REPEATABLE.test(action)) runAction(action);
      }
      return;
    }
    const digit = transformIsModal() ? null : plainDigit(event);
    if (digit !== null) {
      event.preventDefault();
      setOpacityFromDigit(digit);
      return;
    }
    if (app.shortcutsDialogOpen) return; // l'écran des raccourcis capture lui-même les frappes
    const action = resolve(event, keymap.bindings);
    if (!action) return;
    // Pendant Ctrl+T, seuls l'affichage (zoom, règles, repères) reste accessible : le cadre gère Entrée et Échap.
    if (transformIsModal() && !action.startsWith("view.")) {
      event.preventDefault();
      return;
    }
    // Cadre de l'outil Déplacement dont une poignée a été tirée : faire autre chose le valide d'abord.
    if (transformIsPending() && !action.startsWith("view.") && action !== "edit.freeTransform") commitTransform();
    // Empêche aussi les raccourcis de la webview (Ctrl+R recharger, Ctrl++ zoom de page…).
    event.preventDefault();
    if (!event.repeat || REPEATABLE.test(action)) runAction(action);
  }
</script>

<svelte:window
  onmousedowncapture={onMiddle}
  onmouseupcapture={onMiddle}
  onauxclick={(event) => event.button === 1 && event.preventDefault()}
  ondragstart={onDragStart}
  ondragenter={(event) => (carriesFiles(event) || carriesLink(event)) && (app.externalDrag = true)}
  ondragend={endDragState}
  onkeydowncapture={(event) => event.key === "Alt" && fileHover && (dropAsSmart = true)}
  onkeyupcapture={(event) => event.key === "Alt" && (dropAsSmart = false)}
  onpointerdowncapture={endDragState} onkeydown={onKeyDown} onpaste={onPaste} onfocus={() => ((app.ownClipboard = false), void checkSmartLinks())} ondragover={onDragOver} ondrop={onDrop} ondragleave={(event) => event.relatedTarget === null && ((fileHover = false), (app.externalDrag = false))} />

<div class="app">
  <MenuBar />
  <!-- Écran d'accueil : sans document, ou par Fichier > Accueil. Il recouvre l'espace de travail, qui reste monté
       (l'affichage de l'image n'est pas recréé en revenant aux documents). -->
  {#if app.homeVisible}
    <div class="home-host"><HomeScreen dropping={fileHover} /></div>
  {/if}
  <OptionsBar />
  <div class="workspace">
    <Toolbar />
    <aside class="left">
      <ColorPanel />
      <Panel title={t("Propriétés et réglages")} grow tabs={LEFT_TABS} bind:tab={app.leftTab}>
        {#if app.leftTab === "adjustments"}
          <AdjustmentsPanel />
        {:else if app.leftTab === "history"}
          <HistoryPanel bare />
        {:else if brushInProperties}
          <!-- Pinceau ou gomme en main : Propriétés montre les pinceaux (formes, matières, dynamique, préréglages). -->
          <BrushPicker inline />
        {:else if app.activeLayer}
          {@const layer = app.activeLayer}
          <dl class="properties">
            <dt>{t("Calque")}</dt>
            <dd>{t(layer.name)}</dd>
            {#if !layer.smart}
              <!-- Un objet dynamique montre la position de son contenu, dans ses propres propriétés. -->
              <dt>{t("Position")}</dt>
              <dd>X {layer.offset[0]} px · Y {layer.offset[1]} px</dd>
            {/if}
          </dl>
          {#if layer.smart}
            {#key layer.id}
              <SmartProperties {layer} smart={layer.smart} />
            {/key}
          {/if}
          {#if layer.adjustment}
            {#key layer.id}
              <AdjustmentProperties {layer} adjustment={layer.adjustment} />
            {/key}
          {/if}
          {#if layer.shape}
            {#key layer.id}
              <ShapeProperties {layer} shape={layer.shape} />
            {/key}
          {/if}
          {#if layer.text}
            <!-- La clé recrée l'éditeur quand on change de calque : nouvelle séance d'édition, nouvel état d'historique. -->
            {#key layer.id}
              <TextProperties {layer} text={layer.text} />
            {/key}
          {/if}
        {:else}
          <p class="later">{t("Aucune propriété")}</p>
        {/if}
      </Panel>
    </aside>
    <main>
      <DocTabs />
      <Viewport />
    </main>
    <aside class="right">
      <LayersPanel />
    </aside>
  </div>
  <StatusBar />
</div>
{#if fileHover && !app.homeVisible}
  <div class="dropzone">
    <p>{app.active ? (dropAsSmart ? t("Déposer pour placer comme objet dynamique") : t("Déposer pour ajouter comme calque")) : t("Déposer pour ouvrir")}</p>
    {#if app.active}<small>{t("Alt : objet dynamique")}</small>{/if}
  </div>
{/if}
{#if app.sample}
  <!-- Pastille de la pipette : couleur prélevée, juste au-dessus du pointeur. -->
  <div class="sample" style:left="{app.sample.x}px" style:top="{app.sample.y}px" style:background={rgbToHex(app.sample.rgb)} aria-hidden="true"></div>
{/if}
<WindowEdges />
<SplashScreen />
<NewDocumentDialog />
<AboutDialog />
<!-- Travail en cours (enregistrement, ouverture, import…) : un filet animé tout en haut de la fenêtre. -->
<!-- Sauvegarde automatique : un filet qui avance, tout en haut ; le travail continue pendant ce temps. -->
<div class="savebar" class:on={app.autosaveProgress !== null} role="progressbar" aria-label={t("Sauvegarde automatique")} aria-valuenow={Math.round((app.autosaveProgress ?? 0) * 100)}>
  <div style:width="{Math.max(3, Math.round((app.autosaveProgress ?? 0) * 100))}%"></div>
</div>
<RecoveryDialog />
<div class="busybar" class:on={app.busy !== null} role="progressbar" aria-label={app.busy ?? t("Aucun travail en cours")} aria-hidden={app.busy === null}><div></div></div>
<SavePrompt />
<ExportProgress />
{#if app.brushMenu && app.paintTool}
  <BrushQuickMenu x={app.brushMenu.x} y={app.brushMenu.y} />
{/if}
{#if app.shapeDialog}
  <ShapeDialog />
{/if}
{#if app.modifySelectionOpen}
  <ModifySelectionDialog />
{/if}
<PreferencesDialog />
<CopyrightDialog />
{#if app.effectsLayer !== null}
  {@const target = app.active?.info.layers.find((l) => l.id === app.effectsLayer)}
  {#if target}
    {#key target.id}
      <EffectsDialog layer={target} />
    {/key}
  {/if}
{/if}
{#if app.guideLayoutOpen && app.active}
  <GuideLayoutDialog />
{/if}
{#if app.exportOpen}
  <ExportDialog />
{/if}
{#if app.liquifyOpen}
  <LiquifyDialog />
{/if}
{#if app.filterDialog}
  {#key app.filterDialog}
    <FilterDialog kind={app.filterDialog} />
  {/key}
{/if}
<ShortcutsDialog />
<FeatherDialog />
<ErrorToast />

<style>
  .savebar {
    position: fixed;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    z-index: 951;
    pointer-events: none;
    opacity: 0;
    transition: opacity 200ms ease;
  }
  .savebar.on {
    opacity: 1;
    background: rgba(255, 255, 255, 0.12);
  }
  .savebar div {
    height: 100%;
    background: linear-gradient(90deg, #fff, rgba(120, 140, 255, 0.95));
    transition: width 150ms linear;
  }
  .busybar {
    position: fixed;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    z-index: 950;
    overflow: hidden;
    opacity: 0;
    pointer-events: none;
    transition: opacity 150ms var(--ease) 150ms; /* un travail très court ne fait pas clignoter la barre */
  }
  .busybar.on {
    opacity: 1;
    background: rgba(255, 255, 255, 0.14);
  }
  .busybar div {
    width: 35%;
    height: 100%;
    background: linear-gradient(90deg, transparent, #fff, rgba(120, 140, 255, 0.95), transparent);
    animation: busy-sweep 1.2s linear infinite;
  }
  @keyframes busy-sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(290%);
    }
  }
  .app {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .home-host {
    position: absolute;
    top: 36px; /* sous la barre de menus */
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 40;
  }
  .workspace {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  aside {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-panel);
  }
  .left {
    width: 272px;
    border-right: 1px solid var(--border);
  }
  .right {
    width: 304px;
    border-left: 1px solid var(--border);
  }
  .sample {
    position: fixed;
    z-index: 60;
    width: 34px;
    height: 34px;
    margin: -52px 0 0 -17px;
    border-radius: 50%;
    border: 2px solid #fff;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.55),
      0 6px 18px rgba(0, 0, 0, 0.5);
    pointer-events: none;
  }
  .dropzone {
    position: fixed;
    inset: 8px;
    z-index: 50;
    display: grid;
    place-items: center;
    border-radius: var(--radius-l);
    border: 1.5px dashed var(--accent);
    background: rgba(12, 12, 13, 0.72);
    pointer-events: none;
  }
  .dropzone {
    align-content: center;
    gap: 6px;
  }
  .dropzone p {
    margin: 0;
    font-size: 15px;
  }
  .dropzone small {
    color: var(--text-muted);
    font-size: 12px;
  }
  .properties {
    margin: 0;
    padding: 14px;
    display: grid;
    grid-template-columns: 72px 1fr;
    gap: 8px 10px;
  }
  .properties dt {
    color: var(--text-3);
  }
  .properties dd {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .later {
    margin: 0;
    padding: 14px;
    color: var(--text-muted);
  }
</style>
