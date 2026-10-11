<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { edit } from "../lib/engine";
  import { app } from "../state/app.svelte";

  let dialog = $state<HTMLDialogElement>();
  let radius = $state(5);
  const valid = $derived(Number.isFinite(radius) && radius >= 0.1 && radius <= 250);

  $effect(() => {
    if (app.featherDialogOpen) dialog?.showModal();
    else dialog?.close();
  });

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (!valid) return;
    app.featherDialogOpen = false;
    void edit({ op: "feather", radius });
  }
</script>

<dialog bind:this={dialog} onclose={() => (app.featherDialogOpen = false)}>
  <form onsubmit={submit}>
    <h2>{t("Contour progressif")}</h2>
    <label>{t("Rayon (px)")} <input type="number" min="0.1" max="250" step="0.1" bind:value={radius} /></label>
    <div class="actions">
      <button type="button" class="ghost" onclick={() => (app.featherDialogOpen = false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!valid}>OK</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 300px;
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
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
  }
  label {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-3);
  }
  input {
    width: 90px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    text-align: right;
    user-select: text;
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
