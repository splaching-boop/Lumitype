<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { endAdjustmentPreview, previewAdjustment, warmAdjustment } from "../lib/adjust-preview";
  import { editLatest, whenIdle } from "../lib/engine";
  import type { Adjustment, LayerInfo } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import type { GradientStop } from "../lib/gradient";
  import GradientStops from "./GradientStops.svelte";
  import HueRangeBar, { type HueRange, MIN_FEATHER, MIN_SPAN, MAX_SPAN, fitFeathers, forward } from "./HueRangeBar.svelte";

  // Curseurs du calque de réglage sélectionné, appliqués en direct ; une série de mouvements ne
  // fait qu'un état d'historique.
  let { layer, adjustment: engineAdjustment }: { layer: LayerInfo; adjustment: Adjustment } = $props();
  /** Valeurs en cours de réglage (en avance sur le moteur) : les curseurs et l'aperçu les suivent sans attendre. */
  let live = $state<Adjustment | null>(null);
  const adjustment = $derived(live ?? engineAdjustment);

  interface Control {
    key: string;
    label: string;
    min: number;
    max: number;
    step: number;
    neutral: number;
    unit: string;
    /** Titre de section affiché avant ce curseur. */
    group?: string;
    /** Dégradé de fond de la piste : donne le sens du curseur (bleu -> jaune pour la température…). */
    track?: string;
  }
  const slider = (key: string, label: string, min: number, max: number, neutral = 0, step = 1, unit = "", extra: Partial<Control> = {}): Control => ({ key, label, min, max, step, neutral, unit, ...extra });
  const CONTROLS: Record<Adjustment["type"], Control[]> = {
    brightnessContrast: [slider("brightness", "Luminosité", -100, 100), slider("contrast", "Contraste", -100, 100)],
    hueSaturation: [
      slider("hue", "Teinte", -180, 180, 0, 1, "°", { track: "linear-gradient(90deg, #00ffff, #0000ff, #ff00ff, #ff0000, #ffff00, #00ff00, #00ffff)" }),
      slider("saturation", "Saturation", -100, 100, 0, 1, "", { track: "linear-gradient(90deg, #8a8a8a, #ff2d2d)" }),
      slider("lightness", "Luminosité", -100, 100, 0, 1, "", { track: "linear-gradient(90deg, #000000, #ffffff)" }),
    ],
    levels: [slider("black", "Noirs", 0, 253), slider("gamma", "Tons moyens", 0.1, 5, 1, 0.01), slider("white", "Blancs", 2, 255, 255)],
    invert: [],
    negative: [],
    gradientMap: [slider("smoothness", "Lissage", 0, 100, 100, 1, " %")],
    develop: [
      slider("temperature", t("Température"), -100, 100, 0, 1, "", { group: t("Balance des blancs"), track: t("linear-gradient(90deg, #3b7bd6, #d9d9d9, #e8b53a)") }),
      slider("tint", "Nuance", -100, 100, 0, 1, "", { track: "linear-gradient(90deg, #4caf50, #d9d9d9, #c850c0)" }),
      slider("exposure", "Exposition", -5, 5, 0, 0.05, " IL", { group: t("Tonalité") }),
      slider("contrast", "Contraste", -100, 100),
      slider("highlights", "Hautes lumières", -100, 100),
      slider("shadows", "Ombres", -100, 100),
      slider("whites", "Blancs", -100, 100),
      slider("blacks", "Noirs", -100, 100),
      slider("clarity", t("Clarté"), -100, 100, 0, 1, "", { group: t("Présence") }),
      slider("dehaze", "Correction du voile", -100, 100),
      slider("vibrance", "Vibrance", -100, 100),
      slider("saturation", "Saturation", -100, 100),
      slider("redHue", t("Rouge primaire"), -100, 100, 0, 1, "", { group: t("Étalonnage · teinte"), track: t("linear-gradient(90deg, #e0407a, #e53935, #f08a24)") }),
      slider("greenHue", "Vert primaire", -100, 100, 0, 1, "", { track: "linear-gradient(90deg, #c6d22a, #43a047, #26b3a0)" }),
      slider("blueHue", "Bleu primaire", -100, 100, 0, 1, "", { track: "linear-gradient(90deg, #29a3d6, #3949ab, #8e44c9)" }),
    ],
  };

  // Le panneau vient de s'ouvrir sur ce calque : on prépare l'aperçu avant même le premier geste.
  $effect(() => {
    warmAdjustment(layer.id, engineAdjustment);
  });

  const controls = $derived(CONTROLS[adjustment.type]);
  const values = $derived(adjustment as unknown as Record<string, number>);
  function send(next: Adjustment): void {
    // Niveaux : le point noir reste sous le point blanc.
    if (next.type === "levels") next.black = Math.min(next.black, next.white - 2);
    live = next;
    // Instantané : la carte graphique applique le réglage à l'image tout de suite ; le moteur suit en coulisse.
    previewAdjustment(layer.id, next, layer.opacity * layer.fill);
    editLatest(`reglage-${layer.id}`, { op: "setAdjustment", id: layer.id, adjustment: next });
  }

  /** Fin d'un geste (curseur relâché, double-clic, réinitialisation) : l'image du moteur reprend la main. */
  function settle(): void {
    endAdjustmentPreview(layer.id);
    void whenIdle(`reglage-${layer.id}`).then(() => (live = null));
  }

  function set(key: string, value: number): void {
    if (Number.isFinite(value)) send({ ...adjustment, [key]: value } as Adjustment);
  }

  function reset(): void {
    send({ ...adjustment, ...Object.fromEntries(controls.map((c) => [c.key, c.neutral])) } as Adjustment);
    settle();
  }

  // ---- Teinte/Saturation : plage de teintes visée, choisie à la pipette sur l'image.
  const RAINBOW = "linear-gradient(90deg, #ff0000, #ffff00, #00ff00, #00ffff, #0000ff, #ff00ff, #ff0000)";
  const hueOf = ([r, g, b]: number[]): number | null => {
    const [max, min] = [Math.max(r ?? 0, g ?? 0, b ?? 0), Math.min(r ?? 0, g ?? 0, b ?? 0)];
    if (max - min < 12) return null; // un gris n'a pas de teinte
    const d = max - min;
    const h = max === r ? (((g ?? 0) - (b ?? 0)) / d) % 6 : max === g ? ((b ?? 0) - (r ?? 0)) / d + 2 : ((r ?? 0) - (g ?? 0)) / d + 4;
    return (h * 60 + 360) % 360;
  };
  /**
   * Plage visée, telle que la montre la barre : quatre angles en degrés. Un fichier d'avant les
   * quatre poignées n'a que le centre et la demi-largeur : on les retrouve ici à l'identique, en
   * gardant la transition sur le dernier tiers, comme le faisait le moteur.
   */
  const range = $derived.by<HueRange | null>(() => {
    if (adjustment.type !== "hueSaturation") return null;
    const { rangeStart = 0, rangeEnd = 0, featherIn = 0, featherOut = 0, rangeCenter = 0, rangeWidth = 0 } = adjustment;
    if (forward(rangeStart, rangeEnd) > 0) return { start: rangeStart, end: rangeEnd, featherIn, featherOut };
    if (rangeWidth > 0) {
      const full = Math.min(180, Math.max(5, rangeWidth)) * 0.67;
      const fade = Math.max(MIN_FEATHER, Math.min(180, Math.max(5, rangeWidth)) - full);
      return { start: (rangeCenter - full + 360) % 360, end: (rangeCenter + full) % 360, featherIn: fade, featherOut: fade };
    }
    return null;
  });

  /** Envoie la plage au moteur ; les anciens champs sont remis à zéro pour ne pas reprendre le dessus. */
  function setRange(next: HueRange | null): void {
    const cleared = { rangeCenter: 0, rangeWidth: 0 };
    if (!next) send({ ...adjustment, rangeStart: 0, rangeEnd: 0, featherIn: 0, featherOut: 0, ...cleared } as Adjustment);
    else send({ ...adjustment, rangeStart: next.start, rangeEnd: next.end, featherIn: next.featherIn, featherOut: next.featherOut, ...cleared } as Adjustment);
  }

  /** Pipette : viser cette couleur, élargir la plage jusqu'à elle, ou la resserrer pour l'exclure. */
  let pipette = $state<"set" | "add" | "sub">("set");
  const PIPETTES = [
    { mode: "set" as const, label: t("Pipette : ne viser que la couleur cliquée") },
    { mode: "add" as const, label: t("Pipette + : élargir la plage jusqu'à la couleur cliquée") },
    { mode: "sub" as const, label: t("Pipette − : resserrer la plage pour exclure la couleur cliquée") },
  ];

  function armPipette(mode: "set" | "add" | "sub"): void {
    pipette = mode;
    app.eyedropperTarget = (rgb) => {
      const hue = hueOf(rgb);
      app.eyedropperTarget = null;
      if (hue === null) {
        app.error = t("Cette couleur est un gris : elle n'a pas de teinte à cibler.");
        return;
      }
      const current = range;
      if (mode === "set" || !current) {
        // Plage serrée sur la couleur cliquée : le curseur Teinte ne décale plus qu'elle.
        setRange(fitFeathers({ start: (hue - 10 + 360) % 360, end: (hue + 10) % 360, featherIn: 30, featherOut: 30 }));
      } else if (mode === "add") {
        // La barre claire la plus proche vient chercher la couleur, avec une petite marge.
        const before = forward(hue, current.start);
        const after = forward(current.end, hue);
        const next = before <= after ? (hue - 5 + 360) % 360 : (hue + 5) % 360;
        const width = before <= after ? forward(next, current.end) : forward(current.start, next);
        if (width <= MAX_SPAN) setRange(fitFeathers(before <= after ? { ...current, start: next } : { ...current, end: next }));
        else app.error = t("La plage est au maximum : au-delà, plus aucune couleur ne serait épargnée.");
      } else {
        // On resserre du côté le plus proche pour exclure la couleur, sans écraser la plage.
        const fromStart = forward(current.start, hue);
        const toEnd = forward(hue, current.end);
        const next = fromStart <= toEnd ? { ...current, start: (hue + 2) % 360 } : { ...current, end: (hue - 2 + 360) % 360 };
        if (span(next) >= MIN_SPAN) setRange(next);
      }
      settle();
    };
    app.eyedropperArmed = true;
  }

  const span = (r: HueRange) => forward(r.start, r.end);

  // ---- Transfert de dégradé : les arrêts, dans la rangée partagée avec l'outil Dégradé.
  const mapStops = $derived<GradientStop[]>(adjustment.type === "gradientMap" ? adjustment.stops : []);
  function setStops(stops: GradientStop[]): void {
    if (adjustment.type === "gradientMap") send({ ...adjustment, stops });
  }
  function toggle(key: "reverse" | "dither", value: boolean): void {
    if (adjustment.type === "gradientMap") {
      send({ ...adjustment, [key]: value });
      settle();
    }
  }

  const shown = (control: Control) => {
    const value = values[control.key] ?? control.neutral;
    const text = control.step < 1 ? value.toFixed(2) : String(Math.round(value));
    return `${value > 0 && control.min < 0 ? "+" : ""}${text}${control.unit}`;
  };
