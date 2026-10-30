<script lang="ts" module>
  /** Icônes des entrées (traits de 24 × 24, comme le reste de My ref). */
  export const MENU_ICONS = {
    rename: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
    folder: "M3 7h7l2 2h9v10H3z",
    cover: "M4 4h16v16H4zM4 15l5-5 4 4 3-3 4 4",
    trash: "M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13",
  } as const;

  export type RefMenuEntry =
    | {
        label: string;
        icon?: keyof typeof MENU_ICONS;
        /** Raccourci affiché à droite (« F2 »). */
        shortcut?: string;
        /** Entrée cochée (✓ couleur d'accent) : menus à choix unique comme « Trier par ». */
        checked?: boolean;
        danger?: boolean;
        run: () => void;
      }
    | "separator";
</script>

<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import { dismissGuard } from "../../lib/menu-dismiss";

  // Menu de My ref (maquettes « collection » et « clic droit ») : options alignées à gauche, icône ou coche, lignes de
  // 32 px, séparateurs, entrée de suppression en rouge. Posé en position fixe au point (x, y) — sous le bouton qui
  // l'ouvre, centré (`align="center"`), ou sous le pointeur (clic droit) — et toujours ramené dans la fenêtre près
  // des bords. Se ferme au clic dehors, sur Échap, au défilement et quand la fenêtre perd le focus.
  let {
    x,
    y,
    align = "start",
    head = "",
    headStyle = "plain",
    entries,
    anchor = null,
    label = "",
    onclose,
  }: {
    x: number;
    y: number;
    /** « center » : `x` est le milieu du menu (menu ouvert sous un bouton) ; « start » : son bord gauche. */
    align?: "start" | "center";
    /** En-tête gris : « Trier par » (petites capitales), ou « Art · 4 éléments » (clic droit). */
    head?: string;
    headStyle?: "caps" | "plain";
    entries: RefMenuEntry[];
    /** Bouton qui ouvre le menu : un appui dessus est laissé à son propre clic (qui referme le menu). */
    anchor?: HTMLElement | null;
    label?: string;
    onclose: () => void;
  } = $props();

  // Le clic droit d'ouverture produit des signaux résiduels (appui en retard, `blur` de la fenêtre sous CEF) :
  // ils ne referment pas le menu aussitôt (voir lib/menu-dismiss.ts).
  const guard = dismissGuard();
  onDestroy(() => guard.dispose());

  let menu = $state<HTMLDivElement>();
  let position = $state({ left: -9999, top: -9999 });
  const checks = $derived(entries.some((entry) => entry !== "separator" && entry.checked !== undefined));

  // Ramené dans la fenêtre (marge de 8 px), même ouvert près d'un bord.
  $effect(() => {
    if (!menu) return;
    const [width, height] = [menu.offsetWidth, menu.offsetHeight];
    const left = align === "center" ? x - width / 2 : x;
    const flipped = y + height > window.innerHeight - 8 && y - height > 8 ? y - height : y;
    position = {
      left: Math.round(Math.max(8, Math.min(left, window.innerWidth - width - 8))),
      top: Math.round(Math.max(8, Math.min(flipped, window.innerHeight - height - 8))),
    };
  });

  // Le clavier arrive dans le menu : flèches haut et bas, Entrée sur l'option, Échap pour fermer.
  onMount(() => {
    void tick().then(() => items()[0]?.focus({ preventScroll: true }));
  });

  const items = () => Array.from(menu?.querySelectorAll<HTMLButtonElement>('[role^="menuitem"]') ?? []);

  /** Écouté à la phase de capture : Échap ferme le menu sans quitter l'accueil ni fermer autre chose. */
  function onKey(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onclose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End") return;
    const list = items();
    if (list.length === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home" ? 0 : event.key === "End" ? list.length - 1 : event.key === "ArrowDown" ? (at + 1) % list.length : (at - 1 + list.length) % list.length;
    list[next]?.focus();
  }

  function choose(run: () => void): void {
    onclose();
    run();
  }
</script>

<svelte:window
  onkeydowncapture={onKey}
  onpointerdown={(event) => {
    if (event.target instanceof Node && (menu?.contains(event.target) || anchor?.contains(event.target))) return;
    if (guard.pointerDownCloses(event)) onclose();
  }}
  onblur={() => guard.onWindowBlur(onclose)}
  onresize={onclose}
/>
<svelte:document onscrollcapture={(event) => !(event.target instanceof Node && menu?.contains(event.target)) && onclose()} />

<div
  class="menu"
  role="menu"
  tabindex="-1"
  aria-label={label || head || undefined}
  bind:this={menu}
  style:left="{position.left}px"
  style:top="{position.top}px"
  oncontextmenu={(event) => event.preventDefault()}
>
  {#if head}<div class="head" class:caps={headStyle === "caps"}>{head}</div>{/if}
  {#each entries as entry, index (index)}
    {#if entry === "separator"}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        role={entry.checked !== undefined ? "menuitemradio" : "menuitem"}
        aria-checked={entry.checked}
        class:danger={entry.danger}
        onclick={() => choose(entry.run)}
      >
        {#if checks}
          <span class="chk" aria-hidden="true">{entry.checked ? "✓" : ""}</span>
        {:else if entry.icon}
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d={MENU_ICONS[entry.icon]} /></svg>
        {/if}
        <span class="label">{entry.label}</span>
        {#if entry.shortcut}<kbd>{entry.shortcut}</kbd>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 90;
    min-width: 236px;
    max-width: min(360px, calc(100vw - 16px));
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    box-sizing: border-box;
    padding: 5px;
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    display: flex;
    flex-direction: column;
    outline: none;
    animation: drop 120ms var(--ease);
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
  .head {
    padding: 6px 10px;
    margin-bottom: 4px;
    font-size: 11px;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .head.caps {
    padding: 6px 10px 4px;
    margin-bottom: 0;
    border-bottom: 0;
    font-size: 10.5px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  button {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    flex: none;
    padding: 0 10px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    white-space: nowrap;
    cursor: pointer;
    outline: none;
  }
  button:hover,
  button:focus-visible {
    background: var(--bg-hover);
  }
  button:focus-visible {
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  svg {
    width: 15px;
    height: 15px;
    flex: none;
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 1.7;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  kbd {
    margin-left: auto;
    padding-left: 16px;
    font: inherit;
    font-size: 11px;
    color: var(--text-muted);
  }
  .chk {
    width: 15px;
    flex: none;
    text-align: center;
    color: var(--accent);
    font-size: 13px;
  }
  .danger {
    color: var(--danger);
  }
  .danger svg {
    stroke: var(--danger);
  }
  .sep {
    height: 1px;
    flex: none;
    margin: 5px 4px;
    background: var(--border-strong);
  }
</style>
