<script lang="ts">
  import CategoryIcon from "./CategoryIcon.svelte";
  import { t } from "../lib/i18n.svelte";
  import { createDocument, runAction } from "../lib/actions";
  import { type Format, SIZE_SERIES, formatCategories, labelOf, shapeOf } from "../lib/formats";
  import { loadCustomFormats } from "../lib/custom-formats";
  import { type QuickFormat, type UsedFormat, describe, formatHistory, fromFormat, loadFormatHistory } from "../lib/recent-formats";
  import { app } from "../state/app.svelte";
  import PrintFields from "./print/PrintFields.svelte";
  import { DEFAULT_PRINT, UNITS, type Unit, formatLength, fromPx, loadPrintPrefs, mmToPx, printPrefs, round, setPref, toPx } from "../lib/print.svelte";
  import { colorSettings } from "../lib/color-settings.svelte";

  // Fenêtre Nouveau document (Ctrl+N) : séries d'écrans et de papier, les mêmes catégories que la page Formats de
  // l'accueil (dernier utilisé, essentiels, impression, réseaux sociaux, puis les catégories personnelles —
  // lib/formats.ts, lib/custom-formats.ts), ou une taille libre.
  const MAX_SIDE = 30000;
  const CUSTOM = "custom";
  const USED = "used";
  /** Une catégorie du catalogue se repère par « cat:<id> » dans la colonne. */
  const catKey = (id: string) => `cat:${id}`;

  let dialog = $state<HTMLDialogElement>();
  let formatId = $state("16:9");
  let sizeIndex = $state(2);
  let portrait = $state(false);
  let transparent = $state(false);
  // Nom par défaut posé à l'ouverture, dans la langue du moment (la boîte est créée avant le chargement de la
  // langue) ; un nom tapé par l'utilisateur est gardé d'une ouverture à l'autre.
  let name = $state("");
  let nameTyped = false;
  let custom = $state({ width: 1920, height: 1080 });
  /** Résolution du nouveau document : 300 ppp par défaut pour tous les modèles (un format personnel ou une entrée
   *  de l'historique reprend la sienne), modifiable. Pour un écran, elle ne change pas les pixels, seulement la
   *  taille imprimée. */
  let dpi = $state(300);
  /** Couleurs du document à la création, et réglages d'impression (fonds perdus, zone de sécurité, encrage). */
  let colors = $state<"rgb" | "cmyk">("rgb");
  let print = $state({ bleedOn: false, bleedMm: DEFAULT_PRINT.bleedMm, safeOn: false, safeMm: DEFAULT_PRINT.safeMm, inkOn: false, inkLimit: DEFAULT_PRINT.inkLimit });
  /** Unité des champs de la taille personnalisée (mémorisée). */
  const unit = $derived(printPrefs.unit);
  /** Champ de taille dans l'unité choisie : lu converti, écrit reconverti en pixels. */
  const inUnit = (px: number) => round(fromPx(px, unit, dpi), unit);
  const fromUnit = (value: number) => Math.max(1, Math.round(toPx(Number(value) || 0, unit, dpi)));
  /** Format choisi dans une catégorie du catalogue (ou clé d'une entrée de l'historique), et sa taille HD ou non. */
  let picked = $state<{ key: string; hd: boolean } | null>(null);

  // `$formatCategories` est lu pour qu'un format personnel renommé ou supprimé se mette à jour dans l'historique.
  const used = $derived.by(() => (void $formatCategories, $formatHistory.map(describe)));
  const series = $derived(SIZE_SERIES.find((f) => f.id === formatId) ?? null);
  const category = $derived($formatCategories.find((c) => catKey(c.id) === formatId) ?? null);
  const pickedFormat = $derived<Format | null>(category && picked ? (category.formats.find((f) => f.id === picked?.key) ?? null) : null);
  const pickedUsed = $derived<QuickFormat | null>(formatId === USED && picked ? (used.find((u) => u.key === picked?.key) ?? null) : null);

  const dimensions = $derived.by(() => {
    if (series) {
      const size = series.sizes[Math.min(sizeIndex, series.sizes.length - 1)];
      if (size) return portrait ? { width: size.short, height: size.long, dpi } : { width: size.long, height: size.short, dpi };
    }
    if (pickedFormat) {
      const size = picked?.hd && pickedFormat.hd ? pickedFormat.hd : pickedFormat;
      return { width: size.width, height: size.height, dpi };
    }
    if (pickedUsed) return { width: pickedUsed.width, height: pickedUsed.height, dpi };
    return { ...custom, dpi };
  });
  /** Ce qui sera retenu dans l'historique des formats : le format du catalogue choisi, s'il y en a un. */
  const usedFormat = $derived<UsedFormat | undefined>(
    pickedFormat
      ? { ...fromFormat(pickedFormat, picked?.hd ?? false), transparent }
      : pickedUsed
        ? { width: pickedUsed.width, height: pickedUsed.height, dpi: pickedUsed.dpi, transparent, ...(pickedUsed.formatId ? { formatId: pickedUsed.formatId } : {}), ...(pickedUsed.hd ? { hd: true } : {}) }
        : undefined,
  );
  const inRange = (value: number, max: number) => Number.isInteger(value) && value >= 1 && value <= max;
  const valid = $derived(name.trim() !== "" && inRange(dimensions.width, MAX_SIDE) && inRange(dimensions.height, MAX_SIDE) && inRange(dimensions.dpi, 9600));
  /** Aperçu : la feuille tient dans un carré de 210 px, proportions exactes. */
  const preview = $derived.by(() => {
    const scale = 150 / Math.max(dimensions.width, dimensions.height, 1);
    return { width: Math.max(4, Math.round(dimensions.width * scale)), height: Math.max(4, Math.round(dimensions.height * scale)) };
  });
  /** Orientation libre seulement pour une série ou une taille libre : un format de plateforme a la sienne. */
  const orientable = $derived(!!series || formatId === CUSTOM);

  /** Formats d'impression : papier, catégories Impression et Cartes de visite. */
  const isPrintFormat = (id: string) => id === "papier" || id === catKey("impression") || id === catKey("cartes");

  function printDefaults(id: string): void {
    const on = isPrintFormat(id);
    colors = on ? "cmyk" : "rgb";
    print = { ...print, bleedOn: on, safeOn: on, inkOn: on };
  }

  function chooseFormat(id: string): void {
    formatId = id;
    picked = null;
    if (id !== CUSTOM) printDefaults(id);
    dpi = 300;
    const next = SIZE_SERIES.find((f) => f.id === id);
    if (next) {
      portrait = next.portrait; // le papier démarre en portrait, les écrans en paysage
      sizeIndex = Math.min(2, next.sizes.length - 1);
      return;
    }
    const first = id === USED ? used[0] : $formatCategories.find((c) => catKey(c.id) === id)?.formats[0];
    if (first) pick("key" in first ? first.key : first.id, false);
  }

  function pick(key: string, hd: boolean): void {
    picked = { key, hd };
    const entry = formatId === USED ? used.find((u) => u.key === key) : undefined;
    if (entry) transparent = entry.transparent; // une entrée de l'historique recrée le même document, fond compris
    dpi = entry?.dpi ?? 300;
    const mine = category?.group === "custom" ? category.formats.find((f) => f.id === key) : undefined;
    if (mine) transparent = !!mine.transparent; // un format personnel a son propre fond
    if (mine?.custom) dpi = mine.dpi;
  }

  function setOrientation(value: boolean): void {
    portrait = value;
    // Taille libre : basculer l'orientation échange largeur et hauteur.
    if (formatId === CUSTOM && custom.width > custom.height === value) custom = { ...custom, width: custom.height, height: custom.width };
  }

  $effect(() => {
    if (app.newDocumentDialogOpen) {
      if (!nameTyped) name = t("Sans titre");
      void loadFormatHistory();
      void loadCustomFormats();
      void loadPrintPrefs();
    }
  });
  $effect(() => {
    if (app.newDocumentDialogOpen) dialog?.showModal();
    else dialog?.close();
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!valid) return;
    app.newDocumentDialogOpen = false;
    const anyPrint = print.bleedOn || print.safeOn || (print.inkOn && colors === "cmyk");
    const setup = anyPrint ? { bleedMm: print.bleedOn ? print.bleedMm : 0, safeMm: print.safeOn ? print.safeMm : 0, inkLimit: print.inkOn && colors === "cmyk" ? print.inkLimit : 0 } : null;
    await createDocument({ name: name.trim(), ...dimensions, transparent, print: setup, cmyk: colors === "cmyk" ? { ...colorSettings.cmyk } : null }, usedFormat);
  }

  function openFile(): void {
    app.newDocumentDialogOpen = false;
    runAction("file.open");
  }