</script>

<div class="adjust">
  {#if adjustment.type === "hueSaturation"}
    <div class="target">
      <span>{range ? `Couleurs visées : ${Math.round((range.start + span(range) / 2) % 360)}°` : t("Couleurs visées : toutes")}</span>
      {#if range}<button onclick={() => (setRange(null), settle())}>{t("Toutes")}</button>{/if}
    </div>
    <!-- Barres de repère : en haut les teintes d'origine, en bas ce qu'elles deviennent. -->
    <div class="bars" aria-hidden="true">
      <div style:background={RAINBOW}></div>
      <div style:background={RAINBOW} class="shifted" style:--shift="{-(values.hue ?? 0) / 3.6}%"></div>
    </div>
    {#if range}
      <!-- Quatre poignées, comme Photoshop : barres claires = correction entière, triangles = transition. -->
      <HueRangeBar {range} onchange={(next) => setRange(next)} onsettle={settle} />
    {/if}
    <div class="pipettes">
      <!-- Les trois pipettes de Photoshop : viser, élargir, resserrer. Même taille, même centrage,
           et un badge + ou − à la pointe pour les distinguer de l'outil Pipette de la barre. -->
      {#each PIPETTES as tool (tool.mode)}
        <button
          class:armed={app.eyedropperArmed && pipette === tool.mode}
          disabled={tool.mode !== "set" && !range}
          title={t(tool.label)}
          aria-label={t(tool.label)}
          onclick={() => armPipette(tool.mode)}
        >
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
            <path d="M15 4.5l4.5 4.5-2.8 2.8-4.5-4.5zM11.7 7.8L3.5 16v2.2h2.2l8.2-8.2" />
            {#if tool.mode !== "set"}<path d="M14.5 18.5h6" />{/if}
            {#if tool.mode === "add"}<path d="M17.5 15.5v6" />{/if}
          </svg>
        </button>
      {/each}
    </div>
  {/if}
  {#if adjustment.type === "gradientMap"}
    <div class="map">
      <GradientStops stops={mapStops} onchange={setStops} onsettle={settle} reversible={false} />
      <div class="toggles">
        <label class="check"><input type="checkbox" checked={adjustment.reverse} onchange={(event) => toggle("reverse", event.currentTarget.checked)} /> {t("Inverser")}</label>
        <label class="check"><input type="checkbox" checked={adjustment.dither} onchange={(event) => toggle("dither", event.currentTarget.checked)} /> {t("Simuler (tramage)")}</label>
      </div>
    </div>
  {/if}
  {#each controls as control (control.key)}
    {#if control.group}
      <h3>{t(control.group)}</h3>
    {/if}
    <label>
      <span>{t(control.label)}</span>
      <output>{shown(control)}</output>
      <input
        type="range"
        class:tinted={control.track !== undefined}
        style:--track={control.track}
        min={control.min}
        max={control.max}
        step={control.step}
        value={values[control.key] ?? control.neutral}
        oninput={(event) => set(control.key, event.currentTarget.valueAsNumber)}
        onchange={settle}
        ondblclick={() => (set(control.key, control.neutral), settle())}
      />
    </label>
  {:else}
    <p>{adjustment.type === "negative" ? t("Négatif : sans paramètre, tout est retourné, le blanc devient noir — un négatif de pellicule.") : t("Inverser : sans paramètre, retourne les couleurs mais laisse le papier blanc (le blanc pur n'est pas touché). Écrêté (par défaut), il ne vise que le calque du dessous.")}</p>
  {/each}
  {#if controls.length > 0}
    <button onclick={reset}>{t("Réinitialiser")}</button>
  {/if}
  <p>
    {layer.clipped
      ? t("Écrêté : ne corrige que le calque juste dessous. Ctrl+Alt+G le libère pour corriger tout ce qui est dessous.")
      : t("Corrige tous les calques situés dessous. Ctrl+Alt+G l'écrête sur le seul calque du dessous.")}
    Opacité, masque et mode de fusion dosent l'effet. Double-clic sur un curseur : valeur neutre.
  </p>
  {#if adjustment.type === "gradientMap"}
    <p class="note">{t("La luminosité de chaque pixel choisit une couleur le long du dégradé : le noir prend l'arrêt de gauche, le blanc celui de droite. L'opacité d'un arrêt laisse passer la couleur d'origine.")}</p>
  {/if}
  {#if adjustment.type === "develop"}
    <p class="note">{t("Exposition et balance des blancs sont calculées en lumière linéaire, comme dans Lightroom. Clarté et Correction du voile sont ici des approximations pixel par pixel (un calque de réglage ne voit pas le voisinage) : pour le rendu exact, et pour la Texture, utilisez Filtre &gt; Texture, clarté, voile.")}</p>
  {/if}
</div>

<style>
  .map {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-bottom: 4px;
  }
  .toggles {
    display: flex;
    gap: 16px;
    flex-wrap: wrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
  }
  .target {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-3);
  }
  .target span {
    flex: 1;
    min-width: 0;
  }
  .target button {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text);
    white-space: nowrap;
  }
  /* Les trois pipettes du réglage : trois carrés identiques, icône centrée, aucun texte. */
  .pipettes {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
  }
  .pipettes button {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text-2);
  }
  .pipettes button:not(:disabled):hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .pipettes button.armed {
    background: var(--pill-bg);
    border-color: transparent;
    color: var(--pill-text);
  }
  .pipettes button:disabled {
    color: var(--text-disabled);
  }
  .pipettes svg {
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .bars {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 3px;
    overflow: hidden;
    border-radius: var(--radius-s);
  }
  .bars div {
    height: 9px;
  }
  /* La barre du bas est la même bande de teintes, glissée de la valeur du curseur (elle se répète sur les bords). */
  .bars .shifted {
    width: 300%;
    margin-left: -100%;
    background-size: 33.333% 100% !important;
    background-position: 0 0 !important;
    transform: translateX(calc(var(--shift) / 3));
  }
  .note {
    margin: 4px 0 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .adjust {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
  }
  h3 {
    margin: 8px 0 -2px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  h3:first-child {
    margin-top: 0;
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px;
    color: var(--text-3);
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  input {
    grid-column: 1 / -1;
    width: 100%;
  }
  /* Piste colorée : indique le sens du curseur. Le dessin (piste, bouton) est celui de tous les curseurs,
     dans theme/base.css ; seul le fond change. */
  input.tinted {
    background: var(--track);
  }
  button {
    align-self: flex-start;
    height: 26px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text-2);
  }
  button:hover {
    color: var(--text);
    border-color: #fff;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
</style>
