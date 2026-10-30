<script lang="ts">
  // Visionneuse Lumitype : une image et celles de son dossier, dans une fenêtre à part (`lumitype --visionneuse`).
  //
  // Molette sur l'image : zoom autour du pointeur ; glisser : déplacer l'image agrandie ; double-clic : ajuster ↔ 100 %.
  // Molette sur la pellicule : défilement de la bande. Clavier : ← → (images), Début / Fin, + − 0 1 (zoom), R
  // (pivoter), I (infos), C (comparer), F11 (plein écran), Échap (sortir du plein écran ou du diaporama), Suppr
  // (corbeille), Ctrl+C (copier), Ctrl+P (imprimer), Ctrl+E (modifier dans Lumitype), Ctrl+O (ouvrir).
  import { onMount, untrack } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { t } from "../lib/i18n.svelte";
  import { formatBytes } from "../lib/format-bytes";
  import WindowControls from "../components/WindowControls.svelte";
  import WindowEdges from "../components/WindowEdges.svelte";
  import logo from "../assets/logo/logo-64.png";
  import { type ViewerEntry, type ViewerInfo, viewerApi } from "./viewer-api";
  import Histogram from "./Histogram.svelte";
  import Filmstrip from "./Filmstrip.svelte";
  import { dropThumb, requestThumb, thumbs } from "./thumb-cache.svelte";

  const win = getCurrentWindow();
  const EXTENSIONS = ["png", "jpg", "jpeg", "jfif", "webp", "avif", "gif", "bmp", "tif", "tiff", "heic", "heif", "dng", "arw", "cr2", "cr3", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "x3f", "3fr", "iiq", "erf", "kdc", "mrw", "raw", "rwl", "srf", "sr2", "crw", "ori", "fff", "mos", "dcr", "dcs", "mef", "ari"];

  let items = $state<ViewerEntry[]>([]);
  let folder = $state("");
  let index = $state(-1);
  const current = $derived(index >= 0 ? items[index] : undefined);

  // ---- images chargées par paliers (URL d'objet) : la vignette de la pellicule tout de suite, puis une version à la
  // taille de l'écran, puis la pleine définition seulement quand l'affichage en demande plus (zoom, grand écran). Une
  // photo de 24 Mpx ou un PNG de 40 Mo s'affiche ainsi aussitôt, sans attendre le décodage du fichier entier.
  type Tier = "screen" | "full";
  const urls = new Map<string, string>(); // clé « palier:chemin »
  const pending = new Map<string, Promise<string>>();
  let shownUrl = $state<string | null>(null);
  let shownTier = $state<"thumb" | Tier>("screen");
  /** Version « écran » de l'image courante : base de l'histogramme (jamais la pleine définition, trop lourde). */
  let screenUrl = $state<string | null>(null);
  let natural = $state({ w: 0, h: 0 });
  let loading = $state(false);
  let sharpening = $state(false);
  let error = $state("");
  let info = $state<ViewerInfo | null>(null);

  /** Côté de la version « écran » : celui de la zone d'affichage en pixels réels, par paliers de 512 (cache stable). */
  function screenSide(): number {
    const px = Math.max(stageSize.w, stageSize.h) * (window.devicePixelRatio || 1);
    return Math.min(4096, Math.max(1024, Math.ceil(px / 512) * 512));
  }

  /** Une image décodée avant d'être montrée : la bascule ne passe jamais par un écran vide. */
  async function decoded(url: string): Promise<void> {
    const img = new Image();
    img.src = url;
    await img.decode().catch(() => undefined);
  }

  function fetchTier(path: string, tier: Tier): Promise<string> {
    const key = `${tier}:${path}`;
    const ready = urls.get(key);
    if (ready) return Promise.resolve(ready);
    let promise = pending.get(key);
    if (!promise) {
      const call = tier === "full" ? viewerApi.image(path) : viewerApi.screen(path, screenSide());
      promise = call.then(async (buffer) => {
        const url = URL.createObjectURL(new Blob([buffer]));
        await decoded(url);
        urls.set(key, url);
        pending.delete(key);
        return url;
      });
      promise.catch(() => pending.delete(key));
      pending.set(key, promise);
    }
    return promise;
  }

  /** Oublie les images éloignées : versions « écran » à plus de 3 images, pleine définition hors de l'image courante. */
  function trim(): void {
    const near = new Set(items.slice(Math.max(0, index - 3), index + 4).map((e) => e.path));
    if (compare && compareEntry) near.add(compareEntry.path);
    for (const [key, url] of urls) {
      const [tier, ...rest] = key.split(":");
      const path = rest.join(":");
      const keep = tier === "full" ? path === current?.path : near.has(path);
      if (!keep) {
        URL.revokeObjectURL(url);
        urls.delete(key);
      }
    }
  }

  function forget(path: string): void {
    for (const tier of ["screen", "full"]) {
      const url = urls.get(`${tier}:${path}`);
      if (url) URL.revokeObjectURL(url);
      urls.delete(`${tier}:${path}`);
    }
  }

  let request = 0;
  let fullFor: string | null = null;
  async function show(path: string): Promise<void> {
    const mine = ++request;
    error = "";
    info = null;
    fullFor = null;
    sharpening = false;
    screenUrl = null;
    const cached = urls.get(`screen:${path}`);
    if (cached) {
      shownUrl = cached;
      shownTier = "screen";
      screenUrl = cached;
    } else {
      // La vignette, agrandie, le temps que la version « écran » arrive ; jamais l'image précédente (on croirait que
      // le changement n'a pas eu lieu). Pas encore de vignette : elle est demandée et montrée dès qu'elle arrive.
      shownUrl = thumbs[path] ?? null;
      shownTier = "thumb";
      if (!thumbs[path]) requestThumb(path);
    }
    loading = !cached;
    viewerApi.info(path).then(
      (value) => {
        if (mine === request) info = value;
      },
      () => undefined,
    );
    try {
      // Molette ou flèches tenues : on ne lance pas un développement par image survolée, seulement pour celle où
      // l'on s'arrête (les vignettes suffisent en passant).
      if (!cached) await new Promise((resolve) => setTimeout(resolve, 120));
      if (mine !== request) return;
      const url = await fetchTier(path, "screen");
      if (mine !== request) return;
      shownUrl = url;
      shownTier = "screen";
      screenUrl = url;
    } catch (failure) {
      if (mine !== request) return;
      error = describe(failure);
      shownUrl = null;
    } finally {
      if (mine === request) loading = false;
    }
    // Voisines préparées d'avance (version « écran » seulement).
    for (const near of [items[index + 1], items[index - 1]]) if (near) void fetchTier(near.path, "screen").catch(() => undefined);
    trim();
  }

  // Vignette arrivée pendant le chargement d'une image qui n'en avait pas encore.
  $effect(() => {
    const entry = current;
    if (entry && shownTier === "thumb" && !shownUrl && thumbs[entry.path]) shownUrl = thumbs[entry.path]!;
  });

  /** Pleine définition, chargée dès que l'image affichée a moins de pixels que l'écran n'en montre. */
  $effect(() => {
    const entry = current;
    const known = info;
    if (!entry || !known || shownTier !== "screen" || natural.w === 0 || fullFor === entry.path) return;
    const shownPixels = known.width * scale * (window.devicePixelRatio || 1);
    if (shownPixels <= natural.w * 1.05 || known.width <= natural.w) return;
    fullFor = entry.path;
    sharpening = true;
    const mine = request;
    fetchTier(entry.path, "full").then(
      (url) => {
        if (mine !== request) return;
        shownUrl = url;
        shownTier = "full";
        sharpening = false;
      },
      (failure) => {
        if (mine !== request) return;
        sharpening = false;
        toast(describe(failure));
      },
    );
  });

  function describe(failure: unknown): string {
    if (typeof failure === "string") return failure;
    if (failure && typeof failure === "object" && "message" in failure) return String((failure as { message: unknown }).message);
    return String(failure);
  }

  $effect(() => {
    const entry = current;
    if (!entry) return;
    untrack(() => {
      resetView();
      void show(entry.path);
      void win.setTitle(`${entry.name} — Lumitype Visionneuse`).catch(() => undefined);
    });
  });

  async function openPath(path: string): Promise<void> {
    try {
      const listing = await viewerApi.folder(path);
      folder = listing.folder;
      items = listing.items;
      const at = items.findIndex((e) => e.path === path);
      index = at >= 0 ? at : items.length > 0 ? 0 : -1;
    } catch (failure) {
      toast(describe(failure));
    }
  }

  async function chooseFile(): Promise<void> {
    const path = await open({ multiple: false, filters: [{ name: t("Images"), extensions: [...EXTENSIONS, ...EXTENSIONS.map((e) => e.toUpperCase())] }] });
    if (typeof path === "string") await openPath(path);
  }

  function go(step: number): void {
    if (items.length === 0) return;
    index = Math.min(items.length - 1, Math.max(0, index + step));
  }

  // ---- zoom et déplacement
  let stage = $state<HTMLDivElement>();
  let stageSize = $state({ w: 1, h: 1 });
  /** `null` : ajusté à la fenêtre (suit sa taille). */
  let zoom = $state<number | null>(null);
  let pan = $state({ x: 0, y: 0 });
  /** Taille réelle de l'image (celle du fichier, connue par ses infos), quel que soit le palier affiché. */
  const base = $derived(info && info.width > 0 ? { w: info.width, h: info.height } : natural);
  const fit = $derived(base.w > 0 ? Math.min(1, (stageSize.w - 32) / base.w, (stageSize.h - 32) / base.h) : 1);
  const scale = $derived(zoom ?? fit);
  const ZOOM_MIN = 0.02;
  const ZOOM_MAX = 16;

  function resetView(): void {
    zoom = null;
    pan = { x: 0, y: 0 };
  }

  function zoomAt(next: number, clientX?: number, clientY?: number): void {
    const target = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, next));
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    // Le point sous le pointeur reste sous le pointeur.
    const cx = (clientX ?? rect.left + rect.width / 2) - (rect.left + rect.width / 2);
    const cy = (clientY ?? rect.top + rect.height / 2) - (rect.top + rect.height / 2);
    const ratio = target / scale;
    pan = { x: cx - (cx - pan.x) * ratio, y: cy - (cy - pan.y) * ratio };
    zoom = target;
    if (zoom <= fit + 1e-6) resetView();
  }

  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    if (!shownUrl) return;
    const factor = Math.exp(-event.deltaY * (event.deltaMode === 1 ? 0.05 : 0.0015));
    zoomAt(scale * factor, event.clientX, event.clientY);
  }

  let drag: { x: number; y: number; px: number; py: number } | null = null;
  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || zoom === null) return;
    // Boutons posés sur l'image (précédente, suivante, plein écran) : leur clic passe. Capturer le pointeur ici le
    // détournait vers l'image dès qu'elle était zoomée — impossible alors de changer de photo.
    if ((event.target as HTMLElement | null)?.closest("button")) return;
    drag = { x: event.clientX, y: event.clientY, px: pan.x, py: pan.y };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }
  function onPointerMove(event: PointerEvent): void {
    if (!drag) return;
    pan = { x: drag.px + event.clientX - drag.x, y: drag.py + event.clientY - drag.y };
  }
  function onPointerUp(): void {
    drag = null;
  }
  function onDoubleClick(event: MouseEvent): void {
    if (zoom === null) zoomAt(1, event.clientX, event.clientY);
    else resetView();
  }

  // ---- comparaison : l'image courante et une autre côte à côte (clic sur la pellicule pour choisir l'autre).
  let compare = $state(false);
  let compareEntry = $state<ViewerEntry | null>(null);
  let compareUrl = $state<string | null>(null);
  function toggleCompare(): void {
    compare = !compare;
    if (compare) {
      compareEntry = items[index + 1] ?? items[index - 1] ?? null;
      if (compareEntry) void fetchTier(compareEntry.path, "screen").then((url) => (compareUrl = url), () => (compareUrl = null));
    } else {
      compareEntry = null;
      compareUrl = null;
    }
  }
  function pick(at: number): void {
    if (compare) {
      const entry = items[at];
      if (!entry || at === index) return;
      compareEntry = entry;
      compareUrl = null;
      void fetchTier(entry.path, "screen").then((url) => (compareUrl = url), () => (compareUrl = null));
      return;
    }
    index = at;
  }

  // ---- actions
  let message = $state("");
  let messageTimer = 0;
  function toast(text: string): void {
    message = text;
    clearTimeout(messageTimer);
    messageTimer = window.setTimeout(() => (message = ""), 4000);
  }

  let busy = $state(false);
  async function rotate(): Promise<void> {
    const entry = current;
    if (!entry || busy) return;
    if (info && !info.rotatable) {
      toast(t("Ce format ne se pivote pas sans perte : ouvrez l'image dans Lumitype pour la pivoter et l'exporter."));
      return;
    }
    busy = true;
    try {
      await viewerApi.rotate(entry.path, 1);
      forget(entry.path);
      dropThumb(entry.path, true);
      resetView();
      await show(entry.path);
    } catch (failure) {
      toast(describe(failure));
    } finally {
      busy = false;
    }
  }

  async function copy(): Promise<void> {
    if (!current) return;
    try {
      await viewerApi.copy(current.path);
      toast(t("Image copiée dans le presse-papiers."));
    } catch (failure) {
      toast(describe(failure));
    }
  }

  function print(): void {
    if (!shownUrl) return;
    const frame = document.createElement("iframe");
    frame.style.cssText = "position:fixed;right:0;bottom:0;width:0;height:0;border:0";
    document.body.append(frame);
    const doc = frame.contentDocument;
    if (!doc) return;
    doc.open();
    doc.write(`<!doctype html><html><head><style>@page{margin:10mm}html,body{margin:0;height:100%}body{display:flex;align-items:center;justify-content:center}img{max-width:100%;max-height:100vh;object-fit:contain}</style></head><body><img src="${shownUrl}"></body></html>`);
    doc.close();
    const img = doc.querySelector("img");
    const go = () => {
      frame.contentWindow?.focus();
      frame.contentWindow?.print();
      setTimeout(() => frame.remove(), 1000);
    };
    if (img?.complete) go();
    else img?.addEventListener("load", go, { once: true });
  }

  let confirming = $state(false);
  async function trash(): Promise<void> {
    confirming = false;
    const entry = current;
    if (!entry) return;
    try {
      await viewerApi.trash(entry.path);
      forget(entry.path);
      dropThumb(entry.path);
      const at = index;
      items = items.filter((e) => e.path !== entry.path);
      index = items.length === 0 ? -1 : Math.min(at, items.length - 1);
      if (index < 0) {
        shownUrl = null;
        info = null;
      }
      toast(t("« {name} » est dans la corbeille.", { name: entry.name }));
    } catch (failure) {
      toast(describe(failure));
    }
  }

  async function edit(): Promise<void> {
    if (!current) return;
    try {
      await viewerApi.edit(current.path);
    } catch (failure) {
      toast(describe(failure));
    }
  }

  // ---- plein écran, diaporama, panneaux
  let fullscreen = $state(false);
  async function toggleFullscreen(force?: boolean): Promise<void> {
    const next = force ?? !fullscreen;
    await win.setFullscreen(next).catch(() => undefined);
    fullscreen = next;
    resetView();
  }

  let slideshow = $state(false);
  let slideTimer = 0;
  function toggleSlideshow(): void {
    slideshow = !slideshow;
    clearInterval(slideTimer);
    if (slideshow) {
      void toggleFullscreen(true);
      slideTimer = window.setInterval(() => {
        if (index < items.length - 1) index++;
        else index = 0;
      }, 4000);
    }
  }
  function stopSlideshow(): void {
    if (!slideshow) return;
    slideshow = false;
    clearInterval(slideTimer);
  }

  const stored = (key: string, fallback: boolean): boolean => {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : v === "1";
    } catch {
      return fallback;
    }
  };
  const remember = (key: string, value: boolean): void => {
    try {
      localStorage.setItem(key, value ? "1" : "0");
    } catch {
      // stockage indisponible : réglage de la séance seulement
    }
  };
  let infoOpen = $state(stored("visionneuse.infos", true));
  let filmOpen = $state(stored("visionneuse.pellicule", true));
  $effect(() => remember("visionneuse.infos", infoOpen));
  $effect(() => remember("visionneuse.pellicule", filmOpen));

  // ---- clavier
  function onKey(event: KeyboardEvent): void {
    if ((event.target as HTMLElement)?.closest("input, select, textarea")) return;
    const ctrl = event.ctrlKey || event.metaKey;
    if (confirming) {
      if (event.key === "Enter") void trash();
      if (event.key === "Escape") confirming = false;
      event.preventDefault();
      return;
    }
    const key = event.key;
    const run = (action: () => void) => {
      event.preventDefault();
      action();
    };
    if (ctrl && key.toLowerCase() === "c") return run(() => void copy());
    if (ctrl && key.toLowerCase() === "p") return run(print);
    if (ctrl && key.toLowerCase() === "e") return run(() => void edit());
    if (ctrl && key.toLowerCase() === "o") return run(() => void chooseFile());
    if (ctrl) return;
    switch (key) {
      case "ArrowRight":
      case "PageDown":
      case " ":
        return run(() => go(1));
      case "ArrowLeft":
      case "PageUp":
      case "Backspace":
        return run(() => go(-1));
      case "Home":
        return run(() => (index = items.length ? 0 : -1));
      case "End":
        return run(() => (index = items.length - 1));
      case "+":
      case "=":
        return run(() => zoomAt(scale * 1.25));
      case "-":
        return run(() => zoomAt(scale / 1.25));
      case "0":
        return run(resetView);
      case "1":
        return run(() => zoomAt(1));
      case "r":
      case "R":
        return run(() => void rotate());
      case "i":
      case "I":
        return run(() => (infoOpen = !infoOpen));
      case "c":
      case "C":
        return run(toggleCompare);
      case "F11":
        return run(() => void toggleFullscreen());
      case "F5":
        return run(toggleSlideshow);
      case "Delete":
        return run(() => current && (confirming = true));
      case "Escape":
        if (slideshow) return run(stopSlideshow);
        if (fullscreen) return run(() => void toggleFullscreen(false));
        if (compare) return run(toggleCompare);
    }
  }

  onMount(() => {
    const observer = new ResizeObserver(() => {
      if (stage) stageSize = { w: stage.clientWidth, h: stage.clientHeight };
    });
    if (stage) observer.observe(stage);
    void viewerApi.start().then((path) => {
      if (path) void openPath(path);
    });
    // Glisser une image (ou un dossier) sur la fenêtre.
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "drop" && event.payload.paths[0]) void openPath(event.payload.paths[0]);
    });
    // Revenir sur la fenêtre : le dossier a pu changer (images ajoutées ou supprimées ailleurs).
    const onFocus = () => {
      if (!folder) return;
      const path = current?.path;
      void viewerApi.folder(folder).then((listing) => {
        items = listing.items;
        const at = path ? items.findIndex((e) => e.path === path) : -1;
        index = at >= 0 ? at : Math.min(index, items.length - 1);
      }, () => undefined);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      observer.disconnect();
      void unlisten.then((stop) => stop());
      window.removeEventListener("focus", onFocus);
      for (const url of urls.values()) URL.revokeObjectURL(url);
    };
  });

  const percent = $derived(`${Math.round(scale * 100)} %`);
  /** Curseur de zoom : échelle logarithmique de 2 % à 1600 %. */
  const sliderValue = $derived(Math.round((Math.log(scale / ZOOM_MIN) / Math.log(ZOOM_MAX / ZOOM_MIN)) * 1000));
  function onSlider(event: Event): void {
    const v = Number((event.currentTarget as HTMLInputElement).value) / 1000;
    zoomAt(ZOOM_MIN * Math.pow(ZOOM_MAX / ZOOM_MIN, v));
  }

  const bytes = formatBytes;
  const date = (ms: number) => (ms ? new Date(ms).toLocaleString(document.documentElement.lang || "fr", { dateStyle: "medium", timeStyle: "short" }) : "");
  const shortProfile = (name: string | null) => (name ? (name.length > 22 ? `${name.slice(0, 21)}…` : name) : null);
