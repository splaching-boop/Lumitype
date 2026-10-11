<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { api } from "../lib/ipc";
  import { ensureBundledFont } from "../lib/fonts";

  // Liste des graisses d'une famille, façon liste des polices : chaque style survolé ou parcouru aux
  // flèches s'applique tout de suite au texte (aperçu), le clic ou Entrée le retient, Échap ou la sortie
  // de la liste remet le style d'avant. Chaque ligne est rendue dans sa propre graisse.
  export type Style = { weight: number; italic: boolean };

  let {
    value,
    styles,
    family,
    bundled = false,
    onpreview,
    onrestore,
    ondone,
  }: {
    /** `null` : plusieurs styles dans la plage visée, le champ affiche « ? ». */
    value: Style | null;
    styles: Style[];
    /** Famille affichée, pour rendre chaque ligne dans sa graisse. */
    family: string | null;
    /** Famille incluse avec Lumitype : son fichier est chargé dans la webview pour rendre les lignes. */
    bundled?: boolean;
    onpreview: (style: Style, temporary: boolean) => void;
    onrestore: () => void;
    ondone: () => void;
  } = $props();

  const WEIGHT_NAMES: Record<number, string> = { 100: "Thin", 200: "ExtraLight", 300: "Light", 400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold", 900: "Black" };
  const baseName = (style: Style) => {
    const base = WEIGHT_NAMES[Math.round(style.weight / 100) * 100] ?? String(style.weight);
    return style.italic ? (base === "Regular" ? "Italic" : `${base} Italic`) : base;
  };
  /**
   * Nom affiché. Deux graisses voisines peuvent porter le même nom (Roboto Thin est déclaré 250, Light 300) :
   * le poids est alors ajouté pour les distinguer.
   */
  const names = $derived.by(() => {
    const count = new Map<string, number>();
    for (const style of styles) count.set(baseName(style), (count.get(baseName(style)) ?? 0) + 1);
    return new Map(styles.map((style) => [key(style), (count.get(baseName(style)) ?? 0) > 1 ? `${baseName(style)} ${style.weight}` : baseName(style)]));
  });
  const key = (style: Style) => `${style.weight}${style.italic ? "i" : ""}`;
  const same = (a: Style | null, b: Style | null) => a !== null && b !== null && a.weight === b.weight && a.italic === b.italic;
  const known = $derived(value !== null && styles.some((s) => same(s, value)));
  const label = $derived(value === null ? "?" : (names.get(key(value)) ?? `${baseName(value)} ${t("(approché)")}`));

  let button = $state<HTMLButtonElement>();
  let list = $state<HTMLDivElement>();
  let open = $state(false);
  let place = $state({ left: 0, top: 0, height: 420 });
  /** Style d'avant l'ouverture : Échap y revient. */
  let original: Style | null = null;
  /** Vrai tant que le style affiché ne vient que d'un survol. */
  let hovering = false;

  // Même garde que la liste des polices : un défilement au clavier passe la liste sous une souris immobile,
  // et le navigateur signale alors une entrée de pointeur qu'il faut ignorer.
  let pointerAt: { x: number; y: number } | null = null;
  let pointerMoved = true;

  function show(): void {
    if (open) return;
    if (bundled && family) void ensureBundledFont(family, () => api.bundledFontFile(family));
    original = value;
    const rect = button?.getBoundingClientRect();
    if (rect) place = { left: rect.left, top: rect.bottom + 4, height: Math.max(160, Math.min(420, window.innerHeight - rect.bottom - 16)) };
    open = true;
    queueMicrotask(() => list?.querySelector(".current")?.scrollIntoView({ block: "nearest" }));
  }

  function close(restore: boolean): void {
    if (hovering) onrestore();
    else if (restore && original !== null && !same(original, value)) onpreview(original, false);
    hovering = false;
    open = false;
  }

  function hover(style: Style): void {
    if (!pointerMoved || same(style, value)) return;
    hovering = true;
    onpreview(style, true);
  }

  function leaveList(): void {
    if (!hovering) return;
    hovering = false;
    onrestore();
  }

  function choose(style: Style): void {
    hovering = false;
    onpreview(style, false);
  }

  function step(direction: 1 | -1): void {
    if (styles.length === 0) return;
    const index = value === null ? -1 : styles.findIndex((s) => same(s, value));
    const next = styles[index < 0 ? 0 : Math.min(styles.length - 1, Math.max(0, index + direction))];
    if (next && !same(next, value)) choose(next);
    pointerMoved = false;
    queueMicrotask(() => list?.querySelector(".current")?.scrollIntoView({ block: "nearest" }));
  }

  function onKeyDown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      show();
      step(event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (open) {
        close(false);
        ondone();
      } else show();
    } else if (event.key === "Escape") {
      event.preventDefault();
      if (open) close(true);
      else ondone();
    }
  }

  function onListPointerMove(event: PointerEvent): void {
    if (pointerAt === null || pointerAt.x !== event.clientX || pointerAt.y !== event.clientY) pointerMoved = true;
    pointerAt = { x: event.clientX, y: event.clientY };
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (open && !(event.target instanceof Element && (event.target.closest(".style-list") || event.target === button))) close(false);
  }}
/>

<div class="picker">
  <button
    class="field"
    class:approx={value !== null && !known}
    bind:this={button}
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open ? close(false) : show())}
    onkeydown={onKeyDown}
  >
    <span class="label" style:font-family={family ? `"${family}"` : "inherit"} style:font-weight={value?.weight ?? 400} style:font-style={value?.italic ? "italic" : "normal"}>{label}</span>
    <svg viewBox="0 0 10 6" aria-hidden="true"><path d="M1 1l4 4 4-4" /></svg>
  </button>
  {#if open}
    <div class="style-list" role="listbox" tabindex="-1" bind:this={list} onmousedown={(event) => event.preventDefault()} onpointerleave={leaveList} onpointermove={onListPointerMove} style:left="{place.left}px" style:top="{place.top}px" style:max-height="{place.height}px">
      {#each styles as style (key(style))}
        <button
          role="option"
          tabindex="-1"
          aria-selected={same(style, value)}
          class:current={same(style, value)}
          style:font-family={family ? `"${family}"` : "inherit"}
          style:font-weight={style.weight}
          style:font-style={style.italic ? "italic" : "normal"}
          onpointerenter={() => hover(style)}
          onpointerdown={(event) => {
            event.preventDefault();
            choose(style);
            close(false);
            ondone();
          }}
        >
          {names.get(key(style))}
        </button>
      {:else}
        <p>{t("Aucun style")}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }
  .field {
    width: 100%;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    text-align: left;
  }
  .field:focus-visible {
    outline: 1px solid var(--accent);
  }
  .field.approx .label {
    color: var(--text-3);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .field svg {
    width: 10px;
    height: 6px;
    flex-shrink: 0;
    fill: none;
    stroke: var(--text-3);
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .style-list {
    position: fixed;
    z-index: 50;
    width: 320px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .style-list button {
    height: 30px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    padding: 0 10px;
    border-radius: var(--radius-s);
    text-align: left;
    font-size: 15px;
    color: var(--text-2);
  }
  .style-list button:hover {
    background: var(--bg-hover);
  }
  .style-list button.current {
    background: var(--bg-active);
    color: #fff;
  }
  p {
    margin: 0;
    padding: 8px 10px;
    color: var(--text-muted);
  }
</style>
