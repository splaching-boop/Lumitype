<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../lib/i18n.svelte";

  // Confirmation dans l'application (maquette « collection », vue 3), à la place de la boîte du système : icône
  // corbeille, titre, question où `{name}` est mis en gras, petite ligne d'explication, Annuler (focus par défaut,
  // Échap) et un bouton rouge. Sert à la suppression d'une collection et au retrait d'une image de sa collection.
  // `role="dialog"` : les raccourcis de l'application ne réagissent pas derrière elle (voir keymap.ts).
  let {
    title,
    message,
    name = "",
    detail = "",
    confirmLabel,
    onconfirm,
    oncancel,
  }: {
    title: string;
    /** Question déjà traduite ; `{name}` y est remplacé par `name`, en gras. */
    message: string;
    name?: string;
    detail?: string;
    confirmLabel: string;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();

  let box = $state<HTMLDivElement>();
  let cancelButton = $state<HTMLButtonElement>();
  const parts = $derived(message.split("{name}"));
  /** Élément qui avait le focus : il le retrouve à la fermeture. */
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

  onMount(() => {
    cancelButton?.focus();
    return () => previous?.isConnected && previous.focus({ preventScroll: true });
  });

  /**
   * Écouté à la phase de capture : Échap annule ici (sans fermer la visionneuse ni quitter l'accueil), et Tab reste
   * entre les deux boutons.
   */
  function onKey(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      oncancel();
      return;
    }
    if (event.key === "Tab") {
      const buttons = Array.from(box?.querySelectorAll("button") ?? []);
      const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
      event.preventDefault();
      event.stopPropagation();
      buttons[(at + (event.shiftKey ? -1 : 1) + buttons.length) % buttons.length]?.focus();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<div class="shade" role="presentation" onpointerdown={(event) => event.target === event.currentTarget && oncancel()}>
  <div class="dlg" role="dialog" aria-modal="true" aria-labelledby="myref-confirm-title" aria-describedby="myref-confirm-text" bind:this={box}>
    <div class="ic" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13" /></svg></div>
    <h3 id="myref-confirm-title">{title}</h3>
    <p id="myref-confirm-text">{#each parts as part, index (index)}{#if index > 0}<b>{name}</b>{/if}{part}{/each}</p>
    {#if detail}<p class="small">{detail}</p>{/if}
    <div class="row">
      <button bind:this={cancelButton} onclick={oncancel}>{t("Annuler")}</button>
      <button class="del" onclick={onconfirm}>{confirmLabel}</button>
    </div>
  </div>
</div>

<style>
  .shade {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.55);
    animation: fade 140ms var(--ease);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .dlg {
    width: 380px;
    max-width: calc(100vw - 32px);
    box-sizing: border-box;
    margin-top: -6vh;
    padding: 22px;
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    color: var(--text);
    animation: rise 160ms var(--ease);
  }
  @keyframes rise {
    from {
      transform: translateY(6px);
    }
  }
  .ic {
    width: 36px;
    height: 36px;
    margin-bottom: 14px;
    display: grid;
    place-items: center;
    background: rgba(255, 122, 92, 0.12);
    color: var(--danger);
  }
  .ic svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
  }
  h3 {
    margin: 0 0 8px;
    font-size: 15px;
  }
  p {
    margin: 0;
    color: var(--text-2);
    font-size: 12.5px;
    line-height: 1.55;
    overflow-wrap: anywhere;
  }
  p b {
    color: var(--text);
  }
  p.small {
    margin-top: 8px;
    color: var(--text-muted);
    font-size: 11.5px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
  }
  button {
    height: 32px;
    padding: 0 16px;
    font: inherit;
    font-size: 12.5px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  button:hover {
    background: var(--bg-hover);
  }
  /* Annuler a le focus à l'ouverture : Entrée ne supprime rien par mégarde. */
  button:focus {
    outline: none;
    border-color: var(--text-3);
  }
  button:focus-visible {
    outline: 2px solid var(--text-3);
    outline-offset: 2px;
  }
  .del {
    background: #c8392b;
    border-color: #c8392b;
    color: var(--text-strong);
  }
  .del:hover {
    background: #d9443a;
  }
</style>
