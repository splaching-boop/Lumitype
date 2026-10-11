<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { t } from "../lib/i18n.svelte";

  // Boutons de fenêtre (réduire, agrandir ou restaurer, fermer) dessinés dans la barre de menus :
  // la fenêtre n'a plus de barre de titre du système, la barre de menus en tient lieu.
  // Fermer passe par la demande de fermeture habituelle (App.svelte propose d'enregistrer).
  const win = getCurrentWindow();
  let maximized = $state(false);

  async function refresh(): Promise<void> {
    maximized = await win.isMaximized().catch(() => false);
  }

  onMount(() => {
    void refresh();
    const unlisten = win.onResized(() => void refresh());
    return () => void unlisten.then((stop) => stop());
  });
</script>

<div class="controls" role="group" aria-label={t("Fenêtre")}>
  <button title={t("Réduire")} aria-label={t("Réduire")} onclick={() => void win.minimize()}>
    <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5h10" /></svg>
  </button>
  <button title={maximized ? t("Restaurer") : t("Agrandir")} aria-label={maximized ? t("Restaurer") : t("Agrandir")} onclick={() => void win.toggleMaximize().then(refresh)}>
    {#if maximized}
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M2.5 2.5V.6h6.9v6.9H7.5M.6 2.5h6.9v6.9H.6z" /></svg>
    {:else}
      <svg viewBox="0 0 10 10" aria-hidden="true"><rect x="0.6" y="0.6" width="8.8" height="8.8" /></svg>
    {/if}
  </button>
  <button class="close" title={t("Fermer")} aria-label={t("Fermer")} onclick={() => void win.close()}>
    <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1 1l8 8M9 1l-8 8" /></svg>
  </button>
</div>

<style>
  .controls {
    display: flex;
    align-self: stretch;
    margin-left: 6px;
  }
  button {
    width: 46px;
    display: grid;
    place-items: center;
    color: var(--text-3);
    transition: background 120ms var(--ease), color 120ms var(--ease);
  }
  button:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--text);
  }
  button.close:hover {
    background: var(--accent);
    color: #fff;
  }
  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.2;
  }
</style>
