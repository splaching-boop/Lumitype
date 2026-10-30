<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../lib/i18n.svelte";
  import { type FormatDraft, MAX_DPI, MAX_SIDE, validDraft } from "../../lib/custom-formats";
  import { app } from "../../state/app.svelte";

  // Petit panneau « Nouveau format » / « Modifier le format » de la page Formats (maquette lot 3, panneau 1) :
  // nom, largeur et hauteur en pixels (1 à 30000, comme la fenêtre Nouveau document), résolution, fond, et
  // « Reprendre le document ouvert » qui remplit les champs avec le document actif. Posé sous la carte qui l'a
  // ouvert, ramené dans la fenêtre. `role="dialog"` : les raccourcis de l'application s'effacent (voir keymap.ts).
  let {
    initial,
    editing = false,
    anchor,
    onsave,
    oncancel,
  }: {
    initial: FormatDraft;
    /** Modification d'un format existant (titre et bouton « Enregistrer ») plutôt qu'un ajout. */
    editing?: boolean;
    /** Rectangle de la carte d'origine, en coordonnées de la fenêtre. */
    anchor: { left: number; top: number; bottom: number };
    onsave: (draft: FormatDraft) => void;
    oncancel: () => void;
  } = $props();

  const uid = $props.id();
  // Copie de travail : le format d'origine ne change qu'à l'enregistrement.
  // svelte-ignore state_referenced_locally
  let draft = $state<FormatDraft>({ ...initial });
  let panel = $state<HTMLDivElement>();
  let nameInput = $state<HTMLInputElement>();
  let position = $state({ left: 0, top: 0 });

  const valid = $derived(validDraft(draft));
  const side = (v: number) => Number.isInteger(v) && v >= 1 && v <= MAX_SIDE;
  const active = $derived(app.active?.info ?? null);
  /** Élément qui avait le focus : il le retrouve à la fermeture. */
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

  onMount(() => {
    nameInput?.focus();
    nameInput?.select();
    return () => previous?.isConnected && previous.focus({ preventScroll: true });
  });

  // Sous la carte, ou au-dessus quand la place manque en bas ; toujours entièrement dans la fenêtre.
  $effect(() => {
    if (!panel) return;
    const width = panel.offsetWidth;
    const height = panel.offsetHeight;
    const below = anchor.bottom + 6;
    const top = below + height <= window.innerHeight - 8 ? below : Math.max(8, anchor.top - height - 6);
    position = { left: Math.max(8, Math.min(anchor.left, window.innerWidth - width - 8)), top: Math.max(8, Math.min(top, window.innerHeight - height - 8)) };
  });

  /** Le document actif : sa taille, sa résolution, et son nom si le champ est encore vide. */
  function fromActive(): void {
    if (!active) return;
    draft = { ...draft, width: active.width, height: active.height, dpi: active.dpi, label: draft.label.trim() ? draft.label : active.name.replace(/\.[^.]+$/, "") };
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (valid) onsave({ ...draft, label: draft.label.trim() });
  }

  /** Échap ferme le panneau (sans quitter l'accueil derrière). */
  function onKey(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    oncancel();
  }
</script>

<svelte:window
  onkeydowncapture={onKey}
  onpointerdown={(event) => {
    if (event.target instanceof Node && !panel?.contains(event.target)) oncancel();
  }}
/>

<div class="panel" role="dialog" aria-modal="false" aria-labelledby="{uid}-title" bind:this={panel} style:left="{position.left}px" style:top="{position.top}px">
  <form onsubmit={submit}>
    <h4 id="{uid}-title">{editing ? t("Modifier le format") : t("Nouveau format")}</h4>
    <label class="field">
      <span class="caption">{t("Nom")}</span>
      <input type="text" maxlength="80" bind:this={nameInput} bind:value={draft.label} />
    </label>
    <div class="pair">
      <label class="unit" class:bad={!side(draft.width)}>
        <input type="number" min="1" max={MAX_SIDE} step="1" aria-label={t("Largeur (px)")} bind:value={draft.width} />
        <span>{t("L px")}</span>
      </label>
      <label class="unit" class:bad={!side(draft.height)}>
        <input type="number" min="1" max={MAX_SIDE} step="1" aria-label={t("Hauteur (px)")} bind:value={draft.height} />
        <span>{t("H px")}</span>
      </label>
    </div>
    <div class="pair">
      <label class="unit" class:bad={!(Number.isInteger(draft.dpi) && draft.dpi >= 1 && draft.dpi <= MAX_DPI)}>
        <input type="number" min="1" max={MAX_DPI} step="1" aria-label={t("Résolution (ppp)")} bind:value={draft.dpi} />
        <span>{t("ppp")}</span>
      </label>
      <select aria-label={t("Fond")} bind:value={draft.transparent}>
        <option value={false}>{t("Blanc")}</option>
        <option value={true}>{t("Transparent")}</option>
      </select>
    </div>
    <p class="hint">
      {#if active}
        <button type="button" class="link" onclick={fromActive}>{t("Reprendre le document ouvert")}</button>
        <span>· {active.width} × {active.height} px · {active.dpi} ppp</span>
      {:else}
        <span>{t("De 1 à {MAX_SIDE} px par côté.", { MAX_SIDE })}</span>
      {/if}
    </p>
    <div class="row">
      <button type="button" class="btn" onclick={oncancel}>{t("Annuler")}</button>
      <button type="submit" class="btn primary" disabled={!valid}>{editing ? t("Enregistrer") : t("Ajouter")}</button>
    </div>
  </form>
</div>

<style>
  .panel {
    position: fixed;
    z-index: 90;
    width: 392px;
    max-width: calc(100vw - 16px);
    box-sizing: border-box;
    padding: 18px;
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    color: var(--text);
    animation: rise 140ms var(--ease);
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h4 {
    margin: 0 0 2px;
    font-size: 14px;
    font-weight: 600;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .caption {
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  input,
  select,
  .unit {
    height: 34px;
    box-sizing: border-box;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  input,
  select {
    padding: 0 10px;
    outline: none;
    user-select: text;
  }
  input:focus,
  select:focus,
  .unit:focus-within {
    border-color: var(--text-3);
  }
  /* Champ et unité dans le même cadre : « 1920   L px ». */
  .unit {
    display: flex;
    align-items: center;
    padding-right: 10px;
  }
  /* Pas de flèches : elles recouvriraient l'unité. */
  .unit input {
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .unit input::-webkit-inner-spin-button,
  .unit input::-webkit-outer-spin-button {
    appearance: none;
    margin: 0;
  }
  .unit input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
  }
  .unit span {
    font-size: 12px;
    color: var(--text-muted);
  }
  .unit.bad {
    border-color: var(--danger);
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text-muted);
  }
  .link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .link:hover {
    color: var(--text);
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }
  .btn {
    height: 34px;
    padding: 0 16px;
    border: 1px solid var(--border-strong);
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
  }
  .btn:hover {
    background: var(--bg-hover);
  }
  .btn.primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #fff;
  }
  .btn.primary:hover {
    filter: brightness(1.08);
  }
  .btn:disabled {
    opacity: 0.4;
    filter: none;
  }
</style>
