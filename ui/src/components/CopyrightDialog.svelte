<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { DEFAULT_METADATA, type MetadataSettings, authorName, saveMetadata, suggestedCopyright } from "../lib/metadata";
  import { app } from "../state/app.svelte";

  // Menu Copyright › Métadonnées par défaut : ce que chaque image exportée porte (auteur, copyright, mots-clés…).
  // Les champs sont édités sur une copie ; « Enregistrer » les garde (fichier metadata.json), « Annuler » les oublie.
  let dialog = $state<HTMLDialogElement>();
  let draft = $state<MetadataSettings>({ ...DEFAULT_METADATA });
  let first = $state<HTMLInputElement>();

  $effect(() => {
    if (app.copyrightOpen) {
      draft = { ...untrack(() => app.metadata) };
      dialog?.showModal();
      first?.focus();
    } else dialog?.close();
  });

  const author = $derived(authorName(draft));
  const suggestion = $derived(suggestedCopyright(draft));

  function cancel(): void {
    app.copyrightOpen = false;
  }

  function save(): void {
    saveMetadata({ ...draft, firstName: draft.firstName.trim(), lastName: draft.lastName.trim(), copyright: draft.copyright.trim(), contact: draft.contact.trim(), keywords: draft.keywords.trim(), description: draft.description.trim() });
    app.copyrightOpen = false;
  }
</script>

<dialog bind:this={dialog} onclose={cancel}>
  <form onsubmit={(event) => (event.preventDefault(), save())}>
    <header>
      <h2>{t("Copyright et métadonnées")}</h2>
      <button type="button" class="icon" aria-label={t("Fermer")} onclick={cancel}>
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
      </button>
    </header>
    <div class="body">
      <p>{t("Inscrites dans chaque image exportée (PNG, JPEG, WebP, TIFF) ; la description et les mots-clés se modifient pour une image dans la fenêtre Exporter.")}</p>

      <label class="row">
        <span>{t("Prénom")}</span>
        <input type="text" bind:this={first} bind:value={draft.firstName} autocomplete="given-name" spellcheck="false" />
      </label>
      <label class="row">
        <span>{t("Nom")}</span>
        <input type="text" bind:value={draft.lastName} autocomplete="family-name" spellcheck="false" />
      </label>
      <label class="row">
        <span>{t("Mention de copyright")}</span>
        <div class="with-hint">
          <input type="text" bind:value={draft.copyright} placeholder={suggestion || t("© année Prénom Nom — Tous droits réservés")} spellcheck="false" />
          {#if suggestion && !draft.copyright.trim()}
            <button type="button" class="link" onclick={() => (draft.copyright = `${suggestion} — ${t("Tous droits réservés")}`)}>{t("Proposer « {suggestion} — Tous droits réservés »", { suggestion })}</button>
          {/if}
        </div>
      </label>
      <label class="row">
        <span>{t("Site ou contact")}</span>
        <input type="text" bind:value={draft.contact} placeholder={t("https://… ou adresse e-mail")} spellcheck="false" />
      </label>
      <label class="row">
        <span>{t("Mots-clés")}</span>
        <input type="text" bind:value={draft.keywords} placeholder={t("séparés par des virgules")} spellcheck="false" />
      </label>
      <label class="row">
        <span>{t("Description par défaut")}</span>
        <input type="text" bind:value={draft.description} placeholder={t("(vide)")} />
      </label>

      <div class="checks">
        <label class="check"><input type="checkbox" bind:checked={draft.embed} /> {t("Inscrire ces métadonnées dans les images exportées")}</label>
        <label class="check" class:off={!draft.embed}><input type="checkbox" bind:checked={draft.software} disabled={!draft.embed} /> {t("Ajouter « Créé avec Lumitype » (champ Logiciel)")}</label>
        <label class="check" class:off={!draft.embed}><input type="checkbox" bind:checked={draft.date} disabled={!draft.embed} /> {t("Inscrire aussi la date de création (celle de l'export)")}</label>
      </div>

      <p class="fine">
        {t("Champs écrits : Auteur / Artiste, Copyright, Description, Mots-clés, Site, Logiciel, Date — dans l'EXIF (JPEG, TIFF, WebP), le XMP (dc:creator, dc:rights, dc:subject…) et les blocs texte du PNG.")}
        {#if author}{t("Auteur inscrit : {author}.", { author })}{/if}
      </p>
    </div>
    <footer>
      <button type="button" class="ghost" onclick={cancel}>{t("Annuler")}</button>
      <button type="submit" class="pill">{t("Enregistrer")}</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(600px, calc(100vw - 48px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header {
    height: 54px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 0 22px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .icon {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .icon:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 22px 6px;
    max-height: calc(100vh - 220px);
    overflow-y: auto;
  }
  p {
    margin: 0 0 6px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .fine {
    margin-top: 4px;
    font-size: 11.5px;
  }
  .row {
    display: grid;
    grid-template-columns: 160px 1fr;
    align-items: center;
    gap: 12px;
    color: var(--text-3);
  }
  .row input[type="text"] {
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .row input::placeholder {
    color: var(--text-disabled);
  }
  .with-hint {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    color: var(--text-muted);
    font-size: 11.5px;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover {
    color: var(--text);
  }
  .checks {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .check.off {
    opacity: 0.45;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 14px 22px 18px;
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
