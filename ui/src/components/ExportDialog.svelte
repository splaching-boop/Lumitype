<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { listen } from "@tauri-apps/api/event";
  import { save } from "@tauri-apps/plugin-dialog";
  import { untrack } from "svelte";
  import { inSaveFolder } from "../lib/actions";
  import { type ExportFormat, type ExportOptions, type ImageMetadata, api } from "../lib/ipc";
  import { authorName, buildMetadata } from "../lib/metadata";
  import { app } from "../state/app.svelte";

  // Fenêtre d'export. L'aperçu montre le VRAI fichier : l'image est encodée par le moteur avec les
  // réglages courants, affichée telle quelle, et son poids est celui, exact, du fichier qui sera écrit.
  const FORMATS: { id: ExportFormat; label: string; extension: string; hint: string; alpha: boolean }[] = [
    { id: "png", label: "PNG", extension: "png", hint: t("Sans perte, avec transparence. Le choix par défaut."), alpha: true },
    { id: "jpeg", label: "JPEG", extension: "jpg", hint: t("Compact, sans transparence (fond blanc) : idéal pour les photos."), alpha: false },
    { id: "webp", label: "WebP", extension: "webp", hint: t("Léger, avec transparence : le meilleur format pour le web."), alpha: true },
    { id: "tiff", label: "TIFF", extension: "tif", hint: t("Sans perte ni compression : impression et archivage."), alpha: true },
  ];
  const SCALES = [25, 50, 100, 150, 200];

  const doc = untrack(() => app.active);
  const native = { width: doc?.info.width ?? 1, height: doc?.info.height ?? 1 };

  let dialog = $state<HTMLDialogElement>();
  let pane = $state<HTMLDivElement>();
  let name = $state(untrack(() => doc?.info.name ?? "Sans titre"));
  let format = $state<ExportFormat>("png");
  let quality = $state(85);
  /** WebP seulement : sans perte (le curseur de qualité est alors sans effet). */
  let lossless = $state(false);
  let percent = $state(100);
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
  /** Le curseur de qualité agit sur le JPEG et le WebP avec perte. */
  const lossy = $derived(format === "jpeg" || (format === "webp" && !lossless));
  const options = $derived<ExportOptions>({ format, quality, lossless: format === "webp" && lossless, scale: percent / 100, metadata: metadataNow() });

  $effect(() => {
    dialog?.showModal();
  });

  // Aperçu instantané : l'image est composée une fois (gardée tant que le document et la taille ne changent pas),
  // puis chaque réglage ne demande qu'une vue d'ensemble légère et un poids estimé. Le poids exact suit en coulisse.
  // Le moteur dit comment il a encodé l'aperçu : 0 = PNG, 1 = JPEG, 2 = WebP (formats avec perte montrés tels quels).
  const mimeOf = (kind: number) => (kind === 1 ? "image/jpeg" : kind === 2 ? "image/webp" : "image/png");
  let request = 0;
  $effect(() => {
    const current = $state.snapshot(options);
    if (!doc) return;
    const mine = ++request;
    computing = true;
    let exactTimer: ReturnType<typeof setTimeout> | undefined;
    const timer = setTimeout(() => {
      const side = Math.round(Math.min(2048, Math.max(pane?.clientWidth ?? 800, pane?.clientHeight ?? 600) * (window.devicePixelRatio || 1) * 1.25));
      api.exportOverview(doc.info.id, current, side).then(
        (buffer) => {
          if (mine !== request) return;
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
          // Poids exact : l'image entière encodée, seulement si les réglages ne bougent plus.
          if (!exact) {
            exactTimer = setTimeout(() => {
              api.exportExactSize(doc.info.id, current).then(
                (exactBytes) => mine === request && (size = { bytes: exactBytes, exact: true }),
                () => undefined,
              );
            }, 600);
          }
        },
        (error: unknown) => {
          if (mine === request) [computing, failure] = [false, String(error)];
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
          if (mine !== detailRequest || result !== overview) return;
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

  const weight = (bytes: number) => (bytes >= 1048576 ? `${(bytes / 1048576).toFixed(1).replace(".", ",")} Mo` : `${Math.max(1, Math.round(bytes / 1024))} Ko`);

  function close(): void {
    if (result) URL.revokeObjectURL(result.url);
    dropDetail();
    void api.exportPreviewRelease().catch(() => undefined);
    app.exportOpen = false;
  }

  async function exportNow(): Promise<void> {
    if (!doc) return;
    // La date inscrite est celle de l'export, pas celle du dernier réglage.
    const chosen: ExportOptions = { ...$state.snapshot(options), metadata: metadataNow() };
    const extension = spec?.extension ?? "png";
    const base = name.trim() || "Sans titre";
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

<svelte:window onkeydown={onModifier} onkeyup={onModifier} onblur={() => (modifier = null)} />

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close())}>
  <form onsubmit={(event) => (event.preventDefault(), void exportNow())}>
    <header>
      <h2>{t("Exporter")}</h2>
      <span>{doc?.info.name} · {native.width} × {native.height} px</span>
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
        <div class="chip top-right">{computing ? "Encodage…" : `Aperçu du rendu ${spec?.label ?? ""}`}</div>
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

        <div class="field">
          <span>{t("Format")}</span>
          <div class="segmented">
            {#each FORMATS as item (item.id)}
              <button type="button" class:on={item.id === format} onclick={() => (format = item.id)}>{t(item.label)}</button>
            {/each}
          </div>
          <p>{t(spec?.hint ?? "")}</p>
        </div>

        {#if format === "jpeg" || format === "webp"}
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
            <span>{t(spec?.label ?? "")} · {spec?.alpha ? t("transparence conservée") : t("fond blanc")}</span>
            <span>{t("{width} × {height} px", { width: target.width, height: target.height })}</span>
          </div>
        </div>
      </aside>
    </div>

    <footer>
      <span>{size && !size.exact ? t("Poids estimé sur une partie de l'image ; le poids exact suit dans un instant.") : t("Le poids est celui du fichier réellement encodé avec ces réglages.")}</span>
      <div class="spacer"></div>
      <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!doc}>{t("Exporter…")}</button>
    </footer>
  </form>
</dialog>

<style>
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
    background: #0f0f11;
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
    color: #fff;
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
    border-color: rgba(255, 255, 255, 0.22);
    color: #fff;
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
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.22);
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
    background: #0f0f11;
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
