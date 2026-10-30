<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { type BrushKind, MIXER_DYNAMICS, NEUTRAL_DYNAMICS } from "../lib/ipc";
  import { brushLibrary, strokePreview } from "../lib/brush-library.svelte";
  import { app } from "../state/app.svelte";

  // Pinceau, dans le panneau Propriétés : ronds de base (Net, Flou), accès au panneau des pinceaux (dossiers, packs
  // importés, aperçus), et toute la dynamique du pinceau choisi, pinceau double et texture compris.
  const BASICS = [
    { id: "hard", label: "Net", hint: t("Rond à bord franc"), icon: "M12 5a7 7 0 100 14 7 7 0 000-14z", fill: true, soft: false },
    { id: "soft", label: "Flou", hint: t("Rond à bord fondu"), icon: "M12 8a4 4 0 100 8 4 4 0 000-8z", fill: true, soft: true },
  ] as const;

  // `inline` : le contenu est posé tel quel dans le panneau Propriétés ; sinon, c'est le bouton de la barre
  // d'options, qui ouvre le panneau des pinceaux.
  let { inline = false }: { inline?: boolean } = $props();

  const settings = $derived(app.brushes[app.paintTool ?? "brush"]);
  const kind = $derived(app.brushKind);
  const tip = $derived(kind.type === "tip" ? app.brushTips.find((t) => t.id === kind.id) : undefined);
  const chosen = $derived(app.brushKey ? brushLibrary.all.find((e) => e.key === app.brushKey) : undefined);
  const current = $derived(chosen?.name ?? tip?.name ?? (kind.type === "spray" ? t("Spray") : kind.type === "mixer" ? t("Mélangeur") : settings.hardness >= 0.5 ? t("Net") : t("Flou")));
  let trigger = $state<HTMLButtonElement>();
  // Pinceau double et texture mis de côté quand on les décoche : on peut les rallumer sans rechoisir le pinceau.
  let parkedDual = $state(app.brushDual);
  let parkedTexture = $state(app.brushTexture);
  $effect(() => {
    void app.brushKey;
    if (app.brushDual) parkedDual = app.brushDual;
    if (app.brushTexture) parkedTexture = app.brushTexture;
  });

  onMount(() => {
    brushLibrary.load().catch((error: unknown) => app.fail(error));
  });

  /** Changer de forme remet la dynamique à zéro : chaque forme repart de son comportement naturel. */
  function choose(next: BrushKind, hardness?: number): void {
    app.brushKind = next;
    app.brushDynamics = next.type === "mixer" ? { ...MIXER_DYNAMICS } : { ...NEUTRAL_DYNAMICS };
    app.brushDual = null;
    app.brushTexture = null;
    parkedDual = null;
    parkedTexture = null;
    app.brushKey = next.type === "round" ? (hardness === 0 ? "basic:soft" : "basic:hard") : null;
    if (hardness !== undefined) settings.hardness = hardness;
  }

  function chooseBasic(id: (typeof BASICS)[number]["id"]): void {
    choose({ type: "round" }, id === "hard" ? 1 : 0);
  }

  function openPanel(): void {
    const rect = trigger?.getBoundingClientRect();
    app.brushPanel = app.brushPanel ? null : { x: rect ? rect.left : 80, y: rect ? rect.bottom + 6 : 80 };
  }

  // Aperçu vivant (voir le bloc `.live`) : redemandé au moteur à chaque réglage, une fois le curseur posé.
  let livePreview = $state<string | null>(null);
  let liveWidth = $state(240);
  const liveParams = $derived({
    size: settings.size,
    hardness: settings.hardness,
    opacity: settings.opacity,
    color: [255, 255, 255] as [number, number, number],
    erase: false,
    pressureSize: settings.pressureSize,
    pressureOpacity: settings.pressureOpacity,
    kind: $state.snapshot(app.brushKind),
    dynamics: $state.snapshot(app.brushDynamics),
    dual: $state.snapshot(app.brushDual),
    texture: $state.snapshot(app.brushTexture),
  });
  $effect(() => {
    if (!inline) return;
    const params = liveParams;
    const width = liveWidth;
    const timer = setTimeout(() => strokePreview(params, width, 70, true).then((url) => (livePreview = url), () => undefined), 90);
    return () => clearTimeout(timer);
  });

  const percent = (value: number) => `${Math.round(value * 100)} %`;

  // Taille du pinceau sur une échelle exponentielle : le premier cinquième de la course couvre le détail (1 à 5 px),
  // la fin monte vite jusqu'à `MAX_SIZE`. Position 0..1000 sur le curseur.
  const MAX_SIZE = 2500;
  const sizeToPosition = (size: number) => Math.round((Math.log(Math.max(1, size)) / Math.log(MAX_SIZE)) * 1000);
  const positionToSize = (position: number) => Math.max(1, Math.round(Math.exp((position / 1000) * Math.log(MAX_SIZE))));
  /** Graduations affichées sous le curseur (valeurs en px, à leur position exponentielle). */
  const SIZE_TICKS = [1, 5, 25, 100, 500, 2500];
