<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../lib/i18n.svelte";
  import { runAction } from "../lib/actions";
  import { preloadAnimations } from "../lib/media";
  import { type ToolGroup, TOOL_GROUPS, activateVariant, currentVariant, loadToolGroups, rememberVariant, shownVariant } from "../lib/tool-groups.svelte";
  import { keymap } from "../state/keymap.svelte";
  import Icon from "./Icon.svelte";
  import ToolFlyout from "./ToolFlyout.svelte";
  import ToolTip from "./ToolTip.svelte";

  // Outils empilés, comme dans Photoshop (voir lib/tool-groups.svelte.ts) : chaque case montre le dernier outil
  // utilisé de son groupe, avec un petit triangle en bas à droite quand le groupe en compte plusieurs. Clic droit,
  // appui long ou clic sur le triangle : la liste des outils du groupe s'ouvre à côté de la barre.

  /** Temps de survol avant l'apparition de la bulle d'aide. */
  const DELAY = 1000;
  /** Court sursis à la sortie : passer d'un outil au suivant ne referme pas la bulle. */
  const GRACE = 120;
  /** Appui long qui ouvre la liste du groupe, comme dans Photoshop. */
  const LONG_PRESS = 350;

  let shown: { group: ToolGroup; top: number } | null = $state(null);
  let open = false;
  let appear: number | undefined;
  let vanish: number | undefined;

  /** Liste ouverte : groupe, et position (à droite de la barre, à hauteur de la case). */
  let flyout = $state<{ group: ToolGroup; left: number; top: number } | null>(null);
  let press: number | undefined;
  /** L'appui long vient d'ouvrir la liste : le clic qui suit le relâchement ne doit pas changer d'outil. */
  let swallowClick = false;

  const current = $derived(currentVariant());

  // La case suit l'outil en main, quelle que soit la façon dont il a été choisi (raccourci, barre d'options,
  // fenêtre « Créer une forme »…) : c'est lui qu'elle affiche ensuite.
  $effect(() => {
    if (current) rememberVariant(current.id);
  });

  onMount(() => void loadToolGroups());

  function enter(group: ToolGroup, event: MouseEvent): void {
    clearTimeout(vanish);
    if (flyout) return; // la liste ouverte suffit, pas de bulle par-dessus
    const top = (event.currentTarget as HTMLElement).getBoundingClientRect().top;
    // Bulle déjà ouverte : elle suit le curseur d'un outil à l'autre sans réclamer le délai,
    // comme dans Photoshop. Sinon, on attend la seconde complète.
    if (open) {
      shown = { group, top };
      return;
    }
    clearTimeout(appear);
    appear = window.setTimeout(() => {
      open = true;
      shown = { group, top };
    }, DELAY);
  }

  function leave(): void {
    clearTimeout(appear);
    clearTimeout(vanish);
    clearTimeout(press);
    vanish = window.setTimeout(() => {
      open = false;
      shown = null;
    }, GRACE);
  }

  /** Au clic, l'aide n'a plus lieu d'être : l'outil est pris en main. */
  function dismiss(): void {
    clearTimeout(appear);
    clearTimeout(vanish);
    open = false;
    shown = null;
  }

  function openFlyout(group: ToolGroup, slot: HTMLElement): void {
    if (group.variants.length < 2) return;
    dismiss();
    const rect = slot.getBoundingClientRect();
    const bar = slot.closest("nav")?.getBoundingClientRect();
    flyout = { group, left: (bar?.right ?? rect.right) + 2, top: rect.top };
  }

  function onPointerDown(group: ToolGroup, event: PointerEvent): void {
    if (event.button !== 0 || group.variants.length < 2) return;
    const slot = event.currentTarget as HTMLElement;
    clearTimeout(press);
    press = window.setTimeout(() => {
      swallowClick = true;
      openFlyout(group, slot);
    }, LONG_PRESS);
  }

  function onClick(group: ToolGroup, event: MouseEvent): void {
    clearTimeout(press);
    if (swallowClick) {
      swallowClick = false;
      return;
    }
    dismiss();
    // Le petit triangle ouvre la liste au lieu de reprendre l'outil.
    if ((event.target as Element).closest(".more")) {
      openFlyout(group, event.currentTarget as HTMLElement);
      return;
    }
    flyout = null;
    activateVariant(shownVariant(group).id);
  }

  /** Raccourcis affichés dans la bulle : la lettre de l'outil, puis Maj + la lettre pour passer au suivant. */
  function keysOf(group: ToolGroup): string {
    const variant = shownVariant(group);
    const letter = keymap.label(variant.action);
    const sameLetter = group.variants.filter((v) => v.action === variant.action).length > 1;
    const cycle = group.cycle && sameLetter ? keymap.label(group.cycle) : "";
    return [letter, cycle].filter(Boolean).join(" · ");
  }

  function labelOf(group: ToolGroup): string {
    const variant = shownVariant(group);
    const keys = keysOf(group);
    const base = keys ? `${t(variant.label)} (${keys})` : t(variant.label);
    return group.variants.length > 1 ? `${base} — ${t("clic droit : autres outils")}` : base;
  }

  // Les animations sont mises en cache une fois l'application ouverte, jamais pendant son démarrage.
  $effect(() => {
    const later = window.setTimeout(preloadAnimations, 2000);
    return () => clearTimeout(later);
  });
