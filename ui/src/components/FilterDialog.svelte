<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { describeError } from "../lib/errors";
  import { open } from "@tauri-apps/plugin-dialog";
  import { untrack } from "svelte";
  import { edit } from "../lib/engine";
  import { CATEGORIES, FALLBACK_SPEC, FILTERS, NEW_FILTERS, type GalleryKind, defaultsOf, engineValues, previewOf, specOf, swatchOf, valuesOf } from "../lib/filters";
  import { replaceFilter } from "../lib/smart-filters";
  import { favorites, recentFilters, recordFilter, rememberApplied, toggleFavorite } from "../lib/filter-memory";
  import { runAction } from "../lib/actions";
  import { type BlurPin, type Filter, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Galerie de filtres : à gauche l'aperçu du calque filtré, au centre les filtres rangés par catégorie,
  // à droite les réglages du filtre choisi. L'aperçu est calculé par le moteur sur une réduction du calque :
  // il suit les curseurs sans attendre, quelle que soit la taille de l'image, et ne touche pas au document.
  // L'image elle-même n'est filtrée qu'à la validation. Changer de filtre garde les réglages des autres.
  let { kind: kindProp }: { kind: Exclude<Filter["type"], "liquify"> } = $props();

  const PREVIEW_SIDE = 760;
  const MAX_FIELD_BLUR = 100;
  const docId = untrack(() => app.activeId);
  const layerId = untrack(() => app.activeLayer?.id ?? null);

  // Le filtre est figé à l'ouverture : la propriété vient de l'état global, qui repasse à « aucun filtre »
  // au moment même où l'on valide (la lire alors envoyait au moteur un filtre sans type).
  let kind = $state<Exclude<GalleryKind, "liquify">>(untrack(() => (specOf(kindProp) ? kindProp : "gaussianBlur")));
  const spec = $derived(specOf(kind) ?? FALLBACK_SPEC);
  /** Réglages de chaque filtre visité : y revenir les retrouve. */
  /**
   * Filtre dynamique rouvert (double-clic dans le panneau Calques) : ses réglages au départ, et « Appliquer » le
   * remplace à sa place dans la liste du calque.
   */
  const editing = untrack(() => {
    const target = app.filterEdit;
    const layer = target && app.active?.info.layers.find((l) => l.id === target.layerId);
    const item = target && layer?.filters?.list[target.index];
    return target && layer && item ? { layer: $state.snapshot(layer), index: target.index, filter: $state.snapshot(item.filter) as Filter } : null;
  });
  let settings = $state<Record<string, Record<string, number | boolean | string>>>(
    untrack(() => ({ [kind]: editing && editing.filter.type === kind ? valuesOf(specOf(kind) ?? FALLBACK_SPEC, editing.filter) : defaultsOf(specOf(kind) ?? FALLBACK_SPEC) })),
  );
  const values = $derived(settings[kind] ?? {});

  /** Catégorie affichée : « all », « favorites », « recent » ou l'identifiant d'une catégorie. La galerie s'ouvre
   *  toujours sur « Tous », le filtre demandé (menu, Ctrl+F) restant sélectionné. */
  let category = $state("all");
  /**
   * La Fluidité ouvre son propre module (pinceau de déformation) : elle reste dans le menu Filtre mais pas dans la
   * galerie, où rien ne s'y règle.
   */
  const inGallery = (types: readonly GalleryKind[]): GalleryKind[] => types.filter((type) => type !== "liquify");
  const GALLERY_COUNT = FILTERS.filter((f) => f.type !== "liquify").length;
  let search = $state("");
  let favoriteList = $state<GalleryKind[]>(favorites());
  const recentList = recentFilters();
  /** Interrupteur « Activé » : éteint, l'aperçu montre le calque d'origine. */
  let enabled = $state(true);

  let dialog = $state<HTMLDialogElement>();
  let canvas = $state<HTMLCanvasElement>();

  function choose(next: GalleryKind): void {
    if (next === "liquify") {
      // La Fluidité se peint au pinceau, dans sa propre fenêtre.
      app.filterDialog = null;
      runAction("filter.liquify");
      return;
    }
    const target = specOf(next);
    if (!target) return;
    settings[next] ??= defaultsOf(target);
    kind = next;
    failure = null;
    enabled = true;
  }

  /** Remet les réglages du filtre courant à leurs valeurs par défaut. */
  function reset(): void {
    settings[kind] = defaultsOf(spec);
  }

  function set(key: string, value: number | boolean | string): void {
    settings[kind] = { ...settings[kind], [key]: value };
  }

  /** Filtres montrés au centre, groupés : une catégorie, ou toutes (vue « Tous », recherche). */
  const groups = $derived.by((): { label: string; types: GalleryKind[] }[] => {
    const query = search.trim().toLocaleLowerCase();
    if (query) {
      const found = FILTERS.filter((f) => t(f.label).toLocaleLowerCase().includes(query) || f.label.toLocaleLowerCase().includes(query)).map((f) => f.type);
      return [{ label: t("Résultats"), types: inGallery(found) }];
    }
    if (category === "all") return CATEGORIES.map((c) => ({ label: t(c.label), types: inGallery(c.types) }));
    if (category === "favorites") return [{ label: t("Favoris"), types: inGallery(favoriteList) }];
    if (category === "recent") return [{ label: t("Récents"), types: inGallery(recentList) }];
    const found = CATEGORIES.find((c) => c.id === category);
    return found ? [{ label: t(found.label), types: inGallery(found.types) }] : [];
  });
  const compact = $derived(search.trim() === "" && category === "all");

  // Filtre Verre : texture chargée par l'utilisateur (une image dont la clarté sert de relief).
  let loadedTexture = $state<string | null>(null);
  async function loadTexture(): Promise<void> {
    try {
      const path = await open({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"] }] });
      if (typeof path !== "string") return;
      const [name, width, height] = await api.loadGlassTexture(path);
      loadedTexture = `${name} · ${width} × ${height} px`;
      set("texture", "custom");
      // Même réglage, nouvelle texture : l'aperçu doit être refait.
      textureSerial += 1;
    } catch (error) {
      failure = describeError(error);
    }
  }
  let textureSerial = $state(0);
  // Flou de profondeur de champ : une pastille au centre pour commencer, comme dans Photoshop.
  let pins = $state<BlurPin[]>(untrack(() => (editing?.filter.type === "fieldBlur" ? structuredClone(editing.filter.pins) : [{ x: 0.5, y: 0.5, blur: 15 }])));
  let selectedPin = $state(0);
  let draggingPin = $state(false);
  /** Réglage du flou en tournant la bague de la pastille. */
  let turningRing = $state(false);
  /** Flou radial : le centre suit le pointeur tant que le bouton est enfoncé. */
  let draggingCenter = $state(false);
  let failure = $state<string | null>(null);
  let size = $state({ width: 3, height: 2 });
  /** Place disponible pour l'aperçu ; le cadre y tient en entier, aux proportions du calque. */
  let room = $state({ width: 0, height: 0 });
  const frame = $derived.by(() => {
    const [w, h] = [Math.max(0, room.width - 36), Math.max(0, room.height - 36)];
    const k = Math.min(w / size.width, h / size.height);
    return { width: Math.floor(size.width * k), height: Math.floor(size.height * k) };
  });

  const filter = $derived((kind === "fieldBlur" ? { type: kind, pins } : { type: kind, ...engineValues(spec, values) }) as Filter);
  /** Un centre à placer dans l'aperçu : flou radial, cercles de la demi-teinte. */
  const hasCenter = $derived(kind === "radialBlur" || (kind === "halftone" && values.pattern === "circles"));
  /** Filtre neutre : l'aperçu du calque tel qu'il est (interrupteur éteint). */
  const IDENTITY: Filter = { type: "vignette", amount: 0, size: 0, softness: 0 };

  $effect(() => {
    dialog?.showModal();
  });

  // ---- Zoom et déplacement dans l'aperçu : molette pour zoomer sous le pointeur, bouton du milieu (ou
  // Espace + glisser) pour se déplacer. En zoomant, l'aperçu est recalculé plus finement. « Ajuster » recadre.
  let view = $state({ zoom: 1, x: 0, y: 0 });
  let panning: { x: number; y: number } | null = null;
  let spaceHeld = $state(false);
  /**
   * Finesse demandée au moteur : la taille RÉELLE du cadre à l'écran (pixels physiques) multipliée par le zoom de
   * l'aperçu, arrondie par paliers de 256 px pour ne pas tout recalculer au moindre changement. Plafond : 4 096 px
   * (et jamais plus que le calque lui-même, côté moteur), ramené à 2 048 px pour le flou de profondeur de champ,
   * dont chaque aperçu floute l'image sept fois.
   */
  const previewSide = $derived.by(() => {
    const shown = Math.max(frame.width, frame.height, 1) * (window.devicePixelRatio || 1) * view.zoom;
    const ceiling = kind === "fieldBlur" ? 2048 : 4096;
    return Math.min(ceiling, Math.max(PREVIEW_SIDE, Math.ceil(shown / 256) * 256));
  });
  let loading = $state(false);

  function zoomAt(factor: number, px: number, py: number): void {
    const zoom = Math.min(8, Math.max(1, view.zoom * factor));
    const k = zoom / view.zoom;
    // Le point sous le pointeur reste sous le pointeur ; à 100 % l'aperçu se recentre.
    view = zoom === 1 ? { zoom: 1, x: 0, y: 0 } : { zoom, x: px - (px - view.x) * k, y: py - (py - view.y) * k };
  }

  function onStageWheel(event: WheelEvent): void {
    event.preventDefault();
    // Flou à pastilles : Alt + molette règle le flou de la pastille sélectionnée ; la molette seule zoome, comme partout.
    if (kind === "fieldBlur" && event.altKey) return onWheel(event);
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    zoomAt(Math.exp(-event.deltaY * 0.0015), event.clientX - rect.left - rect.width / 2, event.clientY - rect.top - rect.height / 2);
  }

  function onStageDown(event: PointerEvent): void {
    if (event.button !== 1 && !(event.button === 0 && spaceHeld)) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    panning = { x: event.clientX, y: event.clientY };
  }

  function onStageMove(event: PointerEvent): void {
    if (panning) {
      view = { ...view, x: view.x + event.clientX - panning.x, y: view.y + event.clientY - panning.y };
      panning = { x: event.clientX, y: event.clientY };
      return;
    }
    if (draggingCenter) {
      const at = pinPosition(event);
      settings[kind] = { ...settings[kind], x: at.x, y: at.y };
      return;
    }
    dragPin(event);
  }

  function release(): void {
    draggingPin = false;
    turningRing = false;
    draggingCenter = false;
    panning = null;
  }

  // Aperçu : **un seul calcul à la fois**. Un réglage modifié pendant un calcul ne fait que **remplacer la
  // demande en attente** : c'est la dernière qui part (plusieurs calculs simultanés d'un gros flou épuisaient
  // la mémoire).
  type PreviewJob = { filter: Filter; side: number; target: HTMLCanvasElement };
  let queued: PreviewJob | null = null;
  let running = false;

  async function pumpPreview(): Promise<void> {
    if (running || docId === null || layerId === null) return;
    running = true;
    loading = true;
    while (queued) {
      const job = queued;
      queued = null;
      try {
        const buffer = await api.filterPreview(docId, layerId, job.filter, job.side, false, editing?.index ?? null);
        const header = new DataView(buffer);
        const [width, height] = [header.getUint32(0, true), header.getUint32(4, true)];
        job.target.width = width;
        job.target.height = height;
        job.target.getContext("2d")?.putImageData(new ImageData(new Uint8ClampedArray(buffer, 8), width, height), 0, 0);
        [size, failure] = [{ width, height }, null];
      } catch (error: unknown) {
        failure = describeError(error);
      }
    }
    [running, loading] = [false, false];
  }

  $effect(() => {
    const current = enabled ? ($state.snapshot(filter) as Filter) : IDENTITY;
    void textureSerial; // une nouvelle texture chargée relance l'aperçu, même à réglages identiques
    const target = canvas;
    const side = previewSide;
    if (docId === null || layerId === null || !target) return;
    queued = { filter: current, side, target };
    void pumpPreview();
  });

  function close(apply: boolean): void {
    // Le filtre est relevé AVANT de fermer la fenêtre (voir plus haut).
    const chosen = $state.snapshot(filter) as Filter;
    app.filterDialog = null;
    app.filterEdit = null;
    if (apply && enabled && editing) {
      recordFilter(kind);
      replaceFilter(editing.layer, editing.index, chosen);
      return;
    }
    if (apply && enabled && layerId !== null) {
      recordFilter(kind);
      rememberApplied(chosen);
      void edit({ op: "applyFilter", id: layerId, filter: chosen });
    }
  }

  const format = (value: number, step: number) => (step < 1 ? value.toFixed(1) : String(Math.round(value)));

  // ---- Pastilles du flou de profondeur de champ, centre du flou radial

  function pinPosition(event: PointerEvent): { x: number; y: number } {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return { x: 0.5, y: 0.5 };
    return { x: Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)), y: Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height)) };
  }

  function onCanvasDown(event: PointerEvent): void {
    if (event.button !== 0) return;
    if (hasCenter) {
      const at = pinPosition(event);
      settings[kind] = { ...settings[kind], x: at.x, y: at.y };
      draggingCenter = true;
      (event.currentTarget as Element).closest(".stage")?.setPointerCapture(event.pointerId);
      return;
    }
    if (kind !== "fieldBlur") return;
    // La nouvelle pastille reprend la force de la pastille sélectionnée : on ajuste ensuite.
    pins.push({ ...pinPosition(event), blur: pins[selectedPin]?.blur ?? 15 });
    selectedPin = pins.length - 1;
    startDrag(event);
  }

  function startDrag(event: PointerEvent): void {
    draggingPin = true;
    (event.currentTarget as Element).closest(".stage")?.setPointerCapture(event.pointerId);
  }

  function dragPin(event: PointerEvent): void {
    const pin = pins[selectedPin];
    if (!pin || kind !== "fieldBlur") return;
    if (turningRing) pin.blur = blurFromAngle(event, pin);
    else if (draggingPin) Object.assign(pin, pinPosition(event));
  }

  /**
   * Comme dans Photoshop : la bague se tourne. L'angle du pointeur autour de la pastille, compté
   * dans le sens horaire depuis le haut, donne la force du flou (un tour complet = le maximum).
   */
  function blurFromAngle(event: PointerEvent, pin: BlurPin): number {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return pin.blur;
    const [cx, cy] = [rect.left + pin.x * rect.width, rect.top + pin.y * rect.height];
    const turn = (Math.atan2(event.clientX - cx, cy - event.clientY) / (2 * Math.PI) + 1) % 1;
    // Près du haut, l'angle saute de 100 % à 0 % : on reste du côté où l'on était.
    const previous = pin.blur / MAX_FIELD_BLUR;
    const held = Math.abs(turn - previous) > 0.6 ? (previous > 0.5 ? 1 : 0) : turn;
    return Math.round(held * MAX_FIELD_BLUR);
  }

  /** Alt + molette au-dessus de l'aperçu : règle le flou de la pastille sélectionnée, par pas de 2. */
  function onWheel(event: WheelEvent): void {
    const pin = pins[selectedPin];
    if (kind !== "fieldBlur" || !pin) return;
    event.preventDefault();
    pin.blur = Math.min(MAX_FIELD_BLUR, Math.max(0, pin.blur - Math.sign(event.deltaY) * 2));
  }

  function removePin(): void {
    if (pins.length <= 1) return;
    pins.splice(selectedPin, 1);
    selectedPin = Math.max(0, selectedPin - 1);
  }

  /** Arc de la bague : sa longueur montre la force du flou de la pastille. */
  const ring = (blur: number) => {
    const circumference = 2 * Math.PI * 22;
    return `${(Math.min(blur, MAX_FIELD_BLUR) / MAX_FIELD_BLUR) * circumference} ${circumference}`;
  };

  /** Pictogrammes du rail (traits simples, 16 x 16). */
  const ICONS: Record<string, string> = {
    all: '<rect x="2" y="2" width="5" height="5"/><rect x="9" y="2" width="5" height="5"/><rect x="2" y="9" width="5" height="5"/><rect x="9" y="9" width="5" height="5"/>',
    favorites: '<path d="M8 2l1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3 4.3 13.3l.8-4.1L2 6.3l4.2-.5z"/>',
    recent: '<circle cx="8" cy="8" r="6"/><path d="M8 4.5V8l2.5 1.5"/>',
    blur: '<circle cx="8" cy="8" r="5.5" stroke-dasharray="2 2"/>',
    sharpen: '<path d="M8 2l5 12H3z"/>',
    grain: '<circle cx="4" cy="4" r=".8"/><circle cx="10" cy="5" r=".8"/><circle cx="6" cy="9" r=".8"/><circle cx="12" cy="11" r=".8"/><circle cx="4" cy="13" r=".8"/><circle cx="9" cy="13" r=".8"/>',
    distort: '<path d="M2 5c3-3 5 3 8 0s4 0 4 0M2 11c3-3 5 3 8 0s4 0 4 0"/>',
    light: '<circle cx="8" cy="8" r="3"/><path d="M8 1v2M8 13v2M1 8h2M13 8h2M3 3l1.4 1.4M11.6 11.6L13 13M3 13l1.4-1.4M11.6 4.4L13 3"/>',
    print: '<circle cx="4" cy="4" r="1.6"/><circle cx="10" cy="4" r="1.2"/><circle cx="4" cy="10" r="1.2"/><circle cx="10" cy="10" r=".8"/><circle cx="14" cy="7" r=".8"/>',
    retro: '<rect x="2" y="3" width="12" height="9"/><path d="M5 14h6M2 6h12M2 9h12"/>',
    glitch: '<path d="M2 4h7M5 7h9M2 10h5M8 13h6"/>',
    art: '<path d="M2 8a6 6 0 0 1 6-6M8 14a6 6 0 0 1 6-6M2 14a6 6 0 0 1 6-6M8 8a6 6 0 0 1 6-6"/>',
  };
  const RAIL = $derived([
    { id: "all", label: t("Tous"), count: GALLERY_COUNT },
    { id: "favorites", label: t("Favoris"), count: favoriteList.length },
    { id: "recent", label: t("Récents"), count: recentList.length },
  ]);