</script>

{#if !inline}
  <button class="trigger" bind:this={trigger} title={t("Pinceaux")} onclick={openPanel}>
    {current}
    <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
  </button>
{/if}

{#snippet slider(label: string, value: number, min: number, max: number, step: number, shown: string, set: (v: number) => void)}
  <label class="slider">
    <span>{t(label)}</span>
    <output>{shown}</output>
    <input type="range" {min} {max} {step} {value} oninput={(event) => set(event.currentTarget.valueAsNumber)} />
  </label>
{/snippet}

{#if inline}
  <div class="panel inline">
    <div class="column">
      <p>{t("Récemment utilisés")}</p>
      <div class="basics recent">
        {#each brushLibrary.recent as entry (entry.key)}
          {@const entryTip = entry.kind.type === "tip" ? app.brushTips.find((x) => entry.kind.type === "tip" && x.id === entry.kind.id) : undefined}
          <button title={t(entry.name)} class:on={app.brushKey === entry.key} onclick={() => brushLibrary.apply(entry)}>
            {#if entryTip && app.brushThumbnails[entryTip.id]}
              <img src={app.brushThumbnails[entryTip.id]} alt="" />
            {:else}
              <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true" class="fill" class:soft={entry.kind.type === "round" && entry.hardness < 0.5}><path d="M12 5a7 7 0 100 14 7 7 0 000-14z" /></svg>
            {/if}
            <span>{t(entry.name)}</span>
          </button>
        {:else}
          {#each BASICS as basic (basic.id)}
            <button title={t(basic.hint)} class:on={app.brushKey === `basic:${basic.id}` || (!app.brushKey && kind.type === "round" && current === basic.label)} onclick={() => chooseBasic(basic.id)}>
              <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true" class:fill={basic.fill} class:soft={basic.soft}><path d={basic.icon} /></svg>
              <span>{t(basic.label)}</span>
            </button>
          {/each}
        {/each}
      </div>

      <p>{t("Pinceau")}</p>
      <button class="wide current" onclick={() => (app.brushPanel = app.brushPanel ? null : { x: 300, y: 90 })}>
        {#if tip && app.brushThumbnails[tip.id]}<img src={app.brushThumbnails[tip.id]} alt="" />{/if}
        <span>{t(current)}</span>
        <em>{t("Tous les pinceaux…")}</em>
      </button>
    </div>

    <div class="column">
      <!-- Aperçu vivant : un coup bref en courbe, pression de 0 à 100 %, redessiné par le moteur à chaque réglage. Il
           reste en haut pendant qu'on fait défiler les réglages : on voit ce que fait chacun. -->
      <div class="live" title={t("Coup bref, pression de 0 à 100 %")} bind:clientWidth={liveWidth}>
        {#if livePreview}<img src={livePreview} alt="" />{/if}
        <span class="live-name">{t(current)}</span>
        <span class="live-ramp"><i></i>{t("pression 0 → 100 %")}</span>
      </div>
      <!-- Taille : grande barre exponentielle, juste sous l'aperçu (détail fin au début, montée rapide à la fin). -->
      <label class="size">
        <span class="size-head"><span>{t("Taille")}</span><strong>{Math.round(settings.size)} px</strong></span>
        <input type="range" min="0" max="1000" step="1" value={sizeToPosition(settings.size)} oninput={(event) => (settings.size = positionToSize(event.currentTarget.valueAsNumber))} />
        <span class="ticks">{#each SIZE_TICKS as tick (tick)}<i style:left="{sizeToPosition(tick) / 10}%">{tick}</i>{/each}</span>
      </label>
      {#if app.paintTool === "brush"}
        <div class="dynamic-color">
          <label class="check"><input type="checkbox" checked={brushLibrary.dynamicColor} onchange={(event) => (brushLibrary.dynamicColor = event.currentTarget.checked)} /> <strong>{t("Couleur dynamique")}</strong></label>
          <p>{t("Chaque trait prend une nuance très proche de la couleur choisie, jamais tout à fait la même (teinte ±2 %, saturation et luminosité ±4 %).")}</p>
        </div>
      {/if}

      {#if kind.type === "round"}
        <h5>{t("Pression")}</h5>
        <label class="check"><input type="checkbox" bind:checked={settings.pressureSize} /> {t("Pression → taille")}</label>
        <label class="check"><input type="checkbox" bind:checked={settings.pressureOpacity} /> {t("Pression → opacité")}</label>
        {@render slider("Taille minimale (pression)", app.brushDynamics.minSize, 0, 1, 0.01, percent(app.brushDynamics.minSize), (v) => (app.brushDynamics.minSize = v))}
      {:else}
        <h5>{t("Forme de la pointe")}</h5>
        {@render slider("Espacement", app.brushDynamics.spacing, 0, 3, 0.01, app.brushDynamics.spacing > 0 ? percent(app.brushDynamics.spacing) : `Auto${tip ? ` (${percent(tip.spacing)})` : ""}`, (v) => (app.brushDynamics.spacing = v))}
        {#if kind.type === "tip"}
          {@render slider("Angle", app.brushDynamics.angle, -180, 180, 1, `${Math.round(app.brushDynamics.angle)}°`, (v) => (app.brushDynamics.angle = v))}
          {@render slider("Rondeur", app.brushDynamics.roundness || 1, 0.05, 1, 0.01, percent(app.brushDynamics.roundness || 1), (v) => (app.brushDynamics.roundness = v >= 1 ? 0 : v))}
          <label class="check"><input type="checkbox" bind:checked={app.brushDynamics.followDirection} /> {t("Suivre la direction du trait")}</label>
          <label class="check"><input type="checkbox" bind:checked={app.brushDynamics.flipX} /> {t("Retourner horizontalement")}</label>

          <h5>{t("Pression")}</h5>
          <label class="check"><input type="checkbox" bind:checked={settings.pressureSize} /> {t("Pression → taille")}</label>
          <label class="check"><input type="checkbox" bind:checked={settings.pressureOpacity} /> {t("Pression → opacité")}</label>
          {@render slider("Taille minimale (pression)", app.brushDynamics.minSize, 0, 1, 0.01, percent(app.brushDynamics.minSize), (v) => (app.brushDynamics.minSize = v))}

          <h5>{t("Variations")}</h5>
          {@render slider("Variation de taille", app.brushDynamics.sizeJitter, 0, 1, 0.01, percent(app.brushDynamics.sizeJitter), (v) => (app.brushDynamics.sizeJitter = v))}
          {@render slider("Variation d'angle", app.brushDynamics.angleJitter, 0, 1, 0.01, percent(app.brushDynamics.angleJitter), (v) => (app.brushDynamics.angleJitter = v))}
          {@render slider("Variation d'opacité", app.brushDynamics.opacityJitter, 0, 1, 0.01, percent(app.brushDynamics.opacityJitter), (v) => (app.brushDynamics.opacityJitter = v))}
        {/if}

        <h5>{t("Dispersion")}</h5>
        {@render slider("Dispersion", app.brushDynamics.scatter, 0, 3, 0.01, percent(app.brushDynamics.scatter), (v) => (app.brushDynamics.scatter = v))}

        {#if kind.type === "tip"}
          {#if parkedDual}
            <h5><label class="check"><input type="checkbox" checked={!!app.brushDual} onchange={(event) => (app.brushDual = event.currentTarget.checked ? parkedDual : null)} /> {t("Pinceau double")}</label></h5>
            {#if app.brushDual}
              {@const dual = app.brushDual}
              {@render slider("Taille du pinceau double", dual.ratio, 0.05, 4, 0.01, percent(dual.ratio), (v) => (dual.ratio = v))}
              {@render slider("Dispersion du pinceau double", dual.scatter, 0, 4, 0.01, percent(dual.scatter), (v) => (dual.scatter = v))}
            {/if}
          {/if}
          {#if parkedTexture}
            <h5><label class="check"><input type="checkbox" checked={!!app.brushTexture} onchange={(event) => (app.brushTexture = event.currentTarget.checked ? parkedTexture : null)} /> {t("Texture")}</label></h5>
            {#if app.brushTexture}
              {@const texture = app.brushTexture}
              {@render slider("Profondeur", texture.depth, 0, 1, 0.01, percent(texture.depth), (v) => (texture.depth = v))}
              {@render slider("Échelle", texture.scale, 0.05, 3, 0.01, percent(texture.scale), (v) => (texture.scale = v))}
              <label class="check"><input type="checkbox" bind:checked={texture.invert} /> {t("Inverser la texture")}</label>
            {/if}
          {/if}
        {:else if kind.type === "spray"}
          {@render slider("Densité", app.brushDynamics.amount || 1, 0.1, 4, 0.05, percent(app.brushDynamics.amount || 1), (v) => (app.brushDynamics.amount = v))}
        {:else}
          <!-- Les trois réglages du mélange de couleurs de Clip Studio Paint. -->
          <h5>{t("Mélange")}</h5>
          {@render slider("Quantité de peinture", app.brushDynamics.paint, 0, 1, 0.01, percent(app.brushDynamics.paint), (v) => (app.brushDynamics.paint = v))}
          {@render slider("Densité de la peinture", app.brushDynamics.amount || 0.7, 0.05, 1, 0.01, percent(app.brushDynamics.amount || 0.7), (v) => (app.brushDynamics.amount = v))}
          {@render slider("Étirement des couleurs", app.brushDynamics.stretch, 0, 0.98, 0.01, percent(app.brushDynamics.stretch), (v) => (app.brushDynamics.stretch = v))}
        {/if}
        {#if kind.type !== "mixer"}
          <h5>{t("Flux")}</h5>
          {@render slider("Flux", app.brushDynamics.flow || 1, 0.02, 1, 0.01, percent(app.brushDynamics.flow || 1), (v) => (app.brushDynamics.flow = v >= 1 ? 0 : v))}
        {/if}
        <button class="wide" onclick={() => (app.brushDynamics = kind.type === "mixer" ? { ...MIXER_DYNAMICS } : { ...NEUTRAL_DYNAMICS })}>{t("Réinitialiser la dynamique")}</button>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Dans Propriétés : une seule colonne, sans cadre ni ombre, à la largeur du panneau. */
  .panel.inline {
    position: static;
    width: auto;
    max-height: none;
    grid-template-columns: 1fr;
    padding: 12px 14px;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    overflow: visible;
  }
  .trigger {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    white-space: nowrap;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  svg.fill {
    fill: currentColor;
    stroke: none;
  }
  svg.soft {
    filter: blur(2.2px);
  }
  .panel {
    box-sizing: border-box;
    display: grid;
    gap: 18px;
  }
  .column {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  p {
    margin: 6px 0 0;
    color: var(--text-muted);
  }
  p:first-child {
    margin-top: 0;
  }
  .basics {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .basics button {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 8px 4px 7px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text-2);
    font-size: 11px;
  }
  .basics button:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .basics button.on {
    background: var(--bg-active);
    border-color: var(--accent);
    color: var(--text);
  }
  .basics.recent {
    grid-template-columns: repeat(5, 1fr);
  }
  .basics span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .basics img {
    width: 26px;
    height: 26px;
    object-fit: contain;
  }
  .size {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
  }
  .size-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    color: var(--text-2);
  }
  .size-head strong {
    color: var(--text);
    font-size: 18px;
    font-variant-numeric: tabular-nums;
  }
  .size input {
    width: 100%;
    height: 18px;
  }
  .ticks {
    position: relative;
    height: 14px;
    color: var(--text-muted);
    font-size: 10px;
  }
  .ticks i {
    position: absolute;
    transform: translateX(-50%);
    font-style: normal;
  }
  .ticks i:first-child {
    transform: none;
  }
  .ticks i:last-child {
    transform: translateX(-100%);
  }
  .dynamic-color {
    margin-top: 10px;
    padding: 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
  }
  .dynamic-color p {
    margin: 4px 0 0;
    font-size: 11px;
  }
  .wide {
    height: 28px;
    flex-shrink: 0;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text);
  }
  .wide:hover {
    border-color: var(--text);
  }
  .wide.current {
    height: 36px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    text-align: left;
  }
  .current img {
    filter: invert(var(--ink-invert));
    width: 26px;
    height: 26px;
    object-fit: contain;
  }
  .current span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .current em {
    font-style: normal;
    color: var(--text-muted);
  }
  .slider {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px;
    color: var(--text-3);
  }
  .slider input {
    grid-column: 1 / -1;
    width: 100%;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .live {
    position: sticky;
    top: 0;
    z-index: 2;
    height: 86px;
    display: grid;
    place-items: center;
    margin: 0 -4px 4px;
    background: var(--bg-well);
    border: 1px solid var(--border);
  }
  .live img {
    max-width: 100%;
    height: 70px;
    filter: invert(var(--ink-invert));
  }
  .live-name {
    position: absolute;
    left: 6px;
    top: 4px;
    color: var(--text-2);
    font-size: 11px;
  }
  .live-ramp {
    position: absolute;
    right: 6px;
    bottom: 4px;
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-muted);
    font-size: 10px;
  }
  .live-ramp i {
    width: 28px;
    height: 4px;
    background: linear-gradient(90deg, transparent, var(--text-2));
  }
  h5 {
    margin: 10px 0 2px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  h5 .check {
    text-transform: none;
    letter-spacing: 0;
    font-weight: 600;
  }
</style>
