<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import BlendSelect from "./BlendSelect.svelte";
  import { onDestroy, untrack } from "svelte";
  import { rgbToHex } from "../lib/color";
  import { dropLatest, edit, editLatest, whenIdle } from "../lib/engine";
  import { cancelEffectsPreview, finishEffectsPreview, holdEffectsSource, previewEffects } from "../lib/effects-preview";
  import type { LayerEffect, LayerInfo, Rgb } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import ColorPopover from "./ColorPopover.svelte";

  // Effets de calque (fx). Panneau flottant, qu'on déplace par son titre : l'image reste visible et
  // utilisable pendant qu'on règle. La liste de gauche montre chaque COUCHE d'effet ; le « + » d'un type
  // en ajoute une (trois contours, deux ombres…). Tout s'applique en direct ; Annuler revient à l'état d'avant.
  let { layer }: { layer: LayerInfo } = $props();

  type Kind = LayerEffect["type"];
  const LABELS: Record<Kind, string> = {
    dropShadow: "Ombre portée",
    innerShadow: "Ombre interne",
    outerGlow: "Lueur externe",
    innerGlow: "Lueur interne",
    stroke: "Contour",
    colorOverlay: "Incrustation de couleur",
    gradientOverlay: "Incrustation de dégradé",
  };
  const DEFAULTS: Record<Kind, LayerEffect> = {
    // Modes par défaut de Photoshop : ombres en Produit, lueurs en Superposition, le reste en Normal.
    dropShadow: { type: "dropShadow", enabled: true, color: [0, 0, 0], opacity: 0.55, angle: 120, distance: 12, size: 16, blend: "multiply" },
    innerShadow: { type: "innerShadow", enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 120, distance: 6, size: 10, blend: "multiply" },
    outerGlow: { type: "outerGlow", enabled: true, color: [255, 236, 170], opacity: 0.75, size: 24, blend: "screen" },
    innerGlow: { type: "innerGlow", enabled: true, color: [255, 255, 255], opacity: 0.6, size: 14, blend: "screen" },
    stroke: { type: "stroke", enabled: true, color: [255, 255, 255], opacity: 1, width: 6, position: "outside", blend: "normal" },
    colorOverlay: { type: "colorOverlay", enabled: true, color: [255, 0, 80], opacity: 1, blend: "normal" },
    gradientOverlay: { type: "gradientOverlay", enabled: true, from: [255, 80, 120], to: [80, 120, 255], opacity: 1, angle: 90, radial: false, blend: "normal" },
  };
  const ORDER: Kind[] = ["dropShadow", "innerShadow", "outerGlow", "innerGlow", "stroke", "colorOverlay", "gradientOverlay"];

  /** Couche sélectionnée : son rang dans la liste des effets du calque. */
  let selected = $state(untrack(() => Math.max(0, layer.effects.findIndex((e) => e.enabled))));
  let picking = $state<{ key: "color" | "from" | "to"; x: number; y: number } | null>(null);
  // Position du panneau : au milieu de la zone de travail à l'ouverture, puis là où on le pose.
  let position = $state(untrack(() => ({ x: Math.max(24, Math.round(window.innerWidth / 2 - 330)), y: 96 })));
  let dragging: { dx: number; dy: number } | null = null;

  /**
   * Effets réglés mais pas encore rendus par le moteur : la carte graphique les montre déjà (lib/effects-preview.ts).
   * `null` : ceux du calque.
   */
  let draft = $state<LayerEffect[] | null>(null);
  const effects = $derived(draft ?? layer.effects);
  const current = $derived(effects[selected]);
  /** Lignes de la liste : chaque type, suivi de ses couches (il en montre une « vide » tant qu'il n'en a aucune). */
  const rows = $derived(ORDER.flatMap((kind) => {
    const own = effects.map((effect, index) => ({ effect, index })).filter((entry) => entry.effect.type === kind);
    return own.length > 0 ? own.map((entry, n) => ({ kind, index: entry.index as number | null, enabled: entry.effect.enabled, first: n === 0, count: own.length, n })) : [{ kind, index: null as number | null, enabled: false, first: true, count: 0, n: 0 }];
  }));

  /** Tout ce qui se fait dans cette fenêtre forme un seul geste : Annuler le retire en bloc de l'historique. */
  const gesture = untrack(() => `fx-${layer.id}-${Date.now()}`);
  /** Au moins un réglage est parti au moteur : Annuler doit retirer le geste de l'historique. */
  let touched = false;
  /** Pause d'un curseur après laquelle le moteur rend tout de même l'effet exact (sans attendre le relâchement). */
  const COMMIT_AFTER_MS = 300;
  let commitTimer: ReturnType<typeof setTimeout> | null = null;
  /** Derniers effets de l'aperçu partis au moteur (pour ne pas les renvoyer). */
  let committed: LayerEffect[] | null = null;
  // Pixels seuls du calque chargés dès l'ouverture : le premier pas d'un curseur n'attend rien.
  untrack(() => holdEffectsSource(layer.id));

  /**
   * Nouveaux effets. Si la carte graphique compose le document, elle les montre tout de suite et le moteur ne les
   * rend qu'au relâchement (`now`) ou après une courte pause ; sinon chaque pas passe par le moteur, comme avant.
   */
  function send(next: LayerEffect[], now = false): void {
    if (commitTimer) clearTimeout(commitTimer);
    commitTimer = null;
    if (previewEffects(layer.id, next)) {
      draft = next;
      if (now) commit();
      else commitTimer = setTimeout(commit, COMMIT_AFTER_MS);
      return;
    }
    draft = null;
    cancelEffectsPreview(layer.id);
    touched = true;
    editLatest(gesture, { op: "setEffects", id: layer.id, effects: next, gesture });
  }

  /** Les effets de l'aperçu partent au moteur ; son rendu remplacera l'aperçu dès qu'il sera chargé. */
  function commit(): void {
    if (commitTimer) clearTimeout(commitTimer);
    commitTimer = null;
    const list = draft;
    if (!list || list === committed) return;
    committed = list;
    touched = true;
    editLatest(gesture, { op: "setEffects", id: layer.id, effects: list, gesture });
    void whenIdle(gesture).then(() => {
      if (draft !== list) return; // réglé de nouveau entre-temps : l'aperçu continue
      draft = null;
      finishEffectsPreview(layer.id);
    });
  }

  onDestroy(() => {
    commit();
    holdEffectsSource(null);
  });
  const copy = () => $state.snapshot(effects) as LayerEffect[];

  /** Ajoute une couche du type donné, à la suite des couches de ce type. */
  function add(kind: Kind): void {
    const list = copy();
    const last = list.map((e) => e.type).lastIndexOf(kind);
    const at = last >= 0 ? last + 1 : list.length;
    // Une couche de plus du même type reprend les réglages de la précédente, en plus large : un second contour se voit tout de suite.
    const previous = last >= 0 ? list[last] : undefined;
    let layerEffect: LayerEffect = { ...(previous ?? DEFAULTS[kind]), enabled: true };
    if (previous && layerEffect.type === "stroke") layerEffect = { ...layerEffect, width: Math.min(100, layerEffect.width * 2) };
    list.splice(at, 0, layerEffect);
    send(list, true);
    selected = at;
  }

  function toggle(row: (typeof rows)[number], on: boolean): void {
    if (row.index === null) {
      if (on) add(row.kind);
      return;
    }
    const list = copy();
    const target = list[row.index];
    if (target) list[row.index] = { ...target, enabled: on };
    send(list, true);
    if (on) selected = row.index;
  }

  function remove(index: number): void {
    const list = copy();
    list.splice(index, 1);
    send(list, true);
    selected = Math.max(0, Math.min(selected, list.length - 1));
  }

  /** `now` : réglage terminé (curseur relâché, valeur saisie, choix d'un bouton), le moteur le rend aussitôt. */
  function patch(change: Record<string, unknown>, now = false): void {
    if (!current) return;
    const list = copy();
    list[selected] = { ...list[selected], ...change, enabled: true } as LayerEffect;
    send(list, now);
  }

  function close(keep: boolean): void {
    // `layer` est dérivé de `app.effectsLayer` : on garde son identifiant AVANT de fermer, sinon il n'existe plus.
    const id = layer.id;
    if (keep) {
      commit();
      app.effectsLayer = null;
      return;
    }
    app.effectsLayer = null;
    // Annuler : l'aperçu en cours disparaît tout de suite.
    if (commitTimer) clearTimeout(commitTimer);
    commitTimer = null;
    draft = null;
    cancelEffectsPreview(id);
    if (!touched) return;
    // Annuler : les réglages encore en route sont abandonnés, puis tout le geste est retiré — les effets
    // reviennent exactement à ce qu'ils étaient à l'ouverture, sans laisser de trace dans l'historique.
    dropLatest(gesture);
    void whenIdle(gesture).then(() => edit({ op: "cancelGesture", gesture }));
  }

  const field = (key: string): number => (current as unknown as Record<string, number> | undefined)?.[key] ?? 0;
  const has = (key: string): boolean => current !== undefined && key in current;
  const colorOf = (key: "color" | "from" | "to"): Rgb => ((current as unknown as Record<string, Rgb> | undefined)?.[key] ?? [0, 0, 0]) as Rgb;

  function onHeaderDown(event: PointerEvent): void {
    if ((event.target as Element).closest("button")) return;
    dragging = { dx: event.clientX - position.x, dy: event.clientY - position.y };
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
  }
  function onHeaderMove(event: PointerEvent): void {
    if (!dragging) return;
    position = { x: Math.min(window.innerWidth - 120, Math.max(-400, event.clientX - dragging.dx)), y: Math.min(window.innerHeight - 60, Math.max(0, event.clientY - dragging.dy)) };
  }
