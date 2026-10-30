<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onDestroy, onMount, untrack } from "svelte";
  import { hexToRgb, hsvToRgb, rgbToHex, rgbToHsv, type Hsv } from "../lib/color";
  import { dropLatest, edit, editLatest, whenIdle } from "../lib/engine";
  import { type GradientStop, resolveStops, toCss } from "../lib/gradient";
  import { api, type FillContent, type FillGradientStyle, type PatternInfo, type Rgb } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import { backgroundInMode, foregroundInMode } from "../lib/color-mode.svelte";
  import ColorPopover from "./ColorPopover.svelte";
  import ColorWheel from "./ColorWheel.svelte";
  import GradientStops from "./GradientStops.svelte";

  // Calques de remplissage (Calque > Nouveau calque de remplissage) : Couleur unie, Dégradé (« Fond en dégradé »),
  // Motif. Comme Photoshop, le calque est créé dès l'ouverture (au-dessus du calque actif, masque blanc) et suit
  // chaque réglage en direct ; OK le garde, Annuler le retire sans laisser de trace dans l'historique. Rouverte par un
  // double-clic sur la vignette du calque, la boîte modifie son remplissage (Annuler rend l'ancien).
  // Panneau flottant, non modal : on voit l'image, et pour un dégradé, glisser dans l'image en déplace le centre.
  const opened = untrack(() => app.fillDialog!);
  const kind = opened.kind;
  /** Calque modifié, ou `null` : un calque est créé au-dessus de celui qui était actif à l'ouverture. */
  const target = opened.layer;
  const above = untrack(() => app.activeLayer?.id ?? null);
  const existing = untrack(() => (target === null ? undefined : app.active?.info.layers.find((l) => l.id === target)?.fillLayer ?? undefined));

  function defaults(): FillContent {
    if (existing && existing.type === kind) return $state.snapshot(existing) as FillContent;
    if (kind === "solidColor") return { type: "solidColor", color: [...foregroundInMode()] as Rgb };
    if (kind === "gradient") {
      // Dégradé de départ : premier plan vers arrière-plan, de bas en haut, comme Photoshop.
      const stops = resolveStops({ stops: [], dynamic: "foregroundToBackground" }, foregroundInMode(), backgroundInMode());
      return { type: "gradient", stops, style: "linear", angle: 90, scale: 100, reverse: false, offset: [0, 0] };
    }
    return { type: "pattern", pattern: "points", scale: 100, color: [0, 0, 0], background: [255, 255, 255] };
  }

  let fill = $state<FillContent>(untrack(defaults));
  const original = untrack(() => $state.snapshot(fill) as FillContent);
  /** Tous les réglages de la boîte forment un seul geste : Annuler le retire en bloc. */
  const gesture = `remplissage-${Date.now()}`;
  let sent = false;

  function push(): void {
    sent = true;
    const content = $state.snapshot(fill) as FillContent;
    editLatest(gesture, target === null ? { op: "addFill", fill: content, above, gesture } : { op: "setFill", id: target, fill: content, gesture });
  }

  function update(change: Partial<FillContent>): void {
    fill = { ...fill, ...change } as FillContent;
    push();
  }

  function close(keep: boolean): void {
    app.fillDialog = null;
    app.canvasDrag = null;
    if (keep || !sent) return;
    dropLatest(gesture);
    void whenIdle(gesture).then(() => edit({ op: "cancelGesture", gesture }));
  }

  onMount(() => {
    // Création : le calque apparaît tout de suite, avec le remplissage de départ.
    if (target === null) push();
    // Couleur unie : Alt + clic sur l'image prélève la couleur, comme dans le sélecteur de couleur.
    if (kind === "solidColor") app.eyedropperTarget = (rgb) => setSolid(rgb);
    if (kind === "gradient") {
      // Glisser dans l'image déplace le centre du dégradé.
      let start: { x: number; y: number; offset: [number, number] } | null = null;
      app.canvasDrag = {
        begin: (x, y) => {
          if (fill.type === "gradient") start = { x, y, offset: [...fill.offset] as [number, number] };
        },
        move: (x, y) => {
          if (!start || fill.type !== "gradient") return;
          update({ offset: [Math.round(start.offset[0] + x - start.x), Math.round(start.offset[1] + y - start.y)] });
        },
        end: () => (start = null),
      };
    }
  });
  onDestroy(() => {
    if (app.canvasDrag) app.canvasDrag = null;
    if (kind === "solidColor") app.eyedropperTarget = null;
  });

  // ---- Fenêtre déplaçable par son titre.
  const WIDTH = kind === "pattern" ? 460 : kind === "gradient" ? 380 : 320;
  let position = $state({ x: Math.max(24, Math.round(window.innerWidth / 2 - WIDTH / 2)), y: 96 });
  let dragging: { dx: number; dy: number } | null = null;
  function onHeaderDown(event: PointerEvent): void {
    if ((event.target as Element).closest("button")) return;
    dragging = { dx: event.clientX - position.x, dy: event.clientY - position.y };
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
  }
  function onHeaderMove(event: PointerEvent): void {
    if (dragging) position = { x: Math.max(0, event.clientX - dragging.dx), y: Math.max(0, event.clientY - dragging.dy) };
  }

  const TITLES = { solidColor: "Couleur unie", gradient: "Fond en dégradé", pattern: "Fond de motif" } as const;

  // ---- Couleur unie : roue de couleur, code hexadécimal, ancienne et nouvelle couleur.
  let hsv = $state<Hsv>(untrack(() => rgbToHsv(fill.type === "solidColor" ? fill.color : [0, 0, 0])));
  let hexDraft = $state<string | null>(null);
  function setSolid(rgb: Rgb, keepHue = true): void {
    if (keepHue) hsv = rgbToHsv(rgb, hsv.h);
    update({ color: rgb });
  }

  // ---- Dégradé.
  const STYLES: { value: FillGradientStyle; label: string }[] = [
    { value: "linear", label: "Linéaire" },
    { value: "radial", label: "Radial" },
    { value: "angle", label: "Angle" },
    { value: "reflected", label: "Réfléchi" },
    { value: "diamond", label: "Losange" },
  ];
  let stopsOpen = $state(false);
  let stopsEditor = $state<ReturnType<typeof GradientStops>>();
  const stops = $derived<GradientStop[]>(fill.type === "gradient" ? fill.stops : []);
  function setStops(next: GradientStop[]): void {
    update({ stops: next });
  }
  const angle = $derived(fill.type === "gradient" ? fill.angle : 0);
  /** Cadran d'angle : l'angle suit le pointeur autour du centre (sens trigonométrique, 0° à droite). */
  function dialAngle(event: PointerEvent): void {
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    const dx = event.clientX - (rect.left + rect.width / 2);
    const dy = event.clientY - (rect.top + rect.height / 2);
    if (Math.hypot(dx, dy) < 2) return;
    let degrees = Math.round((Math.atan2(-dy, dx) * 180) / Math.PI);
    if (event.shiftKey) degrees = Math.round(degrees / 15) * 15;
    update({ angle: degrees });
  }
  let dialing = false;

  // ---- Motif.
  let catalog = $state<PatternInfo[]>([]);
  const swatches = $state<Record<string, string>>({});
  let picking = $state<{ key: "color" | "background"; x: number; y: number } | null>(null);
  onMount(() => {
    if (kind !== "pattern") return;
    void api
      .patternCatalog()
      .then(async (list) => {
        catalog = list;
        for (const pattern of list) {
          const bytes = new Uint8ClampedArray(await api.patternSwatch(pattern.id, 48, 1));
          if (bytes.length !== 48 * 48 * 4) continue;
          const canvas = document.createElement("canvas");
          canvas.width = canvas.height = 48;
          canvas.getContext("2d")?.putImageData(new ImageData(bytes, 48, 48), 0, 0);
          swatches[pattern.id] = canvas.toDataURL();
        }
      })
      .catch(() => (app.error = t("Bibliothèque de motifs indisponible.")));
  });
  const patternColor = (key: "color" | "background"): Rgb => (fill.type === "pattern" ? fill[key] : [0, 0, 0]);
  function openPicker(key: "color" | "background", event: MouseEvent): void {
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    picking = picking?.key === key ? null : { key, x: Math.min(window.innerWidth - 240, rect.right + 12), y: Math.max(12, rect.top - 160) };
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key !== "Escape" || picking || stopsEditor?.pickingColor()) return;
    if (stopsOpen) stopsOpen = false;
    else close(false);
  }}
