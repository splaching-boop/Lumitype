<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { edit } from "../lib/engine";
  import { CORNERS, GROUPS, TEMPLATES, templateById, type Corner, type Hints, type Layout } from "../lib/guide-templates";
  import { loadSavedLayouts, removeLayout, saveLayout, savedLayouts } from "../lib/guide-layouts.svelte";
  import type { Guide } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Disposition de repères. À gauche, un schéma du document montre exactement les repères qui seront
  // créés ; à droite, les modèles prédéfinis et, tout en haut, le mode sur mesure (colonnes, rangées,
  // marges). Tous les modèles sont décrits en proportions : la même disposition suit n'importe quel format.
  const info = untrack(() => app.active?.info);
  const [W, H] = [info?.width ?? 1, info?.height ?? 1];

  let dialog = $state<HTMLDialogElement>();
  let chosen = $state<string>("thirds");
  let corner = $state<Corner>("bottomLeft");
  let columns = $state({ on: true, count: 3, gutter: 0 });
  let rows = $state({ on: false, count: 2, gutter: 0 });
  let margin = $state({ on: false, top: 0, right: 0, bottom: 0, left: 0 });
  let clear = $state(true);

  const CUSTOM = "custom";
  /** Les repères déjà posés dans le document, tels quels : pour les enregistrer comme disposition. */
  const CURRENT = "current";
  const template = $derived(chosen === CUSTOM || chosen === CURRENT ? undefined : templateById(chosen));
  const saved = $derived(savedLayouts.all.find((item) => item.id === chosen));
  const hasCurrent = (info?.guides.length ?? 0) > 0;

  /** Enregistrement d'une disposition : le nom, saisi en place. */
  let naming = $state<string | null>(null);
  let nameField = $state<HTMLInputElement>();

  $effect(() => {
    dialog?.showModal();
    void loadSavedLayouts();
  });

  function startNaming(): void {
    naming = saved?.name ?? template?.name ?? (chosen === CURRENT ? t("Repères actuels") : t("Sur mesure"));
    queueMicrotask(() => nameField?.select());
  }

  /** Ce que montre l'aperçu, ramené en proportions, sous le nom saisi. */
  function commitName(): void {
    if (naming === null) return;
    const layout = saveLayout(naming, guides.vertical.map((v) => v / W), guides.horizontal.map((v) => v / H));
    naming = null;
    chosen = layout.id;
  }

  /** Positions des repères d'une bande de `count` cases séparées par `gutter`, entre `from` et `to`. */
  function band(from: number, to: number, count: number, gutter: number): number[] {
    const n = Math.max(1, Math.round(count));
    const g = Math.max(0, gutter);
    const cell = (to - from - (n - 1) * g) / n;
    if (cell <= 0) return [];
    const positions: number[] = [];
    for (let i = 0; i < n; i++) positions.push(from + i * (cell + g), from + i * (cell + g) + cell);
    return positions;
  }

  /** Arrondi au pixel et dédoublonné : sans gouttière, deux cases voisines partagent le même repère. */
  const unique = (values: number[], max: number): number[] => [...new Set(values.map((v) => Math.round(v)).filter((v) => v >= 0 && v <= max))].sort((a, b) => a - b);

  /** La disposition sur mesure, en pixels. */
  function custom(): { vertical: number[]; horizontal: number[] } {
    const m = margin.on ? margin : { top: 0, right: 0, bottom: 0, left: 0 };
    return {
      vertical: [...(columns.on ? band(m.left, W - m.right, columns.count, columns.gutter) : []), ...(margin.on ? [m.left, W - m.right] : [])],
      horizontal: [...(rows.on ? band(m.top, H - m.bottom, rows.count, rows.gutter) : []), ...(margin.on ? [m.top, H - m.bottom] : [])],
    };
  }

  const layout = $derived<Layout | undefined>(template ? template.build(corner) : saved ? { vertical: saved.vertical, horizontal: saved.horizontal } : undefined);

  /** Les repères actuels du document, en pixels. */
  const currentGuides = () => ({
    vertical: (info?.guides ?? []).filter((g) => g.orientation === "vertical").map((g) => g.position),
    horizontal: (info?.guides ?? []).filter((g) => g.orientation === "horizontal").map((g) => g.position),
  });

  const guides = $derived.by(() => {
    const raw = layout ? { vertical: layout.vertical.map((v) => v * W), horizontal: layout.horizontal.map((v) => v * H) } : chosen === CURRENT ? currentGuides() : custom();
    return { vertical: unique(raw.vertical, W), horizontal: unique(raw.horizontal, H) };
  });

  const hints = $derived<Hints | undefined>(layout?.hints);
  const total = $derived(guides.vertical.length + guides.horizontal.length);

  const scale = Math.min(460 / W, 330 / H);
  const [pw, ph] = [W * scale, H * scale];
  const existing = $derived(clear ? [] : (info?.guides ?? []));

  /** Chemin d'une suite d'arcs, mis à l'échelle d'un aperçu `w × h`. */
  function spiral(list: NonNullable<Hints["arcs"]>, w: number, h: number): string {
    let d = "";
    for (const a of list) d += `M${a.sx * w} ${a.sy * h}A${a.rx * w} ${a.ry * h} 0 0 ${a.sweep} ${a.ex * w} ${a.ey * h}`;
    return d;
  }

  /** Chemin d'un parcours de lecture, décrit en proportions dans le modèle. */
  const reading = (path: string, w: number, h: number): string => path.replace(/([ML])([\d.]+) ([\d.]+)/g, (_, cmd, x, y) => `${cmd}${Number(x) * w} ${Number(y) * h}`);

  const [tw, th] = W >= H ? [46, (46 * H) / W] : [(32 * W) / H, 32];

  function close(apply: boolean): void {
    app.guideLayoutOpen = false;
    if (!apply) return;
    const list: Guide[] = [...guides.vertical.map((position): Guide => ({ orientation: "vertical", position })), ...guides.horizontal.map((position): Guide => ({ orientation: "horizontal", position }))];
    void edit({ op: "setGuides", guides: list, clear });
    app.guidesVisible = true;
  }
