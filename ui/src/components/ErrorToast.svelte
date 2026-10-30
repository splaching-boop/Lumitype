<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { app } from "../state/app.svelte";

  // Message d'erreur : s'efface seul en fondu au bout de deux secondes. Le survoler le retient (le temps de le
  // lire ou de le copier) ; une nouvelle erreur relance le délai.
  const VISIBLE_MS = 2000;
  const FADE_MS = 400;
  let fading = $state(false);
  let hovered = $state(false);
  let toast = $state<HTMLDivElement>();

  // Le message est un « popover » manuel : il vit dans la couche supérieure, au-dessus des boîtes de dialogue
  // modales (showModal), là où un simple `position: fixed; z-index` resterait caché sous elles.
  $effect(() => {
    const el = toast;
    if (!el) return;
    try {
      el.showPopover();
    } catch {
      // navigateur sans popover : l'élément reste visible comme un bloc fixe ordinaire
    }
    return () => {
      try {
        el.hidePopover();
      } catch {
        // déjà fermé ou retiré du document
      }
    };
  });

  $effect(() => {
    const message = app.error;
    if (!message || hovered) return;
    fading = false;
    const fade = setTimeout(() => (fading = true), VISIBLE_MS);
    const clear = setTimeout(() => {
      if (app.error === message) app.error = null;
      fading = false;
    }, VISIBLE_MS + FADE_MS);
    return () => {
      clearTimeout(fade);
      clearTimeout(clear);
    };
  });
</script>

{#if app.error}
  <div
    class="toast"
    class:fading
    role="alert"
    popover="manual"
    bind:this={toast}
    style:--fade="{FADE_MS}ms"
    onpointerenter={() => ((hovered = true), (fading = false))}
    onpointerleave={() => (hovered = false)}
  >
    {#if app.errorCode?.message === app.error}<code title={t("Code de l'erreur, à citer dans un rapport de bug")}>{app.errorCode.code}</code>{/if}
    <span>{t(app.error)}</span>
    <button onclick={() => (app.error = null)}>{t("Fermer")}</button>
  </div>
{/if}

<style>
  .toast {
    /* Remise à zéro du style navigateur des popovers (inset: 0, marge auto, bordure et fond « canvas »). */
    position: fixed;
    inset: auto;
    left: 50%;
    bottom: 44px;
    margin: 0;
    width: max-content;
    overflow: visible;
    color: var(--text);
    z-index: 30;
    transform: translateX(-50%);
    max-width: 640px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 10px 10px 12px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--danger);
    box-shadow: var(--shadow-menu);
    user-select: text;
    opacity: 1;
    transition: opacity var(--fade) ease;
  }
  .toast.fading {
    opacity: 0;
  }
  code {
    flex-shrink: 0;
    padding: 2px 6px;
    border-radius: var(--radius-xs);
    background: var(--danger);
    color: var(--on-accent);
    font: 600 11px/1.4 ui-monospace, monospace;
    letter-spacing: 0.02em;
  }
  button {
    flex-shrink: 0;
    height: 26px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
  }
</style>
