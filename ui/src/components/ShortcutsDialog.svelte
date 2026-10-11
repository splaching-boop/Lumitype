<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { ACTION_CATEGORIES, ACTION_LABELS } from "../lib/action-catalog";
  import type { ActionId } from "../lib/actions";
  import { type Chord, chordFromEvent, chordLabel } from "../lib/keymap";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";

  let dialog = $state<HTMLDialogElement>();
  let search = $state("");
  /** Action en attente d'une frappe. */
  let recording = $state<ActionId | null>(null);
  /** Frappe capturée qui entre en conflit : l'utilisateur doit trancher. */
  let pending = $state<{ action: ActionId; chord: Chord; conflicts: ActionId[] } | null>(null);
  let notice = $state<string | null>(null);

  const categories = $derived(
    ACTION_CATEGORIES.map((category) => ({
      name: category.name,
      actions: category.actions.filter((id) => ACTION_LABELS[id].toLowerCase().includes(search.trim().toLowerCase())),
    })).filter((category) => category.actions.length > 0),
  );

  $effect(() => {
    if (app.shortcutsDialogOpen) dialog?.showModal();
    else dialog?.close();
  });

  function close(): void {
    recording = null;
    pending = null;
    notice = null;
    app.shortcutsDialogOpen = false;
  }

  function startRecording(action: ActionId): void {
    pending = null;
    notice = null;
    recording = action;
  }

  /** Capture la prochaine frappe. Phase de capture : aucun raccourci de l'application ne se déclenche. */
  function onKeyDown(event: KeyboardEvent): void {
    if (!recording) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      recording = null;
      return;
    }
    const chord = chordFromEvent(event);
    if (!chord) return; // touche de modification seule : on attend la suite
    const action = recording;
    recording = null;

    if (chord.mods === 0 && /^(Digit|Numpad)\d$/.test(chord.value)) {
      notice = "Les chiffres seuls sont réservés au réglage de l'opacité (1 = 10 % … 0 = 100 %).";
      return;
    }
    const conflicts = keymap.conflicts(chord, action);
    if (conflicts.length > 0) pending = { action, chord, conflicts };
    else keymap.assign(action, chord);
  }

  function confirmPending(): void {
    if (pending) keymap.assign(pending.action, pending.chord);
    pending = null;
  }
</script>

<svelte:window onkeydowncapture={onKeyDown} />

<dialog bind:this={dialog} onclose={close} oncancel={(event) => recording && event.preventDefault()}>
  <header>
    <h2>{t("Raccourcis clavier")}</h2>
    <input type="search" placeholder={t("Rechercher une commande")} bind:value={search} />
    <button class="ghost" onclick={() => keymap.resetAll()}>{t("Tout réinitialiser")}</button>
  </header>

  <p class="info">
    {t("Les lettres suivent le caractère gravé sur la touche ; chiffres, crochets et pavé numérique suivent la position de la touche, identique en AZERTY et en QWERTY.")}
  </p>

  {#if pending}
    <div class="conflict" role="alert">
      <span>
        <strong>{chordLabel(pending.chord)}</strong> {t("est déjà utilisé par")}
        {pending.conflicts.map((id) => `« ${t(ACTION_LABELS[id])} »`).join(", ")}.
      </span>
      <button class="ghost" onclick={() => (pending = null)}>{t("Annuler")}</button>
      <button class="pill" onclick={confirmPending}>{t("Réattribuer")}</button>
    </div>
  {:else if notice}
    <div class="conflict" role="alert"><span>{notice}</span></div>
  {/if}

  <div class="list">
    <!-- Rien n'est construit tant que l'écran est fermé. -->
    {#each app.shortcutsDialogOpen ? categories : [] as category (category.name)}
      <h3>{t(category.name)}</h3>
      {#each category.actions as action (action)}
        <div class="row">
          <span class="label">{t(ACTION_LABELS[action])}</span>
          <div class="chords">
            {#each keymap.chordsOf(action) as chord, index (index)}
              <span class="chip">
                {chordLabel(chord)}
                <button aria-label={t("Retirer {chord}", { chord: chordLabel(chord) })} onclick={() => keymap.remove(action, chord)}>×</button>
              </span>
            {/each}
            {#if recording === action}
              <span class="chip recording">{t("Appuyez sur les touches… (Échap : annuler)")}</span>
            {:else}
              <button class="add" aria-label={t("Ajouter un raccourci à {action}", { action: t(ACTION_LABELS[action]) })} onclick={() => startRecording(action)}>+</button>
            {/if}
          </div>
          <button class="reset" disabled={!keymap.isCustomized(action)} onclick={() => keymap.reset(action)}>{t("Défaut")}</button>
        </div>
      {/each}
    {/each}
  </div>

  <footer><button class="pill" onclick={close}>{t("Fermer")}</button></footer>
</dialog>

<style>
  dialog {
    width: 720px;
    max-height: 80vh;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    color: var(--text);
    box-shadow: var(--shadow-menu);
  }
  dialog[open] {
    display: flex;
    flex-direction: column;
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  header,
  footer {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 20px;
  }
  footer {
    justify-content: flex-end;
    border-top: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
  }
  h3 {
    margin: 14px 0 4px;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-muted);
  }
  input[type="search"] {
    flex: 1;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .info {
    margin: 0;
    padding: 0 20px 10px;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .conflict {
    margin: 0 20px 8px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--danger);
  }
  .conflict span {
    flex: 1;
    line-height: 1.5;
  }
  .list {
    /* Base « auto » et non 0 : la fenêtre n'a qu'une hauteur maximale. Avec une base nulle, le moteur de rendu
       de Linux (WebKitGTK) écrasait la liste à zéro — l'écran des raccourcis paraissait vide ; Chromium, lui,
       la dépliait. Ainsi, elle prend sa hauteur naturelle et défile au-delà de 80 % de l'écran. */
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 20px 12px;
  }
  .row {
    min-height: 34px;
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--border);
  }
  .label {
    flex: 1;
  }
  .chords {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
  }
  .chip {
    height: 24px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 10px;
    border-radius: var(--radius-pill);
    background: var(--bg-active);
  }
  .chip button {
    width: 16px;
    color: var(--text-muted);
  }
  .chip button:hover {
    color: var(--text);
  }
  .chip.recording {
    padding: 0 10px;
    background: transparent;
    border: 1px dashed var(--accent);
    color: var(--text-2);
  }
  .add {
    width: 24px;
    height: 24px;
    border-radius: 50%;
    border: 1px solid var(--border-strong);
    color: var(--text-3);
  }
  .add:hover {
    color: var(--text);
  }
  .reset {
    width: 52px;
    color: var(--text-muted);
  }
  .reset:disabled {
    visibility: hidden;
  }
  .ghost,
  .pill {
    height: 28px;
    padding: 0 14px;
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
