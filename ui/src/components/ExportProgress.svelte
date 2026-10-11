<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { app } from "../state/app.svelte";

  // Barre d'avancement de l'export final : la coque signale chaque étape (composition des calques,
  // mise à l'échelle, encodage, écriture) ; cette fenêtre les affiche jusqu'à la fin.
  const percent = $derived(Math.round((app.exportProgress?.progress ?? 0) * 100));
</script>

{#if app.exportProgress}
  <div class="veil" role="status" aria-live="polite">
    <div class="card">
      <div class="line"><strong>{t("Export en cours")}</strong><output>{percent} %</output></div>
      <div class="bar"><div style:width="{Math.max(3, percent)}%"></div></div>
      <p>{t(app.exportProgress.label)}…</p>
    </div>
  </div>
{/if}

<style>
  .veil {
    position: fixed;
    inset: 0;
    z-index: 900;
    display: grid;
    place-items: center;
    background: rgba(5, 5, 6, 0.6);
    cursor: progress;
  }
  .card {
    width: 380px;
    box-sizing: border-box;
    padding: 22px;
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  .line {
    display: flex;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  strong {
    font-weight: 500;
    font-size: 14px;
  }
  output {
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
  .bar {
    height: 6px;
    border-radius: var(--radius-xs);
    background: rgba(255, 255, 255, 0.12);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    border-radius: var(--radius-xs);
    background: var(--text);
    transition: width 160ms linear;
  }
  p {
    margin: 12px 0 0;
    color: var(--text-muted);
  }
</style>