/>

{#snippet number(value: number, min: number, max: number, step: number, unit: string, apply: (v: number) => void)}
  <span class="number">
    <input
      type="number"
      {min}
      {max}
      {step}
      {value}
      onkeydown={(event) => event.stopPropagation()}
      onchange={(event) => Number.isFinite(event.currentTarget.valueAsNumber) && apply(Math.min(max, Math.max(min, event.currentTarget.valueAsNumber)))}
    />{unit}
  </span>
{/snippet}

<div class="panel" role="dialog" aria-label={t(TITLES[kind])} style:left="{position.x}px" style:top="{position.y}px" style:width="{WIDTH}px">
  <header role="presentation" onpointerdown={onHeaderDown} onpointermove={onHeaderMove} onpointerup={() => (dragging = null)}>
    <h2>{t(TITLES[kind])}</h2>
    <div class="spacer"></div>
    <button class="icon" aria-label={t("Fermer sans appliquer")} onclick={() => close(false)}>×</button>
  </header>

  <section>
    {#if fill.type === "solidColor"}
      <div class="solid">
        <ColorWheel {hsv} size={196} onchange={(next) => ((hsv = next), setSolid(hsvToRgb(next), false))} />
        <div class="compare">
          <span>{t("Nouvelle")}</span>
          <div class="swatch-pair">
            <span style:background={rgbToHex(fill.color)}></span>
            <span style:background={rgbToHex(original.type === "solidColor" ? original.color : fill.color)}></span>
          </div>
          <span>{t("Actuelle")}</span>
          <input
            class="hex"
            aria-label={t("Code hexadécimal")}
            value={hexDraft ?? rgbToHex(fill.color)}
            oninput={(event) => (hexDraft = event.currentTarget.value)}
            onkeydown={(event) => {
              event.stopPropagation();
              if (event.key === "Enter") event.currentTarget.blur();
            }}
            onchange={() => {
              const rgb = hexDraft === null ? null : hexToRgb(hexDraft);
              hexDraft = null;
              if (rgb) setSolid(rgb);
            }}
          />
        </div>
      </div>
      <p class="hint">{t("Alt + clic sur l'image : prélever une couleur")}</p>
    {:else if fill.type === "gradient"}
      <div class="line">
        <span>{t("Dégradé")}</span>
        <button class="gradient" class:open={stopsOpen} aria-expanded={stopsOpen} title={t("Modifier le dégradé")} onclick={() => (stopsOpen = !stopsOpen)}>
          <span class="bar" style:background={toCss(stops)}></span>
          <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true"><path d="M2 3.5l3 3 3-3" /></svg>
        </button>
      </div>
      {#if stopsOpen}
        <div class="stops">
          <GradientStops bind:this={stopsEditor} {stops} onchange={setStops} reversible={false}>
            <button class="preset" title={t("Premier plan → arrière-plan")} aria-label={t("Premier plan vers arrière-plan")} onclick={() => setStops(resolveStops({ stops: [], dynamic: "foregroundToBackground" }, foregroundInMode(), backgroundInMode()))}>
              <span style:background={toCss(resolveStops({ stops: [], dynamic: "foregroundToBackground" }, foregroundInMode(), backgroundInMode()))}></span>
            </button>
            <button class="preset" title={t("Premier plan → transparent")} aria-label={t("Premier plan vers transparent")} onclick={() => setStops(resolveStops({ stops: [], dynamic: "foregroundToTransparent" }, foregroundInMode(), backgroundInMode()))}>
              <span style:background={toCss(resolveStops({ stops: [], dynamic: "foregroundToTransparent" }, foregroundInMode(), backgroundInMode()))}></span>
            </button>
          </GradientStops>
        </div>
      {/if}
      <div class="line">
        <span>{t("Style")}</span>
        <select value={fill.style} onchange={(event) => update({ style: event.currentTarget.value as FillGradientStyle })}>
          {#each STYLES as style (style.value)}
            <option value={style.value}>{t(style.label)}</option>
          {/each}
        </select>
      </div>
      <div class="line">
        <span>{t("Angle")}</span>
        <div class="angle">
          <svg
            class="dial"
            viewBox="-20 -20 40 40"
            role="slider"
            tabindex="0"
            aria-label={t("Angle")}
            aria-valuemin={-180}
            aria-valuemax={180}
            aria-valuenow={angle}
            onpointerdown={(event) => {
              dialing = true;
              (event.currentTarget as Element).setPointerCapture(event.pointerId);
              dialAngle(event);
            }}
            onpointermove={(event) => dialing && dialAngle(event)}
            onpointerup={() => (dialing = false)}
            onkeydown={(event) => {
              if (event.key === "ArrowLeft" || event.key === "ArrowUp") update({ angle: angle + 1 });
              else if (event.key === "ArrowRight" || event.key === "ArrowDown") update({ angle: angle - 1 });
              else return;
              event.preventDefault();
              event.stopPropagation();
            }}
          >
            <circle r="17" />
            <line x1="0" y1="0" x2={Math.cos((angle * Math.PI) / 180) * 15} y2={-Math.sin((angle * Math.PI) / 180) * 15} />
            <circle class="hub" r="2" />
          </svg>
          {@render number(angle, -180, 180, 1, "°", (v) => update({ angle: Math.round(v) }))}
        </div>
      </div>
      <div class="line">
        <span>{t("Échelle")}</span>
        <div class="scale">
          <input type="range" min="10" max="150" step="1" value={fill.scale} oninput={(event) => update({ scale: event.currentTarget.valueAsNumber })} />
          {@render number(fill.scale, 10, 150, 1, " %", (v) => update({ scale: Math.round(v) }))}
        </div>
      </div>
      <label class="check"><input type="checkbox" checked={fill.reverse} onchange={(event) => update({ reverse: event.currentTarget.checked })} /> {t("Inverser")}</label>
      <p class="hint">
        {t("Glissez dans l'image pour déplacer le centre du dégradé.")}
        {#if fill.offset[0] !== 0 || fill.offset[1] !== 0}
          <button class="link" onclick={() => update({ offset: [0, 0] })}>{t("Recentrer")}</button>
        {/if}
      </p>
    {:else if fill.type === "pattern"}
      <div class="patterns" role="radiogroup" aria-label={t("Motif")}>
        {#each catalog as pattern (pattern.id)}
          <button role="radio" aria-checked={fill.pattern === pattern.id} class:on={fill.pattern === pattern.id} title={t(pattern.name)} onclick={() => update({ pattern: pattern.id })}>
            {#if swatches[pattern.id]}<img src={swatches[pattern.id]} alt="" />{/if}
            <span>{t(pattern.name)}</span>
          </button>
        {/each}
      </div>
      <div class="line">
        <span>{t("Échelle")}</span>
        <div class="scale">
          <input type="range" min="10" max="1000" step="1" value={fill.scale} oninput={(event) => update({ scale: event.currentTarget.valueAsNumber })} />
          {@render number(fill.scale, 10, 1000, 1, " %", (v) => update({ scale: Math.round(v) }))}
        </div>
      </div>
      <div class="line">
        <span>{t("Couleurs")}</span>
        <div class="colors">
          <button class="color" title={t("Couleur du motif")} onclick={(event) => openPicker("color", event)}><span style:background={rgbToHex(fill.color)}></span>{t("Motif")}</button>
          <button class="color" title={t("Couleur du fond")} onclick={(event) => openPicker("background", event)}><span style:background={rgbToHex(fill.background)}></span>{t("Fond")}</button>
          <button
            class="icon swap"
            title={t("Permuter les couleurs")}
            aria-label={t("Permuter les couleurs")}
            onclick={() => fill.type === "pattern" && update({ color: fill.background, background: fill.color })}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M3 6h9l-2.5-2.5M13 10H4l2.5 2.5" /></svg>
          </button>
        </div>
      </div>
    {/if}
  </section>

  <footer>
    <button class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
    <button class="pill" onclick={() => close(true)}>OK</button>
  </footer>
</div>

{#if picking}
  {@const key = picking.key}
  <ColorPopover color={patternColor(key)} x={picking.x} y={picking.y} onchange={(rgb) => update({ [key]: rgb } as Partial<FillContent>)} onclose={() => (picking = null)} />
{/if}

<style>
  .panel {
    position: fixed;
    z-index: 40;
    max-width: calc(100vw - 24px);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 20px;
  }
  header {
    height: 46px;
    border-bottom: 1px solid var(--border);
    cursor: move;
    touch-action: none;
    user-select: none;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .spacer {
    flex: 1;
  }
  .icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-m);
    font-size: 18px;
    color: var(--text-muted);
  }
  .icon:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  footer {
    height: 54px;
    justify-content: flex-end;
    gap: 8px;
    padding-right: 20px;
    border-top: 1px solid var(--border);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px;
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
  .line {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 30px;
  }
  .line > span:first-child {
    width: 64px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  .line select {
    flex: 1;
  }
  .gradient {
    flex: 1;
    min-width: 0;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px 0 3px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text-2);
  }
  .gradient.open {
    border-color: var(--accent);
  }
  .gradient .bar {
    flex: 1;
    height: 22px;
    border-radius: var(--radius-xs);
    background-color: #fff;
    background-image: linear-gradient(45deg, #ccc 25%, transparent 25%), linear-gradient(-45deg, #ccc 25%, transparent 25%);
  }
  .gradient svg,
  .swap svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .stops {
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg-menu);
  }
  .preset {
    width: 34px;
    height: 22px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-xs);
  }
  .preset span {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 1px;
  }
  .angle,
  .scale {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .scale input[type="range"] {
    flex: 1;
    min-width: 0;
  }
  .dial {
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  .dial circle {
    fill: var(--bg-field);
    stroke: var(--border-strong);
    stroke-width: 1.2;
  }
  .dial:focus-visible circle {
    stroke: var(--accent);
  }
  .dial line {
    stroke: var(--accent);
    stroke-width: 2;
    stroke-linecap: round;
  }
  .dial .hub {
    fill: var(--accent);
    stroke: none;
  }
  .number {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--text-3);
    white-space: nowrap;
  }
  .number input {
    width: 58px;
    box-sizing: border-box;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .hint {
    margin: 0;
    font-size: 11px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .link {
    margin-left: 6px;
    color: var(--accent);
    text-decoration: underline;
  }
  .solid {
    display: flex;
    gap: 18px;
    align-items: center;
  }
  .compare {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--text-muted);
  }
  .swatch-pair {
    display: flex;
    flex-direction: column;
    width: 54px;
    border: 1px solid var(--border-strong);
  }
  .swatch-pair span {
    height: 34px;
  }
  .hex {
    width: 76px;
    margin-top: 6px;
    text-align: center;
    box-sizing: border-box;
  }
  .patterns {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 6px;
  }
  .patterns button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 4px 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-muted);
    font-size: 10px;
    overflow: hidden;
  }
  .patterns button:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .patterns button.on {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--text);
  }
  .patterns img {
    width: 44px;
    height: 44px;
    display: block;
    image-rendering: pixelated;
    border: 1px solid rgb(var(--ink) / 0.12);
  }
  .patterns span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .colors {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .color {
    height: 28px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 4px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .color span {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-xs);
    border: 1px solid rgb(var(--ink) / 0.25);
  }
</style>
