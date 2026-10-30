<script lang="ts">
  // Image › Mode › Couleurs CMJN… : passe le document en mode de TRAVAIL en CMJN, dans l'espace CMJN des Paramètres de
  // couleur (un seul CMJN pour tout Lumitype). Les pixels restent en RVB (pas de couches séparées) ; tant que dure le
  // mode, l'épreuve écran de cet espace reste allumée (Épreuve CMJN est grisée), le sélecteur de couleur signale les
  // couleurs hors gamme, et l'export se fait en CMJN dans cet espace. Le document le garde, même si les Paramètres de
  // couleur changent ensuite ; repasser par ici lui applique les nouveaux.
  import { t } from "../lib/i18n.svelte";
  import { convertColorMode, modeDialogs, workflowSpace } from "../lib/color-mode.svelte";
  import { colorSettings, profileName, sameSpace } from "../lib/color-settings.svelte";
  import type { CmykSpace } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  let dialog = $state<HTMLDialogElement>();
  const target = $derived(colorSettings.cmyk);
  const current = $derived(workflowSpace(app.active?.info));

  $effect(() => {
    if (modeDialogs.cmyk) dialog?.showModal();
    else dialog?.close();
  });

  function close(): void {
    modeDialogs.cmyk = false;
  }

  function describe(space: CmykSpace): string {
    const intent = space.intent === "perceptual" ? t("perceptive") : t("colorimétrie relative");
    return space.bpc ? `${intent}, ${t("compensation du point noir")}` : intent;
  }

  function settings(): void {
    close();
    colorSettings.dialogOpen = true;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    close();
    await convertColorMode({ kind: "cmykWorkflow", ...target });
  }
</script>

<dialog bind:this={dialog} onclose={close} aria-label={t("Couleurs CMJN")}>
  <form onsubmit={submit}>
    <h2>{t("Couleurs CMJN")}</h2>
    <p class="intro">{t("Mode de travail pour l'impression : l'épreuve écran reste allumée, les couleurs hors gamme sont signalées et l'export se fait en CMJN. Les pixels restent en RVB : repasser en RVB ne perd rien.")}</p>
    <div class="space">
      <span class="lbl">{t("Profil")}</span>
      <b>{profileName(target.profile)}</b>
      <small>{describe(target)}</small>
    </div>
    {#if current && !sameSpace(current, target)}
      <p class="note">{t("Le document est actuellement en {profile} ({options}) : il passera dans l'espace ci-dessus.", { profile: profileName(current.profile), options: describe(current) })}</p>
    {/if}
    <div class="actions">
      <button type="button" class="ghost left" onclick={settings}>{t("Paramètres de couleur…")}</button>
      <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={sameSpace(current, target)}>OK</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 440px;
    padding: 18px 20px 20px;
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
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .intro,
  .note {
    margin: 0;
    color: var(--text-3);
    line-height: 1.45;
  }
  .space {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    background: var(--bg-active);
  }
  .lbl {
    font-size: 11px;
    color: var(--text-muted);
  }
  small {
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions .left {
    margin-right: auto;
  }
  .actions button {
    height: 32px;
    padding: 0 18px;
    border-radius: var(--radius-m);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--accent, #ec2a3c);
    color: #fff;
    font-weight: 500;
  }
  .pill:disabled {
    opacity: 0.5;
  }
</style>
