<script lang="ts">
  // Aide > Signaler un bug… : une description libre, et ce qui aide à reproduire le problème (version, système,
  // carte graphique, dernière erreur avec son code), joint seulement si l'utilisateur le laisse coché. Le rapport
  // part sur GitHub par une issue pré-remplie (le navigateur s'ouvre), ou se copie pour être collé ailleurs.
  // Rien n'est envoyé sans que l'utilisateur valide lui-même l'issue sur GitHub.
  import { i18n, t } from "../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { api, type BugReportContext } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  const ISSUES_URL = "https://github.com/splaching-boop/Lumitype/issues/new";
  /** GitHub refuse les adresses trop longues : au-delà, le rapport est tronqué (et reste copiable en entier). */
  const MAX_BODY = 6000;

  let dialog = $state<HTMLDialogElement>();
  let description = $state("");
  let attach = $state(true);
  let context = $state<BugReportContext | null>(null);
  let notice = $state<string | null>(null);

  // Ne dépend que de l'ouverture : le contexte est lu hors suivi, sinon son arrivée relançait l'effet et effaçait
  // le message (« copié ») à peine affiché.
  $effect(() => {
    if (app.bugReportOpen) {
      notice = null;
      dialog?.showModal();
      if (!untrack(() => context)) api.bugReportContext().then((info) => (context = info), () => undefined);
    } else dialog?.close();
  });

  const gpu = $derived(app.performance.gpuPreviews ? t("composition GPU activée") : t("composition GPU désactivée"));
  const lastError = $derived(app.lastError ? `${app.lastError.code ?? t("sans code")} — ${app.lastError.message}` : t("aucune"));

  /** Informations jointes, en français et en clair : c'est ce que lira la personne qui corrige. */
  const details = $derived(
    [
      `Lumitype ${__APP_VERSION__} · ${context?.os ?? "?"} · ${context?.arch ?? "?"}`,
      `Langue de l'interface : ${i18n.locale} · Système : ${navigator.language}`,
      `Carte graphique : ${gpu}`,
      `Dernière erreur : ${lastError}`,
    ].join("\n"),
  );

  function report(): string {
    const text = description.trim() || "(pas de description)";
    return attach ? `${text}\n\n---\n${details}` : text;
  }

  function title(): string {
    const first = description.trim().split("\n")[0] ?? "";
    const base = first.length > 0 ? first.slice(0, 80) : "Bug";
    return app.lastError?.code && attach ? `[${app.lastError.code}] ${base}` : base;
  }

  async function copy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(`${title()}\n\n${report()}`);
      notice = t("Rapport copié dans le presse-papiers.");
    } catch (error) {
      app.fail(error);
    }
  }

  async function openIssue(): Promise<void> {
    let body = report();
    if (body.length > MAX_BODY) body = `${body.slice(0, MAX_BODY)}\n\n(rapport tronqué)`;
    const url = `${ISSUES_URL}?title=${encodeURIComponent(title())}&body=${encodeURIComponent(body)}`;
    try {
      await api.openProjectLink(url);
      notice = t("Le navigateur s'ouvre sur GitHub : vérifiez le rapport, puis validez l'issue.");
    } catch (error) {
      app.fail(error);
    }
  }
</script>

<dialog bind:this={dialog} onclose={() => (app.bugReportOpen = false)}>
  <h2>{t("Signaler un bug")}</h2>
  <textarea bind:value={description} placeholder={t("Décrivez ce qui s'est passé et comment le reproduire…")} rows="6"></textarea>
  <label class="attach"><input type="checkbox" bind:checked={attach} /> {t("Joindre ces informations")}</label>
  {#if attach}<pre class="details">{details}</pre>{/if}
  <p class="hint">{t("Un compte GitHub est nécessaire pour publier le rapport. Sans compte, copiez-le et envoyez-le autrement.")}</p>
  {#if notice}<p class="notice" role="status">{notice}</p>{/if}
  <div class="buttons">
    <button class="pill" onclick={copy}>{t("Copier le rapport")}</button>
    <button class="pill" onclick={() => (app.bugReportOpen = false)}>{t("Annuler")}</button>
    <button class="pill primary" onclick={openIssue}>{t("Ouvrir sur GitHub")}</button>
  </div>
</dialog>

<style>
  dialog {
    width: 520px;
    max-width: calc(100vw - 32px);
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
    margin: 0 0 14px;
    font-size: 17px;
    font-weight: 500;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    resize: vertical;
    user-select: text;
  }
  .attach {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    color: var(--text-2);
  }
  .details {
    margin: 8px 0 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text-muted);
    font: 12px/1.6 ui-monospace, monospace;
    white-space: pre-wrap;
    user-select: text;
  }
  .hint,
  .notice {
    margin: 10px 0 0;
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.5;
  }
  .notice {
    color: var(--text);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  .pill {
    height: 30px;
    padding: 0 14px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
</style>
