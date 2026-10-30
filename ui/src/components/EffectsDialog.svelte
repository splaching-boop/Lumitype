<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import BlendSelect from "./BlendSelect.svelte";
  import BlendIfRamp from "./BlendIfRamp.svelte";
  import { onDestroy, onMount, untrack } from "svelte";
  import { rgbToHex } from "../lib/color";
  import { dropLatest, edit, editLatest, whenIdle } from "../lib/engine";
  import { cancelEffectsPreview, finishEffectsPreview, holdEffectsSource, previewEffects } from "../lib/effects-preview";
  import { api, type BevelStyle, type BlendIf, type BlendIfChannel, type BlendMode, type EditOp, type LayerEffect, type LayerInfo, type PatternInfo, type Rgb } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import ColorPopover from "./ColorPopover.svelte";

  // Effets de calque (fx). Panneau flottant, qu'on déplace par son titre : l'image reste visible et
  // utilisable pendant qu'on règle. La liste de gauche montre les options de fusion puis chaque COUCHE d'effet,
  // dans l'ordre de Photoshop ; le « + » d'un type en ajoute une (trois contours, deux ombres…) là où Photoshop
  // le permet. Tout s'applique en direct ; Annuler revient à l'état d'avant.
  let { layer }: { layer: LayerInfo } = $props();

  type Kind = LayerEffect["type"];
  const LABELS: Record<Kind, string> = {
    bevelEmboss: "Biseautage et estampage",
    stroke: "Contour",
    innerShadow: "Ombre interne",
    innerGlow: "Lueur interne",
    satin: "Satin",
    colorOverlay: "Incrustation de couleur",
    gradientOverlay: "Incrustation de dégradé",
    patternOverlay: "Incrustation de motif",
    outerGlow: "Lueur externe",
    dropShadow: "Ombre portée",
  };
  const DEFAULTS: Record<Kind, LayerEffect> = {
    // Réglages par défaut de Photoshop : ombres et satin en Produit, lueurs en Superposition, le reste en Normal.
    dropShadow: { type: "dropShadow", enabled: true, color: [0, 0, 0], opacity: 0.55, angle: 120, distance: 12, size: 16, blend: "multiply" },
    innerShadow: { type: "innerShadow", enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 120, distance: 6, size: 10, blend: "multiply" },
    outerGlow: { type: "outerGlow", enabled: true, color: [255, 236, 170], opacity: 0.75, size: 24, blend: "screen" },
    innerGlow: { type: "innerGlow", enabled: true, color: [255, 255, 255], opacity: 0.6, size: 14, blend: "screen" },
    stroke: { type: "stroke", enabled: true, color: [255, 255, 255], opacity: 1, width: 6, position: "outside", blend: "normal" },
    colorOverlay: { type: "colorOverlay", enabled: true, color: [255, 0, 80], opacity: 1, blend: "normal" },
    gradientOverlay: { type: "gradientOverlay", enabled: true, from: [255, 80, 120], to: [80, 120, 255], opacity: 1, angle: 90, radial: false, blend: "normal" },
    patternOverlay: { type: "patternOverlay", enabled: true, pattern: "points", scale: 100, color: [0, 0, 0], background: [255, 255, 255], opacity: 1, blend: "normal" },
    satin: { type: "satin", enabled: true, color: [0, 0, 0], opacity: 0.5, angle: 19, distance: 11, size: 14, invert: true, blend: "multiply" },
    bevelEmboss: {
      type: "bevelEmboss",
      enabled: true,
      style: "inner_bevel",
      technique: "smooth",
      depth: 1,
      direction: "up",
      size: 5,
      soften: 0,
      angle: 120,
      altitude: 30,
      highlight: [255, 255, 255],
      highlightOpacity: 0.75,
      highlightBlend: "screen",
      shadow: [0, 0, 0],
      shadowOpacity: 0.75,
      shadowBlend: "multiply",
    },
  };
  /** Ordre de la liste : celui de Photoshop (du haut de la pile vers le bas). */
  const ORDER: Kind[] = ["bevelEmboss", "stroke", "innerShadow", "innerGlow", "satin", "colorOverlay", "gradientOverlay", "patternOverlay", "outerGlow", "dropShadow"];
  /** Types dont Photoshop admet plusieurs couches (bouton « + »). */
  const MULTIPLE = new Set<Kind>(["stroke", "innerShadow", "colorOverlay", "gradientOverlay", "dropShadow"]);
  const BEVEL_STYLES: [BevelStyle, string][] = [
    ["outer_bevel", "Biseau externe"],
    ["inner_bevel", "Biseau interne"],
    ["emboss", "Estampage"],
    ["pillow_emboss", "Estampage paysager"],
  ];

  /** Couche sélectionnée : son rang dans la liste des effets du calque. */
  let selected = $state(untrack(() => Math.max(0, layer.effects.findIndex((e) => e.enabled))));
  /** Page affichée à droite : un effet (`selected`) ou les options de fusion du calque. */
  let page = $state<"effect" | "blending">("effect");
  type ColorKey = "color" | "from" | "to" | "background" | "highlight" | "shadow";
  let picking = $state<{ key: ColorKey; x: number; y: number } | null>(null);
  // Position du panneau : au milieu de la zone de travail à l'ouverture, puis là où on le pose.
  let position = $state(untrack(() => ({ x: Math.max(24, Math.round(window.innerWidth / 2 - 350)), y: 96 })));
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

  /**
   * Options de fusion réglées dans cette fenêtre (`null` : pas touchées, celles du calque). Elles partent au moteur
   * avec les effets, dans la même opération (`setStyle`) : un seul geste, qu'Annuler retire en bloc.
   */
  type Blending = { blend: BlendMode; opacity: number; fill: number; blendIf: BlendIf };
  let blending = $state<Blending | null>(null);
  /** Comparaison sans effet : plages entières, sur le gris. */
  const NO_RANGE: BlendIf = { channel: "gray", this: [0, 0, 255, 255], under: [0, 0, 255, 255] };
  const shownBlending = $derived(blending ?? { blend: layer.blend, opacity: layer.opacity, fill: layer.fill, blendIf: layer.blendIf ?? NO_RANGE });
  const CHANNELS: [BlendIfChannel, string][] = [
    ["gray", "Gris"],
    ["red", "Rouge"],
    ["green", "Vert"],
    ["blue", "Bleu"],
  ];

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

  /** Opération envoyée au moteur : les effets, plus les options de fusion si on les a réglées ici. */
  const styleOp = (list: LayerEffect[]): EditOp => ({ op: "setStyle", ids: [layer.id], effects: list, ...(blending ? $state.snapshot(blending) : {}), gesture });

  /**
   * Nouveaux effets. Si la carte graphique compose le document, elle les montre tout de suite et le moteur ne les
   * rend qu'au relâchement (`now`) ou après une courte pause ; sinon chaque pas passe par le moteur, comme avant
   * (biseautage et incrustation de motif passent toujours par lui).
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
    editLatest(gesture, styleOp(next));
  }

  /** Les effets de l'aperçu partent au moteur ; son rendu remplacera l'aperçu dès qu'il sera chargé. */
  function commit(): void {
    if (commitTimer) clearTimeout(commitTimer);
    commitTimer = null;
    const list = draft;
    if (!list || list === committed) return;
    committed = list;
    touched = true;
    editLatest(gesture, styleOp(list));
    void whenIdle(gesture).then(() => {
      if (draft !== list) return; // réglé de nouveau entre-temps : l'aperçu continue
      draft = null;
      finishEffectsPreview(layer.id);
    });
  }

  /** Options de fusion : rendues par le moteur à chaque pas, avec les effets du moment. */
  function setBlending(change: Partial<Blending>): void {
    blending = { ...$state.snapshot(shownBlending), ...change };
    if (draft) {
      committed = null;
      commit();
      return;
    }
    touched = true;
    editLatest(gesture, styleOp(copy()));
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
    // Un seul exemplaire possible (biseautage, satin…) : on sélectionne celui qui existe.
    if (last >= 0 && !MULTIPLE.has(kind)) {
      page = "effect";
      selected = last;
      return;
    }
    const at = last >= 0 ? last + 1 : list.length;
    // Une couche de plus du même type reprend les réglages de la précédente, en plus large : un second contour se voit tout de suite.
    const previous = last >= 0 ? list[last] : undefined;
    let layerEffect: LayerEffect = { ...(previous ?? DEFAULTS[kind]), enabled: true };
    if (previous && layerEffect.type === "stroke") layerEffect = { ...layerEffect, width: Math.min(100, layerEffect.width * 2) };
    list.splice(at, 0, layerEffect);
    send(list, true);
    page = "effect";
    selected = at;
  }

  /** Menu du bouton fx : la fenêtre s'ouvre (ou se règle) sur cet effet, ajouté et coché s'il manquait. */
  function focusOn(target: string): void {
    if (target === "blending") {
      page = "blending";
      return;
    }
    const kind = target as Kind;
    if (!(kind in LABELS)) return;
    const index = effects.findIndex((e) => e.type === kind);
    if (index < 0) add(kind);
    else [page, selected] = ["effect", index];
  }
  onMount(() => {
    const target = app.effectsFocus;
    if (target !== null) {
      app.effectsFocus = null;
      focusOn(target);
    }
  });
  // Fenêtre déjà ouverte sur ce calque, puis nouveau choix dans le menu fx : la page suit.
  $effect(() => {
    const target = app.effectsFocus;
    if (target === null) return;
    untrack(() => {
      app.effectsFocus = null;
      focusOn(target);
    });
  });

  function toggle(row: (typeof rows)[number], on: boolean): void {
    if (row.index === null) {
      if (on) add(row.kind);
      return;
    }
    const list = copy();
    const target = list[row.index];
    if (target) list[row.index] = { ...target, enabled: on };
    send(list, true);
    if (on) [page, selected] = ["effect", row.index];
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
    // Annuler : les réglages encore en route sont abandonnés, puis tout le geste est retiré — effets et options de
    // fusion reviennent exactement à ce qu'ils étaient à l'ouverture, sans laisser de trace dans l'historique.
    dropLatest(gesture);
    void whenIdle(gesture).then(() => edit({ op: "cancelGesture", gesture }));
  }

  const field = (key: string): number => (current as unknown as Record<string, number> | undefined)?.[key] ?? 0;
  const has = (key: string): boolean => current !== undefined && key in current;
  const colorOf = (key: ColorKey): Rgb => ((current as unknown as Record<string, Rgb> | undefined)?.[key] ?? [0, 0, 0]) as Rgb;

  // ---- Incrustation de motif : la bibliothèque des calques de remplissage Motif, vignettes comprises.
  let catalog = $state<PatternInfo[]>([]);
  const thumbs = $state<Record<string, string>>({});
  let catalogAsked = false;
  $effect(() => {
    if (current?.type !== "patternOverlay" || catalogAsked) return;
    catalogAsked = true;
    void api
      .patternCatalog()
      .then(async (list) => {
        catalog = list;
        for (const pattern of list) {
          const bytes = new Uint8ClampedArray(await api.patternSwatch(pattern.id, 40, 1));
          if (bytes.length !== 40 * 40 * 4) continue;
          const canvas = document.createElement("canvas");
          canvas.width = canvas.height = 40;
          canvas.getContext("2d")?.putImageData(new ImageData(bytes, 40, 40), 0, 0);
          thumbs[pattern.id] = canvas.toDataURL();
        }
      })
      .catch(() => (app.error = t("Bibliothèque de motifs indisponible.")));
  });

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
    <span>{t(label)}</span>
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