</script>

<nav class="toolbar" aria-label={t("Outils")}>
  {#each TOOL_GROUPS as group (group.id)}
    {@const variant = shownVariant(group)}
    {@const active = current !== null && group.variants.includes(current)}
    {@const stacked = group.variants.length > 1}
    <button
      class:active
      class:open={flyout?.group === group}
      aria-label={labelOf(group)}
      aria-pressed={active}
      aria-haspopup={stacked ? "menu" : undefined}
      onclick={(event) => onClick(group, event)}
      ondblclick={() => variant.doubleClick && runAction(variant.doubleClick)}
      oncontextmenu={(event) => {
        event.preventDefault();
        openFlyout(group, event.currentTarget);
      }}
      onpointerdown={(event) => onPointerDown(group, event)}
      onpointerup={() => clearTimeout(press)}
      onpointercancel={() => clearTimeout(press)}
      onmouseenter={(event) => enter(group, event)}
      onmouseleave={leave}
    >
      <Icon name={variant.icon} size={17} />
      {#if stacked}
        <!-- Petit triangle en bas à droite : la case cache d'autres outils. Un clic dessus ouvre leur liste. -->
        <span class="more" aria-hidden="true"></span>
      {/if}
    </button>
  {/each}
</nav>

{#if shown && !flyout}
  {@const variant = shownVariant(shown.group)}
  <ToolTip
    tool={variant.tool}
    top={shown.top}
    name={variant.name}
    keys={keysOf(shown.group)}
    more={shown.group.variants.length > 1 ? t("Clic droit : autres outils") : ""}
  />
{/if}

{#if flyout}
  <ToolFlyout group={flyout.group} left={flyout.left} top={flyout.top} onclose={() => (flyout = null)} />
{/if}

<style>
  .toolbar {
    width: 48px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 0;
    background: var(--bg-bar);
    border-right: 1px solid var(--border);
  }
  button {
    position: relative;
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-3);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  button:hover,
  button.open {
    background: var(--bg-hover);
    color: var(--text);
  }
  button.active {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  /* Triangle plein de 4 px dans le coin bas droit, comme dans Photoshop. Zone de clic un peu plus large que le dessin. */
  .more {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 10px;
    height: 10px;
  }
  .more::after {
    content: "";
    position: absolute;
    right: 3px;
    bottom: 3px;
    border-style: solid;
    border-width: 0 0 4px 4px;
    border-color: transparent transparent currentColor transparent;
    opacity: 0.75;
  }
</style>