</script>

<svelte:window onkeydown={(event) => event.key === "Escape" && !picking && close(false)} />

{#snippet slider(key: string, label: string, min: number, max: number, step: number, unit: string, scale: number)}
  <label class="slider">
    <span>{label}</span>
    <!-- La valeur se saisit aussi au clavier. -->
    <span class="number">
      <input
        type="number"
        min={min * scale}
        max={max * scale}
        step={step * scale}
        value={Math.round(field(key) * scale * 10) / 10}
        onkeydown={(event) => event.stopPropagation()}
        onchange={(event) => Number.isFinite(event.currentTarget.valueAsNumber) && patch({ [key]: Math.min(max, Math.max(min, event.currentTarget.valueAsNumber / scale)) }, true)}
      />{unit}
    </span>
    <!-- Chaque pas : aperçu par la carte graphique ; au relâchement, le moteur rend l'effet exact. -->
    <input type="range" {min} {max} {step} value={field(key)} oninput={(event) => patch({ [key]: event.currentTarget.valueAsNumber })} onchange={() => commit()} />
  </label>
{/snippet}

{#snippet swatch(key: "color" | "from" | "to", label: string)}
  <div class="line">
    <span>{label}</span>
    <button
      class="color"
      onclick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        picking = picking?.key === key ? null : { key, x: Math.min(window.innerWidth - 300, rect.right + 12), y: Math.max(12, rect.top - 120) };
      }}
    >
      <span style:background={rgbToHex(colorOf(key))}></span>{rgbToHex(colorOf(key))}
    </button>
  </div>
{/snippet}

