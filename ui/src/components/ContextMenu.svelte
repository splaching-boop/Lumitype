<script lang="ts" module>
  import { t } from "../lib/i18n.svelte";
  /** Rangée de pastilles de couleur : `current` est la couleur cochée (`null` : aucune), `choose` reçoit le choix. */
  export type ColorEntry = { colors: { value: string | null; color: string | null; name: string }[]; current: string | null; choose: (value: string | null) => void; label: string };
  export type MenuEntry = { label: string; shortcut?: string; disabled?: boolean; run: () => void } | ColorEntry | "separator";
</script>

<script lang="ts">
  import { onDestroy } from "svelte";
  import { dismissGuard } from "../lib/menu-dismiss";

  let { x, y, entries, onclose }: { x: number; y: number; entries: MenuEntry[]; onclose: () => void } = $props();

  // Les signaux résiduels du clic droit d'ouverture (appui livré en retard, `blur` de la fenêtre dû au
  // changement de focus X11 sous CEF) ne doivent pas refermer le menu aussitôt : voir `menu-dismiss.ts`.
  const guard = dismissGuard();
  onDestroy(() => guard.dispose());

  let menu = $state<HTMLDivElement>();
  let position = $state({ left: 0, top: 0 });

  // Le menu reste entièrement dans la fenêtre, même ouvert près d'un bord.
  $effect(() => {
    if (!menu) return;
    position = {
      left: Math.max(4, Math.min(x, window.innerWidth - menu.offsetWidth - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - menu.offsetHeight - 4)),
    };
  });
</script>

<svelte:window
  onpointerdown={(event) => {
    if (event.target instanceof Node && menu?.contains(event.target)) return;
    if (guard.pointerDownCloses(event)) onclose();
  }}
  onkeydown={(event) => event.key === "Escape" && onclose()}
  onblur={() => guard.onWindowBlur(onclose)}
/>

<!-- Un clic droit résiduel sur le menu lui-même (il s'ouvre sous le pointeur) ne doit pas faire apparaître
     le menu natif du moteur, qui prendrait le focus et refermerait celui-ci. -->
<div class="menu" role="menu" tabindex="-1" bind:this={menu} style:left="{position.left}px" style:top="{position.top}px" oncontextmenu={(event) => event.preventDefault()}>
  {#each entries as entry, index (index)}
    {#if entry === "separator"}
      <div class="separator" role="separator"></div>
    {:else if "colors" in entry}
      <div class="colors" role="group" aria-label={t(entry.label)}>
        <span class="caption">{t(entry.label)}</span>
        <div class="swatches">
          {#each entry.colors as swatch (swatch.value ?? "none")}
            <button
              role="menuitemradio"
              class="swatch"
              class:none={swatch.color === null}
              class:current={swatch.value === entry.current}
              aria-checked={swatch.value === entry.current}
              title={t(swatch.name)}
              style:background={swatch.color ?? "transparent"}
              onclick={() => {
                onclose();
                entry.choose(swatch.value);
              }}
            ></button>
          {/each}
        </div>
      </div>
    {:else}
      <button
        role="menuitem"
        disabled={entry.disabled}
        onclick={() => {
          onclose();
          entry.run();
        }}
      >
        <span>{t(entry.label)}</span>
        {#if entry.shortcut}<span class="shortcut">{entry.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed;
    z-index: 40;
    min-width: 250px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  button {
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    text-align: left;
  }
  button:disabled {
    color: var(--text-disabled);
  }
  button:not(:disabled):hover {
    background: var(--bg-active);
  }
  .shortcut {
    color: var(--text-muted);
  }
  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border-strong);
  }
  .colors {
    padding: 4px 10px 6px;
  }
  .caption {
    display: block;
    color: var(--text-muted);
    font-size: 11px;
    margin-bottom: 6px;
  }
  .swatches {
    display: flex;
    gap: 7px;
  }
  .swatch {
    width: 18px;
    height: 18px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid transparent;
    box-sizing: border-box;
    position: relative;
  }
  .swatch.none {
    border: 1.5px solid var(--text-muted);
  }
  .swatch.none::after {
    content: "";
    position: absolute;
    left: 3px;
    top: 6.5px;
    width: 9px;
    height: 1.5px;
    background: var(--text-muted);
    transform: rotate(-45deg);
  }
  .swatch.current {
    border-color: #fff;
  }
  .swatch:hover {
    outline: 2px solid rgba(255, 255, 255, 0.35);
    outline-offset: 1px;
  }
</style>
