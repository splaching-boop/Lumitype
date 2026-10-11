<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { edit } from "../lib/engine";
  import { app } from "../state/app.svelte";
  import Panel from "./Panel.svelte";

  // `bare` : posé dans un onglet d'un autre panneau, sans son propre cadre ni son titre.
  let { bare = false }: { bare?: boolean } = $props();

  let list = $state<HTMLOListElement>();

  // L'état courant reste visible quand l'historique s'allonge ou qu'on annule.
  $effect(() => {
    void app.active?.info.historyCursor;
    void app.active?.info.history.length;
    list?.querySelector(".current")?.scrollIntoView({ block: "nearest" });
  });
</script>

{#snippet content()}
  {#if app.active}
    {@const { history, historyCursor } = app.active.info}
    <ol bind:this={list}>
      {#each history as label, index (index)}
        <li>
          <button
            class:current={index === historyCursor}
            class:undone={index > historyCursor}
            onclick={() => edit({ op: "jump", index })}
          >
            {t(label)}
          </button>
        </li>
      {/each}
    </ol>
  {:else}
    <p>{t("Aucun document")}</p>
  {/if}
{/snippet}

{#if bare}
  {@render content()}
{:else}
  <Panel title={t("Historique")}>{@render content()}</Panel>
{/if}

<style>
  ol {
    height: 168px;
    margin: 0;
    padding: 6px 8px;
    list-style: none;
    overflow-y: auto;
  }
  button {
    width: 100%;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    text-align: left;
    color: var(--text-2);
  }
  button:hover {
    background: var(--bg-hover);
  }
  button.current {
    background: var(--bg-active);
    color: #fff;
  }
  /* États annulés : encore accessibles, mais un nouveau geste les effacera. */
  button.undone {
    color: var(--text-disabled);
    font-style: italic;
  }
  p {
    margin: 0;
    padding: 14px;
    color: var(--text-muted);
  }
</style>
