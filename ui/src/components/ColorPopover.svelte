<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount, untrack } from "svelte";
  import { hexToRgb, hsvToRgb, rgbToHex, rgbToHsv, type Hsv } from "../lib/color";
  import type { Rgb } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import ColorWheel from "./ColorWheel.svelte";

  // Sélecteur de couleur flottant : roue, code hexadécimal, nuancier, et pipette sur l'image
  // avec Alt. Indépendant des couleurs de premier plan et d'arrière-plan.
  let {
    color,
    x,
    y,
    onchange,
    onclose,
    keepOpenWithin = null,
  }: {
    color: Rgb;
    x: number;
    y: number;
    onchange: (rgb: Rgb) => void;
    onclose: () => void;
    /** Un clic dans cet élément ne ferme pas le sélecteur : la rangée d'arrêts d'un dégradé, où l'on passe d'un arrêt à l'autre. */
    keepOpenWithin?: HTMLElement | null;
  } = $props();

  let panel = $state<HTMLDivElement>();
  // Couleur de départ uniquement : ensuite, c'est ce sélecteur qui fait foi.
  let hsv = $state<Hsv>(untrack(() => rgbToHsv(color)));
  let hexDraft = $state<string | null>(null);
  let position = $state({ left: 0, top: 0 });

  const GRAYS = [0, 36, 73, 109, 146, 182, 219, 255].map((v): Rgb => [v, v, v]);
  const HUES = [0, 30, 60, 120, 180, 210, 270, 320];
  const NUANCIER: Rgb[] = [
    ...GRAYS,
    ...HUES.map((h) => hsvToRgb({ h, s: 1, v: 1 })),
    ...HUES.map((h) => hsvToRgb({ h, s: 0.55, v: 1 })),
    ...HUES.map((h) => hsvToRgb({ h, s: 1, v: 0.55 })),
  ];

  function set(rgb: Rgb): void {
    hsv = rgbToHsv(rgb, hsv.h); // un gris garde la teinte courante de la roue
    onchange(rgb);
  }

  function commitHex(): void {
    const rgb = hexDraft === null ? null : hexToRgb(hexDraft);
    if (rgb) set(rgb);
    hexDraft = null;
  }

  function close(): void {
    // La couleur retenue rejoint les couleurs récentes, sans doublon, huit au plus.
    const chosen = hsvToRgb(hsv);
    app.recentColors = [chosen, ...app.recentColors.filter((c) => rgbToHex(c) !== rgbToHex(chosen))].slice(0, 8);
    onclose();
  }

  onMount(() => {
    // Tant que le sélecteur est ouvert, Alt + clic sur l'image lui envoie la couleur pointée.
    app.eyedropperTarget = (rgb) => set(rgb);
    return () => (app.eyedropperTarget = null);
  });

  $effect(() => {
    if (!panel) return;
    position = {
      left: Math.max(8, Math.min(x, window.innerWidth - panel.offsetWidth - 8)),
      top: Math.max(8, Math.min(y, window.innerHeight - panel.offsetHeight - 8)),
    };
  });
</script>

<svelte:window
  onpointerdown={(event) => {
    // Un clic sur l'image avec Alt est une prise de couleur, pas une fermeture.
    const inside = event.target instanceof Node && (panel?.contains(event.target) || keepOpenWithin?.contains(event.target));
    if (!inside && !event.altKey) close();
  }}
  onkeydown={(event) => event.key === "Escape" && close()}
/>

<div class="popover" role="dialog" aria-label={t("Couleur")} bind:this={panel} style:left="{position.left}px" style:top="{position.top}px">
  <ColorWheel {hsv} size={168} onchange={(next) => ((hsv = next), onchange(hsvToRgb(next)))} />

  <div class="line">
    <span class="preview" style:background={rgbToHex(hsvToRgb(hsv))}></span>
    <input
      aria-label={t("Code hexadécimal")}
      value={hexDraft ?? rgbToHex(hsvToRgb(hsv))}
      oninput={(event) => (hexDraft = event.currentTarget.value)}
      onchange={commitHex}
      onkeydown={(event) => event.key === "Enter" && commitHex()}
    />
  </div>

  <div class="swatches" role="group" aria-label={t("Nuancier")}>
    {#each NUANCIER as swatch, index (index)}
      <button style:background={rgbToHex(swatch)} aria-label={rgbToHex(swatch)} title={rgbToHex(swatch)} onclick={() => set(swatch)}></button>
    {/each}
  </div>
  {#if app.recentColors.length > 0}
    <p>{t("Récentes")}</p>
    <div class="swatches" role="group" aria-label={t("Couleurs récentes")}>
      {#each app.recentColors as swatch, index (index)}
        <button style:background={rgbToHex(swatch)} aria-label={rgbToHex(swatch)} title={rgbToHex(swatch)} onclick={() => set(swatch)}></button>
      {/each}
    </div>
  {/if}
  <p class="hint">{t("Alt + clic sur l'image : prélever une couleur")}</p>
</div>

<style>
  .popover {
    position: fixed;
    z-index: 45;
    width: 216px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  .line {
    display: flex;
    gap: 8px;
  }
  .preview {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.3);
  }
  input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 4px;
  }
  .swatches button {
    aspect-ratio: 1;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.14);
  }
  .swatches button:hover {
    outline: 1.5px solid #fff;
    outline-offset: 1px;
  }
  p {
    margin: 0;
    color: var(--text-muted);
  }
  .hint {
    font-size: 11px;
  }
</style>