</script>

<svelte:window
  onkeydown={(event) => event.code === "Space" && !(event.target instanceof HTMLInputElement) && ((spaceHeld = true), event.preventDefault())}
  onkeyup={(event) => event.code === "Space" && (spaceHeld = false)}
/>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <header>
      <h2>{t("Galerie de filtres")}</h2>
    </header>

    <div class="body">
      <!-- Aperçu -->
      <section class="preview">
        <div
          class="stage"
          class:pins={kind === "fieldBlur" || hasCenter}
          class:grab={spaceHeld}
          role="presentation"
          bind:clientWidth={room.width}
          bind:clientHeight={room.height}
          onpointerdowncapture={onStageDown}
          onpointermove={onStageMove}
          onpointerup={release}
          onpointercancel={release}
          onwheel={onStageWheel}
        >
          <!-- Barre de chargement discrète, la même que celle de la bannière de lancement. -->
          <div class="loading" class:on={loading} aria-hidden="true"><div></div></div>
          <div class="frame" style:width="{frame.width * view.zoom}px" style:height="{frame.height * view.zoom}px" style:transform="translate({view.x}px, {view.y}px)">
            <canvas bind:this={canvas} onpointerdown={onCanvasDown}></canvas>
            {#if hasCenter && enabled}
              <span class="center" style:left="{Number(values.x) * 100}%" style:top="{Number(values.y) * 100}%" aria-hidden="true"></span>
            {/if}
            {#if kind === "fieldBlur"}
              {#each pins as pin, index (index)}
                <button
                  type="button"
                  class="pin"
                  draggable="false"
                  ondragstart={(event) => event.preventDefault()}
                  class:on={index === selectedPin}
                  style:left="{pin.x * 100}%"
                  style:top="{pin.y * 100}%"
                  aria-label="Pastille {index + 1}, flou {Math.round(pin.blur)} pixels"
                  aria-pressed={index === selectedPin}
                  onpointerdown={(event) => {
                    event.stopPropagation();
                    selectedPin = index;
                    startDrag(event);
                  }}
                  onkeydown={(event) => (event.key === "Delete" || event.key === "Backspace") && removePin()}
                >
                  {#if index === selectedPin}
                    <!-- Bague : on la saisit et on tourne pour régler le flou. -->
                    <svg viewBox="0 0 56 56" aria-hidden="true">
                      <circle
                        cx="28"
                        cy="28"
                        r="22"
                        class="ring-back"
                        role="presentation"
                        onpointerdown={(event) => {
                          event.stopPropagation();
                          turningRing = true;
                          (event.currentTarget as Element).closest(".stage")?.setPointerCapture(event.pointerId);
                        }}
                      />
                      <circle cx="28" cy="28" r="22" class="ring" stroke-dasharray={ring(pin.blur)} />
                    </svg>
                    <span class="badge">Flou : {Math.round(pin.blur)}</span>
                  {/if}
                </button>
              {/each}
            {/if}
          </div>
          {#if failure}<p class="failure">{failure}</p>{/if}
        </div>
        <div class="controls">
          <button type="button" class="ghost small" onclick={() => (view = { zoom: 1, x: 0, y: 0 })}>{t("Ajuster")}</button>
          <button type="button" class="ghost small switch" class:off={!enabled} role="switch" aria-checked={enabled} onclick={() => (enabled = !enabled)}>
            <i aria-hidden="true"></i>{enabled ? t("Activé") : t("Désactivé")}
          </button>
        </div>
      </section>

      <!-- Rail des catégories -->
      <nav class="rail" aria-label={t("Catégories de filtres")}>
        <input class="search" type="search" placeholder={t("Rechercher…")} bind:value={search} />
        {#each RAIL as item (item.id)}
          <button type="button" class="ri" class:on={category === item.id && !search} title={item.label} aria-label={item.label} onclick={() => ((category = item.id), (search = ""))}>
            <svg viewBox="0 0 16 16" aria-hidden="true">{@html ICONS[item.id]}</svg><span>{item.label}</span><em>{item.count}</em>
          </button>
        {/each}
        <hr />
        {#each CATEGORIES as item (item.id)}
          <button type="button" class="ri" class:on={category === item.id && !search} title={t(item.label)} aria-label={t(item.label)} onclick={() => ((category = item.id), (search = ""))}>
            <svg viewBox="0 0 16 16" aria-hidden="true">{@html ICONS[item.id]}</svg><span>{t(item.label)}</span><em>{inGallery(item.types).length}</em>
          </button>
        {/each}
      </nav>

      <!-- Vignettes -->
      <div class="gallery">
        {#each groups as group (group.label)}
          <h3>{group.label}{#if compact}<em>{group.types.length}</em>{/if}</h3>
          {#if group.types.length === 0}
            <p class="empty">{category === "favorites" ? t("Clic droit sur un filtre pour l'ajouter aux favoris.") : t("Aucun filtre.")}</p>
          {/if}
          <div class="tiles" class:compact>
            {#each group.types as type (type)}
              {@const item = specOf(type)}
              {#if item}
                <button
                  type="button"
                  class="tile"
                  class:on={type === kind}
                  title={t(item.hint)}
                  onclick={() => choose(type)}
                  oncontextmenu={(event) => (event.preventDefault(), (favoriteList = toggleFavorite(type)))}
                >
                  <img src={previewOf(type)} alt="" draggable="false" />
                  {#if favoriteList.includes(type)}<i class="fav" aria-label={t("Favori")}>★</i>{/if}
                  {#if NEW_FILTERS.has(type)}<i class="new" aria-label={t("Nouveau")}></i>{/if}
                  <span>{t(item.label)}</span>
                </button>
              {/if}
            {/each}
          </div>
        {/each}
      </div>

      <!-- Réglages -->
      <aside>
        <div class="title">
          <h4>{t(spec.label)}</h4>
          <button type="button" class="link" onclick={reset}>{t("Réinitialiser")}</button>
        </div>
        <p class="hint">{t(spec.hint)}</p>
        {#if kind === "fieldBlur"}
          {@const pin = pins[selectedPin]}
          {#if pin}
            <label>
              <span>{t("Flou de la pastille")}</span>
              <output>{Math.round(pin.blur)} px</output>
              <input type="range" min="0" max={MAX_FIELD_BLUR} step="1" value={pin.blur} oninput={(event) => (pin.blur = event.currentTarget.valueAsNumber)} />
            </label>
          {/if}
          <button type="button" class="ghost small" disabled={pins.length <= 1} onclick={removePin}>{t("Supprimer la pastille")}</button>
          <p>{t("Cliquez dans l'aperçu pour poser une pastille, glissez-la pour la déplacer. Tournez sa bague (ou Alt + molette) pour régler le flou. Une seule pastille floute toute l'image ; avec plusieurs, le flou varie progressivement de l'une à l'autre. Une pastille à 0 garde sa zone nette.")}</p>
        {/if}
        {#each spec.choices ?? [] as option (option.key)}
          {#if option.display === "swatches"}
            <div class="field">
              <span>{t(option.label)}</span>
              <div class="swatches" style:grid-template-columns={option.options.length <= 4 ? "repeat(4, 1fr)" : null}>
                {#each option.options as entry (entry.value)}
                  <button type="button" class="swatch" class:on={values[option.key] === entry.value} title={t(entry.label)} aria-label={t(entry.label)} onclick={() => set(option.key, entry.value)}>
                    <img src={swatchOf(kind, entry.value)} alt="" draggable="false" />
                  </button>
                {/each}
              </div>
              <em class="picked">{t(option.options.find((o) => o.value === values[option.key])?.label ?? "")}</em>
            </div>
          {:else if option.display === "segmented"}
            <div class="field">
              <span>{t(option.label)}</span>
              <div class="segmented" role="radiogroup" aria-label={t(option.label)}>
                {#each option.options as entry (entry.value)}
                  <button type="button" role="radio" aria-checked={values[option.key] === entry.value} class:on={values[option.key] === entry.value} onclick={() => set(option.key, entry.value)}>{t(entry.label)}</button>
                {/each}
              </div>
            </div>
          {:else}
            <label class="choice">
              <span>{t(option.label)}</span>
              <select value={values[option.key]} onchange={(event) => set(option.key, event.currentTarget.value)}>
                {#each option.options as entry (entry.value)}
                  <option value={entry.value}>{t(entry.label)}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/each}
        {#each spec.colors ?? [] as item (item.key)}
          <label class="color">
            <span>{t(item.label)}</span>
            <input type="color" value={String(values[item.key] ?? item.value)} oninput={(event) => set(item.key, event.currentTarget.value)} />
          </label>
        {/each}
        {#if kind === "halftone" && values.pattern === "circles"}
          <p>{t("Cliquez dans l'aperçu pour placer le centre des cercles.")}</p>
        {/if}
        {#each spec.params as item (item.key)}
          <label>
            <span>{t(item.label)}</span>
            <output>{format(Number(values[item.key]), item.step)}{item.unit}</output>
            <input type="range" min={item.min} max={item.max} step={item.step} value={Number(values[item.key])} oninput={(event) => set(item.key, event.currentTarget.valueAsNumber)} />
          </label>
        {/each}
        {#each spec.toggles as item (item.key)}
          <label class="check"><input type="checkbox" checked={values[item.key] === true} onchange={(event) => set(item.key, event.currentTarget.checked)} /> {t(item.label)}</label>
        {/each}
        {#if kind === "glass"}
          <button type="button" class="ghost small" onclick={loadTexture}>{t("Charger une texture…")}</button>
          <p>{loadedTexture ?? t("Toute image convient : ses zones claires forment les bosses du verre, ses zones sombres les creux. Elle est répétée pour couvrir l'image.")}</p>
        {/if}
        {#if spec.seeded}
          <button type="button" class="ghost small" onclick={() => set("seed", (Number(values.seed) || 0) + 1)}>⟳ {t("Variante")}</button>
        {/if}
        {#if app.activeLayer?.shape || app.activeLayer?.text}
          <p class="notice">{app.activeLayer.shape ? t("Ce calque est une forme : il sera pixellisé pour recevoir le filtre, et ne sera plus réglable ensuite. Ctrl+Z annule le tout.") : t("Ce calque est un texte : il sera pixellisé pour recevoir le filtre, et ne sera plus réglable ensuite. Ctrl+Z annule le tout.")}</p>
        {/if}
        <div class="spacer"></div>
        <p>{app.active?.info.hasSelection ? t("Sera appliqué à la zone sélectionnée du calque actif.") : t("Sera appliqué à tout le calque actif.")}</p>
      </aside>
    </div>

    <footer>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!enabled}>{t("Appliquer")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: calc(100vw - 64px);
    max-width: 1680px;
    height: calc(100vh - 64px);
    max-height: 1040px;
    margin: auto;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  form {
    display: grid;
    grid-template-rows: 50px minmax(0, 1fr) 56px;
    height: 100%;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 20px;
  }
  header {
    border-bottom: 1px solid var(--border);
  }
  footer {
    justify-content: flex-end;
    gap: 8px;
    border-top: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 168px minmax(250px, 330px) 280px;
    min-height: 0;
  }

  /* ---- Aperçu */
  .preview {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--bg-canvas);
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    padding: 18px;
    overflow: hidden;
    touch-action: none;
  }
  .stage.grab {
    cursor: grab;
  }
  .controls {
    display: flex;
    gap: 8px;
    padding: 0 18px 14px;
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .switch i {
    position: relative;
    width: 26px;
    height: 14px;
    border-radius: 7px;
    background: var(--accent);
  }
  .switch i::after {
    content: "";
    position: absolute;
    right: 2px;
    top: 2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #fff;
    transition: right 120ms var(--ease);
  }
  .switch.off i {
    background: var(--border-strong);
  }
  .switch.off i::after {
    right: 14px;
  }
  /* Le cadre épouse les proportions du calque et tient dans la zone d'aperçu. */
  .frame {
    position: relative;
    flex-shrink: 0;
    background: repeating-conic-gradient(#d0d0d0 0% 25%, #ffffff 0% 50%) 0 0 / 16px 16px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .stage.pins canvas {
    cursor: crosshair;
  }
  .center {
    position: absolute;
    width: 18px;
    height: 18px;
    margin: -9px 0 0 -9px;
    border-radius: 50%;
    border: 2px solid #fff;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.6),
      inset 0 0 0 1px rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  .pin {
    position: absolute;
    width: 16px;
    height: 16px;
    margin: -8px 0 0 -8px;
    padding: 0;
    border-radius: 50%;
    background: #fff;
    border: 2px solid var(--accent);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.5);
    cursor: grab;
    -webkit-user-drag: none;
    user-select: none;
  }
  .pin.on {
    background: var(--accent);
    border-color: #fff;
  }
  .pin svg {
    position: absolute;
    left: -22px;
    top: -22px;
    width: 56px;
    height: 56px;
    transform: rotate(-90deg);
    fill: none;
    pointer-events: none;
    overflow: visible;
  }
  /* Piste de la bague : large pour être saisie facilement ; seul son trait réagit, pas son centre. */
  .ring-back {
    stroke: rgba(0, 0, 0, 0.45);
    stroke-width: 9;
    pointer-events: stroke;
    cursor: ew-resize;
  }
  .ring {
    stroke: #fff;
    stroke-width: 3.5;
    stroke-linecap: round;
    pointer-events: none;
  }
  .badge {
    position: absolute;
    left: 50%;
    bottom: 42px;
    transform: translateX(-50%);
    padding: 3px 8px;
    border-radius: var(--radius-s);
    background: rgba(0, 0, 0, 0.72);
    color: var(--text-strong);
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
  .loading {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    overflow: hidden;
    opacity: 0;
    transition: opacity 150ms var(--ease) 120ms; /* les calculs très courts ne font pas clignoter la barre */
    z-index: 2;
  }
  .loading.on {
    opacity: 1;
    background: rgb(var(--ink) / 0.14);
  }
  .loading div {
    width: 40%;
    height: 100%;
    background: linear-gradient(90deg, transparent, #fff, rgba(120, 140, 255, 0.95), transparent);
    animation: sweep 1.1s linear infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
  .failure {
    position: absolute;
    left: 18px;
    right: 18px;
    bottom: 10px;
    margin: 0;
    color: var(--danger, #ff8080);
  }

  /* ---- Rail */
  .rail {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 12px 8px;
    border-left: 1px solid var(--border);
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .search {
    height: 28px;
    margin-bottom: 8px;
    padding: 0 9px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  .ri {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    color: var(--text-2);
    text-align: left;
  }
  .ri:hover {
    background: var(--bg-hover);
  }
  .ri svg {
    width: 15px;
    height: 15px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.3;
  }
  .ri span {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ri em {
    font-style: normal;
    color: var(--text-muted);
    font-size: 11px;
  }
  .ri.on {
    background: var(--bg-hover);
    color: var(--text);
    box-shadow: inset 2px 0 var(--accent);
  }
  .rail hr {
    width: 100%;
    border: 0;
    border-top: 1px solid var(--border);
    margin: 6px 0;
  }

  /* ---- Vignettes */
  .gallery {
    padding: 12px 16px;
    overflow-y: auto;
  }
  .gallery h3 {
    position: sticky;
    top: -12px;
    z-index: 1;
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0 -16px 8px;
    padding: 6px 16px;
    background: var(--bg-panel);
    font-size: 12.5px;
    font-weight: 600;
  }
  .gallery h3 em {
    font-style: normal;
    font-weight: 400;
    font-size: 11px;
    color: var(--text-muted);
  }
  .empty {
    margin: 0 0 12px;
    color: var(--text-muted);
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(124px, 1fr));
    gap: 14px 10px;
    margin-bottom: 14px;
  }
  .tiles.compact {
    grid-template-columns: repeat(auto-fill, minmax(78px, 1fr));
    gap: 8px 8px;
  }
  .tile {
    position: relative;
    display: block;
    padding: 0;
    text-align: left;
    color: var(--text-2);
  }
  .tile img {
    display: block;
    width: 100%;
    aspect-ratio: 3 / 2;
    object-fit: cover;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .tile span {
    display: block;
    margin-top: 4px;
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tiles.compact .tile span {
    font-size: 10px;
    margin-top: 2px;
  }
  .tile:hover img {
    border-color: var(--border-strong);
  }
  .tile.on img {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .tile.on span {
    color: var(--text);
  }
  .tile .new {
    position: absolute;
    right: 4px;
    top: 4px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 2px var(--bg-panel);
  }
  .tile .fav {
    position: absolute;
    left: 5px;
    top: 2px;
    font-style: normal;
    font-size: 11px;
    color: var(--text-strong);
    text-shadow: 0 0 3px #000;
  }

  /* ---- Réglages */
  aside {
    display: flex;
    flex-direction: column;
    gap: 13px;
    padding: 16px 18px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  .title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  h4 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .link {
    color: var(--text-muted);
    font-size: 11.5px;
  }
  .link:hover {
    color: var(--text);
  }
  .hint {
    margin-top: -6px;
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px;
    color: var(--text-3);
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  input[type="range"] {
    grid-column: 1 / -1;
    width: 100%;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-3);
  }
  label.color {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  label.color input {
    width: 44px;
    height: 22px;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: none;
    cursor: pointer;
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 5px;
  }
  .swatch {
    padding: 0;
    border-radius: var(--radius-s);
  }
  .swatch img {
    display: block;
    width: 100%;
    aspect-ratio: 1;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .swatch.on img {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .picked {
    font-style: normal;
    color: var(--text);
    font-size: 11.5px;
  }
  .segmented {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .segmented button {
    flex: 1;
    height: 26px;
    padding: 0 6px;
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
  }
  .segmented button + button {
    border-left: 1px solid var(--border);
  }
  .segmented button.on {
    background: var(--accent);
    color: var(--on-accent);
  }
  .spacer {
    flex: 1;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .ghost,
  .pill {
    height: 32px;
    padding: 0 18px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .ghost:disabled,
  .pill:disabled {
    opacity: 0.35;
  }
  .ghost.small {
    align-self: flex-start;
    height: 28px;
    padding: 0 12px;
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .choice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-3);
  }
  .choice select {
    height: 28px;
    max-width: 170px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  .notice {
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: rgba(255, 170, 60, 0.12);
    border: 1px solid rgba(255, 170, 60, 0.4);
    color: #ffd9a8;
  }

  /* Fenêtre étroite : le rail passe en icônes. */
  @media (max-width: 1250px) {
    .body {
      grid-template-columns: minmax(0, 1fr) 52px minmax(220px, 300px) 260px;
    }
    .ri span,
    .ri em,
    .search {
      display: none;
    }
  }
</style>
