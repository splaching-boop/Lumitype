<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { app } from "../state/app.svelte";

  // Message d'erreur : s'efface seul en fondu au bout de deux secondes. Le survoler le retient (le temps de le
  // lire ou de le copier) ; une nouvelle erreur relance le délai.
  const VISIBLE_MS = 2000;
  const FADE_MS = 400;
  let fading = $state(false);
  let hovered = $state(false);

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
    style:--fade="{FADE_MS}ms"
    onpointerenter={() => ((hovered = true), (fading = false))}
    onpointerleave={() => (hovered = false)}
  >
    <span>{t(app.error)}</span>
    <button onclick={() => (app.error = null)}>{t("Fermer")}</button>
  </div>
{/if}

<style>
  .toast {
    position: fixed;
    left: 50%;
    bottom: 44px;
    z-index: 30;
    transform: translateX(-50%);
    max-width: 640px;
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 10px 10px 16px;
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
  button {
    flex-shrink: 0;
    height: 26px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
  }
</style>