{#snippet percent(label: string, value: number, apply: (value: number) => void)}
  <!-- Options de fusion : de 0 à 100 %, rendues par le moteur à chaque pas. -->
  <label class="slider">
    <span>{t(label)}</span>
    <span class="number">
      <input
        type="number"
        min="0"
        max="100"
        step="1"
        value={Math.round(value * 100)}
        onkeydown={(event) => event.stopPropagation()}
        onchange={(event) => Number.isFinite(event.currentTarget.valueAsNumber) && apply(Math.min(1, Math.max(0, event.currentTarget.valueAsNumber / 100)))}
      /> %
    </span>
    <input type="range" min="0" max="1" step="0.01" {value} oninput={(event) => apply(event.currentTarget.valueAsNumber)} />
  </label>
{/snippet}

{#snippet swatch(key: ColorKey, label: string)}
  <div class="line">
    <span>{t(label)}</span>
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

{#snippet segmented(key: string, options: readonly (readonly [string, string])[], value: string)}
  <div class="segmented">
    {#each options as [id, text] (id)}
      <button class:on={value === id} onclick={() => patch({ [key]: id }, true)}>{t(text)}</button>
    {/each}
  </div>
{/snippet}

{#snippet blendOf(key: "blend" | "highlightBlend" | "shadowBlend", label: string, value: BlendMode)}
  <div class="blend">
    <span>{t(label)}</span>
    <!-- Même menu que pour les calques : survoler un mode l'applique en direct, à cet effet seulement. -->
    <BlendSelect {value} exclude={["dissolve"]} onpreview={(mode) => patch({ [key]: mode })} onchoose={(mode) => patch({ [key]: mode }, true)} />
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
      <div class="entry options" class:on={page === "blending"}>
        <button class="name" onclick={() => (page = "blending")}>{t("Options de fusion")}</button>
      </div>
      <div class="rule"></div>
      {#each rows as row (`${row.kind}-${row.n}`)}
        <div class="entry" class:on={page === "effect" && row.index !== null && selected === row.index}>
          <input type="checkbox" aria-label="{t('Activer')} {t(LABELS[row.kind])}" checked={row.enabled} onchange={(event) => toggle(row, event.currentTarget.checked)} />
          <button class="name" onclick={() => (row.index === null ? add(row.kind) : ([page, selected] = ["effect", row.index]))}>{t(LABELS[row.kind])}{row.count > 1 ? ` ${row.n + 1}` : ""}</button>
          {#if row.index !== null && row.count > 1}
            <button class="mini" title={t("Retirer cette couche")} onclick={() => row.index !== null && remove(row.index)}>−</button>
          {/if}
          {#if row.first && MULTIPLE.has(row.kind)}
            <button class="mini" title="{t('Ajouter une couche')} : {t(LABELS[row.kind])}" onclick={() => add(row.kind)}>+</button>
          {/if}
        </div>
      {/each}
    </nav>
    <section>
      {#if page === "blending"}
        <h3>{t("Options de fusion")}</h3>
        <h4>{t("Options générales")}</h4>
        <div class="blend">
          <span>{t("Mode de fusion")}</span>
          <BlendSelect value={shownBlending.blend} onpreview={(mode) => setBlending({ blend: mode })} onchoose={(mode) => setBlending({ blend: mode })} />
        </div>
        {@render percent("Opacité", shownBlending.opacity, (opacity) => setBlending({ opacity }))}
        <h4>{t("Options avancées")}</h4>
        {@render percent("Opacité du fond", shownBlending.fill, (fill) => setBlending({ fill }))}
        <p>{t("L'opacité du fond ne touche que les pixels du calque : ses effets (ombre, contour, biseau…) gardent leur opacité.")}</p>
        {#if layer.blend !== "pass_through" && !layer.adjustment}
          <h4>{t("Comparaison")}</h4>
          <div class="line">
            <span>{t("Comparaison sur")}</span>
            <select value={shownBlending.blendIf.channel} onchange={(event) => setBlending({ blendIf: { ...$state.snapshot(shownBlending.blendIf), channel: event.currentTarget.value as BlendIfChannel } })}>
              {#each CHANNELS as [id, text] (id)}<option value={id}>{t(text)}</option>{/each}
            </select>
          </div>
          <BlendIfRamp label={t("Calque actif")} channel={shownBlending.blendIf.channel} value={shownBlending.blendIf.this} onchange={(value) => setBlending({ blendIf: { ...$state.snapshot(shownBlending.blendIf), this: value } })} />
          <BlendIfRamp label={t("Calque du dessous")} channel={shownBlending.blendIf.channel} value={shownBlending.blendIf.under} onchange={(value) => setBlending({ blendIf: { ...$state.snapshot(shownBlending.blendIf), under: value } })} />
          <p class="hint">{t("Alt + glisser un triangle le scinde en deux : le calque apparaît en fondu entre les deux moitiés.")}</p>
        {/if}
      {:else if current}
        <h3>{t(LABELS[current.type])}</h3>
        {#if current.type === "bevelEmboss"}
          <h4>{t("Structure")}</h4>
          <div class="line">
            <span>{t("Style")}</span>
            <select value={current.style} onchange={(event) => patch({ style: event.currentTarget.value }, true)}>
              {#each BEVEL_STYLES as [id, text] (id)}<option value={id}>{t(text)}</option>{/each}
            </select>
          </div>
          <div class="line">
            <span>{t("Technique")}</span>
            {@render segmented("technique", [["smooth", "Lisse"], ["chisel_hard", "Ciselé dur"]], current.technique)}
          </div>
          {@render slider("depth", "Profondeur", 0.01, 10, 0.01, " %", 100)}
          <div class="line">
            <span>{t("Direction")}</span>
            {@render segmented("direction", [["up", "Haut"], ["down", "Bas"]], current.direction)}
          </div>
          {@render slider("size", "Taille", 0, 250, 1, " px", 1)}
          {@render slider("soften", "Flou", 0, 16, 1, " px", 1)}
          <h4>{t("Ombrage")}</h4>
          {@render slider("angle", "Angle", -180, 180, 1, "°", 1)}
          {@render slider("altitude", "Altitude", 0, 90, 1, "°", 1)}
          {@render blendOf("highlightBlend", "Mode des hautes lumières", current.highlightBlend)}
          {@render swatch("highlight", "Couleur des hautes lumières")}
          {@render slider("highlightOpacity", "Opacité des hautes lumières", 0, 1, 0.01, " %", 100)}
          {@render blendOf("shadowBlend", "Mode des ombres", current.shadowBlend)}
          {@render swatch("shadow", "Couleur des ombres")}
          {@render slider("shadowOpacity", "Opacité des ombres", 0, 1, 0.01, " %", 100)}
        {:else}
          {#if current.type === "gradientOverlay"}
            {@render swatch("from", "Couleur de départ")}
            {@render swatch("to", "Couleur d'arrivée")}
            <div class="preview" style:background="linear-gradient(90deg, {rgbToHex(current.from)}, {rgbToHex(current.to)})"></div>
            <label class="check"><input type="checkbox" checked={current.radial} onchange={(event) => patch({ radial: event.currentTarget.checked }, true)} /> {t("Radial (du centre vers les bords)")}</label>
          {:else if current.type === "patternOverlay"}
            <div class="patterns" role="radiogroup" aria-label={t("Motif")}>
              {#each catalog as pattern (pattern.id)}
                <button role="radio" aria-checked={current.pattern === pattern.id} class:on={current.pattern === pattern.id} title={t(pattern.name)} onclick={() => patch({ pattern: pattern.id }, true)}>
                  {#if thumbs[pattern.id]}<img src={thumbs[pattern.id]} alt="" />{/if}
                  <span>{t(pattern.name)}</span>
                </button>
              {/each}
            </div>
            {@render swatch("color", "Couleur du motif")}
            {@render swatch("background", "Couleur du fond")}
            <button class="link" onclick={() => current?.type === "patternOverlay" && patch({ color: current.background, background: current.color }, true)}>{t("Permuter les couleurs")}</button>
          {:else}
            {@render swatch("color", "Couleur")}
          {/if}
          {@render blendOf("blend", "Mode de fusion", current.blend ?? "normal")}
          {#if (current.type === "dropShadow" || current.type === "outerGlow") && (current.blend ?? "normal") !== "normal"}
            <p class="hint">{t("Se fond dans ce qui est sous le calque.")}</p>
          {/if}
          {@render slider("opacity", "Opacité", 0, 1, 0.01, " %", 100)}
          {#if has("angle")}{@render slider("angle", current.type === "gradientOverlay" || current.type === "satin" ? "Angle" : "Angle de la lumière", -180, 180, 1, "°", 1)}{/if}
          {#if has("distance")}{@render slider("distance", "Distance", 0, 250, 1, " px", 1)}{/if}
          {#if has("size")}{@render slider("size", current.type === "satin" ? "Taille" : "Taille (flou)", 0, 250, 1, " px", 1)}{/if}
          {#if has("scale")}{@render slider("scale", "Échelle", 1, 1000, 1, " %", 1)}{/if}
          {#if has("width")}{@render slider("width", "Épaisseur", 1, 100, 1, " px", 1)}{/if}
          {#if current.type === "satin"}
            <label class="check"><input type="checkbox" checked={current.invert} onchange={(event) => patch({ invert: event.currentTarget.checked }, true)} /> {t("Inverser")}</label>
          {/if}
          {#if current.type === "stroke"}
            <div class="line">
              <span>{t("Position")}</span>
              {@render segmented("position", [["outside", "Extérieur"], ["center", "Centre"], ["inside", "Intérieur"]], current.position)}
            </div>
            <p>{t("Plusieurs contours : le premier de la liste passe au-dessus. Pour un double contour, gardez le plus fin en premier.")}</p>
          {/if}
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
    width: 700px;
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
    width: 250px;
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
    color: var(--text-strong);
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
    color: var(--text-strong);
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
    border: 1px solid rgb(var(--ink) / 0.35);
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
    background: var(--bg-well);
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
    color: var(--text-strong);
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
  h4 {
    margin: 4px 0 -4px;
    font-size: 11px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-muted);
  }
  .rule {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
  .entry.options {
    padding-left: 12px;
  }
  select {
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  .link {
    align-self: flex-start;
    color: var(--text-2);
    text-decoration: underline;
  }
  .patterns {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 4px;
  }
  .patterns button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 3px 1px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-muted);
    font-size: 9px;
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
    width: 36px;
    height: 36px;
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
  .hint {
    margin: -4px 0 0;
    color: var(--text-muted);
    font-size: 11px;
  }
</style>
