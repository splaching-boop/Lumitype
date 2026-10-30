<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { formatBytes } from "../lib/format-bytes";
  import { describeError } from "../lib/errors";
  import { listen } from "@tauri-apps/api/event";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { artboardsOf } from "../lib/artboards.svelte";
  import { untrack } from "svelte";
  import { inSaveFolder } from "../lib/actions";
  import { type AvifSpeed, type ExportColor, type ExportFormat, type ExportOptions, type ImageMetadata, type PdfQuality, api } from "../lib/ipc";
  import { authorName, buildMetadata } from "../lib/metadata";
  import { profileName } from "../lib/color-settings.svelte";
  import { colorModeOf, exportSpace } from "../lib/color-mode.svelte";
  import { app } from "../state/app.svelte";
  import { printPrefs } from "../lib/print.svelte";

  // Fenêtre d'export. L'aperçu montre le VRAI fichier : l'image est encodée par le moteur avec les
  // réglages courants, affichée telle quelle, et son poids est celui, exact, du fichier qui sera écrit.
  const FORMATS: { id: ExportFormat; label: string; extension: string; hint: string; alpha: boolean }[] = [
    { id: "png", label: "PNG", extension: "png", hint: t("Sans perte, avec transparence. Le choix par défaut."), alpha: true },
    { id: "jpeg", label: "JPEG", extension: "jpg", hint: t("Compact, sans transparence (fond blanc) : idéal pour les photos."), alpha: false },
    { id: "webp", label: "WebP", extension: "webp", hint: t("Léger, avec transparence : le meilleur format pour le web."), alpha: true },
    { id: "tiff", label: "TIFF", extension: "tif", hint: t("Sans perte ni compression : impression et archivage."), alpha: true },
    { id: "pdf", label: "PDF", extension: "pdf", hint: t("Une page à la taille du document (selon sa résolution) : pour l'imprimeur, en RVB ou en CMJN."), alpha: true },
    { id: "avif", label: "AVIF", extension: "avif", hint: t("Le plus léger, avec transparence : en général 20 à 50 % plus léger que le WebP à qualité égale, mais quelques secondes d'encodage."), alpha: true },
  ];
  /** Vitesses d'encodage de l'AVIF : préréglages 9, 6 et 4 de rav1e (voir AvifSpeed, crates/engine/src/io.rs). */
  const AVIF_SPEEDS: { id: AvifSpeed; label: string }[] = [
    { id: "fast", label: t("Rapide") },
    { id: "balanced", label: t("Équilibrée") },
    { id: "max", label: t("Maximale") },
  ];
  /** Formats qui savent écrire du CMJN et porter un profil ICC (le PNG et le WebP restent en RVB, sans profil). */
  const COLOR_FORMATS: readonly ExportFormat[] = ["pdf", "tiff", "jpeg"];
  /** Qualités du PDF, comme dans Photoshop : sans perte (Flate) ou JPEG 92 / 80 / 60 (voir crates/engine/src/io.rs). */
  const PDF_QUALITIES: { id: PdfQuality; label: string }[] = [
    { id: "lossless", label: t("Qualité maximale (sans perte)") },
    { id: "high", label: t("Haute") },
    { id: "medium", label: t("Moyenne") },
    { id: "low", label: t("Légère") },
  ];
  const SCALES = [25, 50, 100, 150, 200];

  const doc = untrack(() => app.active);
  // Plans de travail : un plan seul, tous (un fichier par plan) ou le document entier. Par défaut, celui où l'on
  // travaille (la liste du panneau Calques) : un plan isolé → ce plan ; tous les plans affichés → tous.
  const boards = untrack(() => (doc ? artboardsOf(doc.info.layers) : []));
  const docName = untrack(() => doc?.info.name ?? t("Sans titre"));
  let choice = $state<number | "all" | "document">(untrack(() => (doc?.boardFocus != null && boards.some((b) => b.id === doc.boardFocus) ? doc.boardFocus : boards.length > 0 ? "all" : "document")));
  let boardMenu = $state(false);
  const exportBoard = $derived(typeof choice === "number" ? (boards.find((b) => b.id === choice) ?? null) : null);
  const choiceLabel = (c: typeof choice) => (c === "all" ? t("Tous les plans") : c === "document" ? t("Document entier") : (boards.find((b) => b.id === c)?.name ?? ""));
  const native = $derived(exportBoard ? { width: exportBoard.artboard.width, height: exportBoard.artboard.height } : { width: doc?.info.width ?? 1, height: doc?.info.height ?? 1 });
  function pickBoard(next: typeof choice): void {
    choice = next;
    boardMenu = false;
    name = typeof next === "number" ? `${docName} · ${choiceLabel(next)}` : docName;
  }
  /** Résolution du document : fixe la taille de la page d'un PDF (px × 25,4 / ppp, en mm), quelle que soit l'échelle. */
  const dpi = untrack(() => (doc?.info.dpi && doc.info.dpi > 0 ? doc.info.dpi : 72));
  const mm = (px: number) => ((px * 25.4) / dpi).toFixed(1).replace(".", ",").replace(/,0$/, "");

  let dialog = $state<HTMLDialogElement>();
  let pane = $state<HTMLDivElement>();
  let name = $state(untrack(() => (doc?.boardFocus != null ? `${doc.info.name} · ${doc.info.layers.find((l) => l.id === doc.boardFocus)?.name ?? ""}` : (doc?.info.name ?? t("Sans titre")))));
  /** Mode du document (Image › Mode) : il fixe les couleurs proposées par défaut (gris, bitmap 1 bit, CMJN). */
  const docMode = untrack(() => colorModeOf(doc?.info));
  // Document en mode CMJN : PDF en CMJN avec le profil du document par défaut, comme on l'enverrait à l'imprimeur.
  let format = $state<ExportFormat>(docMode.kind === "cmykWorkflow" ? "pdf" : "png");
  let quality = $state(85);
  /** WebP seulement : sans perte (le curseur de qualité est alors sans effet). */
  let lossless = $state(false);
  let percent = $state(100);
  /** PDF, TIFF, JPEG : RVB ou CMJN (espace du document en mode CMJN, sinon celui des Paramètres de couleur). */
  let color = $state<ExportColor>(docMode.kind === "grayscale" ? "gray" : docMode.kind === "bitmap" ? "bitmap" : docMode.kind === "cmykWorkflow" ? "cmyk" : "rgb");
  const space = untrack(() => exportSpace(doc?.info));
  /** « Inclure le profil ICC » : coché par défaut, comme « Couleur : Profil ICC » dans Photoshop. */
  let embedProfile = $state(true);
  let pdfQuality = $state<PdfQuality>("high");
  /** PDF CMJN = PDF/X-4 (la norme d'impression) : traits de coupe et fonds perdus, s'il y en a. */
  let cropMarks = $state(true);
  let includeBleed = $state(true);
  /** AVIF : vitesse d'encodage (rapide par défaut). */
  let avifSpeed = $state<AvifSpeed>("fast");
  /** Vue d'ensemble : taille du fichier (`width`, `height`) et de l'image envoyée (`w`, `h`). */
  let result = $state<{ url: string; width: number; height: number; w: number; h: number } | null>(null);
  /** Poids : estimé tout de suite, puis exact dès que l'encodage complet (en coulisse) a fini. */
  let size = $state<{ bytes: number; exact: boolean } | null>(null);
  /** Zone visible à pleine résolution, encodée comme dans le fichier, posée par-dessus la vue d'ensemble. */
  let detail = $state<{ url: string; x: number; y: number; w: number; h: number } | null>(null);
  let computing = $state(false);
  let failure = $state<string | null>(null);

  // Métadonnées (menu Copyright) : inscrites par défaut si elles sont activées ; description et mots-clés propres
  // à cette image, pré-remplis avec les valeurs par défaut.
  let embedMeta = $state(untrack(() => app.metadata.embed));
  let description = $state(untrack(() => app.metadata.description));
  let keywords = $state(untrack(() => app.metadata.keywords));
  const author = $derived(authorName(app.metadata));
  /** Ce qui sera inscrit avec les champs courants (la date, s'il y en a une, est celle du moment de l'appel). */
  const metadataNow = (): ImageMetadata | null => (embedMeta && app.metadata.embed ? buildMetadata(app.metadata, { description, keywords }) : null);

  const spec = $derived(FORMATS.find((f) => f.id === format) ?? FORMATS[0]);
  const target = $derived({ width: Math.max(1, Math.round((native.width * percent) / 100)), height: Math.max(1, Math.round((native.height * percent) / 100)) });
  /** Le curseur de qualité agit sur le JPEG, le WebP avec perte et l'AVIF. */
  const lossy = $derived(format === "jpeg" || format === "avif" || (format === "webp" && !lossless));
  const colorFormat = $derived(COLOR_FORMATS.includes(format));
  /**
   * Couleurs proposées pour ce format : RVB ; CMJN (PDF, TIFF, JPEG) ; niveaux de gris en vrai fichier gris 8 bits
   * (PNG, TIFF, JPEG, PDF) ; bitmap 1 bit (PNG, TIFF, PDF), pour un document en mode Bitmap seulement.
   */
  const colorChoices = $derived.by<ExportColor[]>(() => {
    const out: ExportColor[] = ["rgb"];
    if (colorFormat) out.push("cmyk");
    if (format !== "webp" && format !== "avif") out.push("gray");
    if (docMode.kind === "bitmap" && (format === "png" || format === "tiff" || format === "pdf")) out.push("bitmap");
    return out;
  });
  /**
   * Couleurs réellement écrites : le choix, s'il existe pour ce format ; sinon le plus proche (bitmap → gris en JPEG).
   * Un document gris ou bitmap exporté en WebP ou AVIF garde ses pixels gris (le moteur les y ramène).
   */
  const effectiveColor = $derived.by<ExportColor>(() => {
    if (colorChoices.includes(color)) return color;
    if (color === "bitmap" && colorChoices.includes("gray")) return "gray";
    if (color === "gray" || color === "bitmap") return color; // WebP, AVIF : pixels ramenés au gris, fichier RVB
    return "rgb";
  });
  const cmyk = $derived(colorFormat && effectiveColor === "cmyk");
  /** PDF en CMJN : toujours au format PDF/X-4 (profil incorporé, boîtes de coupe, encrage limité). */
  const pdfX4 = $derived(format === "pdf" && cmyk);
  const grayFile = $derived(effectiveColor === "gray" || effectiveColor === "bitmap");
  /** Profil incorporé si la case est cochée : celui de presse en CMJN, sRGB en RVB ; aucun en gris (pas de profil gris). */
  const iccName = $derived(cmyk ? profileName(space.profile) : "sRGB");
  const options = $derived<ExportOptions>({
    format,
    quality,
    lossless: format === "webp" && lossless,
    scale: percent / 100,
    metadata: metadataNow(),
    color: cmyk ? "cmyk" : grayFile ? effectiveColor : "rgb",
    cmykSpace: space,
    embedProfile: colorFormat && embedProfile,
    pdfQuality,
    avifSpeed,
    artboard: exportBoard?.id ?? null,
    pdfPrint: pdfX4
      ? { bleedMm: exportBoard ? 0 : (doc?.info.print?.bleedMm ?? 0), includeBleed, marks: cropMarks, inkLimit: doc?.info.print?.inkLimit ?? 0, title: name, pureBlack: printPrefs.textBlack, richBlack: printPrefs.richBlack, dpi: (doc?.info.dpi ?? 72) * (percent / 100) }
      : null,
  });
  /** La transparence survit-elle ? Pas en JPEG, ni en CMJN (l'impression n'en a pas : posée sur du blanc). */
  const keepsAlpha = $derived(!!spec?.alpha && !cmyk && effectiveColor !== "bitmap" && !(grayFile && (format === "tiff" || format === "jpeg")));

  $effect(() => {
    dialog?.showModal();
  });

  // Aperçu instantané : l'image est composée une fois (gardée tant que le document et la taille ne changent pas),
  // puis chaque réglage ne demande qu'une vue d'ensemble légère et un poids estimé. Le poids exact suit en coulisse.
  // Le moteur dit comment il a encodé l'aperçu : 0 = PNG, 1 = JPEG, 2 = WebP (formats avec perte montrés tels quels).
  const mimeOf = (kind: number) => (kind === 1 ? "image/jpeg" : kind === 2 ? "image/webp" : "image/png");
  let request = 0;
  /** Boîte fermée : les réponses encore en vol ne doivent plus créer d'URL d'objet (elles ne seraient jamais révoquées). */
  let closed = false;
  $effect(() => {
    const current = $state.snapshot(options);
    if (!doc || closed) return;
    const mine = ++request;
    computing = true;
    let exactTimer: ReturnType<typeof setTimeout> | undefined;
    const timer = setTimeout(() => {
      const side = Math.round(Math.min(2048, Math.max(pane?.clientWidth ?? 800, pane?.clientHeight ?? 600) * (window.devicePixelRatio || 1) * 1.25));
      api.exportOverview(doc.info.id, current, side).then(
        (buffer) => {
          if (mine !== request || closed) return;
          const head = new DataView(buffer);
          const [width, height, w, h] = [0, 4, 8, 12].map((at) => head.getUint32(at, true)) as [number, number, number, number];
          const bytes = Number(head.getBigUint64(16, true));
          const [exact, kind] = [head.getUint8(24) === 1, head.getUint8(25)];
          if (result) URL.revokeObjectURL(result.url);
          result = { url: URL.createObjectURL(new Blob([buffer.slice(26)], { type: mimeOf(kind) })), width, height, w, h };
          size = { bytes, exact };
          [computing, failure] = [false, null];
          dropDetail();
          scheduleDetail();
          // Poids exact : l'image entière encodée, seulement si les réglages ne bougent plus. Pas pour l'AVIF : son
          // encodage complet prend plusieurs secondes sur tous les cœurs, et ralentirait l'export qui suit.
          if (!exact && current.format !== "avif") {
            exactTimer = setTimeout(() => {
              api.exportExactSize(doc.info.id, current).then(
                (exactBytes) => mine === request && (size = { bytes: exactBytes, exact: true }),
                () => undefined,
              );
            }, 600);
          }
        },
        (error: unknown) => {
          if (mine === request) [computing, failure] = [false, describeError(error)];
        },
      );
    }, 90);
    return () => {
      clearTimeout(timer);
      clearTimeout(exactTimer);
    };
  });

  function dropDetail(): void {
    if (detail) URL.revokeObjectURL(detail.url);
    detail = null;
  }

  /** Zoomé au-delà de la finesse de la vue d'ensemble : la zone visible est demandée à pleine résolution. */
  let detailTimer: ReturnType<typeof setTimeout> | undefined;
  let detailRequest = 0;
  function scheduleDetail(): void {
    clearTimeout(detailTimer);
    const overview = result;
    if (!overview || !pane || !doc) return;
    const finesse = overview.w / overview.width; // pixels de la vue d'ensemble par pixel du fichier
    if (view.zoom <= finesse * 1.05) return dropDetail();
    const x0 = Math.max(0, Math.floor(-view.x / view.zoom));
    const y0 = Math.max(0, Math.floor(-view.y / view.zoom));
    const x1 = Math.min(overview.width, Math.ceil((pane.clientWidth - view.x) / view.zoom));
    const y1 = Math.min(overview.height, Math.ceil((pane.clientHeight - view.y) / view.zoom));
    if (x1 <= x0 || y1 <= y0) return;
    const mine = ++detailRequest;
    const current = $state.snapshot(options);
    const docId = doc.info.id;
    detailTimer = setTimeout(() => {
      api.exportRegion(docId, current, [x0, y0, x1 - x0, y1 - y0]).then(
        (buffer) => {
          if (mine !== detailRequest || result !== overview || closed) return;
          const head = new DataView(buffer);
          const [w, h] = [head.getUint32(0, true), head.getUint32(4, true)];
          const url = URL.createObjectURL(new Blob([buffer.slice(9)], { type: mimeOf(head.getUint8(8)) }));
          if (detail) URL.revokeObjectURL(detail.url);
          detail = { url, x: x0, y: y0, w, h };
        },
        () => undefined,
      );
    }, 110);
  }
  // Déplacer ou zoomer l'aperçu : nouvelle zone visible.
  $effect(() => {
    void view.zoom;
    void view.x;
    void view.y;
    untrack(scheduleDetail);
  });

  // ---- Navigation dans l'aperçu : main par défaut, Ctrl = loupe +, Alt = loupe −.

  /** Zoom en pixels d'écran par pixel du fichier exporté, et décalage du coin de l'image dans le volet. */
  let view = $state({ zoom: 1, x: 0, y: 0 });
  let modifier = $state<"in" | "out" | null>(null);
  let panning = $state<{ x: number; y: number } | null>(null);
  let placed = false;

  function place(zoom: number): void {
    if (!pane || !result) return;
    view = { zoom, x: (pane.clientWidth - result.width * zoom) / 2, y: (pane.clientHeight - result.height * zoom) / 2 };
  }
  const fitZoom = () => (pane && result ? Math.min((pane.clientWidth - 48) / result.width, (pane.clientHeight - 48) / result.height) : 1);

  // Premier affichage : à 100 % (vue « très zoomée » d'une grande image), ou ajusté si l'image tient déjà.
  // Changer la résolution garde le point central de la vue.
  let lastSize = { width: 0, height: 0 };
  $effect(() => {
    if (!result || !pane) return;
    const [w, h] = [result.width, result.height];
    untrack(() => {
      if (!placed) {
        placed = true;
        place(fitZoom() < 1 ? 1 : fitZoom());
      } else if (lastSize.width !== w && pane) {
        const [cx, cy] = [(pane.clientWidth / 2 - view.x) / (lastSize.width * view.zoom), (pane.clientHeight / 2 - view.y) / (lastSize.height * view.zoom)];
        view = { ...view, x: pane.clientWidth / 2 - cx * w * view.zoom, y: pane.clientHeight / 2 - cy * h * view.zoom };
      }
      lastSize = { width: w, height: h };
    });
  });

  function zoomAt(factor: number, px: number, py: number): void {
    const zoom = Math.min(32, Math.max(0.02, view.zoom * factor));
    const k = zoom / view.zoom;
    view = { zoom, x: px - (px - view.x) * k, y: py - (py - view.y) * k };
  }
  const zoomCentered = (factor: number) => pane && zoomAt(factor, pane.clientWidth / 2, pane.clientHeight / 2);

  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || !pane) return;
    const rect = pane.getBoundingClientRect();
    if (modifier) {
      zoomAt(modifier === "in" ? 2 : 0.5, event.clientX - rect.left, event.clientY - rect.top);
      return;
    }
    pane.setPointerCapture(event.pointerId);
    panning = { x: event.clientX, y: event.clientY };
  }

  function onPointerMove(event: PointerEvent): void {
    if (!panning) return;
    view = { ...view, x: view.x + event.clientX - panning.x, y: view.y + event.clientY - panning.y };
    panning = { x: event.clientX, y: event.clientY };
  }

  function onWheel(event: WheelEvent): void {
    if (!pane) return;
    event.preventDefault();
    const rect = pane.getBoundingClientRect();
    zoomAt(Math.exp(-event.deltaY * 0.0015), event.clientX - rect.left, event.clientY - rect.top);
  }

  const onModifier = (event: KeyboardEvent) => (modifier = event.ctrlKey ? "in" : event.altKey ? "out" : null);

  // ---- Réglages

  function setWidth(value: number): void {
    if (Number.isFinite(value) && value >= 1) percent = Math.min(800, Math.max(1, (value / native.width) * 100));
  }
  function setHeight(value: number): void {
    if (Number.isFinite(value) && value >= 1) percent = Math.min(800, Math.max(1, (value / native.height) * 100));
  }

  const weight = formatBytes;

  function close(): void {
    closed = true;
    request += 1;
    detailRequest += 1;
    if (result) URL.revokeObjectURL(result.url);
    dropDetail();
    void api.exportPreviewRelease().catch(() => undefined);
    app.exportOpen = false;
  }

  /** Tous les plans : un fichier par plan (« Nom · Plan 1.png »…), dans le dossier choisi. */
  async function exportAll(chosen: ExportOptions, base: string, extension: string): Promise<void> {
    if (!doc) return;
    try {
      const folder = await open({ directory: true, title: t("Dossier des plans exportés") });
      if (typeof folder !== "string") return;
      close();
      app.exportProgress = { progress: 0, label: t("Préparation") };
      try {
        for (const [index, board] of boards.entries()) {
          app.exportProgress = { progress: index / boards.length, label: t("Plan {n} sur {total} : {name}", { n: index + 1, total: boards.length, name: board.name }) };
          const safe = `${base} · ${board.name}`.replace(/[\\/:*?"<>|]/g, "-");
          await api.exportImage(doc.info.id, joinPath(folder, `${safe}.${extension}`), { ...chosen, artboard: board.id, pdfPrint: chosen.pdfPrint ? { ...chosen.pdfPrint, bleedMm: 0, title: `${base} · ${board.name}` } : null });
        }
      } finally {
        app.exportProgress = null;
      }
    } catch (error) {
      app.fail(error);
    }
  }

  async function exportNow(): Promise<void> {
    if (!doc) return;
    // La date inscrite est celle de l'export, pas celle du dernier réglage.
    const chosen: ExportOptions = { ...$state.snapshot(options), metadata: metadataNow() };
    const extension = spec?.extension ?? "png";
    const base = name.trim() || t("Sans titre");
    if (choice === "all" && boards.length > 0) return exportAll(chosen, base, extension);
    try {
      const path = await save({ defaultPath: await inSaveFolder(`${base}.${extension}`), filters: [{ name: spec?.label ?? "Image", extensions: [extension] }] });
      if (!path) return;
      close();
      // La coque envoie l'avancement étape par étape : il s'affiche dans une barre, jusqu'à la fin.
      app.exportProgress = { progress: 0, label: t("Préparation") };
      const stop = await listen<{ progress: number; label: string }>("export-progress", ({ payload }) => (app.exportProgress = payload));
      try {
        await api.exportImage(doc.info.id, path, chosen);
      } finally {
        stop();
        app.exportProgress = null;
      }
    } catch (error) {
      app.fail(error);
    }
  }
</script>

<script lang="ts" module>
  /** Chemin d'un fichier dans un dossier, avec le séparateur de ce dossier (Windows ou Unix). */
  export function joinPath(folder: string, file: string): string {
    const sep = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
    return folder.endsWith(sep) ? folder + file : folder + sep + file;
  }
</script>

<svelte:window onkeydown={onModifier} onkeyup={onModifier} onblur={() => (modifier = null)} />

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close())}>
  <form onsubmit={(event) => (event.preventDefault(), void exportNow())}>
    <header>
      <h2>{t("Exporter")}</h2>
      <span>{doc?.info.name} · {choice === "all" ? t("{count} plans", { count: boards.length }) : `${native.width} × ${native.height} px`}</span>
      <div class="spacer"></div>
      <button type="button" class="icon" aria-label={t("Fermer")} onclick={close}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </header>

    <div class="body">
      <div
        class="pane"
        bind:this={pane}
        role="application"
        aria-label={t("Aperçu de l'export : glisser pour déplacer, Ctrl + clic pour agrandir, Alt + clic pour réduire")}
        style:cursor={modifier === "in" ? "zoom-in" : modifier === "out" ? "zoom-out" : panning ? "grabbing" : "grab"}
        onpointerdown={onPointerDown}
        onpointermove={onPointerMove}
        onpointerup={() => (panning = null)}
        onpointercancel={() => (panning = null)}
        onwheel={onWheel}
      >
        {#if result}
          <!-- Pixels nets au-delà de 100 % : on juge le rendu réel, pas un lissage du navigateur. -->
          <img
            src={result.url}
            alt=""
            draggable="false"
            class:pixelated={view.zoom > 1}
            style:width="{result.width * view.zoom}px"
            style:height="{result.height * view.zoom}px"
            style:transform="translate({view.x}px, {view.y}px)"
          />
          {#if detail}
            <!-- Zone visible à pleine résolution, encodée comme dans le fichier (défauts JPEG compris). -->
            <img
              class="detail"
              src={detail.url}
              alt=""
              draggable="false"
              class:pixelated={view.zoom > 1}
              style:width="{detail.w * view.zoom}px"
              style:height="{detail.h * view.zoom}px"
              style:transform="translate({view.x + detail.x * view.zoom}px, {view.y + detail.y * view.zoom}px)"
            />
          {/if}
        {/if}
        <div class="chip top-left">{t("Glisser : déplacer")} <i>·</i> {t("Ctrl : loupe +")} <i>·</i> {t("Alt : loupe −")}</div>
        <div class="chip top-right">{computing ? t("Encodage…") : format === "avif" ? t("Aperçu des pixels (compression AVIF non simulée)") : cmyk ? t("Aperçu du rendu {format} · épreuve {profile}", { format: spec?.label ?? "", profile: profileName(space.profile) }) : t("Aperçu du rendu {format}", { format: spec?.label ?? "" })}</div>
        <div class="zoombar" role="presentation" onpointerdown={(event) => event.stopPropagation()}>
          <button type="button" aria-label={t("Zoom arrière")} onclick={() => zoomCentered(0.5)}>−</button>
          <output>{Math.round(view.zoom * 100)} %</output>
          <button type="button" aria-label={t("Zoom avant")} onclick={() => zoomCentered(2)}>+</button>
          <div class="rule"></div>
          <button type="button" class="text" onclick={() => place(fitZoom())}>{t("Ajuster")}</button>
          <button type="button" class="text" onclick={() => place(1)}>100 %</button>
        </div>
        {#if failure}<p class="failure">{failure}</p>{/if}
      </div>

      <aside>
        <label class="field">
          <span>{t("Nom du fichier")}</span>
          <div class="name">
            <input type="text" bind:value={name} spellcheck="false" />
            <em>.{spec?.extension}</em>
          </div>
        </label>

        {#if boards.length > 0}
          <div class="field board">
            <span>{t("Plan de travail")}</span>
            <button type="button" class="board-pick" class:on={boardMenu} aria-haspopup="listbox" aria-expanded={boardMenu} onclick={() => (boardMenu = !boardMenu)}>
              <i class="glyph"></i>
              <b>{choiceLabel(choice)}</b>
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d={boardMenu ? "M6 15l6-6 6 6" : "M6 9l6 6 6-6"} /></svg>
            </button>
            {#if boardMenu}
              <div class="board-list" role="listbox" aria-label={t("Plan de travail")}>
                <button type="button" role="option" aria-selected={choice === "all"} class:on={choice === "all"} onclick={() => pickBoard("all")}><i class="glyph"></i>{t("Tous les plans")}<small>{t("{count} fichiers", { count: boards.length })}</small></button>
                {#each boards as board (board.id)}
                  <button type="button" role="option" aria-selected={choice === board.id} class:on={choice === board.id} onclick={() => pickBoard(board.id)}><i class="glyph"></i>{board.name}<small>{board.artboard.width} × {board.artboard.height}</small></button>
                {/each}
                <button type="button" role="option" aria-selected={choice === "document"} class:on={choice === "document"} onclick={() => pickBoard("document")}><i class="glyph doc"></i>{t("Document entier")}<small>{doc?.info.width} × {doc?.info.height}</small></button>
              </div>
            {/if}
            {#if choice === "all"}<p>{t("Un fichier par plan (« {name} · Plan 1 »…), dans le dossier choisi. L'aperçu montre le document entier.", { name: name.trim() || docName })}</p>{/if}
          </div>
        {/if}

        <div class="field">
          <span>{t("Format")}</span>
          <div class="segmented">
            {#each FORMATS as item (item.id)}
              <button type="button" class:on={item.id === format} onclick={() => (format = item.id)}>{t(item.label)}</button>
            {/each}
          </div>
          {#if format !== "pdf"}<p>{t(spec?.hint ?? "")}</p>{/if}
        </div>

        {#if format === "pdf"}
          <label class="field">
            <span>{t("Qualité")}</span>
            <select bind:value={pdfQuality}>
              {#each PDF_QUALITIES as item (item.id)}
                <option value={item.id}>{item.label}</option>
              {/each}
            </select>
            <p>{t("Niveau de compression de l'image")}</p>
          </label>
          {#if pdfX4}
            <div class="field">
              <span class="line">{t("Norme")} <b class="norm">PDF/X-4</b></span>
              {#if doc?.info.print?.bleedMm && !exportBoard && choice !== "all"}
                <label class="switch" class:off={!includeBleed}><input type="checkbox" bind:checked={includeBleed} /> {t("Inclure les fonds perdus ({mm} mm)", { mm: doc.info.print.bleedMm })}</label>
              {/if}
              <label class="switch" class:off={!cropMarks}><input type="checkbox" bind:checked={cropMarks} /> {t("Traits de coupe")}</label>
              <button type="button" class="verify" onclick={() => (app.preflightOpen = true)}>{t("Vérifier avant impression…")}</button>
            </div>
          {/if}
        {/if}

        {#if colorChoices.length > 1}
          <div class="field">
            <span>{t("Couleurs")}</span>
            <div class="segmented">
              <button type="button" class:on={effectiveColor === "rgb"} onclick={() => (color = "rgb")}>{t("RVB")}</button>
              {#if colorChoices.includes("cmyk")}<button type="button" class:on={effectiveColor === "cmyk"} onclick={() => (color = "cmyk")}>{t("CMJN")}</button>{/if}
              {#if colorChoices.includes("gray")}<button type="button" class:on={effectiveColor === "gray"} onclick={() => (color = "gray")}>{t("Gris")}</button>{/if}
              {#if colorChoices.includes("bitmap")}<button type="button" class:on={effectiveColor === "bitmap"} onclick={() => (color = "bitmap")}>{t("1 bit")}</button>{/if}
            </div>
            {#if cmyk}
              <div class="profile-line" title={docMode.kind === "cmykWorkflow" ? t("Espace CMJN du document (Image › Mode › Couleurs CMJN).") : t("Espace CMJN des Paramètres de couleur (Édition).")}>{t("Profil")} : <b>{profileName(space.profile)}</b></div>
            {/if}
            {#if colorFormat && !grayFile}
              <label class="check"><input type="checkbox" bind:checked={embedProfile} /> {t("Inclure le profil ICC : {name}", { name: iccName })}</label>
            {/if}
            <p>
              {cmyk
                ? t("Converti en CMJN à l'export ({intent}{bpc}), transparence posée sur du blanc. L'aperçu simule l'impression.", { intent: space.intent === "perceptual" ? t("perceptive") : t("colorimétrie relative"), bpc: space.bpc ? t(", compensation du point noir") : "" })
                : effectiveColor === "gray"
                  ? t("Vrai fichier en niveaux de gris 8 bits (une seule couche) : plus léger qu'en RVB, sans profil ICC.")
                  : effectiveColor === "bitmap"
                    ? t("Noir et blanc 1 bit (PNG, PDF ; TIFF compressé PackBits) : le plus léger, posé sur du blanc.")
                    : t("Couleurs de l'écran (sRGB) : pour un imprimeur, préférez le CMJN de son profil.")}
            </p>
          </div>
        {/if}

        {#if format === "jpeg" || format === "webp" || format === "avif"}
          <div class="field">
            <span class="line">
              {t("Qualité")}
              {#if format === "webp"}
                <label class="switch" class:off={!lossless}><input type="checkbox" bind:checked={lossless} /> {t("Sans perte")}</label>
              {/if}
              <output class:muted={!lossy}>{quality} %</output>
            </span>
            <input type="range" min="1" max="100" bind:value={quality} disabled={!lossy} />
            {#if format === "webp"}
              <p>{t("Avec perte : 25 à 35 % plus léger que le JPEG à qualité égale, avec transparence. Sans perte : plus léger que le PNG.")}</p>
            {/if}
          </div>
        {/if}

        {#if format === "avif"}
          {@const speedHint = t("Plus lent = un peu plus léger, utile surtout pour les images graphiques (aplats, texte, captures) ; sur une photo, l'écart est faible.")}
          <div class="field" title={speedHint}>
            <span>{t("Vitesse d'encodage")}</span>
            <div class="segmented" role="radiogroup" aria-label={t("Vitesse d'encodage")}>
              {#each AVIF_SPEEDS as item (item.id)}
                <button type="button" role="radio" aria-checked={avifSpeed === item.id} class:on={avifSpeed === item.id} title={speedHint} onclick={() => (avifSpeed = item.id)}>{item.label}</button>
              {/each}
            </div>
            <p>{avifSpeed === "fast" ? t("Quelques secondes pour une photo de 10 Mpx.") : avifSpeed === "balanced" ? t("Deux à trois fois plus long que Rapide.") : t("Le plus compact, à peine plus long qu'Équilibrée : pour les images graphiques.")}</p>
          </div>
        {/if}

        <div class="field">
          <span class="line">
            {t("Résolution")}
            <button type="button" class="native" class:on={percent === 100} onclick={() => (percent = 100)}>{t("Native")}</button>
          </span>
          <div class="scales">
            {#each SCALES as value (value)}
              <button type="button" class:on={Math.round(percent) === value} onclick={() => (percent = value)}>{value} %</button>
            {/each}
          </div>
          <div class="dimensions">
            <label>L <input type="number" min="1" value={target.width} onchange={(event) => setWidth(event.currentTarget.valueAsNumber)} /></label>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 15l6-6M10 6l1-1a4 4 0 016 6l-1 1M14 18l-1 1a4 4 0 01-6-6l1-1" /></svg>
            <label>H <input type="number" min="1" value={target.height} onchange={(event) => setHeight(event.currentTarget.valueAsNumber)} /></label>
            <label class="percent"><input type="number" min="1" max="800" value={Math.round(percent * 10) / 10} onchange={(event) => Number.isFinite(event.currentTarget.valueAsNumber) && (percent = Math.min(800, Math.max(1, event.currentTarget.valueAsNumber)))} /> %</label>
          </div>
          {#if format === "pdf"}
            <p>{t("Page de {width} × {height} mm ; l'échelle change la finesse ({dpi} ppp), pas la taille imprimée.", { width: mm(native.width), height: mm(native.height), dpi: Math.round((dpi * percent) / 100) })}</p>
          {/if}
        </div>

        <div class="field">
          <span class="line">
            {t("Métadonnées")}
            <label class="switch" class:off={!app.metadata.embed}><input type="checkbox" bind:checked={embedMeta} disabled={!app.metadata.embed} /> {t("Inscrire")}</label>
          </span>
          {#if !app.metadata.embed}
            <p>{t("Désactivées : menu Copyright › Métadonnées par défaut…")}</p>
          {:else}
            <p class="who">{author ? `${author}${app.metadata.copyright ? ` · ${app.metadata.copyright}` : ""}` : t("Aucun auteur renseigné : menu Copyright › Métadonnées par défaut…")}</p>
            <input type="text" class="meta" bind:value={description} disabled={!embedMeta} placeholder={t("Description de cette image")} />
            <input type="text" class="meta" bind:value={keywords} disabled={!embedMeta} placeholder={t("Mots-clés, séparés par des virgules")} spellcheck="false" />
          {/if}
        </div>

        <div class="spacer"></div>

        <div class="weight">
          <div>
            <span>{t("Poids du fichier")}</span>
            <strong class:stale={computing}>{size ? `${size.exact ? "" : "≈ "}${weight(size.bytes)}` : "…"}</strong>
          </div>
          <div class="right">
            <span>{t(spec?.label ?? "")}{cmyk ? ` ${t("CMJN")}` : ""} · {keepsAlpha ? t("transparence conservée") : t("fond blanc")}</span>
            <span>{t("{width} × {height} px", { width: target.width, height: target.height })}</span>
          </div>
        </div>
      </aside>
    </div>

    <footer>
      <span>{size && !size.exact ? (format === "avif" ? t("Poids estimé sur un échantillon de l'image : l'AVIF complet n'est encodé qu'à l'export.") : t("Poids estimé sur une partie de l'image ; le poids exact suit dans un instant.")) : t("Le poids est celui du fichier réellement encodé avec ces réglages.")}</span>
      <div class="spacer"></div>
      <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!doc}>{t("Exporter…")}</button>
    </footer>
  </form>
</dialog>

<style>
  .board {
    position: relative;
  }
  .board-pick {
    height: 34px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    text-align: left;
  }
  .board-pick b {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 500;
  }
  .board-pick svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
  }
  .glyph {
    width: 16px;
    height: 11px;
    flex-shrink: 0;
    border: 1.5px solid currentColor;
    border-radius: 1px;
    opacity: 0.75;
  }
  .glyph.doc {
    border-style: dashed;
  }
  .board-list {
    position: absolute;
    left: 0;
    right: 0;
    top: 62px;
    z-index: 10;
    display: flex;
    flex-direction: column;
    padding: 4px 0;
    max-height: 300px;
    overflow-y: auto;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-menu);
    box-shadow: var(--shadow-menu);
  }
  .board-list button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 12px;
    color: var(--text-2);
    text-align: left;
  }
  .board-list button:hover {
    background: var(--bg-hover);
  }
  .board-list button.on {
    color: var(--text-strong, var(--text));
    background: rgb(var(--ink) / 0.08);
    box-shadow: inset 2px 0 var(--accent);
  }
  .board-list small {
    margin-left: auto;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .norm {
    margin-left: auto;
    padding: 1px 8px;
    border-radius: var(--radius-s);
    background: var(--bg-well);
    font-weight: 600;
    color: var(--text-2);
  }
  .verify {
    align-self: flex-start;
    height: 28px;
    padding: 0 14px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
  }
  dialog {
    width: min(1280px, calc(100vw - 48px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.72);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 22px;
  }
  header {
    height: 56px;
    border-bottom: 1px solid var(--border);
  }
  footer {
    height: 60px;
    border-top: 1px solid var(--border);
  }
  header span,
  footer span {
    color: var(--text-muted);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .spacer {
    flex: 1;
  }
  svg {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
  }
  .icon {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .icon:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .body {
    display: flex;
    /* Assez haut pour que la colonne des réglages tienne sans défiler (sur un écran 1080p et plus). */
    height: min(900px, calc(100vh - 150px));
  }
  .pane {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: repeating-conic-gradient(#2a2a2d 0% 25%, #232326 0% 50%) 0 0 / 20px 20px;
    touch-action: none;
    user-select: none;
  }
  img {
    position: absolute;
    left: 0;
    top: 0;
    max-width: none;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  img.pixelated {
    image-rendering: pixelated;
  }
  img.detail {
    box-shadow: none;
  }
  .chip,
  .zoombar {
    position: absolute;
    display: flex;
    align-items: center;
    border-radius: var(--radius-pill);
    background: rgba(15, 15, 17, 0.82);
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .chip {
    top: 16px;
    height: 28px;
    gap: 8px;
    padding: 0 12px;
    pointer-events: none;
  }
  .chip i {
    color: var(--text-disabled);
    font-style: normal;
  }
  .top-left {
    left: 16px;
  }
  .top-right {
    right: 16px;
  }
  .zoombar {
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    gap: 2px;
    padding: 3px;
    cursor: default;
  }
  .zoombar button {
    width: 28px;
    height: 26px;
    border-radius: var(--radius-pill);
    font-size: 15px;
    color: var(--text);
  }
  .zoombar button.text {
    width: auto;
    padding: 0 10px;
    font-size: inherit;
    color: var(--text-2);
  }
  .zoombar button:hover {
    background: var(--bg-hover);
  }
  .zoombar output {
    min-width: 58px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .rule {
    width: 1px;
    height: 14px;
    margin: 0 4px;
    background: var(--border-strong);
  }
  .failure {
    position: absolute;
    left: 16px;
    bottom: 16px;
    margin: 0;
    color: var(--danger, #ff8080);
  }
  aside {
    width: 340px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 22px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 8px;
    color: var(--text-3);
  }
  .field p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  /* (Un curseur désactivé s'estompe de lui-même : theme/base.css.) */
  output.muted {
    opacity: 0.4;
  }
  .name {
    display: flex;
    align-items: center;
    height: 34px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    overflow: hidden;
  }
  .name input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0 10px;
    border: 0;
    background: transparent;
    color: var(--text);
    user-select: text;
  }
  .name em {
    height: 100%;
    display: flex;
    align-items: center;
    padding: 0 12px;
    border-left: 1px solid var(--border);
    color: var(--text-muted);
    font-style: normal;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-l);
    background: var(--bg-well);
    border: 1px solid var(--border);
  }
  .segmented button {
    flex: 1;
    height: 30px;
    border-radius: var(--radius-m);
    font-weight: 500;
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .native {
    height: 22px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text-3);
  }
  .native.on {
    background: var(--bg-active);
    border-color: rgb(var(--ink) / 0.22);
    color: var(--text-strong);
  }
  .scales {
    display: flex;
    gap: 6px;
  }
  .scales button {
    flex: 1;
    height: 30px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .scales button.on {
    background: rgb(var(--ink) / 0.08);
    border-color: rgb(var(--ink) / 0.22);
  }
  .dimensions {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-muted);
  }
  .dimensions label {
    flex: 1;
    min-width: 0;
    height: 32px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text-muted);
  }
  .dimensions .percent {
    flex: 0 0 74px;
  }
  .dimensions input {
    flex: 1;
    min-width: 0;
    width: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: right;
    font-variant-numeric: tabular-nums;
    user-select: text;
    appearance: textfield;
  }
  .dimensions input::-webkit-inner-spin-button {
    appearance: none;
  }
  .switch {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-3);
  }
  .switch.off {
    opacity: 0.45;
  }
  .profile-line {
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .profile-line b {
    color: var(--text);
    font-weight: 500;
  }
  select {
    height: 32px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .who {
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  input.meta {
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  input.meta:disabled {
    opacity: 0.45;
  }
  input.meta::placeholder {
    color: var(--text-disabled);
  }
  .weight {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-radius: var(--radius-l);
    background: var(--bg-well);
    border: 1px solid var(--border);
    color: var(--text-muted);
  }
  .weight div {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .weight .right {
    align-items: flex-end;
    text-align: right;
    color: var(--text-3);
  }
  .weight strong {
    /* « ≈ 216 Ko » sur une ligne, même quand la colonne de droite est longue. */
    white-space: nowrap;
    font-size: 20px;
    font-weight: 500;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    transition: opacity 120ms var(--ease);
  }
  .weight strong.stale {
    opacity: 0.45;
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
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
</style>