<div class="panel" role="dialog" aria-label={t("Effets de calque")} style:left="{position.x}px" style:top="{position.y}px">
  <header role="presentation" onpointerdown={onHeaderDown} onpointermove={onHeaderMove} onpointerup={() => (dragging = null)}>
    <h2>{t("Effets de calque")}</h2>
    <span>{t(layer.name)}</span>
    <div class="spacer"></div>
    <button class="icon" aria-label={t("Fermer sans appliquer")} onclick={() => close(false)}>×</button>
  </header>
  <div class="body">
    <nav aria-label={t("Effets")}>
      {#each rows as row (`${row.kind}-${row.n}`)}
        <div class="entry" class:on={row.index !== null && selected === row.index}>
          <input type="checkbox" aria-label="Activer {LABELS[row.kind]}" checked={row.enabled} onchange={(event) => toggle(row, event.currentTarget.checked)} />
          <button class="name" onclick={() => (row.index === null ? add(row.kind) : (selected = row.index))}>{LABELS[row.kind]}{row.count > 1 ? ` ${row.n + 1}` : ""}</button>
          {#if row.index !== null && row.count > 1}
            <button class="mini" title={t("Retirer cette couche")} onclick={() => row.index !== null && remove(row.index)}>−</button>
          {/if}
          {#if row.first}
            <button class="mini" title="Ajouter une couche : {LABELS[row.kind]}" onclick={() => add(row.kind)}>+</button>
          {/if}
        </div>
      {/each}
    </nav>
    <section>
      {#if current}
        <h3>{LABELS[current.type]}</h3>
        {#if current.type === "gradientOverlay"}
          {@render swatch("from", "Couleur de départ")}
          {@render swatch("to", "Couleur d'arrivée")}
          <div class="preview" style:background="linear-gradient(90deg, {rgbToHex(current.from)}, {rgbToHex(current.to)})"></div>
          <label class="check"><input type="checkbox" checked={current.radial} onchange={(event) => patch({ radial: event.currentTarget.checked }, true)} /> {t("Radial (du centre vers les bords)")}</label>
        {:else}
          {@render swatch("color", "Couleur")}
        {/if}
        <div class="blend">
          <span>{t("Mode de fusion")}</span>
          <!-- Même menu que pour les calques : survoler un mode l'applique en direct, à cet effet seulement. -->
          <BlendSelect value={current.blend ?? "normal"} exclude={["dissolve"]} onpreview={(mode) => patch({ blend: mode })} onchoose={(mode) => patch({ blend: mode }, true)} />
        </div>
        {#if (current.type === "dropShadow" || current.type === "outerGlow") && (current.blend ?? "normal") !== "normal"}
          <p class="hint">{t("Se fond dans ce qui est sous le calque.")}</p>
        {/if}
        {@render slider("opacity", "Opacité", 0, 1, 0.01, " %", 100)}
        {#if has("angle")}{@render slider("angle", current.type === "gradientOverlay" ? "Angle" : "Angle de la lumière", -180, 180, 1, "°", 1)}{/if}
        {#if has("distance")}{@render slider("distance", "Distance", 0, 200, 1, " px", 1)}{/if}
        {#if has("size")}{@render slider("size", "Taille (flou)", 0, 200, 1, " px", 1)}{/if}
        {#if has("width")}{@render slider("width", "Épaisseur", 1, 100, 1, " px", 1)}{/if}
        {#if current.type === "stroke"}
          <div class="line">
            <span>{t("Position")}</span>
            <div class="segmented">
              {#each [["outside", "Extérieur"], ["center", "Centre"], ["inside", "Intérieur"]] as const as [id, text] (id)}
                <button class:on={current.position === id} onclick={() => patch({ position: id }, true)}>{text}</button>
              {/each}
            </div>
          </div>
          <p>{t("Plusieurs contours : le premier de la liste passe au-dessus. Pour un double contour, gardez le plus fin en premier.")}</p>
        {/if}
        {#if !current.enabled}
          <p>{t("Cette couche est désactivée : bougez un réglage ou cochez-la pour l'activer.")}</p>
        {/if}
      {:else}
        <p>{t("Cochez un effet, ou cliquez sur son « + », pour l'ajouter à ce calque.")}</p>
      {/if}
    </section>
  </div>
  <footer>
    <button class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
    <button class="pill" onclick={() => close(true)}>OK</button>
  </footer>
</div>

{#if picking}
  {@const key = picking.key}
  <ColorPopover color={colorOf(key)} x={picking.x} y={picking.y} onchange={(rgb) => patch({ [key]: rgb })} onclose={() => (picking = null)} />
{/if}

<style>
  .panel {
    position: fixed;
    z-index: 40;
    width: 660px;
    max-width: calc(100vw - 24px);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 20px;
  }
  header {
    height: 48px;
    border-bottom: 1px solid var(--border);
    cursor: move;
    touch-action: none;
    user-select: none;
  }
  header span {
    color: var(--text-muted);
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
  h2,
  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  h3 {
    font-size: 13px;
  }
  .body {
    display: flex;
    min-height: 340px;
    max-height: calc(100vh - 260px);
  }
  nav {
    width: 270px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 8px;
    border-radius: var(--radius-m);
  }
  .entry.on {
    background: var(--bg-active);
  }
  .entry .name {
    flex: 1;
    min-width: 0;
    height: 32px;
    text-align: left;
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .entry.on .name {
    color: #fff;
  }
  .mini {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
    line-height: 1;
  }
  .mini:hover {
    border-color: var(--border-strong);
    color: #fff;
  }
  section {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 20px;
    overflow-y: auto;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-3);
  }
  .slider {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 4px;
    color: var(--text-3);
  }
  .slider input[type="range"] {
    grid-column: 1 / -1;
    width: 100%;
  }
  .number {
    display: flex;
    align-items: center;
    gap: 2px;
    color: var(--text-muted);
  }
  .number input {
    width: 58px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    text-align: right;
    font-variant-numeric: tabular-nums;
    user-select: text;
    appearance: textfield;
  }
  .number input::-webkit-inner-spin-button {
    appearance: none;
  }
  .color {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .color span {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.35);
  }
  .preview {
    height: 14px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
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
    height: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: #fff;
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
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .blend {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-3);
  }
  .hint {
    margin: -4px 0 0;
    color: var(--text-muted);
    font-size: 11px;
  }
</style>
