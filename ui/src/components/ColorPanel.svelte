<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { hexToRgb, hsvToRgb, rgbToHex, rgbToHsv, type Hsv } from "../lib/color";
  import { restyleEditingText } from "../lib/text-edit.svelte";
  import { app } from "../state/app.svelte";
  import { activeColorMode, backgroundInMode, foregroundInMode, luma, workflowSpace } from "../lib/color-mode.svelte";
  import { api, type GamutInfo } from "../lib/ipc";
  import { profileShort } from "../lib/color-settings.svelte";
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
    // Saisie de texte en cours : la couleur choisie ici va aussi au texte sélectionné (à tout le texte sinon), telle
    // que le mode du document peut la montrer.
    restyleEditingText({ color: foregroundInMode() });
  }

  function commitHex(): void {
    const rgb = hexDraft === null ? null : hexToRgb(hexDraft);
    if (rgb) setHsv(rgbToHsv(rgb, hsv.h));
    hexDraft = null;
  }

  // La teinte se règle sur l'anneau : sous la roue, deux barres colorées qui montrent ce qu'elles donnent.
  //  - Saturation : de la couleur passée en noir et blanc (gris de même luminosité perçue) à la couleur vive.
  //    Ce n'est pas la saturation TSL/HSB brute (à 0, un rouge vif y devient blanc) : la position `t` de la
  //    barre mélange le gris et la couleur vive, et se traduit en S et L du modèle pour la roue.
  //  - Luminosité : du noir à la couleur, la luminosité L (HSB) du modèle.
  // Les deux pistes suivent la teinte et l'autre réglage en direct.
  const css = ([r, g, b]: readonly number[]) => `rgb(${r}, ${g}, ${b})`;
  /** Luminosité perçue (0 à 1) de la teinte pure : le gris auquel la barre de saturation mène. */
  const pureLuma = (h: number) => {
    const [r, g, b] = hsvToRgb({ h, s: 1, v: 1 });
    return (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  };
  /** Position de la barre de saturation (0 = noir et blanc, 1 = vive) pour une couleur du modèle. */
  const saturationOf = ({ h, s }: Hsv) => {
    const luma = pureLuma(h);
    return s <= 0 ? 0 : (s * luma) / (1 - s + s * luma);
  };
  /** Nouvelle position de la barre de saturation : S et L du modèle, la couleur vive de départ gardée. */
  function withSaturation(color: Hsv, t: number): Hsv {
    const luma = pureLuma(color.h);
    const mix = (k: number) => (1 - k) * luma + k; // part de la valeur max dans le mélange gris/vive
    const vivid = color.v / mix(saturationOf(color)); // luminosité de la couleur vive, inchangée
    const v = Math.min(1, vivid * mix(t));
    return { h: color.h, s: t <= 0 ? 0 : t / mix(t), v };
  }

  const tracks = $derived.by(() => {
    const vivid = hsvToRgb(withSaturation(hsv, 1));
    const gray = Math.round(0.299 * vivid[0] + 0.587 * vivid[1] + 0.114 * vivid[2]);
    return {
      s: `linear-gradient(90deg, ${css([gray, gray, gray])}, ${css(vivid)})`,
      v: `linear-gradient(90deg, #000, ${css(hsvToRgb({ ...hsv, v: 1 }))})`,
    };
  });

  const SLIDERS = [
    { key: "s", label: "S", title: "Saturation", read: (value: Hsv) => saturationOf(value), write: (value: Hsv, x: number) => withSaturation(value, x) },
    { key: "v", label: "L", title: "Luminosité", read: (value: Hsv) => value.v, write: (value: Hsv, x: number) => ({ ...value, v: x }) },
  ] as const;

  // Image › Mode. En gris : un seul curseur, le noir (K, 0 % = blanc), comme le nuancier Gris de Photoshop ; en
  // bitmap : noir ou blanc. Les pastilles et le code montrent la couleur telle qu'elle sera employée dans ce mode ; la
  // couleur choisie reste intacte (voir lib/color-mode.svelte.ts : un document RVB la retrouve).
  const mode = $derived(activeColorMode().kind);
  const shownForeground = $derived(foregroundInMode());
  const shownBackground = $derived(backgroundInMode());
  const black = $derived(Math.round(((255 - luma(shownForeground)) / 255) * 100));
  function setGray(percent: number): void {
    const v = Math.round(255 - (Math.min(100, Math.max(0, percent)) / 100) * 255);
    app.foreground = [v, v, v];
    hsv = rgbToHsv(app.foreground, hsv.h);
    restyleEditingText({ color: app.foreground });
  }

  // Mode CMJN : la couleur est-elle imprimable dans l'espace CMJN du document ? Aller-retour sRGB → CMJN → sRGB par les
  // tables de l'épreuve écran (moteur, `cmyk::gamut`) : hors gamme, un triangle d'alerte et l'équivalent imprimable.
  const profile = $derived(workflowSpace());
  let gamut = $state<GamutInfo | null>(null);
  let gamutRequest = 0;
  $effect(() => {
    const color: [number, number, number] = [app.foreground[0], app.foreground[1], app.foreground[2]];
    const current = profile;
    const mine = ++gamutRequest;
    if (!current) {
      gamut = null;
      return;
    }
    api.cmykGamut(current, color).then(
      (info) => {
        if (mine === gamutRequest) gamut = info;
      },
      () => {
        if (mine === gamutRequest) gamut = null;
      },
    );
  });
  function intoGamut(): void {
    if (!gamut) return;
    setHsv(rgbToHsv(gamut.printable, hsv.h));
  }

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
        <button class="bg" style:background={rgbToHex(shownBackground)} aria-label={t("Couleur d'arrière-plan : permuter (X)")} onclick={swap}></button>
        <span class="fg" style:background={rgbToHex(shownForeground)} aria-label={t("Couleur de premier plan")}></span>
      </div>
      <input
        class="hex"
        aria-label={t("Code hexadécimal")}
        value={hexDraft ?? rgbToHex(shownForeground)}
        oninput={(event) => (hexDraft = event.currentTarget.value)}
        onchange={commitHex}
        onblur={commitHex}
      />
      {#if profile && gamut?.outOfGamut}
        <button
          class="gamut"
          title={t("Hors gamme CMJN ({profile}) : cette couleur ne s'imprimera pas telle quelle (ΔE {delta}). Cliquer pour la ramener dans la gamme.", { profile: profileShort(profile.profile), delta: gamut.deltaE.toFixed(1).replace(".", ",") })}
          aria-label={t("Ramener dans la gamme")}
          onclick={intoGamut}
        >
          <svg viewBox="0 0 16 14" aria-hidden="true"><path d="M8 1 15 13H1Z" fill="#f2c94c" stroke="#1a1a1a" stroke-width="1" stroke-linejoin="round" /><path d="M8 5v4M8 10.6v1" stroke="#1a1a1a" stroke-width="1.6" stroke-linecap="round" /></svg>
          <span class="printable" style:background={rgbToHex(gamut.printable)}></span>
        </button>
      {/if}
      <button class="text" title={t("Couleurs par défaut (D)")} onclick={reset}>{t("Défaut")}</button>
    </div>

    {#if mode === "grayscale"}
      <label title={t("Noir (K) : 0 % = blanc, 100 % = noir")}>
        <span>K</span>
        <input type="range" class="tinted" min="0" max="100" aria-label={t("Noir")} style:--track="linear-gradient(90deg, #fff, #000)" value={black} oninput={(event) => setGray(event.currentTarget.valueAsNumber)} />
        <output>{black} %</output>
      </label>
      <p class="note">{t("Mode Niveaux de gris : les couleurs sont ramenées au gris de même luminance.")}</p>
    {:else if mode === "bitmap"}
      <div class="bw">
        <button class:on={black === 100} onclick={() => setGray(100)}><i style:background="#000"></i>{t("Noir")}</button>
        <button class:on={black === 0} onclick={() => setGray(0)}><i style:background="#fff"></i>{t("Blanc")}</button>
      </div>
      <p class="note">{t("Mode Bitmap : noir ou blanc seulement ; les gris et les bords adoucis sont seuillés à 50 %.")}</p>
    {:else}
    <ColorWheel {hsv} onchange={setHsv} />

    {#each SLIDERS as slider (slider.key)}
      <label title={t(slider.title)}>
        <span>{t(slider.label)}</span>
        <input
          type="range"
          class="tinted"
          min="0"
          max="100"
          aria-label={t(slider.title)}
          style:--track={tracks[slider.key]}
          style:--thumb={css(hsvToRgb(hsv))}
          value={Math.round(slider.read(hsv) * 100)}
          oninput={(event) => setHsv(slider.write(hsv, event.currentTarget.valueAsNumber / 100))}
        />
        <output>{Math.round(slider.read(hsv) * 100)} %</output>
      </label>
    {/each}
    {/if}
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
    border: 1px solid rgb(var(--ink) / 0.35);
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
  /* Piste colorée plus haute que celle des autres curseurs : c'est elle qui montre la couleur. */
  input.tinted {
    height: 10px;
    margin: 4px 0;
    border: 1px solid var(--border-strong);
    border-radius: 5px;
    background: var(--track);
  }
  /* Pastille : la couleur choisie, cerclée de blanc puis d'un fin contour sombre — lisible sur le noir comme sur le
     blanc de la piste, et sur tous les thèmes. */
  input.tinted::-webkit-slider-thumb {
    width: 16px;
    height: 16px;
    background: var(--thumb);
    border: 2px solid #fff;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.55),
      0 1px 3px rgba(0, 0, 0, 0.35);
    cursor: grab;
  }
  input.tinted::-moz-range-thumb {
    width: 16px;
    height: 16px;
    background: var(--thumb);
    border: 2px solid #fff;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.55),
      0 1px 3px rgba(0, 0, 0, 0.35);
    cursor: grab;
  }
  input.tinted:active::-webkit-slider-thumb {
    cursor: grabbing;
  }
  output {
    text-align: right;
    color: var(--text);
  }
  /* Alerte hors gamme (mode CMJN) : le triangle, et l'équivalent imprimable qu'un clic choisit. */
  .gamut {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 4px;
    border-radius: var(--radius-s);
  }
  .gamut:hover {
    background: var(--bg-active);
  }
  .gamut svg {
    width: 16px;
    height: 14px;
  }
  .printable {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    border: 1px solid rgb(var(--ink) / 0.35);
  }
  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.4;
  }
  .bw {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .bw button {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    color: var(--text-2);
  }
  .bw button.on {
    border-color: var(--border-strong);
    background: var(--bg-active);
    color: var(--text);
  }
  .bw i {
    width: 14px;
    height: 14px;
    border-radius: 3px;
    border: 1px solid rgb(var(--ink) / 0.35);
  }
</style>
