<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { edit } from "../../lib/engine";
  import { exportSpace } from "../../lib/color-mode.svelte";
  import { profileName } from "../../lib/color-settings.svelte";
  import { analyzeInk, ink, inkLimitOf } from "../../lib/print.svelte";
  import { app } from "../../state/app.svelte";

  // Image › Corriger l'encrage… : ramène sous la limite les pixels qui la dépassent. Le gris du mélange cyan +
  // magenta + jaune passe d'abord au noir seul (même couleur, moins d'encre), puis les trois couleurs baissent si
  // besoin. Seuls les pixels au-delà de la limite changent ; Ctrl+Z annule.
  let dialog = $state<HTMLDialogElement>();
  let limit = $state(300);
  let scope = $state<"all" | "layer">("all");
  const info = $derived(app.active?.info ?? null);
  const space = $derived(exportSpace(info));
  const report = $derived(info && ink.docId === info.id && ink.revision === info.revision ? ink.report : null);

  // La limite proposée n'est relue qu'à l'ouverture : le document est lu hors suivi, sinon chaque révision
  // (dont la correction elle-même) écraserait la valeur saisie.
  $effect(() => {
    if (!app.correctInkDialogOpen) {
      dialog?.close();
      return;
    }
    limit = untrack(() => inkLimitOf(app.active?.info));
    dialog?.showModal();
  });
  $effect(() => {
    if (app.correctInkDialogOpen && info) void analyzeInk(info, space, limit);
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const layer = scope === "layer" ? (app.activeLayer?.id ?? null) : null;
    app.correctInkDialogOpen = false;
    await edit({ op: "correctInk", limit, space, layer });
  }
</script>

<dialog bind:this={dialog} onclose={() => (app.correctInkDialogOpen = false)}>
  <form onsubmit={submit}>
    <h2>{t("Corriger l'encrage")}</h2>
    <div class="row">
      <span>{t("Encrage max.")}</span>
      <label class="value"><input type="number" min="150" max="400" step="5" bind:value={limit} /><em>%</em></label>
    </div>
    <div class="row">
      <span>{t("Appliquer à")}</span>
      <div class="segmented">
        <button type="button" class:on={scope === "all"} onclick={() => (scope = "all")}>{t("Tous les calques")}</button>
        <button type="button" class:on={scope === "layer"} disabled={!app.activeLayer} onclick={() => (scope = "layer")}>{t("Calque actif")}</button>
      </div>
    </div>
    <div class="state" class:ok={report && report.overPixels === 0}>
      {#if ink.busy || !report}
        {t("Analyse de l'encrage…")}
      {:else if report.overPixels === 0}
        ✓ {t("Rien ne dépasse {limit} % (maximum {max} %).", { limit: report.limit, max: report.maxPercent })}
      {:else}
        ⚠ {t("{count} zone(s) au-delà de {limit} % · maximum {max} %.", { count: report.regions.length, limit: report.limit, max: report.maxPercent })}
      {/if}
    </div>
    <p class="hint">
      {t("Le gris du mélange C+M+J passe au noir seul (même couleur, moins d'encre), puis les couleurs baissent si besoin. Seuls les pixels au-delà de la limite changent. Profil : {profile}.", { profile: profileName(space.profile) })}
    </p>
    <div class="actions">
      <button type="button" class="ghost" onclick={() => (app.correctInkDialogOpen = false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!!report && report.overPixels === 0}>{t("Corriger")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 400px;
    padding: 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    color: var(--text);
    box-shadow: var(--shadow-menu);
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 500;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
  }
  .value {
    position: relative;
    width: 96px;
  }
  .value input {
    width: 100%;
    height: 30px;
    padding: 0 26px 0 8px;
    text-align: right;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
  }
  .value em {
    position: absolute;
    right: 8px;
    top: 50%;
    transform: translateY(-50%);
    font-style: normal;
    font-size: 11px;
    color: var(--text-muted);
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-well);
    border: 1px solid var(--border);
  }
  .segmented button {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong, var(--text));
  }
  .state {
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: rgba(255, 176, 32, 0.12);
    color: #ffc04d;
  }
  .state.ok {
    background: rgba(62, 207, 110, 0.12);
    color: #3ecf6e;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    height: 28px;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .pill:disabled {
    opacity: 0.35;
  }
</style>