</script>

<svelte:window onkeydown={onKey} />

<div class="viewer" class:full={fullscreen}>
  {#if !fullscreen}
    <header class="title" data-tauri-drag-region>
      <img class="logo" src={logo} alt="" draggable="false" data-tauri-drag-region />
      <span class="app" data-tauri-drag-region>Lumitype <i>{t("Visionneuse")}</i></span>
      <span class="file" data-tauri-drag-region>{current?.name ?? ""}</span>
      <WindowControls />
    </header>
    <nav class="tools">
      <button class="primary" disabled={!current} onclick={() => void edit()} title={t("Ouvrir cette image dans l'éditeur Lumitype (Ctrl+E)")}>
        <svg viewBox="0 0 24 24"><path d="M4 20h4L19 9l-4-4L4 16z" /><path d="M14 6l4 4" /></svg>{t("Modifier dans Lumitype")}
      </button>
      <span class="sep"></span>
      <button disabled={!current || busy} onclick={() => void rotate()} title={t("Pivoter de 90° (R) : enregistré sans perte")}>
        <svg viewBox="0 0 24 24"><path d="M20 11a8 8 0 1 0-2.3 5.7" /><path d="M20 4v7h-7" /></svg>{t("Pivoter")}
      </button>
      <button class:on={compare} disabled={items.length < 2} onclick={toggleCompare} title={t("Deux images côte à côte (C) : cliquez une image de la pellicule pour l'autre")}>
        <svg viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="14" /><path d="M12 3v18" /></svg>{t("Comparer")}
      </button>
      <button disabled={!current} onclick={() => void copy()} title={t("Copier l'image (Ctrl+C)")}>
        <svg viewBox="0 0 24 24"><rect x="8" y="8" width="12" height="12" /><path d="M4 16V4h12" /></svg>{t("Copier")}
      </button>
      <button disabled={!shownUrl} onclick={print} title={t("Imprimer (Ctrl+P)")}>
        <svg viewBox="0 0 24 24"><path d="M6 9V3h12v6" /><rect x="3" y="9" width="18" height="8" /><path d="M7 14h10v7H7z" /></svg>{t("Imprimer")}
      </button>
      <button disabled={!current} onclick={() => (confirming = true)} title={t("Mettre à la corbeille (Suppr)")}>
        <svg viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" /></svg>{t("Corbeille")}
      </button>
      <span class="spacer"></span>
      <button class="icon line" class:on={infoOpen} onclick={() => (infoOpen = !infoOpen)} title={t("Infos (I)")} aria-label={t("Infos")} aria-pressed={infoOpen}>
        <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7.5v.5" /></svg>
      </button>
    </nav>
  {/if}

  <div class="main">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="stage" bind:this={stage} onwheel={onWheel} onpointerdown={onPointerDown} onpointermove={onPointerMove} onpointerup={onPointerUp} ondblclick={onDoubleClick} class:grab={zoom !== null} class:split={compare}>
      {#if info && !fullscreen}
        <div class="badges">
          <span>{info.format}</span>
          {#if info.depth}<span>{info.depth}</span>{/if}
          {#if shortProfile(info.profile)}<span title={info.profile ?? ""}>{shortProfile(info.profile)}</span>{/if}
        </div>
      {/if}
      {#if shownUrl}
        <img
          class="photo"
          class:half={compare}
          src={shownUrl}
          alt={current?.name ?? ""}
          draggable="false"
          onload={(event) => {
            const img = event.currentTarget as HTMLImageElement;
            natural = { w: img.naturalWidth, h: img.naturalHeight };
          }}
          style:width={compare ? undefined : `${base.w * scale}px`}
          style:height={compare ? undefined : `${base.h * scale}px`}
          style:transform={compare ? undefined : `translate(${pan.x}px, ${pan.y}px)`}
        />
        {#if compare}
          <div class="other">
            {#if compareUrl}<img class="photo half" src={compareUrl} alt={compareEntry?.name ?? ""} draggable="false" />{:else}<span class="hint">{t("Cliquez une image de la pellicule à comparer.")}</span>{/if}
            {#if compareEntry}<span class="caption">{compareEntry.name}</span>{/if}
          </div>
          <span class="caption left">{current?.name}</span>
        {/if}
      {:else if !loading && items.length === 0}
        <div class="empty">
          <img src={logo} alt="" width="56" height="56" />
          <h2>{t("Lumitype Visionneuse")}</h2>
          <p>{t("Ouvrez une image, ou glissez-la ici. Les autres images du dossier suivent dans la pellicule.")}</p>
          <button class="primary" onclick={() => void chooseFile()}>{t("Ouvrir une image…")}</button>
        </div>
      {/if}
      {#if error}<p class="error">{error}</p>{/if}
      {#if loading}<div class="spinner" aria-label={t("Chargement…")}></div>{/if}
      {#if sharpening && !loading}<span class="sharpening">{t("Pleine définition…")}</span>{/if}
      {#if items.length > 1}
        <button class="nav left" disabled={index <= 0} onclick={() => go(-1)} aria-label={t("Image précédente")}>‹</button>
        <button class="nav right" disabled={index >= items.length - 1} onclick={() => go(1)} aria-label={t("Image suivante")}>›</button>
      {/if}
      {#if current}
        <button class="full-btn" onclick={() => void toggleFullscreen()} title={fullscreen ? t("Quitter le plein écran (Échap)") : t("Plein écran (F11)")} aria-label={t("Plein écran")}>
          {#if fullscreen}
            <svg viewBox="0 0 24 24"><path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" /></svg>
          {:else}
            <svg viewBox="0 0 24 24"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" /></svg>
          {/if}
        </button>
      {/if}
    </div>

    {#if infoOpen && !fullscreen}
      <aside class="infos">
        {#if current}
          <h3 title={current.name}>{current.name}</h3>
          <div class="muted">
            {[info ? `${info.width} × ${info.height} px` : "", bytes(current.size), `${index + 1} / ${items.length}`].filter(Boolean).join(" · ")}
          </div>
          <div class="lbl">{t("Histogramme")}</div>
          <Histogram url={screenUrl} />
          <div class="lbl">{t("Fichier")}</div>
          <dl>
            <dt>{t("Format")}</dt><dd>{info?.format ?? "…"}{info?.depth ? ` · ${info.depth} / ${t("couche")}` : ""}</dd>
            {#if info?.color}<dt>{t("Couleurs")}</dt><dd>{t(info.color)}</dd>{/if}
            <dt>{t("Profil")}</dt><dd title={info?.profile ?? ""}>{info?.profile ?? t("Aucun (sRGB supposé)")}</dd>
            <dt>{t("Modifié le")}</dt><dd>{date(current.modified)}</dd>
            <dt>{t("Dossier")}</dt><dd class="path" title={folder}>{folder}</dd>
          </dl>
          {#if info && (info.camera || info.exposure || info.taken)}
            <div class="lbl">{t("Prise de vue")}</div>
            <dl>
              {#if info.camera}<dt>{t("Appareil")}</dt><dd>{info.camera}</dd>{/if}
              {#if info.lens}<dt>{t("Objectif")}</dt><dd>{info.lens}</dd>{/if}
              {#if info.exposure}<dt>{t("Réglages")}</dt><dd>{info.exposure}</dd>{/if}
              {#if info.taken}<dt>{t("Prise le")}</dt><dd>{info.taken}</dd>{/if}
            </dl>
          {/if}
        {:else}
          <p class="muted">{t("Aucune image ouverte.")}</p>
        {/if}
      </aside>
    {/if}
  </div>

  {#if filmOpen && !fullscreen && items.length > 0}
    <Filmstrip {items} {index} {folder} compareWith={compare ? (compareEntry?.path ?? null) : null} onpick={pick} />
  {/if}

  {#if !fullscreen}
    <footer class="status">
      <button class:on={slideshow} disabled={items.length < 2} onclick={toggleSlideshow} title={t("Diaporama en plein écran (F5)")}>{t("Diaporama")}</button>
      <button class:on={filmOpen} onclick={() => (filmOpen = !filmOpen)}>{t("Pellicule")}</button>
      <button onclick={() => void toggleFullscreen()}>{t("Plein écran")} <kbd>F11</kbd></button>
      <span class="spacer"></span>
      <button class:on={zoom === null} onclick={resetView} title={t("Ajuster à la fenêtre (0)")}>{t("Ajuster")}</button>
      <button onclick={() => zoomAt(1)} title={t("Taille réelle (1)")}>{percent}</button>
      <button class="step" onclick={() => zoomAt(scale / 1.25)} aria-label={t("Zoom arrière")}>−</button>
      <input type="range" min="0" max="1000" value={sliderValue} oninput={onSlider} aria-label={t("Zoom")} disabled={!shownUrl} />
      <button class="step" onclick={() => zoomAt(scale * 1.25)} aria-label={t("Zoom avant")}>+</button>
    </footer>
  {/if}

  {#if message}<div class="toast" role="status">{message}</div>{/if}

  {#if confirming && current}
    <div class="backdrop" role="presentation" onclick={() => (confirming = false)}>
      <div class="confirm" role="dialog" aria-modal="true" tabindex="-1" onclick={(event) => event.stopPropagation()} onkeydown={() => undefined}>
        <h3>{t("Mettre à la corbeille ?")}</h3>
        <p>{t("« {name} » ira dans la corbeille du système : vous pourrez l'y récupérer.", { name: current.name })}</p>
        <div class="row">
          <button onclick={() => (confirming = false)}>{t("Annuler")}</button>
          <button class="primary" onclick={() => void trash()}>{t("Mettre à la corbeille")}</button>
        </div>
      </div>
    </div>
  {/if}
</div>
{#if !fullscreen}<WindowEdges />{/if}

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    background: var(--bg-app);
  }
  :global(#viewer) {
    height: 100%;
  }
  .viewer {
    display: flex;
    flex-direction: column;
    height: 100%;
    color: var(--text);
    font: var(--size-text) var(--font);
    user-select: none;
  }
  button {
    font: inherit;
    color: inherit;
    background: none;
    border: 0;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  svg {
    width: 16px;
    height: 16px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.7;
  }
  .title {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 12px;
    background: var(--bg-bar);
    border-bottom: 1px solid var(--border);
    position: relative;
    flex: none;
  }
  .logo {
    width: 18px;
    height: 18px;
  }
  .app {
    font-weight: 600;
  }
  .app i {
    font-style: normal;
    font-weight: 400;
    color: var(--text-muted);
  }
  .file {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
    pointer-events: none;
  }
  .title :global(.controls) {
    margin-left: auto;
  }
  .tools {
    height: 44px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 10px;
    background: var(--bg-bar);
    border-bottom: 1px solid var(--border);
  }
  .tools button {
    height: 30px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 10px;
    color: var(--text-2);
  }
  .tools button:hover:not(:disabled),
  .tools button.on {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tools .icon {
    width: 32px;
    padding: 0;
    justify-content: center;
  }
  .tools .line {
    border: 1px solid var(--border-strong);
  }
  .tools .icon.on {
    border-color: var(--accent);
  }
  .primary {
    background: var(--accent) !important;
    color: #fff !important;
    font-weight: 500;
    padding: 0 14px !important;
  }
  .sep {
    width: 1px;
    height: 20px;
    margin: 0 6px;
    background: var(--border-strong);
  }
  .spacer {
    flex: 1;
  }
  .main {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .stage {
    flex: 1;
    min-width: 0;
    min-height: 0;
    position: relative;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #141313;
  }
  .full .stage {
    background: #000;
  }
  .stage.grab {
    cursor: grab;
  }
  .stage.grab:active {
    cursor: grabbing;
  }
  .photo {
    flex: none;
    max-width: none;
    image-rendering: auto;
    box-shadow: 0 18px 50px rgba(0, 0, 0, 0.5);
  }
  .stage.split {
    gap: 16px;
    padding: 16px 56px;
    box-sizing: border-box;
  }
  .photo.half {
    width: auto;
    height: auto;
    max-width: calc((100% - 16px) / 2);
    max-height: 100%;
    object-fit: contain;
  }
  .other {
    flex: 1;
    max-width: calc((100% - 16px) / 2);
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .other .photo.half {
    max-width: 100%;
  }
  .caption {
    position: absolute;
    bottom: 12px;
    right: 25%;
    transform: translateX(50%);
    padding: 3px 8px;
    background: rgba(0, 0, 0, 0.55);
    color: var(--text-2);
  }
  .caption.left {
    right: auto;
    left: 25%;
    transform: translateX(-50%);
  }
  .hint {
    color: var(--text-muted);
  }
  .badges {
    position: absolute;
    left: 16px;
    top: 14px;
    display: flex;
    gap: 6px;
    z-index: 2;
  }
  .badges span {
    padding: 3px 7px;
    background: rgba(0, 0, 0, 0.45);
    border: 1px solid var(--border-strong);
    color: var(--text-2);
    font-size: 11px;
  }
  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 40px;
    height: 56px;
    background: rgba(255, 255, 255, 0.04);
    font-size: 22px;
    color: var(--text-3);
    z-index: 2;
  }
  .nav:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    color: #fff;
  }
  .nav.left {
    left: 14px;
  }
  .nav.right {
    right: 14px;
  }
  .full-btn {
    position: absolute;
    right: 16px;
    bottom: 16px;
    width: 40px;
    height: 40px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.55);
    border: 1px solid var(--border-strong);
    color: #fff;
    z-index: 2;
  }
  .full-btn svg {
    width: 20px;
    height: 20px;
  }
  .full .nav,
  .full .full-btn {
    opacity: 0;
    transition: opacity 0.2s;
  }
  .full .stage:hover .nav,
  .full .stage:hover .full-btn {
    opacity: 1;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 380px;
    text-align: center;
  }
  .empty h2 {
    margin: 6px 0 0;
    font-size: 16px;
  }
  .empty p {
    margin: 0 0 6px;
    color: var(--text-3);
    line-height: 1.5;
  }
  .empty button {
    height: 32px;
  }
  .error {
    position: absolute;
    color: var(--danger);
    max-width: 60%;
    text-align: center;
  }
  .spinner {
    position: absolute;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.12);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
    z-index: 3;
  }
  .sharpening {
    position: absolute;
    right: 68px;
    bottom: 26px;
    padding: 3px 8px;
    background: rgba(0, 0, 0, 0.55);
    color: var(--text-3);
    font-size: 11px;
    z-index: 2;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .infos {
    width: 300px;
    flex: none;
    overflow-y: auto;
    padding: 16px 18px;
    background: var(--bg-panel);
    border-left: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
    user-select: text;
  }
  .infos h3 {
    margin: 0;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-muted);
  }
  .lbl {
    margin-top: 12px;
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  dl {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 6px 8px;
    margin: 0;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  dd.path {
    color: var(--text-2);
  }
  .status {
    height: 32px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px;
    background: var(--bg-bar);
    border-top: 1px solid var(--border);
    color: var(--text-3);
  }
  .status button {
    height: 24px;
    padding: 0 8px;
    color: var(--text-3);
  }
  .status button:hover:not(:disabled),
  .status button.on {
    color: var(--text);
    background: var(--bg-hover);
  }
  .status .step {
    width: 24px;
    padding: 0;
    font-size: 15px;
  }
  kbd {
    font: inherit;
    color: var(--text-muted);
    margin-left: 2px;
  }
  input[type="range"] {
    width: 140px;
    accent-color: var(--accent);
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 120px;
    transform: translateX(-50%);
    padding: 9px 14px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    max-width: 70%;
    z-index: 20;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 30;
  }
  .confirm {
    width: 380px;
    padding: 18px 20px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .confirm h3 {
    margin: 0 0 8px;
    font-size: 14px;
  }
  .confirm p {
    margin: 0 0 14px;
    color: var(--text-3);
    line-height: 1.45;
  }
  .confirm .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .confirm button {
    height: 32px;
    padding: 0 16px;
    border: 1px solid var(--border-strong);
  }
</style>
