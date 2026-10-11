<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { rgbToHex } from "../lib/color";
  import { editLatest } from "../lib/engine";
  import { type LayerInfo, type TextData, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import { type RunStyle, applyStyle, applyToLayer, styleOver } from "../lib/text-runs";
  import ColorPopover from "./ColorPopover.svelte";
  import FontPicker from "./FontPicker.svelte";
  import StylePicker, { type Style } from "./StylePicker.svelte";

  // Réglages d'un calque de texte. Le contenu se tape directement sur l'image (outil Texte).
  let { layer, text }: { layer: LayerInfo; text: TextData } = $props();

  /** Hors saisie, les réglages successifs de ce calque ne font qu'un état d'historique. */
  const ownSession = `reglages-texte-${untrack(() => layer.id)}-${Date.now()}`;
  let colorPopover = $state<{ x: number; y: number } | null>(null);

  $effect(() => {
    if (app.fonts === null) {
      app.fonts = [];
      api.listFonts().then(
        (fonts) => (app.fonts = fonts),
        (error: unknown) => app.fail(error),
      );
    }
  });

  const editing = $derived(app.textEdit?.layerId === layer.id ? app.textEdit : null);
  /** Texte de référence : pendant la saisie, ce qui est tapé fait foi (en avance sur le moteur). */
  const current = $derived<TextData>(editing ? { ...text, content: editing.draft, runs: editing.runs } : text);
  /** Plage visée par police, style et couleur : la sélection en cours de saisie, sinon tout le texte. */
  const range = $derived(editing && editing.selection.end > editing.selection.start ? editing.selection : null);
  /** Valeurs affichées : celles de la plage ; `null` quand elles y varient (affiché « ? »). */
  const shown = $derived(range ? styleOver(current, range.start, range.end) : styleOver(current, 0, current.content.length));

  function send(next: TextData): void {
    // Les réglages choisis, couleur comprise, servent de point de départ au prochain calque de texte.
    const { content: _content, runs: _runs, ...defaults } = next;
    app.textDefaults = defaults;
    if (editing) editing.runs = next.runs ?? [];
    const session = editing?.id ?? ownSession;
    editLatest(session, { op: "setText", id: layer.id, text: next, session });
  }

  /**
   * Plusieurs calques de texte sélectionnés (Maj ou Ctrl + clic dans le panneau Calques), hors saisie : les
   * réglages s'appliquent à tous, chacun gardant son propre contenu.
   */
  const others = $derived(
    editing ? [] : (app.active?.selection ?? []).filter((id) => id !== layer.id).map((id) => app.active?.info.layers.find((l) => l.id === id)).filter((l): l is LayerInfo & { text: TextData } => !!l?.text),
  );

  function sendToOthers(transform: (data: TextData) => TextData): void {
    for (const other of others) {
      const session = `${ownSession}-${other.id}`;
      editLatest(session, { op: "setText", id: other.id, text: transform(other.text), session });
    }
  }

  /** Réglages du calque entier (taille, interlignage, approche, alignement). */
  function change(patch: Partial<TextData>): void {
    send({ ...current, ...patch });
    sendToOthers((data) => ({ ...data, ...patch }));
  }

  /**
   * Police, style et couleur : appliqués à la sélection s'il y en a une pendant la saisie,
   * sinon à tout le calque (les portions perdent alors cette propriété).
   */
  function restyle(patch: RunStyle): void {
    if (range) send({ ...current, runs: applyStyle(current, range.start, range.end, patch) });
    else {
      send(applyToLayer(current, patch));
      sendToOthers((data) => applyToLayer(data, patch));
    }
  }

  /**
   * Le réglage garde le focus après un changement : on peut ainsi parcourir les polices avec
   * les flèches haut et bas et voir le texte changer en direct. Entrée ou Échap rend la main à la saisie.
   */
  function backToTyping(event: KeyboardEvent): void {
    if (event.key !== "Enter" && event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    returnToTyping();
  }

  function returnToTyping(): void {
    if (app.textEdit?.layerId === layer.id) app.focusTextEditor = true;
    else if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  }

  /** Variantes installées de la famille affichée ; sans famille choisie, les quatre styles de base. */
  const styles = $derived(
    app.fonts?.find((f) => f.family === shown.fontFamily)?.styles ?? [
      { weight: 400, italic: false },
      { weight: 400, italic: true },
      { weight: 700, italic: false },
      { weight: 700, italic: true },
    ],
  );

  /** Texte d'avant le survol de la liste des polices, pour le rétablir si rien n'est choisi. */
  let beforeHover: TextData | null = null;

  function restoreAfterHover(): void {
    if (beforeHover) send(beforeHover);
    beforeHover = null;
  }

  /**
   * Changer de famille garde le style s'il existe dans la nouvelle, sinon prend le plus proche.
   * S'applique à la sélection en cours de saisie, sinon à tout le texte. `temporary` : survol de la liste.
   */
  function chooseFamily(family: string, temporary: boolean): void {
    if (temporary) beforeHover ??= $state.snapshot(current) as TextData;
    else beforeHover = null;
    const available = app.fonts?.find((f) => f.family === family)?.styles ?? [];
    const [weight, italic] = [shown.weight ?? text.weight, shown.italic ?? text.italic];
    const cost = (s: { weight: number; italic: boolean }) => Math.abs(s.weight - weight) + (s.italic === italic ? 0 : 1000);
    const closest = [...available].sort((a, b) => cost(a) - cost(b))[0];
    restyle({ fontFamily: family, ...(closest && { weight: closest.weight, italic: closest.italic }) });
  }

  /** Style choisi dans la liste ; `temporary` : simple survol, à défaire par `restoreAfterHover`. */
  function chooseStyle(style: Style, temporary: boolean): void {
    if (temporary) beforeHover ??= $state.snapshot(current) as TextData;
    else beforeHover = null;
    restyle({ weight: style.weight, italic: style.italic });
  }

  const number = (event: Event & { currentTarget: HTMLInputElement }, apply: (value: number) => void) => {
    const value = event.currentTarget.valueAsNumber;
    if (Number.isFinite(value)) apply(value);
  };

  const ALIGNMENTS = [
    { value: "left", label: t("Aligner à gauche"), icon: "M4 6h16M4 12h10M4 18h14" },
    { value: "center", label: "Centrer", icon: "M4 6h16M7 12h10M5 18h14" },
    { value: "right", label: t("Aligner à droite"), icon: "M4 6h16M10 12h10M6 18h14" },
  ] as const;
</script>

<div class="text">
  {#if others.length > 0}
    <p class="many">{t("{n} textes sélectionnés : police, taille, couleur et alignement s'appliquent à tous.", { n: others.length + 1 })}</p>
  {/if}
  <p class="hint">{app.textEdit?.layerId === layer.id ? range ? t("Police, style et couleur s'appliquent au texte sélectionné") : t("Saisie en cours sur l'image · Ctrl+Entrée : valider · Échap : annuler") : t("Outil Texte (T) + clic dans le texte pour le modifier")}</p>

  <label>
    {t("Police")}
    <!-- « ? » : la plage visée mélange plusieurs polices. -->
    <FontPicker value={shown.fontFamily} fonts={app.fonts ?? []} onpreview={chooseFamily} onrestore={restoreAfterHover} ondone={returnToTyping} />
  </label>

  <label>
    {t("Style")}
    <StylePicker
      value={shown.weight === null || shown.italic === null ? null : { weight: shown.weight, italic: shown.italic }}
      {styles}
      family={shown.fontFamily}
      bundled={app.fonts?.find((f) => f.family === shown.fontFamily)?.bundled === true}
      onpreview={chooseStyle}
      onrestore={restoreAfterHover}
      ondone={returnToTyping}
    />
  </label>

  <div class="row">
    <label>{t("Taille (px)")} <input type="number" min="1" max="2000" onkeydown={backToTyping} value={text.size} onchange={(e) => number(e, (size) => change({ size: Math.min(2000, Math.max(1, size)) }))} /></label>
    <label>{t("Interlignage (%)")} <input type="number" min="50" max="500" onkeydown={backToTyping} value={Math.round(text.lineHeight * 100)} onchange={(e) => number(e, (v) => change({ lineHeight: Math.min(5, Math.max(0.5, v / 100)) }))} /></label>
  </div>
  <div class="row">
    <label>{t("Approche")} <input type="number" min="-200" max="2000" onkeydown={backToTyping} step="10" value={text.tracking} onchange={(e) => number(e, (tracking) => change({ tracking }))} /></label>
    <div class="toggles">
      <button class:on={(shown.weight ?? 0) >= 600} aria-pressed={(shown.weight ?? 0) >= 600} aria-label={t("Gras")} title={t("Gras")} onclick={() => restyle({ weight: (shown.weight ?? 0) >= 600 ? 400 : 700 })}><b>G</b></button>
      <button class:on={shown.italic === true} aria-pressed={shown.italic === true} aria-label={t("Italique")} title={t("Italique")} onclick={() => restyle({ italic: shown.italic !== true })}><i>I</i></button>
    </div>
  </div>

  <div class="row">
    <div class="toggles">
      {#each ALIGNMENTS as option (option.value)}
        <button class:on={text.align === option.value} aria-pressed={text.align === option.value} aria-label={t(option.label)} title={t(option.label)} onclick={() => change({ align: option.value })}>
          <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path d={option.icon} /></svg>
        </button>
      {/each}
    </div>
    <button
      class="color"
      title={t("Couleur du texte")}
      onclick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        colorPopover = colorPopover ? null : { x: rect.right + 12, y: rect.top - 120 };
      }}
    >
      {#if shown.color}
        <span style:background={rgbToHex(shown.color)}></span>
        {rgbToHex(shown.color)}
      {:else}
        <!-- Plusieurs couleurs dans la plage visée. -->
        <span class="mixed"></span>
        ?
      {/if}
    </button>
  </div>
</div>

{#if colorPopover}
  <ColorPopover
    color={shown.color ?? text.color}
    x={colorPopover.x}
    y={colorPopover.y}
    onchange={(color) => restyle({ color })}
    onclose={() => {
      colorPopover = null;
      if (app.textEdit?.layerId === layer.id) app.focusTextEditor = true;
    }}
  />
{/if}

<style>
  .text {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
  }
  input {
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
  }
  .hint {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  label {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-3);
  }
  input {
    height: 26px;
    padding: 0 6px;
    min-width: 0;
    user-select: text;
  }
  .row {
    display: flex;
    align-items: flex-end;
    gap: 10px;
  }
  .toggles {
    display: flex;
    gap: 2px;
  }
  .toggles button {
    width: 28px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-3);
  }
  .toggles button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .toggles button.on {
    background: var(--bg-active);
    color: #fff;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
  .color {
    margin-left: auto;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .color:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .color .mixed {
    background: repeating-linear-gradient(45deg, #8b8b93 0 3px, #2a2a2e 3px 6px);
  }
  .color span {
    width: 14px;
    height: 14px;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.35);
  }
  .many {
    margin: 0 0 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: rgba(77, 141, 255, 0.12);
    border: 1px solid rgba(77, 141, 255, 0.35);
    color: var(--text-2);
    line-height: 1.4;
  }
</style>
