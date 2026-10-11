<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { recover } from "../lib/autosave.svelte";
  import { api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Après une fermeture imprévue : les copies de secours trouvées au démarrage, à récupérer (avec tout leur
  // historique) ou à supprimer. « Plus tard » les laisse sur le disque pour le prochain lancement.
  let dialog = $state<HTMLDialogElement>();
  let busy = $state<string | null>(null);

  $effect(() => {
    if (app.recoveries.length > 0) dialog?.showModal();
    else dialog?.close();
  });

  const when = (ms: number) => new Date(ms).toLocaleString("fr-FR", { dateStyle: "medium", timeStyle: "short" });
  const size = (bytes: number) => (bytes > 1048576 ? `${(bytes / 1048576).toFixed(1)} Mo` : `${Math.max(1, Math.round(bytes / 1024))} Ko`);

  async function restore(key: string): Promise<void> {
    busy = key;
    try {
      await recover(key);
      app.recoveries = app.recoveries.filter((r) => r.key !== key);
      app.newDocumentDialogOpen = false;
    } catch (error) {
      app.fail(error);
    } finally {
      busy = null;
    }
  }

  async function discard(keys: string[]): Promise<void> {
    for (const key of keys) await api.recoveryDelete(key).catch((error: unknown) => app.fail(error));
    app.recoveries = app.recoveries.filter((r) => !keys.includes(r.key));
  }
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), (app.recoveries = []))}>
  <h2>{t("Récupération")}</h2>
  <p>{t("Lumitype ne s'est pas fermé normalement. Ces documents avaient été sauvegardés automatiquement ; chacun se rouvre avec tout son historique.")}</p>
  <ul>
    {#each app.recoveries as entry (entry.key)}
      <li>
        <div class="what">
          <strong>{t("Document sauvegardé le {when}", { when: when(entry.savedAt) })}</strong>
          <span>{size(entry.size)}</span>
        </div>
        <button class="pill" disabled={busy !== null} onclick={() => restore(entry.key)}>{busy === entry.key ? t("Ouverture…") : t("Récupérer")}</button>
        <button class="ghost" disabled={busy !== null} onclick={() => discard([entry.key])}>{t("Supprimer")}</button>
      </li>
    {/each}
  </ul>
  <footer>
    <button class="ghost" disabled={busy !== null} onclick={() => discard(app.recoveries.map((r) => r.key))}>{t("Tout supprimer")}</button>
    <button class="ghost" disabled={busy !== null} onclick={() => (app.recoveries = [])}>{t("Plus tard")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(560px, calc(100vw - 48px));
    padding: 22px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.55);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 15px;
    font-weight: 500;
  }
  p {
    margin: 0 0 14px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 50vh;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--radius-l);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border);
  }
  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .what strong {
    font-weight: 500;
  }
  .what span {
    color: var(--text-muted);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  .pill,
  .ghost {
    height: 30px;
    padding: 0 14px;
    border-radius: var(--radius-pill);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
</style>
