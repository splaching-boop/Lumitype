<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "../lib/i18n.svelte";
  import { dismissGuard } from "../lib/menu-dismiss";
  import { type ToolGroup, type ToolVariantId, activateVariant, shownVariant } from "../lib/tool-groups.svelte";
  import { keymap } from "../state/keymap.svelte";
  import Icon from "./Icon.svelte";

  // Liste des outils d'une case de la barre d'outils, ouverte à côté d'elle comme dans Photoshop : un petit carré
  // devant l'outil affiché par la case, l'icône, le nom, et la lettre du raccourci à droite.
  let { group, left, top, onclose }: { group: ToolGroup; left: number; top: number; onclose: () => void } = $props();

  // Le clic droit (ou l'appui long) qui ouvre la liste produit des signaux résiduels qui ne doivent pas la refermer.
  const guard = dismissGuard();
  onDestroy(() => guard.dispose());

  let menu = $state<HTMLDivElement>();
  /** La liste reste dans la fenêtre, même ouverte depuis une case du bas. */
  let y = $state(0);
  $effect(() => {
    if (!menu) return;
    y = Math.max(4, Math.min(top, window.innerHeight - menu.offsetHeight - 4));
    // Le clavier suit : flèches pour parcourir, Entrée pour choisir, Échap pour fermer.
    menu.querySelector<HTMLButtonElement>("button.marked")?.focus({ preventScroll: true });
  });

  const marked = $derived(shownVariant(group).id);

  function choose(id: ToolVariantId): void {
    onclose();
    activateVariant(id);
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onclose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const items = [...(menu?.querySelectorAll<HTMLButtonElement>("button") ?? [])];
    const here = items.indexOf(document.activeElement as HTMLButtonElement);
    const step = event.key === "ArrowDown" ? 1 : -1;
    items[(here + step + items.length) % items.length]?.focus();
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (event.target instanceof Node && menu?.contains(event.target)) return;
    if (guard.pointerDownCloses(event)) onclose();
  }}
  onblur={() => guard.onWindowBlur(onclose)}
  onresize={onclose}
/>

<!-- Les frappes restent dans la liste (une lettre n'y change pas d'outil derrière elle) : role=dialog, voir floatingPanelOpen. -->
<div
  class="flyout"
  role="dialog"
  aria-label={t("Outils du groupe")}
  tabindex="-1"
  bind:this={menu}
  style:left="{left}px"
  style:top="{y}px"
  oncontextmenu={(event) => event.preventDefault()}
  onkeydown={onKeyDown}
>
  {#each group.variants as variant (variant.id)}
    {@const shortcut = keymap.label(variant.action)}
    <button class:marked={variant.id === marked} role="menuitemradio" aria-checked={variant.id === marked} onclick={() => choose(variant.id)}>
      <span class="mark" aria-hidden="true"></span>
      <Icon name={variant.icon} size={16} />
      <span class="label">{t(variant.label)}</span>
      <span class="shortcut">{shortcut}</span>
    </button>
  {/each}
</div>

<style>
  .flyout {
    position: fixed;
    z-index: 40;
    min-width: 240px;
    display: flex;
    flex-direction: column;
    padding: 4px 0;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    animation: appear 90ms var(--ease);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateX(-3px);
    }
  }
  .flyout:focus {
    outline: none;
  }
  button {
    height: 28px;
    display: grid;
    grid-template-columns: 6px 16px 1fr auto;
    align-items: center;
    gap: 9px;
    padding: 0 14px 0 10px;
    border-radius: var(--radius-s);
    color: var(--text-2);
    text-align: left;
    white-space: nowrap;
  }
  button:hover,
  button:focus-visible {
    background: var(--bg-active);
    color: var(--text);
    outline: none;
  }
  /* Petit carré plein devant l'outil affiché par la case, comme dans Photoshop. */
  .mark {
    width: 6px;
    height: 6px;
  }
  .marked {
    color: var(--text);
  }
  .marked .mark {
    background: var(--accent);
  }
  .label {
    padding-right: 28px;
  }
  .shortcut {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
