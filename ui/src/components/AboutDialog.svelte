<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import logo from "../assets/logo/logo-256.png";
  import { app } from "../state/app.svelte";

  let dialog = $state<HTMLDialogElement>();
  $effect(() => {
    if (app.aboutOpen) dialog?.showModal();
    else dialog?.close();
  });
</script>

<dialog bind:this={dialog} onclose={() => (app.aboutOpen = false)}>
  <img class="mark" src={logo} alt="" width="56" height="56" />
  <h2>{t("Lumitype")}</h2>
  <p class="version">{t("Version {__APP_VERSION__}", { __APP_VERSION__ })}</p>
  <p>{t("Éditeur d'images raster : calques, masques, sélections, texte, tracés, réglages non destructifs et filtres.")}</p>
  <dl>
    <dt>{t("Projet initié par")}</dt>
    <dd>{t("Antoine Deg.")}</dd>
    <dt>{t("Codé par")}</dt>
    <dd>{t("l'IA Claude (Anthropic)")}</dd>
  </dl>
  <button class="pill" onclick={() => (app.aboutOpen = false)}>{t("Fermer")}</button>
</dialog>

<style>
  dialog {
    width: 380px;
    box-sizing: border-box;
    padding: 28px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    text-align: center;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  .mark {
    display: block;
    margin: 0 auto 14px;
  }
  h2 {
    margin: 0;
    font-size: 20px;
    font-weight: 500;
  }
  p {
    margin: 10px 0 0;
    color: var(--text-3);
    line-height: 1.5;
  }
  .version {
    margin-top: 4px;
    color: var(--text-muted);
  }
  dl {
    margin: 18px 0 22px;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 12px;
    text-align: left;
  }
  dt {
    color: var(--text-muted);
    text-align: right;
  }
  dd {
    margin: 0;
  }
  .pill {
    height: 32px;
    padding: 0 20px;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
</style>