</script>

{#snippet sheet(width: number, height: number, tone: string)}
  <span class="sheet" style:width="{width}px" style:height="{height}px" style:border-color={tone}></span>
{/snippet}

<dialog bind:this={dialog} onclose={() => (app.newDocumentDialogOpen = false)}>
  <form onsubmit={submit}>
    <header>
      <button type="button" class="icon" aria-label={t("Ouvrir un fichier…")} title={t("Ouvrir un fichier… (Ctrl+O)")} onclick={openFile}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h6l2 2h10v11H3z" /></svg>
      </button>
      <h2>{t("Nouveau document")}</h2>
      <button type="button" class="icon close" aria-label={t("Fermer")} onclick={() => (app.newDocumentDialogOpen = false)}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </header>

    <div class="body">
      <nav aria-label={t("Format")}>
        <span class="caption">{t("Proportions")}</span>
        {#each SIZE_SERIES as item (item.id)}
          <button type="button" class:on={item.id === formatId} onclick={() => chooseFormat(item.id)}>
            <span class="glyph">{@render sheet(item.icon[0], item.icon[1], "currentColor")}</span>
            {t(item.label)}
          </button>
        {/each}
        <span class="rule"></span>
        <span class="caption">{t("Catalogue")}</span>
        {#if used.length > 0}
          <button type="button" class:on={formatId === USED} onclick={() => chooseFormat(USED)}>
            <span class="glyph"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4a8 8 0 1 1-7.4 5M4 4v5h5M12 8v4l3 2" /></svg></span>
            <span class="label">{t("Récents")}</span><em>{used.length}</em>
          </button>
        {/if}
        {#each $formatCategories as item, index (item.id)}
          {#if index > 0 && item.group !== $formatCategories[index - 1]?.group}<span class="rule"></span>{#if item.group === "platform"}<span class="caption">{t("Réseaux sociaux")}</span>{:else}<span class="caption">{t("Mes formats")}</span>{/if}{/if}
          <button type="button" class:on={formatId === catKey(item.id)} onclick={() => chooseFormat(catKey(item.id))}>
            <span class="glyph"><CategoryIcon id={item.id} /></span>
            <span class="label">{labelOf(item, t)}</span><em>{item.formats.length}</em>
          </button>
        {/each}
        <div class="spacer"></div>
        <button type="button" class:on={formatId === CUSTOM} onclick={() => (formatId = CUSTOM)}>
          <span class="glyph"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 8V4h4M16 4h4v4M20 16v4h-4M8 20H4v-4" /></svg></span>
          {t("Personnalisé")}
        </button>
      </nav>

      <section class="sizes">
        <div class="line">
          <h3>{series ? t(series.title) : category ? labelOf(category, t) : formatId === USED ? t("Récents") : t("Taille personnalisée")}</h3>
          {#if orientable}
            <div class="segmented">
              <button type="button" class:on={!portrait} onclick={() => setOrientation(false)}>{@render sheet(16, 11, "currentColor")} {t("Paysage")}</button>
              <button type="button" class:on={portrait} onclick={() => setOrientation(true)}>{@render sheet(11, 16, "currentColor")} {t("Portrait")}</button>
            </div>
          {/if}
        </div>

        {#if category && category.formats.length === 0}
          <p class="hint">{t("Cette catégorie est vide : ajoutez-y des formats depuis la page Formats de l'accueil.")}</p>
        {:else if category}
          <!-- Formats du catalogue : la ligne choisit la taille Standard, le bouton HD la taille HD. -->
          {#each category.formats as item (item.id)}
            {@const box = shapeOf(item.width, item.height, 18)}
            {@const on = picked?.key === item.id}
            <div class="size catalog" class:on>
              <button type="button" class="hit" aria-label={labelOf(item, t)} onclick={() => pick(item.id, false)} ondblclick={() => dialog?.querySelector("form")?.requestSubmit()}></button>
              <span class="glyph"><span class="sheet" style:width={box.width} style:height={box.height}></span></span>
              <span class="stack">
                <span class="name">{labelOf(item, t)}</span>
                <span class="small" title={item.note ? t(item.note) : undefined}>{item.width} × {item.height} px{item.custom ? ` · ${item.dpi} ppp` : ""}{item.transparent ? ` · ${t("transparent")}` : ""}{item.note ? ` · ${t(item.note)}` : ""}</span>
              </span>
              <span class="variants">
                <button type="button" class="variant" class:on={on && !picked?.hd} onclick={() => pick(item.id, false)}>{t("Standard")}</button>
                {#if item.hd}<button type="button" class="variant" class:on={on && picked?.hd} title={`${item.hd.width} × ${item.hd.height} px`} onclick={() => pick(item.id, true)}>HD</button>{/if}
              </span>
            </div>
          {/each}
        {:else if formatId === USED}
          {#each used as item (item.key)}
            <button type="button" class="size" class:on={picked?.key === item.key} onclick={() => pick(item.key, false)} ondblclick={() => dialog?.querySelector("form")?.requestSubmit()}>
              <span class="stack">
                <span class="name">{item.format ? `${labelOf(item.format, t)}${item.hd ? " HD" : ""}` : t("Personnalisé")}</span>
                <span class="small">{item.width} × {item.height} px · {item.dpi} ppp</span>
              </span>
              <span class="detail">{item.transparent ? t("Transparent") : t("Blanc")}</span>
            </button>
          {/each}
        {:else if series}
          {#each series.sizes as size, index (size.name)}
            <button type="button" class="size" class:on={index === sizeIndex} onclick={() => (sizeIndex = index)} ondblclick={() => dialog?.querySelector("form")?.requestSubmit()}>
              <span class="name">{t(size.name)}</span>
              <span class="pixels">{portrait ? size.short : size.long} × {portrait ? size.long : size.short} px</span>
              <span class="detail">{size.detail ? t(size.detail) : ""}</span>
            </button>
          {/each}
        {:else}
          <div class="fields">
            <label>{t("Largeur")} <input type="number" min="0" step="any" bind:value={() => inUnit(custom.width), (v) => (custom.width = fromUnit(v))} /></label>
            <label>{t("Hauteur")} <input type="number" min="0" step="any" bind:value={() => inUnit(custom.height), (v) => (custom.height = fromUnit(v))} /></label>
            <label
              >{t("Unité")}
              <select value={unit} onchange={(e) => setPref("unit", (e.currentTarget as HTMLSelectElement).value as Unit)}>
                {#each UNITS as u (u.id)}<option value={u.id}>{t(u.label)}</option>{/each}
              </select>
            </label>
            <label>{t("Résolution (ppp)")} <input type="number" min="1" max="9600" bind:value={dpi} /></label>
          </div>
          <p class="hint">
            {t("De 1 à {MAX_SIDE} px par côté.", { MAX_SIDE })}
            {#if unit !== "px"}· {formatLength(custom.width, unit, dpi)} × {formatLength(custom.height, unit, dpi)} = {custom.width} × {custom.height} px{/if}
          </p>
        {/if}
      </section>

      <aside>
        <span class="caption">{t("Aperçu")}</span>
        <div class="stage">
          <span class="paper" class:transparent style:width="{preview.width}px" style:height="{preview.height}px">
            {#if print.bleedOn}<span class="frame bleed" style:inset="-{Math.max(3, Math.round((mmToPx(print.bleedMm, dimensions.dpi) * preview.width) / Math.max(1, dimensions.width)))}px"></span>{/if}
            {#if print.safeOn}<span class="frame safe" style:inset="{Math.max(3, Math.round((mmToPx(print.safeMm, dimensions.dpi) * preview.width) / Math.max(1, dimensions.width)))}px"></span>{/if}
          </span>
        </div>
        <dl>
          <dt>{t("Dimensions")}</dt>
          <dd>{dimensions.width || 0} × {dimensions.height || 0} px</dd>
          {#if formatId !== CUSTOM}
            <dt>{t("Résolution")}</dt>
            <dd><label class="dpi"><input type="number" min="1" max="9600" bind:value={dpi} aria-label={t("Résolution")} /><em>{t("ppp")}</em></label></dd>
          {/if}
          {#if print.bleedOn && dimensions.dpi > 0}
            <dt>{t("Avec fonds perdus")}</dt>
            <dd>{dimensions.width + 2 * mmToPx(print.bleedMm, dimensions.dpi)} × {dimensions.height + 2 * mmToPx(print.bleedMm, dimensions.dpi)} px</dd>
          {/if}
          <dt>{t("Orientation")}</dt>
          <dd>{dimensions.height > dimensions.width ? t("Portrait") : dimensions.height === dimensions.width ? t("Carré") : t("Paysage")}</dd>
        </dl>
        <div class="line">
          <span class="muted">{t("Fond")}</span>
          <div class="segmented">
            <button type="button" class:on={!transparent} onclick={() => (transparent = false)}>{t("Blanc")}</button>
            <button type="button" class:on={transparent} onclick={() => (transparent = true)}>{t("Transparent")}</button>
          </div>
        </div>
        <div class="line">
          <span class="muted">{t("Couleurs")}</span>
          <div class="segmented">
            <button type="button" class:on={colors === "rgb"} onclick={() => (colors = "rgb")}>{t("RVB")}</button>
            <button type="button" class:on={colors === "cmyk"} onclick={() => (colors = "cmyk")}>{t("CMJN")}</button>
          </div>
        </div>
        <div class="print">
          <span class="caption">{t("Impression")}</span>
          <PrintFields bind:bleedOn={print.bleedOn} bind:bleedMm={print.bleedMm} bind:safeOn={print.safeOn} bind:safeMm={print.safeMm} bind:inkOn={print.inkOn} bind:inkLimit={print.inkLimit} inkEnabled={colors === "cmyk"} />
        </div>
      </aside>
    </div>

    <footer>
      <label class="inline">{t("Nom")} <input type="text" bind:value={name} oninput={() => (nameTyped = true)} /></label>
      <div class="spacer"></div>
      <button type="button" class="ghost" onclick={() => (app.newDocumentDialogOpen = false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!valid}>{t("Créer")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 1010px;
    max-width: calc(100vw - 32px);
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
  form {
    display: flex;
    flex-direction: column;
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
  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  h3 {
    flex: 1;
    margin: 0;
    font-size: 13px;
    font-weight: 500;
  }
  svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .icon {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-2);
  }
  .icon:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .close {
    color: var(--text-muted);
  }
  .body {
    height: min(580px, calc(100vh - 180px));
    display: flex;
  }
  nav {
    width: 260px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 10px;
    border-right: 1px solid var(--border);
  }
  .caption {
    padding: 8px 10px 6px;
    color: var(--text-muted);
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  nav {
    overflow-y: auto;
  }
  nav button {
    height: 38px;
    white-space: nowrap;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    font-size: 13px;
    color: var(--text-3);
    text-align: left;
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.on,
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .glyph {
    width: 26px;
    height: 18px;
    display: grid;
    place-items: center;
  }
  .sheet {
    display: block;
    border: 1.5px solid;
    border-radius: var(--radius-xs);
  }
  .spacer {
    flex: 1;
  }
  /* Un nom de catégorie personnelle peut être long : il s'arrête avant le compte. */
  nav .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  nav em {
    margin-left: auto;
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .rule {
    height: 1px;
    flex-shrink: 0;
    margin: 4px 10px;
    background: var(--border);
  }
  /* Ligne d'un format du catalogue : le bouton `.hit` couvre la ligne, les boutons Standard / HD sont posés dessus. */
  .size.catalog {
    position: relative;
  }
  .hit {
    position: absolute;
    inset: 0;
    border-radius: inherit;
  }
  .size.catalog > :not(.hit) {
    position: relative;
    pointer-events: none;
  }
  .variants {
    display: flex;
    gap: 4px;
  }
  .variant {
    height: 24px;
    padding: 0 8px;
    pointer-events: auto;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    font-size: 11px;
    color: var(--text-2);
  }
  .variant.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .stack {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .stack .name {
    width: auto;
  }
  .small {
    font-size: 11.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sizes {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 18px 20px;
    overflow-y: auto;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }
  aside .line {
    justify-content: space-between;
    margin: 0;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-well);
    border: 1px solid var(--border);
  }
  .segmented button {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .size {
    height: 48px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 14px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border);
    text-align: left;
    transition: border-color 120ms var(--ease);
  }
  .size:hover {
    border-color: var(--border-strong);
  }
  .size.on {
    background: rgb(var(--ink) / 0.08);
    border-color: rgb(var(--ink) / 0.22);
  }
  .name {
    width: 92px;
    font-size: 13px;
    font-weight: 500;
  }
  .pixels {
    flex: 1;
    color: var(--text-2);
  }
  .detail,
  .muted,
  .hint {
    color: var(--text-muted);
  }
  .hint {
    margin: 6px 0 0;
  }
  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-3);
  }
  label.inline {
    flex-direction: row;
    align-items: center;
    gap: 10px;
  }
  input {
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .inline input {
    width: 240px;
  }
  aside {
    width: 290px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 20px;
    background: var(--bg-bar);
    border-left: 1px solid var(--border);
  }
  aside .caption {
    padding: 0;
  }
  aside {
    overflow-y: auto;
  }
  .print {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  aside .segmented button {
    min-width: 64px;
    justify-content: center;
  }
  select {
    height: 32px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
  }
  .paper {
    position: relative;
  }
  dd .dpi {
    position: relative;
    display: block;
    width: 96px;
  }
  dd .dpi input {
    width: 100%;
    height: 28px;
    padding: 0 30px 0 8px;
    text-align: right;
  }
  dd .dpi em {
    position: absolute;
    right: 8px;
    top: 50%;
    transform: translateY(-50%);
    font-style: normal;
    font-size: 11px;
    color: var(--text-muted);
    pointer-events: none;
  }
  dl {
    align-items: center;
  }
  .frame {
    position: absolute;
    pointer-events: none;
    border: 1px dashed;
  }
  .frame.bleed {
    border-color: #ff4d5e;
  }
  .frame.safe {
    border-color: #4aa3ff;
  }
  .stage {
    height: 190px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-l);
    background: var(--bg-canvas);
    border: 1px solid var(--border);
  }
  .paper {
    background: #f4f4f5;
    border-radius: var(--radius-xs);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    transition:
      width 160ms var(--ease),
      height 160ms var(--ease);
  }
  .paper.transparent {
    background: repeating-conic-gradient(#d0d0d0 0% 25%, #ffffff 0% 50%) 0 0 / 14px 14px;
  }
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 8px;
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
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
    opacity: 0.35;
  }
</style>
