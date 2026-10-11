<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { app } from "../state/app.svelte";

  // « Enregistrer les modifications ? » avant de fermer un document ou de quitter : Oui, Non, Annuler.
  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (app.savePrompt) dialog?.showModal();
    else dialog?.close();
  });

  function answer(choice: "save" | "discard" | "cancel"): void {
    const prompt = app.savePrompt;
    app.savePrompt = null;
    prompt?.resolve(choice);
  }
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), answer("cancel"))}>
  <h2>{t("Enregistrer les modifications ?")}</h2>
  <p>{t("« {name} » a été modifié depuis son dernier enregistrement. Sans enregistrement, ces modifications seront perdues.", { name: app.savePrompt?.name ?? "" })}</p>
  <footer>
    <button class="ghost" onclick={() => answer("cancel")}>{t("Annuler")}</button>
    <div class="spacer"></div>
    <button class="ghost" onclick={() => answer("discard")}>{t("Non")}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button class="pill" autofocus onclick={() => answer("save")}>{t("Oui, enregistrer")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: 440px;
    box-sizing: border-box;
    padding: 24px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  h2 {
    margin: 0 0 10px;
    font-size: 16px;
    font-weight: 500;
  }
  p {
    margin: 0 0 22px;
    color: var(--text-3);
    line-height: 1.55;
  }
  footer {
    display: flex;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .ghost,
  .pill {
    height: 32px;
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
</style>
