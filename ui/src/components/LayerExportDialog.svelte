<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { describeError } from "../lib/errors";
  import { onMount, untrack } from "svelte";
  import { api } from "../lib/ipc";
  import {
    LAYER_FORMATS,
    chooseFolder,
    exportLayersWith,
    extensionOf,
    loadLayerExportSettings,
    optionsOf,
    saveLayerExportSettings,
    scaleOf,
    withProgress,
    type ExportSize,
    type LayerExportSettings,
    type LayerExportZone,
  } from "../lib/layer-export";
  import { app } from "../state/app.svelte";

  // « Exporter sous… » d'un calque ou d'un groupe : format, zone, marge, plusieurs tailles d'un coup, dossier.
  // L'aperçu est le rendu du moteur (calques isolés, effets compris) ; les réglages sont mémorisés pour l'export rapide.
  const request = untrack(() => app.layerExport);
  const doc = untrack(() => app.active);
  const all = untrack(() => (request && doc ? request.ids.map((id) => doc.info.layers.find((l) => l.id === id)).filter((l) => l !== undefined) : []));
  /** Plans de travail : chacun se coche ou se décoche ; leur zone est leur cadre. */
  const boardsMode = untrack(() => request?.zone === "artboard");
  let checked = $state(new Set(untrack(() => all.map((l) => l.id))));
  const layers = $derived(boardsMode ? all.filter((l) => checked.has(l.id)) : all);
  const hasSelection = untrack(() => doc?.info.hasSelection ?? false);

  let dialog = $state<HTMLDialogElement>();
  let settings = $state<LayerExportSettings | null>(null);
  let remember = $state(true);
  let preview = $state<{ url: string; width: number; height: number } | null>(null);
  let failure = $state<string | null>(null);
  let folderMissing = $state(false);

  const title = $derived(boardsMode ? t("Exporter les plans de travail") : layers.length === 1 ? t("Exporter « {name} »", { name: layers[0]!.name }) : request?.separate ? t("Exporter {n} calques séparément", { n: layers.length }) : t("Exporter {n} calques", { n: layers.length }));
  const spec = $derived(LAYER_FORMATS.find((f) => f.id === settings?.format) ?? LAYER_FORMATS[0]!);
  const lossy = $derived(settings?.format === "jpeg" || settings?.format === "webp" || settings?.format === "avif");
  const stem = (name: string) => name.replace(/[\\/:*?"<>|]/g, "_").trim() || "calque";
  /** Nombre de fichiers écrits. */
  const fileCount = $derived((settings?.sizes.length ?? 0) * (request?.separate ? layers.length : 1));

  onMount(() => {
    dialog?.showModal();
    void loadLayerExportSettings().then((loaded) => {
      if (loaded.zone === "selection" && !hasSelection) loaded.zone = "content";
      if (loaded.zone === "artboard") loaded.zone = "content";
      settings = loaded;
    });
  });

  // Aperçu : le premier calque (ou l'ensemble), recalculé quand la zone, la marge ou le format changent.
  $effect(() => {
    if (!settings || !doc || layers.length === 0) return;
    const ids = request?.separate ? [layers[0]!.id] : layers.map((l) => l.id);
    const zone = boardsMode ? "artboard" : settings.zone;
    const margin = settings.margin;
    const options = optionsOf($state.snapshot(settings));
    let alive = true;
    const timer = setTimeout(() => {
      api
        .layerExportOverview(doc.info.id, ids, zone, margin, options, 600)
        .then((buffer) => {
          if (!alive) return;
          const view = new DataView(buffer);
          const [width, height] = [view.getUint32(0, true), view.getUint32(4, true)];
          const kind = view.getUint8(16);
          const blob = new Blob([buffer.slice(17)], { type: kind === 1 ? "image/jpeg" : kind === 2 ? "image/webp" : "image/png" });
          if (preview) URL.revokeObjectURL(preview.url);
          preview = { url: URL.createObjectURL(blob), width, height };
          failure = null;
        })
        .catch((error: unknown) => alive && (failure = describeError(error)));
    }, 120);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  });

  // Vérifié après un temps de repos de la frappe ; une réponse dépassée par une frappe plus récente est ignorée.
  $effect(() => {
    const folder = settings?.folder;
    if (!folder) return void (folderMissing = false);
    let current = true;
    const timer = setTimeout(() => {
      api.directoryExists(folder).then((exists) => {
        if (current) folderMissing = !exists;
      }, () => undefined);
    }, 150);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  });

  function sizeLabel(size: ExportSize): string {
    if (size.kind === "width") return t("{n} px de large", { n: size.value });
    if (size.kind === "height") return t("{n} px de haut", { n: size.value });
    return `${String(size.value).replace(".", ",")}×`;
  }
  function pixelsOf(size: ExportSize): string {
    if (!preview) return "…";
    const k = scaleOf(size, preview.width, preview.height);
    return `${Math.max(1, Math.round(preview.width * k))} × ${Math.max(1, Math.round(preview.height * k))}`;
  }
  /** Taille suivante proposée par « Ajouter une taille » : 2×, 3×, puis 0,5×. */
  function addSize(): void {
    if (!settings) return;
    const used = new Set(settings.sizes.filter((s) => s.kind === "scale").map((s) => s.value));
    const value = [2, 3, 0.5, 4, 1].find((v) => !used.has(v)) ?? 1;
    settings.sizes.push({ kind: "scale", value, suffix: value === 1 ? "" : value < 1 ? `@${String(value).replace(".", "_")}x` : `@${value}x` });
  }
  function setKind(size: ExportSize, kind: ExportSize["kind"]): void {
    if (kind === size.kind || !preview) return;
    const k = scaleOf(size, preview.width, preview.height);
    size.kind = kind;
    size.value = kind === "scale" ? Math.round(k * 100) / 100 : Math.max(1, Math.round((kind === "width" ? preview.width : preview.height) * k));
    if (kind !== "scale") size.suffix = `-${size.value}`;
  }

  async function pickFolder(): Promise<void> {
    if (!settings) return;
    const folder = await chooseFolder(settings.folder);
    if (folder) settings.folder = folder;
  }

  function close(): void {
    if (preview) URL.revokeObjectURL(preview.url);
    app.layerExport = null;
  }

  async function exportNow(): Promise<void> {
    if (!settings || !doc || !preview) return;
    if (!settings.folder || folderMissing) {
      await pickFolder();
      if (!settings.folder) return;
    }
    const chosen = { ...($state.snapshot(settings) as LayerExportSettings), ...(boardsMode ? { zone: "artboard" as const } : {}) };
    const size = { width: preview.width, height: preview.height };
    if (remember) await saveLayerExportSettings({ ...chosen, zone: boardsMode ? settings.zone : chosen.zone });
    close();
    try {
      await withProgress(async () => {
        if (request?.separate) {
          // Chaque calque a sa propre zone : sa taille à 1× est relue avant les tailles fixes en pixels.
          for (const layer of layers) {
            const buffer = await api.layerExportOverview(doc.info.id, [layer.id], chosen.zone, chosen.margin, optionsOf(chosen), 64);
            const view = new DataView(buffer);
            await exportLayersWith([layer.id], layer.name, chosen, view.getUint32(0, true), view.getUint32(4, true));
          }
        } else {
          await exportLayersWith(layers.map((l) => l.id), layers[0]?.name ?? "calque", chosen, size.width, size.height);
        }
      });
    } catch (error) {
      app.fail(error);
    }
  }

  const ZONES: { id: LayerExportZone; label: string }[] = [
    { id: "content", label: "Contenu" },
    { id: "document", label: "Document" },
    { id: "selection", label: "Sélection" },
  ];
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close())}>
  <form novalidate onsubmit={(event) => (event.preventDefault(), void exportNow())}>
    <header>
      <h2>{title}</h2>
      <div class="spacer"></div>
      <button type="button" class="icon" aria-label={t("Fermer")} onclick={close}>✕</button>
    </header>
    {#if settings}
      <div class="body">
        <div class="left">
          <div class="pane">
            {#if preview}<img src={preview.url} alt="" draggable="false" />{/if}
            {#if failure}<p class="failure">{failure}</p>{/if}
          </div>
          <div class="line muted">
            <span>{preview ? t("Zone : {width} × {height} px", { width: preview.width, height: preview.height }) : "…"}</span>
            <span>{spec.alpha ? t("transparence conservée") : t("fond blanc")}</span>
          </div>
          {#if boardsMode}
            <div class="boards">
              {#each all as board (board.id)}
                <label class="check"><input type="checkbox" checked={checked.has(board.id)} onchange={(e) => { const next = new Set(checked); if (e.currentTarget.checked) next.add(board.id); else next.delete(board.id); checked = next; }} /> {board.name}</label>
              {/each}
            </div>
          {:else}
          <div class="line">
            <span class="label">{t("Zone")}</span>
            <div class="segmented">
              {#each ZONES as zone (zone.id)}
                <button type="button" class:on={settings.zone === zone.id} disabled={zone.id === "selection" && !hasSelection} onclick={() => settings && (settings.zone = zone.id)}>{t(zone.label)}</button>
              {/each}
            </div>
          </div>
          <label class="line">
            <span class="label">{t("Marge")}</span>
            <input type="number" min="0" max="4096" step="1" bind:value={settings.margin} />
            <span class="muted">px</span>
          </label>
          {/if}
        </div>

        <div class="right">
          <section>
            <h4>{t("Format")}</h4>
            <div class="segmented">
              {#each LAYER_FORMATS as item (item.id)}
                <button type="button" class:on={item.id === settings.format} onclick={() => settings && (settings.format = item.id)}>{item.label}</button>
              {/each}
            </div>
            {#if lossy}
              <label class="line">
                <span class="label">{t("Qualité")}</span>
                <input type="range" min="1" max="100" bind:value={settings.quality} />
                <output>{settings.quality}</output>
              </label>
            {/if}
          </section>

          <section>
            <h4>{t("Tailles")}</h4>
            <table>
              <thead><tr><th>{t("Échelle")}</th><th>{t("Suffixe")}</th><th>{t("Pixels")}</th><th></th></tr></thead>
              <tbody>
                {#each settings.sizes as size, index (index)}
                  <tr>
                    <td class="scale">
                      <select value={size.kind} onchange={(event) => setKind(size, event.currentTarget.value as ExportSize["kind"])} aria-label={t("Type de taille")}>
                        <option value="scale">{t("Échelle")}</option>
                        <option value="width">{t("Largeur")}</option>
                        <option value="height">{t("Hauteur")}</option>
                      </select>
                      <input type="number" min={size.kind === "scale" ? 0.05 : 1} max={size.kind === "scale" ? 8 : 16000} step={size.kind === "scale" ? "any" : 1} bind:value={size.value} aria-label={sizeLabel(size)} />
                    </td>
                    <td><input class="suffix" type="text" spellcheck="false" bind:value={size.suffix} placeholder={t("aucun")} /></td>
                    <td class="muted">{pixelsOf(size)}</td>
                    <td><button type="button" class="remove" disabled={settings.sizes.length === 1} aria-label={t("Retirer cette taille")} onclick={() => settings?.sizes.splice(index, 1)}>✕</button></td>
                  </tr>
                {/each}
              </tbody>
            </table>
            <button type="button" class="add" onclick={addSize}>＋ {t("Ajouter une taille")}</button>
            <p class="muted">{request?.separate ? t("Un fichier par calque, nommé d'après lui : {example}", { example: `${stem(layers[0]?.name ?? "")}${settings.sizes[0]?.suffix ?? ""}.${extensionOf(settings.format)}` }) : `${stem(layers[0]?.name ?? "")}${settings.sizes[0]?.suffix ?? ""}.${extensionOf(settings.format)}`}</p>
          </section>

          <section>
            <h4>{t("Destination")}</h4>
            <div class="line">
              <input class="folder" type="text" readonly value={settings.folder || t("À choisir")} class:empty={!settings.folder} title={settings.folder} />
              <button type="button" class="ghost" onclick={pickFolder}>{t("Choisir…")}</button>
            </div>
            {#if folderMissing}<p class="warn">{t("Ce dossier n'existe plus : choisissez-en un autre.")}</p>{/if}
            <label class="check"><input type="checkbox" bind:checked={remember} /> {t("Mémoriser pour l'export rapide")}</label>
          </section>
        </div>
      </div>
    {/if}
    <footer>
      <span>{t("{n} fichier(s)", { n: fileCount })}</span>
      <div class="spacer"></div>
      <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!settings || !preview}>{t("Exporter")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(860px, calc(100vw - 48px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 18px;
  }
  header {
    height: 50px;
    border-bottom: 1px solid var(--border);
  }
  footer {
    height: 56px;
    border-top: 1px solid var(--border);
  }
  footer span,
  .muted {
    color: var(--text-muted);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
  }
  h4 {
    margin: 0 0 8px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .body {
    display: flex;
    min-height: 420px;
  }
  .left {
    width: 340px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    border-right: 1px solid var(--border);
  }
  .pane {
    position: relative;
    height: 260px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    background: repeating-conic-gradient(#2a2a2d 0% 25%, #232326 0% 50%) 0 0 / 16px 16px;
    overflow: hidden;
  }
  .pane img {
    width: calc(100% - 24px);
    height: calc(100% - 24px);
    object-fit: contain;
  }
  .failure {
    position: absolute;
    inset: auto 10px 10px;
    margin: 0;
    color: var(--danger, #ff6b6b);
  }
  .right {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  section {
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  section:last-child {
    border-bottom: none;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
    justify-content: space-between;
  }
  .label {
    width: 56px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  .line input[type="number"] {
    width: 80px;
  }
  .line input[type="range"] {
    flex: 1;
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
    height: 28px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .segmented button:disabled {
    opacity: 0.4;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    font-weight: 400;
    color: var(--text-muted);
    padding: 4px;
  }
  td {
    padding: 4px;
    border-top: 1px solid var(--border);
  }
  td.scale {
    display: flex;
    gap: 4px;
  }
  td.scale select {
    width: 96px;
  }
  td.scale input {
    width: 70px;
  }
  .suffix {
    width: 110px;
  }
  .remove {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
  }
  .remove:disabled {
    opacity: 0.3;
  }
  .add {
    align-self: flex-start;
    color: var(--accent, #e5343e);
  }
  .folder {
    flex: 1;
  }
  .folder.empty {
    color: var(--text-muted);
  }
  .warn {
    margin: 0;
    color: var(--danger, #ff6b6b);
  }
  p {
    margin: 0;
  }
  .boards {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 120px;
    overflow-y: auto;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
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
  .pill:disabled {
    opacity: 0.5;
  }
</style>
