<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { BLEND_MENU } from "../lib/blend";
  import { onDestroy } from "svelte";
  import { edit, editLatest } from "../lib/engine";
  import type { BlendMode } from "../lib/ipc";
  import { type DismissGuard, dismissGuard } from "../lib/menu-dismiss";

  // Menu des modes de fusion avec aperçu en direct : survoler (ou parcourir au clavier) un mode
  // l'applique provisoirement à l'image. Rien n'entre dans l'historique tant qu'on n'a pas choisi,
  // et fermer le menu sans choisir remet l'image exactement comme avant.
  // `onpreview` / `onchoose` : pour autre chose qu'un calque (le mode d'un effet) ; `null` = revenir à l'original.
  let {
    value,
    ids = [],
    group = false,
    disabled = false,
    exclude = [],
    onpreview,
    onchoose,
  }: {
    value: BlendMode;
    ids?: number[];
    group?: boolean;
    disabled?: boolean;
    exclude?: BlendMode[];
    onpreview?: (mode: BlendMode) => void;
    onchoose?: (mode: BlendMode) => void;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  /** Mode d'avant l'ouverture, et identifiant du geste d'aperçu en cours. */
  let original: BlendMode = "normal";
  let gesture = "";
  let previewed = false;
  let highlighted = $state<BlendMode>("normal");
  /**
   * Garde de fermeture, créée à chaque ouverture. Sous Chromium embarqué (CEF/X11), un clic fait perdre puis
   * reprendre aussitôt le focus à la fenêtre : la liste se fermait sur ce `blur` passager et annulait l'aperçu
   * avant que le clic n'atteigne le mode choisi — le mode ne s'appliquait jamais.
   */
  let guard: DismissGuard | null = null;
  onDestroy(() => guard?.dispose());

  const entries = $derived<([BlendMode, string] | null)[]>(
    (group ? [["pass_through", "Transfert"] as [BlendMode, string], null, ...BLEND_MENU] : [...BLEND_MENU])
      .filter((entry) => !entry || !exclude.includes(entry[0]))
      // Pas de séparateur en tête ni deux de suite après le retrait d'un mode.
      .filter((entry, index, list) => entry !== null || (index > 0 && list[index - 1] !== null)),
  );
  const modes = $derived(entries.flatMap((entry) => (entry ? [entry[0]] : [])));
  const label = $derived(entries.find((entry) => entry?.[0] === value)?.[1] ?? "Normal");

  function show(): void {
    original = value;
    highlighted = value;
    gesture = `apercu-fusion-${Date.now()}`;
    previewed = false;
    guard?.dispose();
    guard = dismissGuard();
    open = true;
    queueMicrotask(() => list?.querySelector<HTMLElement>(".current")?.focus());
  }

  function preview(mode: BlendMode): void {
    highlighted = mode;
    previewed = true;
    if (onpreview) return onpreview(mode);
    editLatest(gesture, { op: "setProps", ids, patch: { blend: mode }, gesture });
  }

  function choose(mode: BlendMode): void {
    open = false;
    guard?.dispose();
    guard = null;
    if (onchoose) {
      // Effet : on revient à l'original si rien n'a été choisi, sinon le mode choisi reste.
      if (mode !== original || previewed) onchoose(mode);
      root?.querySelector<HTMLElement>(".trigger")?.focus();
      return;
    }
    if (mode === original) {
      if (previewed) void edit({ op: "cancelGesture", gesture });
    } else {
      // Même geste que l'aperçu : un seul état « Mode de fusion » dans l'historique.
      editLatest(gesture, { op: "setProps", ids, patch: { blend: mode }, gesture });
    }
    root?.querySelector<HTMLElement>(".trigger")?.focus();
  }

  const dismiss = () => open && choose(original);

  function onKeyDown(event: KeyboardEvent): void {
    if (!open) return;
    event.stopPropagation();
    const index = modes.indexOf(highlighted);
    const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (step !== 0) {
      const next = modes[Math.min(modes.length - 1, Math.max(0, index + step))];
      if (next) {
        preview(next);
        list?.querySelector<HTMLElement>(`[data-mode="${next}"]`)?.focus();
      }
    } else if (event.key === "Enter" || event.key === " ") choose(highlighted);
    else if (event.key === "Escape") dismiss();
    else return;
    event.preventDefault();
  }
</script>

<svelte:window
  onpointerdown={(event) => {
    if (!open || (event.target instanceof Node && root?.contains(event.target))) return;
    if (guard?.pointerDownCloses(event) ?? true) dismiss();
  }}
  onblur={() => open && guard?.onWindowBlur(dismiss)}
/>

<div class="select" bind:this={root} onkeydown={onKeyDown} role="presentation">
  <button class="trigger" {disabled} aria-haspopup="listbox" aria-expanded={open} aria-label={t("Mode de fusion")} onclick={() => (open ? dismiss() : show())}>
    <span>{t(label)}</span>
    <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
  </button>
  {#if open}
    <div class="list" role="listbox" aria-label={t("Modes de fusion")} bind:this={list}>
      {#each entries as entry, index (index)}
        {#if entry}
          <button
            role="option"
            aria-selected={entry[0] === highlighted}
            class:current={entry[0] === highlighted}
            data-mode={entry[0]}
            onpointerenter={() => entry[0] !== highlighted && preview(entry[0])}
            onpointerdown={(event) => {
              // Choix à l'appui (comme la liste des polices), pas au clic : un changement de focus entre l'appui
              // et le relâchement ne peut plus fermer la liste avant le choix. Le clavier passe par `onKeyDown` ;
              // un « clic » sans pointeur (technologies d'assistance) choisit encore, ci-dessous.
              if (event.button !== 0) return;
              event.preventDefault();
              choose(entry[0]);
            }}
            onclick={(event) => event.detail === 0 && open && choose(entry[0])}
          >
            {t(entry[1])}
          </button>
        {:else}
          <div class="separator" role="separator"></div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .select {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .trigger {
    width: 100%;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
  }
  .trigger:disabled {
    color: var(--text-disabled);
  }
  svg {
    flex-shrink: 0;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .list {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    z-index: 30;
    width: max(100%, 220px);
    max-height: min(70vh, 640px);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .list button {
    height: 26px;
    flex-shrink: 0;
    padding: 0 10px;
    border-radius: var(--radius-s);
    text-align: left;
    color: var(--text-2);
  }
  .list button.current {
    background: var(--bg-active);
    color: #fff;
  }
  .separator {
    height: 1px;
    flex-shrink: 0;
    margin: 3px 6px;
    background: var(--border-strong);
  }
</style>
