<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { hexToRgb, hsvToRgb, rgbToHex, rgbToHsv, type Hsv } from "../lib/color";
  import { restyleEditingText } from "../lib/text-edit.svelte";
  import { app } from "../state/app.svelte";
  import ColorWheel from "./ColorWheel.svelte";
  import Panel from "./Panel.svelte";

  let hsv = $state<Hsv>(rgbToHsv(app.foreground));
  let hexDraft = $state<string | null>(null);

  // La couleur peut changer ailleurs (touches X et D, pipette) : la teinte courante est gardée pour les gris.
  $effect(() => {
    const external = app.foreground;
    if (rgbToHex(hsvToRgb(hsv)) !== rgbToHex(external)) hsv = rgbToHsv(external, hsv.h);
  });

  function setHsv(next: Hsv): void {
    hsv = next;
    app.foreground = hsvToRgb(next);
    // Saisie de texte en cours : la couleur choisie ici va aussi au texte sélectionné (à tout le texte sinon).
    restyleEditingText({ color: app.foreground });
  }

  function commitHex(): void {
    const rgb = hexDraft === null ? null : hexToRgb(hexDraft);
    if (rgb) setHsv(rgbToHsv(rgb, hsv.h));
    hexDraft = null;
  }

  const SLIDERS = [
    { key: "h", label: "T", max: 360, unit: "°", scale: 1 },
    { key: "s", label: "S", max: 100, unit: "%", scale: 100 },
    { key: "v", label: "L", max: 100, unit: "%", scale: 100 },
  ] as const;

  function swap(): void {
    [app.foreground, app.background] = [app.background, app.foreground];
  }

  function reset(): void {
    app.foreground = [0, 0, 0];
    app.background = [255, 255, 255];
  }
</script>

<Panel title={t("Couleur")}>
  <div class="content">
    <div class="top">
      <div class="swatches">
        <button class="bg" style:background={rgbToHex(app.background)} aria-label={t("Couleur d'arrière-plan : permuter (X)")} onclick={swap}></button>
        <span class="fg" style:background={rgbToHex(app.foreground)} aria-label={t("Couleur de premier plan")}></span>
      </div>
      <input
        class="hex"
        aria-label={t("Code hexadécimal")}
        value={hexDraft ?? rgbToHex(app.foreground)}
        oninput={(event) => (hexDraft = event.currentTarget.value)}
        onchange={commitHex}
        onblur={commitHex}
      />
      <button class="text" title={t("Couleurs par défaut (D)")} onclick={reset}>{t("Défaut")}</button>
    </div>

    <ColorWheel {hsv} onchange={setHsv} />

    {#each SLIDERS as slider (slider.key)}
      <label>
        <span>{t(slider.label)}</span>
        <input
          type="range"
          min="0"
          max={slider.max}
          value={Math.round(hsv[slider.key] * slider.scale)}
          oninput={(event) => setHsv({ ...hsv, [slider.key]: event.currentTarget.valueAsNumber / slider.scale })}
        />
        <output>{Math.round(hsv[slider.key] * slider.scale)} {slider.unit}</output>
      </label>
    {/each}
  </div>
</Panel>

<style>
  .content {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px 14px;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .swatches {
    position: relative;
    width: 34px;
    height: 34px;
    flex-shrink: 0;
  }
  .fg,
  .bg {
    position: absolute;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.35);
  }
  .fg {
    top: 0;
    left: 0;
  }
  .bg {
    right: 0;
    bottom: 0;
  }
  .hex {
    width: 84px;
    height: 26px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    user-select: text;
  }
  .text {
    margin-left: auto;
    color: var(--text-muted);
  }
  .text:hover {
    color: var(--text);
  }
  label {
    display: grid;
    grid-template-columns: 12px 1fr 48px;
    align-items: center;
    gap: 10px;
    color: var(--text-muted);
  }
  input[type="range"] {
    width: 100%;
  }
  output {
    text-align: right;
    color: var(--text);
  }
</style>
