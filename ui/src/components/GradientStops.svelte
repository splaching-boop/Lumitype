<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { rgbToHex } from "../lib/color";
  import { type GradientStop, PRESETS, colorAt, sorted, toCss } from "../lib/gradient";
  import ColorPopover from "./ColorPopover.svelte";

  // Rangée d'arrêts d'un dégradé, partagée par l'éditeur de l'outil Dégradé (fenêtre flottante) et
  // le calque de réglage Transfert de dégradé (panneau Propriétés). Un seul rang d'arrêts, chacun
  // portant sa couleur ET son opacité. Cliquer sous la barre ajoute un arrêt, le tirer vers le bas
  // le retire. Chaque changement remonte par `onchange` ; `onsettle` marque la fin d'un geste
  // (pour ne faire qu'un état d'historique d'une glissade).
  let {
    stops,
    onchange,
    onsettle,
    presets = true,
    reversible = true,
    children,
  }: {
    stops: GradientStop[];
    onchange: (stops: GradientStop[]) => void;
    onsettle?: () => void;
    /** Rangée des préréglages sous les arrêts. */
    presets?: boolean;
    /** Bouton « Inverser », qui retourne les positions des arrêts. */
    reversible?: boolean;
    /** Préréglages supplémentaires, placés avant ceux du catalogue. */
    children?: import("svelte").Snippet;
  } = $props();

  let bar = $state<HTMLDivElement>();
  let track = $state<HTMLDivElement>();
  let selected = $state(0);
  let colorPopover = $state<{ x: number; y: number } | null>(null);
  let dragging = $state<{ index: number; removing: boolean } | null>(null);

  const current = $derived(stops[Math.min(selected, stops.length - 1)]);

  function patch(index: number, change: Partial<GradientStop>): void {
    onchange(stops.map((s, i) => (i === index ? { ...s, ...change } : s)));
  }

  const positionOf = (event: PointerEvent): number => {
    const rect = bar?.getBoundingClientRect();
    return rect ? Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)) : 0;
  };

  function addStop(event: PointerEvent): void {
    const position = positionOf(event);
    // L'indice du nouvel arrêt se calcule AVANT `onchange` : si le parent remplace `stops` aussitôt,
    // `stops.length` compterait déjà l'arrêt ajouté et viserait une case vide (la glissade ne
    // déplacerait alors rien).
    const index = stops.length;
    onchange([...stops, { position, color: colorAt(stops, position) }]);
    selected = index;
    dragging = { index, removing: false };
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
  }

  function dragStop(event: PointerEvent): void {
    if (!dragging) return;
    const rect = bar?.getBoundingClientRect();
    // Tiré franchement vers le bas : l'arrêt sera retiré au relâchement (il en faut toujours deux).
    dragging.removing = stops.length > 2 && rect !== undefined && event.clientY > rect.bottom + 46;
    patch(dragging.index, { position: positionOf(event) });
  }

  function endDrag(): void {
    if (dragging?.removing) remove(dragging.index);
    dragging = null;
    onsettle?.();
  }

  function remove(index: number): void {
    if (stops.length <= 2) return;
    onchange(stops.filter((_, i) => i !== index));
    selected = Math.max(0, index - 1);
    onsettle?.();
  }

  /** Retire l'arrêt sélectionné (touche Suppr de l'éditeur flottant). */
  export function removeSelected(): void {
    remove(selected);
  }

  /** Vrai tant que le sélecteur de couleur est ouvert : l'éditeur flottant ne doit alors pas se fermer. */
  export function pickingColor(): boolean {
    return colorPopover !== null;
  }

  function reverse(): void {
    onchange(stops.map((s) => ({ ...s, position: 1 - s.position })));
    onsettle?.();
  }

  /** Arrêt sur lequel le dernier appui a eu lieu (null : appui dans le vide de la rangée). */
  let pressedStop: number | null = null;

  /**
   * Double-clic : ouvre le sélecteur de couleur de l'arrêt visé. Il est écouté sur la rangée et non
   * sur chaque arrêt, car la capture du pointeur (posée sur la rangée pour la glissade) y redirige le
   * `dblclick` sous Chromium. L'arrêt se retrouve par sa position à l'écran, à défaut par le dernier
   * arrêt pressé. Un double-clic dans le vide n'ouvre rien (son premier clic a déjà ajouté un arrêt).
   */
  function openStopColor(event: MouseEvent): void {
    const buttons = Array.from(track?.querySelectorAll<HTMLElement>(".stop") ?? []);
    let index = buttons.findIndex((button) => {
      const rect = button.getBoundingClientRect();
      return event.clientX >= rect.left && event.clientX <= rect.right && event.clientY >= rect.top && event.clientY <= rect.bottom;
    });
    if (index < 0 && pressedStop !== null && pressedStop < buttons.length) index = pressedStop;
    const button = buttons[index];
    if (!button) return;
    const rect = button.getBoundingClientRect();
    selected = index;
    colorPopover = { x: rect.left - 20, y: rect.bottom + 8 };
  }

  function usePreset(next: GradientStop[]): void {
    selected = 0;
    onchange(sorted(next));
    onsettle?.();
  }
</script>

<div class="bar" bind:this={bar}><div style:background={toCss(stops)}></div></div>

