<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { createDocument, runAction } from "../lib/actions";
  import { app } from "../state/app.svelte";

  // Fenêtre d'accueil : formats standard (écrans, papier) ou taille libre. S'ouvre au lancement et sur Ctrl+N.
  const MAX_SIDE = 30000;

  interface Size {
    name: string;
    /** Grand et petit côté en pixels ; l'orientation décide lequel est la largeur. */
    long: number;
    short: number;
    detail?: string;
  }
  interface Format {
    id: string;
    label: string;
    title: string;
    dpi: number;
    portrait: boolean;
    /** Proportions de la petite icône de la liste. */
    icon: [number, number];
    sizes: Size[];
  }

  const screen = (name: string, long: number, short: number): Size => ({ name, long, short });
  const paper = (name: string, short: number, long: number, detail: string): Size => ({ name, long, short, detail });
  const FORMATS: Format[] = [
    { id: "16:9", label: "16:9", title: t("Écran 16:9"), dpi: 72, portrait: false, icon: [24, 14], sizes: [screen("SD", 854, 480), screen("HD", 1280, 720), screen(t("Full HD"), 1920, 1080), screen("QHD", 2560, 1440), screen("UHD 4K", 3840, 2160)] },
    { id: "16:10", label: "16:10", title: t("Écran 16:10"), dpi: 72, portrait: false, icon: [22, 14], sizes: [screen("WXGA", 1280, 800), screen("WXGA+", 1440, 900), screen("WSXGA+", 1680, 1050), screen("WUXGA", 1920, 1200), screen("WQXGA", 2560, 1600)] },
    { id: "21:9", label: "21:9", title: t("Écran 21:9"), dpi: 72, portrait: false, icon: [26, 11], sizes: [screen(t("UW Full HD"), 2560, 1080), screen("UW QHD", 3440, 1440), screen("UW 5K", 5120, 2160)] },
    // Formats ISO 216 à 300 ppp.
    { id: "papier", label: "Papier", title: t("Papier · 300 ppp"), dpi: 300, portrait: true, icon: [13, 18], sizes: [paper("A2", 4961, 7016, t("420 × 594 mm")), paper("A3", 3508, 4961, t("297 × 420 mm")), paper("A4", 2480, 3508, t("210 × 297 mm")), paper("A5", 1748, 2480, t("148 × 210 mm")), paper("A6", 1240, 1748, t("105 × 148 mm"))] },
  ];
  const CUSTOM = "custom";

  let dialog = $state<HTMLDialogElement>();
  let formatId = $state("16:9");
  let sizeIndex = $state(2);
  let portrait = $state(false);
  let transparent = $state(false);
  let name = $state("Sans titre");
  let custom = $state({ width: 1920, height: 1080, dpi: 72 });

  const format = $derived(FORMATS.find((f) => f.id === formatId) ?? null);
  const dimensions = $derived.by(() => {
    const size = format?.sizes[Math.min(sizeIndex, format.sizes.length - 1)];
    if (!format || !size) return { ...custom };
    return portrait ? { width: size.short, height: size.long, dpi: format.dpi } : { width: size.long, height: size.short, dpi: format.dpi };
  });
  const inRange = (value: number, max: number) => Number.isInteger(value) && value >= 1 && value <= max;
  const valid = $derived(name.trim() !== "" && inRange(dimensions.width, MAX_SIDE) && inRange(dimensions.height, MAX_SIDE) && inRange(dimensions.dpi, 9600));
  /** Aperçu : la feuille tient dans un carré de 210 px, proportions exactes. */
  const preview = $derived.by(() => {
    const scale = 210 / Math.max(dimensions.width, dimensions.height, 1);
    return { width: Math.max(4, Math.round(dimensions.width * scale)), height: Math.max(4, Math.round(dimensions.height * scale)) };
  });

  function chooseFormat(id: string): void {
    formatId = id;
    const next = FORMATS.find((f) => f.id === id);
    if (!next) return;
    portrait = next.portrait; // le papier démarre en portrait, les écrans en paysage
    sizeIndex = Math.min(2, next.sizes.length - 1);
  }

  function setOrientation(value: boolean): void {
    portrait = value;
    // Taille libre : basculer l'orientation échange largeur et hauteur.
    if (!format && custom.width > custom.height === value) custom = { ...custom, width: custom.height, height: custom.width };
  }

  $effect(() => {
    if (app.newDocumentDialogOpen) dialog?.showModal();
    else dialog?.close();
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!valid) return;
    app.newDocumentDialogOpen = false;
    await createDocument({ name: name.trim(), ...dimensions, transparent });
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
        <span class="caption">{t("Format")}</span>
        {#each FORMATS as item (item.id)}
          <button type="button" class:on={item.id === formatId} onclick={() => chooseFormat(item.id)}>
            <span class="glyph">{@render sheet(item.icon[0], item.icon[1], "currentColor")}</span>
            {t(item.label)}
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
          <h3>{format?.title ?? t("Taille personnalisée")}</h3>
          <div class="segmented">
            <button type="button" class:on={!portrait} onclick={() => setOrientation(false)}>{@render sheet(16, 11, "currentColor")} {t("Paysage")}</button>
            <button type="button" class:on={portrait} onclick={() => setOrientation(true)}>{@render sheet(11, 16, "currentColor")} {t("Portrait")}</button>
          </div>
        </div>

        {#if format}
          {#each format.sizes as size, index (size.name)}
            <button type="button" class="size" class:on={index === sizeIndex} onclick={() => (sizeIndex = index)} ondblclick={() => dialog?.querySelector("form")?.requestSubmit()}>
              <span class="name">{t(size.name)}</span>
              <span class="pixels">{portrait ? size.short : size.long} × {portrait ? size.long : size.short} px</span>
              <span class="detail">{size.detail ?? ""}</span>
            </button>
          {/each}
        {:else}
          <div class="fields">
            <label>{t("Largeur (px)")} <input type="number" min="1" max={MAX_SIDE} bind:value={custom.width} /></label>
            <label>{t("Hauteur (px)")} <input type="number" min="1" max={MAX_SIDE} bind:value={custom.height} /></label>
            <label>{t("Résolution (ppp)")} <input type="number" min="1" max="9600" bind:value={custom.dpi} /></label>
          </div>
          <p class="hint">{t("De 1 à {MAX_SIDE} px par côté.", { MAX_SIDE })}</p>
        {/if}
      </section>

      <aside>
        <span class="caption">{t("Aperçu")}</span>
        <div class="stage">
          <span class="paper" class:transparent style:width="{preview.width}px" style:height="{preview.height}px"></span>
        </div>
        <dl>
          <dt>{t("Dimensions")}</dt>
          <dd>{dimensions.width || 0} × {dimensions.height || 0} px</dd>
          <dt>{t("Résolution")}</dt>
          <dd>{dimensions.dpi || 0} ppp</dd>
          <dt>{t("Orientation")}</dt>
          <dd>{dimensions.height > dimensions.width ? "Portrait" : dimensions.height === dimensions.width ? t("Carré") : "Paysage"}</dd>
        </dl>
        <div class="line">
          <span class="muted">{t("Fond")}</span>
          <div class="segmented">
            <button type="button" class:on={!transparent} onclick={() => (transparent = false)}>{t("Blanc")}</button>
            <button type="button" class:on={transparent} onclick={() => (transparent = true)}>{t("Transparent")}</button>
          </div>
        </div>
      </aside>
    </div>

    <footer>
      <label class="inline">{t("Nom")} <input type="text" bind:value={name} /></label>
      <div class="spacer"></div>
      <button type="button" class="ghost" onclick={() => (app.newDocumentDialogOpen = false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!valid}>{t("Créer")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 940px;
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
    height: 440px;
    display: flex;
  }
  nav {
    width: 190px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 10px;
    border-right: 1px solid var(--border);
  }
  .caption {
    padding: 0 10px 8px;
    color: var(--text-muted);
  }
  nav button {
    height: 40px;
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
    color: #fff;
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
    background: #0f0f11;
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
    background: rgba(255, 255, 255, 0.08);
    border-color: rgba(255, 255, 255, 0.22);
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
  .stage {
    height: 250px;
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
