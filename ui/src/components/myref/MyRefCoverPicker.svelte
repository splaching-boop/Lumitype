<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../lib/i18n.svelte";
  import { type RefItem, refApi } from "../../lib/myref";
  import { app } from "../../state/app.svelte";
  import RefThumb from "./RefThumb.svelte";

  // « Changer la couverture… » (maquette « clic droit sur une collection ») : une image de la collection, choisie
  // dans la grille de ses vignettes (bordure d'accent), ou la mosaïque automatique des quatre derniers ajouts.
  // Enregistré dans myref.json par le processus Rust (`cover` de la collection).
  let { root, name, ondone, oncancel }: { root: string; name: string; ondone: () => void; oncancel: () => void } = $props();

  let items = $state<RefItem[] | null>(null);
  let mode = $state<"image" | "mosaic">("mosaic");
  let selected = $state<string | null>(null);
  let saving = $state(false);
  let box = $state<HTMLDivElement>();
  const latest = $derived(items ? [...items].sort((a, b) => b.added - a.added).slice(0, 4) : []);

  onMount(() => {
    box?.focus();
    void refApi.collection(root, name).then(
      (view) => {
        items = view.items;
        mode = view.cover && view.items.length > 0 ? "image" : "mosaic";
        selected = view.cover ?? view.items[0]?.file ?? null;
      },
      (error: unknown) => {
        app.fail(error);
        oncancel();
      },
    );
  });

  async function save(): Promise<void> {
    if (saving || (mode === "image" && !selected)) return;
    saving = true;
    try {
      await refApi.setCover(root, name, mode === "image" ? selected : null);
      ondone();
    } catch (error) {
      app.fail(error);
      saving = false;
    }
  }

  /** Phase de capture : Échap annule ici (l'accueil ne se referme pas), Entrée valide hors des boutons. */
  function onKey(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      oncancel();
    } else if (event.key === "Enter" && !(event.target instanceof HTMLButtonElement)) {
      event.preventDefault();
      void save();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<div class="shade" role="presentation" onpointerdown={(event) => event.target === event.currentTarget && oncancel()}>
  <div class="picker" role="dialog" aria-modal="true" aria-labelledby="myref-cover-title" tabindex="-1" bind:this={box}>
    <h4 id="myref-cover-title">{t("Couverture de « {name} »", { name })}</h4>
    <p>{t("Choisissez l'image qui représente la collection.")}</p>
    <div class="mode" role="radiogroup" aria-label={t("Couverture")}>
      <button role="radio" aria-checked={mode === "image"} class:on={mode === "image"} disabled={!items || items.length === 0} onclick={() => (mode = "image")}>{t("Une image")}</button>
      <button role="radio" aria-checked={mode === "mosaic"} class:on={mode === "mosaic"} onclick={() => (mode = "mosaic")}>{t("Mosaïque automatique (4 dernières)")}</button>
    </div>

    {#if !items}
      <div class="wait">{t("Chargement…")}</div>
    {:else if mode === "image"}
      <div class="grid" role="listbox" aria-label={t("Images de la collection")}>
        {#each items as item (item.id)}
          <button
            role="option"
            aria-selected={selected === item.file}
            class:on={selected === item.file}
            title={item.name}
            onclick={() => (selected = item.file)}
            ondblclick={() => ((selected = item.file), void save())}
          >
            <RefThumb {root} {item} fit="cover" />
          </button>
        {/each}
      </div>
    {:else}
      <div class="mosaic-preview">
        <div class="mosaic n{latest.length}">
          {#each latest as item (item.id)}<span><RefThumb {root} {item} fit="cover" /></span>{/each}
        </div>
        <span>{items.length === 0 ? t("La collection est vide : la mosaïque se remplira avec les images ajoutées.") : t("Les quatre images ajoutées en dernier, mises à jour à chaque ajout.")}</span>
      </div>
    {/if}

    <div class="row">
      <button class="btn" onclick={oncancel}>{t("Annuler")}</button>
      <button class="btn primary" disabled={saving || !items || (mode === "image" && !selected)} onclick={save}>
        {mode === "image" ? t("Utiliser cette image") : t("Utiliser la mosaïque")}
      </button>
    </div>
  </div>
</div>

<style>
  .shade {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.55);
    animation: fade 140ms var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .picker {
    width: 640px;
    max-width: calc(100vw - 32px);
    max-height: calc(100vh - 64px);
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    padding: 18px;
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    color: var(--text);
    outline: none;
  }
  h4 {
    margin: 0 0 4px;
    font-size: 15px;
    overflow-wrap: anywhere;
  }
  p {
    margin: 0 0 12px;
    color: var(--text-muted);
    font-size: 12px;
  }
  button {
    font: inherit;
    color: inherit;
    cursor: pointer;
  }
  .mode {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }
  .mode button {
    padding: 5px 10px;
    font-size: 12px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text-2);
  }
  .mode button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .mode button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--text-strong);
  }
  .mode button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 8px;
    min-height: 0;
    overflow-y: auto;
    padding: 2px;
  }
  .grid button {
    height: 80px;
    padding: 0;
    border: 2px solid transparent;
    background: var(--bg-app, #121111);
    overflow: hidden;
  }
  .grid button:hover {
    border-color: var(--border-strong);
  }
  .grid button.on {
    border-color: var(--accent);
  }
  .grid button:focus-visible {
    outline: 2px solid var(--text-3);
    outline-offset: 1px;
  }
  .mosaic-preview {
    display: flex;
    align-items: center;
    gap: 16px;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }
  .mosaic {
    width: 168px;
    height: 168px;
    flex: none;
    box-sizing: border-box;
    padding: 3px;
    gap: 3px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    grid-template-rows: 1fr 1fr;
    border: 1px solid var(--border);
    background: var(--bg-app, #121111);
  }
  .mosaic span {
    display: block;
    min-height: 0;
    overflow: hidden;
  }
  .mosaic.n1 span {
    grid-area: 1 / 1 / 3 / 3;
  }
  .mosaic.n2 span {
    grid-row: 1 / 3;
  }
  .mosaic.n3 span:first-child {
    grid-row: 1 / 3;
  }
  .wait {
    padding: 30px 0;
    text-align: center;
    color: var(--text-muted);
    font-size: 12px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 14px;
  }
  .btn {
    height: 32px;
    padding: 0 16px;
    font-size: 12.5px;
    border: 1px solid var(--border-strong);
    background: transparent;
  }
  .btn:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .btn.primary:hover:not(:disabled) {
    background: var(--accent-strong);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