<!-- Rangée des arrêts : cliquer dans le vide en ajoute un. -->
<div class="track" role="group" aria-label={t("Arrêts du dégradé")} bind:this={track} onpointerdown={(event) => {
    pressedStop = null;
    addStop(event);
  }} ondblclick={openStopColor} onpointermove={dragStop} onpointerup={endDrag} onpointercancel={endDrag}>
  {#each stops as stop, index (index)}
    <button
      class="stop"
      class:on={index === selected}
      class:removing={dragging?.index === index && dragging.removing}
      style:left="{stop.position * 100}%"
      aria-label="Arrêt à {Math.round(stop.position * 100)} %"
      aria-pressed={index === selected}
      onpointerdown={(event) => {
        event.stopPropagation();
        pressedStop = index;
        selected = index;
        dragging = { index, removing: false };
        (event.currentTarget.parentElement as Element).setPointerCapture(event.pointerId);
      }}
    >
      <span style:background="rgba({stop.color[0]}, {stop.color[1]}, {stop.color[2]}, {stop.color[3] / 255})"></span>
    </button>
  {/each}
</div>

{#if current}
  <div class="row">
    <button
      class="color"
      title={t("Couleur de l'arrêt")}
      onclick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        colorPopover = colorPopover ? null : { x: rect.left, y: rect.bottom + 8 };
      }}
    >
      <span style:background={rgbToHex([current.color[0], current.color[1], current.color[2]])}></span>
      {rgbToHex([current.color[0], current.color[1], current.color[2]])}
    </button>
    <label>
      {t("Opacité")}
      <input
        type="range"
        min="0"
        max="100"
        value={Math.round((current.color[3] / 255) * 100)}
        oninput={(event) => patch(selected, { color: [current.color[0], current.color[1], current.color[2], Math.round((event.currentTarget.valueAsNumber / 100) * 255)] })}
        onchange={() => onsettle?.()}
      />
      <output>{Math.round((current.color[3] / 255) * 100)} %</output>
    </label>
  </div>
  <div class="row">
    <label>
      {t("Position")}
      <input
        type="number"
        min="0"
        max="100"
        value={Math.round(current.position * 100)}
        onchange={(event) => {
          if (!Number.isFinite(event.currentTarget.valueAsNumber)) return;
          patch(selected, { position: Math.min(1, Math.max(0, event.currentTarget.valueAsNumber / 100)) });
          onsettle?.();
        }}
      />
      %
    </label>
    <div class="spacer"></div>
    {#if reversible}<button class="ghost" onclick={reverse}>{t("Inverser")}</button>{/if}
    <button class="ghost" disabled={stops.length <= 2} onclick={() => remove(selected)}>{t("Supprimer l'arrêt")}</button>
  </div>
{/if}

{#if presets}
  <p>{t("Préréglages")}</p>
  <div class="presets">
    {@render children?.()}
    {#each PRESETS as preset (preset.name)}
      <button class="preset" title={t(preset.name)} aria-label={t(preset.name)} onclick={() => usePreset(preset.stops)}>
        <span style:background={toCss(preset.stops)}></span>
      </button>
    {/each}
  </div>
{/if}
<p class="hint">{t("Clic sous la barre : ajouter un arrêt · le tirer vers le bas : le retirer")}</p>

{#if colorPopover && current}
  <!-- Le sélecteur suit l'arrêt sélectionné : cliquer un autre arrêt pendant qu'il est ouvert le
       garde ouvert (la rangée est « dedans ») et le recrée sur la couleur de cet arrêt. -->
  {#key selected}
    <ColorPopover
      color={[current.color[0], current.color[1], current.color[2]]}
      x={colorPopover.x}
      y={colorPopover.y}
      keepOpenWithin={track}
      onchange={(rgb) => patch(selected, { color: [rgb[0], rgb[1], rgb[2], current.color[3]] })}
      onclose={() => {
        colorPopover = null;
        onsettle?.();
      }}
    />
  {/key}
{/if}

<style>
  /* Damier sous le dégradé : la transparence se lit d'un coup d'œil. */
  .bar,
  .preset,
  .stop {
    background: repeating-conic-gradient(#3a3a3e 0% 25%, #232326 0% 50%) 0 0 / 10px 10px;
  }
  .bar {
    height: 40px;
    border-radius: var(--radius-m);
    overflow: hidden;
    border: 1px solid var(--border-strong);
  }
  .bar div,
  .preset span,
  .stop span {
    display: block;
    width: 100%;
    height: 100%;
  }
  .track {
    position: relative;
    height: 26px;
    margin: -6px 9px 0;
    cursor: copy;
    touch-action: none;
  }
  .stop {
    position: absolute;
    top: 2px;
    width: 18px;
    height: 22px;
    margin-left: -9px;
    padding: 0;
    border-radius: var(--radius-s);
    overflow: hidden;
    border: 2px solid #8b8b93;
    cursor: grab;
  }
  .stop.on {
    border-color: #fff;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6);
  }
  .stop.removing {
    opacity: 0.25;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
  }
  input[type="range"] {
    width: 110px;
    min-width: 60px;
    flex: 1;
  }
  input[type="number"] {
    width: 54px;
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  output {
    min-width: 38px;
    color: var(--text-2);
  }
  .color {
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px 0 6px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .color span {
    width: 16px;
    height: 16px;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.35);
  }
  .spacer {
    flex: 1;
  }
  .ghost {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text-2);
  }
  .ghost:disabled {
    opacity: 0.35;
  }
  p {
    margin: 2px 0 -4px;
    color: var(--text-muted);
  }
  .presets {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 6px;
  }
  .preset {
    height: 30px;
    padding: 0;
    border-radius: var(--radius-m);
    overflow: hidden;
    border: 1px solid rgba(255, 255, 255, 0.12);
  }
  .preset:hover {
    outline: 1.5px solid #fff;
    outline-offset: 1px;
  }
  .hint {
    margin: 0;
    font-size: 11px;
  }
</style>
