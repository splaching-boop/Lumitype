<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { type RefCollectionSummary, type RefItem, refApi } from "../../lib/myref";
  import { app } from "../../state/app.svelte";
  import CollectionPicker from "./CollectionPicker.svelte";

  // Survol d'une image de My ref : en haut, la liste de ses collections (« Graphisme +1 ») qui s'ouvre pour l'ajouter
  // à une autre collection (+ carré) ou l'en retirer (✓ rouge), ou en créer une nouvelle avec elle dedans ; en bas
  // à droite, le cœur des favoris, animé quand on l'allume. Rien d'autre.
  let {
    root,
    item,
    collections,
    onchanged,
    onmembership,
  }: {
    root: string;
    item: RefItem;
    collections: RefCollectionSummary[];
    /** Fiche modifiée (favori). */
    onchanged: (item: RefItem) => void;
    /** Image ajoutée à une collection ou retirée : la bibliothèque a changé. */
    onmembership: () => void;
  } = $props();

  let open = $state(false);
  let burst = $state(0);
  let hovered = $state(false);

  async function heart(): Promise<void> {
    const next = !item.favorite;
    if (next) burst++;
    try {
      onchanged(await refApi.updateItem(root, item.id, { favorite: next }));
    } catch (error) {
      app.fail(error);
    }
  }

</script>

<div class="hover" class:open class:liked={item.favorite} onpointerenter={() => (hovered = true)} role="group" aria-label={item.name}>
  <div class="top">
    <CollectionPicker {root} {item} {collections} bind:open preload={hovered} {onmembership} />
  </div>

  <button type="button" class="heart" class:on={item.favorite} aria-pressed={item.favorite} aria-label={item.favorite ? t("Retirer des favoris") : t("Ajouter aux favoris")} onclick={() => void heart()}>
    {#key burst}
      {#if burst > 0 && item.favorite}
        {#each [0, 60, 120, 180, 240, 300] as angle (angle)}<i class="spark" style:--a="{angle}deg"></i>{/each}
      {/if}
      <svg class:pop={burst > 0 && item.favorite} viewBox="0 0 24 24" aria-hidden="true"><path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10z" /></svg>
    {/key}
  </button>
</div>

<style>
  .hover {
    position: absolute;
    inset: 0;
    pointer-events: none;
    opacity: 0;
    transition: opacity 120ms var(--ease);
    z-index: 2;
  }
  :global(.card:hover) > .hover,
  :global(.strip-card:hover) > .hover,
  .hover.open,
  .hover:focus-within {
    opacity: 1;
  }
  .hover.open {
    z-index: 20;
  }
  .hover > * {
    pointer-events: auto;
  }
  .top {
    position: absolute;
    left: 8px;
    right: 8px;
    top: 8px;
    z-index: 2;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  /* Cœur : carré sombre, rempli de rouge quand l'image est favorite ; au clic, il se tasse, gonfle et lâche six
     éclats. */
  .heart {
    position: absolute;
    z-index: 1;
    right: 8px;
    bottom: 8px;
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    border: 1px solid rgba(255, 255, 255, 0.16);
    background: rgba(20, 20, 22, 0.88);
    color: #fff;
    transition:
      background 160ms,
      border-color 160ms;
  }
  .heart.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .heart svg {
    width: 17px;
    height: 17px;
    stroke-width: 1.8;
  }
  .heart.on svg {
    fill: #fff;
  }
  .heart svg.pop {
    animation: pop 480ms var(--ease);
  }
  .spark {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 6px;
    height: 6px;
    margin: -3px;
    border-radius: 1px;
    background: #ff6b77;
    pointer-events: none;
    animation: fly 520ms ease-out forwards;
  }
  @keyframes pop {
    0% {
      transform: scale(1);
    }
    30% {
      transform: scale(0.72);
    }
    60% {
      transform: scale(1.32);
    }
    100% {
      transform: scale(1);
    }
  }
  @keyframes fly {
    0%,
    35% {
      opacity: 0;
      transform: rotate(var(--a)) translateY(0);
    }
    50% {
      opacity: 1;
    }
    100% {
      opacity: 0;
      transform: rotate(var(--a)) translateY(-30px) scale(0.5);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .heart svg.pop,
    .spark {
      animation: none;
    }
  }
</style>
