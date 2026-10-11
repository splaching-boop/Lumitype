<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import type { Snippet } from "svelte";

  // `tabs` : plusieurs panneaux partagent le même emplacement ; `tab` désigne celui qui est affiché.
  let {
    title,
    grow = false,
    tabs,
    tab = $bindable(),
    children,
  }: { title: string; grow?: boolean; tabs?: { id: string; label: string }[]; tab?: string; children: Snippet } = $props();
</script>

<section class:grow>
  {#if tabs}
    <div class="tabs" role="tablist" aria-label={title}>
      {#each tabs as item (item.id)}
        <button role="tab" aria-selected={item.id === tab} class:on={item.id === tab} onclick={() => (tab = item.id)}>{t(item.label)}</button>
      {/each}
    </div>
  {:else}
    <h2>{title}</h2>
  {/if}
  <div class="body">{@render children()}</div>
</section>

<style>
  section {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-bottom: 1px solid var(--border);
  }
  section.grow {
    flex: 1;
  }
  h2 {
    height: 34px;
    flex-shrink: 0;
    margin: 0;
    display: flex;
    align-items: center;
    padding: 0 14px;
    font-size: inherit;
    font-weight: 500;
    border-bottom: 1px solid var(--border);
  }
  .tabs {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: stretch;
    gap: 4px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    padding: 0 8px;
    font-weight: 500;
    color: var(--text-muted);
    border-bottom: 1.5px solid transparent;
  }
  .tabs button:hover {
    color: var(--text-2);
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