</script>

{#snippet field(label: string, value: number, set: (v: number) => void, disabled: boolean, max: number)}
  <label class="field">
    {t(label)}
    <input type="number" min="0" {max} {value} {disabled} oninput={(event) => Number.isFinite(event.currentTarget.valueAsNumber) && set(Math.min(max, Math.max(0, event.currentTarget.valueAsNumber)))} />
  </label>
{/snippet}

<!-- Les repères d'un modèle, dessinés dans une boîte `w × h`. `full` ajoute les aides d'aperçu. -->
{#snippet plan(shape: Layout, w: number, h: number, full: boolean)}
  {@const k = full ? 1 : 0.6}
  {#each shape.hints?.bands ?? [] as [from, to], index (index)}
    <rect class="tint" x="0" y={from * h} width={w} height={(to - from) * h} />
  {/each}
  {#if shape.hints?.inner}
    {@const [x, y, iw, ih] = shape.hints.inner}
    <rect class="tint" x={x * w} y={y * h} width={iw * w} height={ih * h} />
  {/if}
  {#if shape.hints?.danger}
    <rect class="danger" x="0" y="0" width={w} height={h} stroke-width={4 * k} />
  {/if}
  {#if shape.hints?.path}
    <path class="reading" d={reading(shape.hints.path, w, h)} stroke-width={1.4 * k} stroke-dasharray="{4 * k} {4 * k}" />
  {/if}
  {#if shape.hints?.fadedV?.length || shape.hints?.fadedH?.length}
    <path class="faded" stroke-width={1.1 * k} stroke-dasharray="{3.4 * k} {2.8 * k}" d={[...(shape.hints.fadedV ?? []).map((x) => `M${x * w} 0V${h}`), ...(shape.hints.fadedH ?? []).map((y) => `M0 ${y * h}H${w}`)].join("")} />
  {/if}
  {#if shape.hints?.arcs}
    <path class="spiral" d={spiral(shape.hints.arcs, w, h)} stroke-width={1.8 * k} />
  {/if}
  <path class="guide" stroke-width={1.5 * k} d={[...shape.vertical.map((x) => `M${x * w} 0V${h}`), ...shape.horizontal.map((y) => `M0 ${y * h}H${w}`)].join("")} />
  {#each shape.hints?.dots ?? [] as [x, y], index (index)}
    <circle class="dot" cx={x * w} cy={y * h} r={2.6 * k} />
  {/each}
  {#if shape.hints?.eye}
    <circle class="eye" cx={shape.hints.eye[0] * w} cy={shape.hints.eye[1] * h} r={7 * k} stroke-width={1.3 * k} stroke-dasharray="{2.6 * k} {2.4 * k}" />
    <circle class="pip" cx={shape.hints.eye[0] * w} cy={shape.hints.eye[1] * h} r={1.6 * k} />
  {/if}
{/snippet}

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <header>
      <h2>{t("Disposition de repères")}</h2>
      <span>{W} × {H} px · {total} {total > 1 ? t("repères") : t("repère")}</span>
    </header>
    <div class="body">
      <div class="stage">
        <svg viewBox="0 0 {pw} {ph}" width={pw} height={ph} aria-label={t("Aperçu des repères")}>
          <rect width={pw} height={ph} class="sheet" />
          {#each existing as guide, index (index)}
            {#if guide.orientation === "vertical"}
              <line class="old" x1={guide.position * scale} y1="0" x2={guide.position * scale} y2={ph} />
            {:else}
              <line class="old" x1="0" y1={guide.position * scale} x2={pw} y2={guide.position * scale} />
            {/if}
          {/each}
          {#if layout}
            {@render plan(layout, pw, ph, true)}
          {:else}
            <path class="guide" stroke-width="1.5" d={[...guides.vertical.map((x) => `M${x * scale} 0V${ph}`), ...guides.horizontal.map((y) => `M0 ${y * scale}H${pw}`)].join("")} />
          {/if}
        </svg>

        {#if template?.turnable}
          <!-- Un seul réglage pour la spirale : le coin où elle se referme. -->
          <div class="corners" role="group" aria-label={t("Coin de la spirale")}>
            {#each CORNERS as option (option.id)}
              <button type="button" class:lit={corner === option.id} onclick={() => (corner = option.id)}>{t(option.label)}</button>
            {/each}
          </div>
        {/if}
        {#if hints}
          <p class="aside-note">{t("Tout ce qui est en ambre n'est qu'un aperçu : seuls les traits bleus deviennent des repères.")}</p>
        {/if}
      </div>

      <aside>
        <button type="button" class="entry" class:on={chosen === CUSTOM} onclick={() => (chosen = CUSTOM)}>
          <svg class="thumb" viewBox="0 0 {tw} {th}" width={tw} height={th} aria-hidden="true">
            <rect width={tw} height={th} class="sheet" />
            <path class="guide" stroke-width="0.9" d="M{tw / 3} 0V{th}M{(tw * 2) / 3} 0V{th}" />
          </svg>
          <span><b>{t("Sur mesure")}</b><small>{t("Colonnes, rangées, marges")}</small></span>
        </button>

        {#if chosen === CUSTOM}
          <div class="tune">
            <fieldset>
              <legend><label class="check"><input type="checkbox" bind:checked={columns.on} /> {t("Colonnes")}</label></legend>
              {@render field("Nombre", columns.count, (v) => (columns.count = Math.max(1, v)), !columns.on, 100)}
              {@render field("Gouttière (px)", columns.gutter, (v) => (columns.gutter = v), !columns.on, W)}
            </fieldset>
            <fieldset>
              <legend><label class="check"><input type="checkbox" bind:checked={rows.on} /> {t("Rangées")}</label></legend>
              {@render field("Nombre", rows.count, (v) => (rows.count = Math.max(1, v)), !rows.on, 100)}
              {@render field("Gouttière (px)", rows.gutter, (v) => (rows.gutter = v), !rows.on, H)}
            </fieldset>
            <fieldset>
              <legend><label class="check"><input type="checkbox" bind:checked={margin.on} /> {t("Marges (px)")}</label></legend>
              {@render field("Haut", margin.top, (v) => (margin.top = v), !margin.on, H)}
              {@render field("Bas", margin.bottom, (v) => (margin.bottom = v), !margin.on, H)}
              {@render field("Gauche", margin.left, (v) => (margin.left = v), !margin.on, W)}
              {@render field("Droite", margin.right, (v) => (margin.right = v), !margin.on, W)}
            </fieldset>
          </div>
        {/if}

        <h3>{t("Mes dispositions")}</h3>
        {#if hasCurrent}
          <button type="button" class="entry" class:on={chosen === CURRENT} onclick={() => (chosen = CURRENT)}>
            <svg class="thumb" viewBox="0 0 {tw} {th}" width={tw} height={th} aria-hidden="true">
              <rect width={tw} height={th} class="sheet" />
              <path class="guide" stroke-width="0.9" d={[...currentGuides().vertical.map((x) => `M${(x / W) * tw} 0V${th}`), ...currentGuides().horizontal.map((y) => `M0 ${(y / H) * th}H${tw}`)].join("")} />
            </svg>
            <span><b>{t("Repères actuels")}</b><small>{t("{n} repères du document", { n: info?.guides.length ?? 0 })}</small></span>
          </button>
        {/if}
        {#each savedLayouts.all as item (item.id)}
          <div class="entry saved" class:on={chosen === item.id}>
            <button type="button" class="pick" onclick={() => (chosen = item.id)}>
              <svg class="thumb" viewBox="0 0 {tw} {th}" width={tw} height={th} aria-hidden="true">
                <rect width={tw} height={th} class="sheet" />
                {@render plan({ vertical: item.vertical, horizontal: item.horizontal }, tw, th, false)}
              </svg>
              <span><b>{item.name}</b><small>{t("{n} repères", { n: item.vertical.length + item.horizontal.length })}</small></span>
            </button>
            <button type="button" class="remove" title={t("Supprimer cette disposition")} aria-label={t("Supprimer cette disposition")} onclick={() => ((chosen = chosen === item.id ? "thirds" : chosen), removeLayout(item.id))}>×</button>
          </div>
        {/each}
        {#if naming !== null}
          <div class="naming">
            <input
              type="text"
              bind:this={nameField}
              bind:value={naming}
              placeholder={t("Nom de la disposition")}
              onkeydown={(event) => {
                if (event.key === "Enter") (event.preventDefault(), commitName());
                if (event.key === "Escape") (event.preventDefault(), event.stopPropagation(), (naming = null));
              }}
            />
            <button type="button" class="pill small" onclick={commitName}>{t("Enregistrer")}</button>
          </div>
        {:else}
          <button type="button" class="save" onclick={startNaming}>{t("Enregistrer cette disposition…")}</button>
        {/if}

        {#each GROUPS as group (group)}
          <h3>{group}</h3>
          {#each TEMPLATES.filter((item) => item.group === group) as item (item.id)}
            <button type="button" class="entry" class:on={chosen === item.id} onclick={() => (chosen = item.id)}>
              <svg class="thumb" viewBox="0 0 {tw} {th}" width={tw} height={th} aria-hidden="true">
                <rect width={tw} height={th} class="sheet" />
                {@render plan(item.build(corner), tw, th, false)}
              </svg>
              <span><b>{t(item.name)}</b><small>{t(item.note)}</small></span>
            </button>
          {/each}
        {/each}
      </aside>
    </div>
    <footer>
      <label class="check"><input type="checkbox" bind:checked={clear} /> {t("Remplacer les repères existants")}</label>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill">{t("Créer les repères")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(920px, calc(100vw - 48px));
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
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 22px;
  }
  header {
    height: 54px;
    border-bottom: 1px solid var(--border);
  }
  header span {
    color: var(--text-muted);
  }
  footer {
    height: 58px;
    gap: 8px;
    border-top: 1px solid var(--border);
  }
  footer .check {
    margin-right: auto;
    color: var(--text-3);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    display: flex;
  }
  .stage {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    background: var(--bg-canvas);
  }
  svg {
    overflow: visible;
  }
  .stage > svg {
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  }
  .aside-note {
    margin: 0;
    max-width: 420px;
    color: var(--text-muted);
    font-size: 11.5px;
    text-align: center;
    line-height: 1.4;
  }
  .corners {
    display: flex;
    gap: 6px;
  }
  .corners button {
    height: 28px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text-3);
  }
  .corners button:hover {
    color: var(--text);
  }
  .corners button.lit {
    border-color: var(--accent);
    color: var(--text);
  }

  /* Aperçu : mêmes couleurs dans le grand schéma et dans les vignettes. */
  .sheet {
    fill: #f4f4f5;
  }
  line.old {
    stroke: #9a9aa2;
    stroke-width: 1.5;
    stroke-dasharray: 4 3;
  }
  .guide {
    stroke: #00b7ff;
    fill: none;
  }
  .spiral {
    stroke: #e0952a;
    fill: none;
    stroke-linecap: round;
  }
  .faded {
    stroke: #8e8880;
    fill: none;
    opacity: 0.45;
  }
  .reading {
    stroke: #e0952a;
    fill: none;
  }
  .tint {
    fill: #e0952a;
    opacity: 0.12;
  }
  .danger {
    fill: none;
    stroke: #e05c5c;
    opacity: 0.35;
  }
  .dot {
    fill: #e0952a;
  }
  .eye {
    fill: none;
    stroke: #e0952a;
  }
  .pip {
    fill: #e0952a;
  }

  aside {
    width: 292px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 14px 12px;
    border-left: 1px solid var(--border);
    max-height: 460px;
    overflow-y: auto;
  }
  h3 {
    margin: 12px 0 2px 6px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.5px;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 6px 8px;
    border-radius: var(--radius-m);
    border: 1px solid transparent;
    text-align: left;
    color: var(--text-3);
  }
  .entry:hover {
    background: var(--bg-hover);
  }
  .entry.on {
    background: var(--bg-active);
    border-color: var(--border-strong);
    color: var(--text);
  }
  /* Disposition enregistrée : la vignette et le nom sont le bouton de choix, la croix à droite la retire. */
  .entry.saved {
    padding: 0;
    gap: 0;
  }
  .entry.saved .pick {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    padding: 6px 8px;
    text-align: left;
    color: inherit;
  }
  .entry.saved .remove {
    flex: none;
    width: 26px;
    height: 26px;
    margin-right: 4px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
    font-size: 15px;
    opacity: 0;
  }
  .entry.saved:hover .remove,
  .entry.saved.on .remove {
    opacity: 1;
  }
  .entry.saved .remove:hover {
    background: var(--bg-hover);
    color: var(--danger);
  }
  .save {
    width: 100%;
    height: 30px;
    margin: 2px 0 6px;
    border-radius: var(--radius-pill);
    border: 1px dashed var(--border-strong);
    color: var(--text-3);
    font-size: 12px;
  }
  .save:hover {
    color: var(--text);
    border-style: solid;
  }
  .naming {
    display: flex;
    gap: 6px;
    margin: 2px 0 6px;
  }
  .naming input {
    flex: 1;
    min-width: 0;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent);
    background: var(--bg-field);
    color: var(--text);
  }
  .pill.small {
    height: 30px;
    padding: 0 12px;
    font-size: 12px;
  }
  .thumb {
    flex: none;
    border-radius: var(--radius-xs);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
  }
  .entry span {
    min-width: 0;
    display: grid;
  }
  .entry b {
    font-weight: 500;
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry small {
    color: var(--text-muted);
    font-size: 11px;
  }

  .tune {
    display: grid;
    gap: 10px;
    padding: 4px 2px 6px;
  }
  fieldset {
    margin: 0;
    padding: 12px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
  }
  legend {
    padding: 0 6px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-3);
  }
  input[type="number"] {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  input:disabled {
    opacity: 0.4;
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
