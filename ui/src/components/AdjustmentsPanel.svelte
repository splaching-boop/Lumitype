<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { ACTION_LABELS } from "../lib/action-catalog";
  import { type ActionId, isActionEnabled, runAction } from "../lib/actions";

  // Onglet Réglages : la liste des calques de réglage à créer, et rien d'autre. Les curseurs d'un
  // réglage vivent dans Propriétés, affichés quand son calque est sélectionné : aucun risque de créer
  // un doublon en touchant un curseur.
  const ENTRIES: { action: ActionId; icon: string }[] = [
    { action: "adjust.develop", icon: "M4 7h10M18 7h2M4 12h4M12 12h8M4 17h12M20 17h0M14 5v4M8 10v4M16 15v4" },
    { action: "adjust.brightnessContrast", icon: "M12 4v2M12 18v2M4 12h2M18 12h2M6.3 6.3l1.4 1.4M16.3 16.3l1.4 1.4M6.3 17.7l1.4-1.4M16.3 7.7l1.4-1.4M12 8a4 4 0 100 8z" },
    { action: "adjust.levels", icon: "M4 19h16M6 19v-5M9 19v-9M12 19V6M15 19v-7M18 19v-3" },
    { action: "adjust.hueSaturation", icon: "M4 6h16v4H4zM4 14h16v4H4zM9 6v4M14 14v4" },
    { action: "adjust.gradientMap", icon: "M4 6h16v12H4zM8 6v12M12 6v12M16 6v12" },
    { action: "adjust.invert", icon: "M4 5h16v14H4zM4 19L20 5" },
    { action: "adjust.negative", icon: "M4 5h16v14H4zM4 5l16 14M12 5v14" },
  ];

</script>

<div class="list">
  {#each ENTRIES as entry (entry.action)}
    <button disabled={!isActionEnabled(entry.action)} onclick={() => runAction(entry.action)}>
      <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d={entry.icon} /></svg>
      {ACTION_LABELS[entry.action].replace("Réglage ", "")}
    </button>
  {/each}
</div>

<p>
  {t("Un clic crée le calque de réglage, écrêté sur le calque sélectionné : il ne corrige que lui. Ses curseurs s'affichent dans Propriétés, et y reviennent chaque fois que vous sélectionnez ce calque.")}
</p>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 8px;
    border-bottom: 1px solid var(--border);
  }
  .list button {
    height: 30px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    color: var(--text-2);
    text-align: left;
  }
  .list button:hover:not(:disabled) {
    background: var(--bg-hover);
    color: var(--text);
  }
  .list button:disabled {
    color: var(--text-disabled);
  }
  svg {
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  p {
    margin: 0;
    padding: 14px;
    color: var(--text-muted);
    line-height: 1.5;
  }
</style>
