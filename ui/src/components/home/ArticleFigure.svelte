<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import cursorAdd from "../../assets/cursors/pen-add.png";
  import cursorConvert from "../../assets/cursors/pen-convert.png";
  import cursorDelete from "../../assets/cursors/pen-delete.png";
  import cursorPen from "../../assets/cursors/pen.png";
  import type { Figure } from "../../lib/news";
  import { banner } from "./banner";

  // Figures détaillées du corps d'un article : de petites mises en scène de l'interface, dessinées en HTML.
  let { figure }: { figure: Figure } = $props();

  // Pinceau flou : couverture au centre après n clics (un tiers par clic, comme le moteur).
  const clicks = [1, 2, 4, 8].map((n) => ({ n, alpha: 1 - (2 / 3) ** n }));
  const photo = banner;
</script>

<div class="figure" data-figure={figure}>
  {#if figure === "clipLayers"}
    <div class="stage">
      {#if photo}<img class="photo warm" src={photo} alt="" />{:else}<span class="photo warm tone"></span>{/if}
    </div>
    <div class="panel">
      <span class="panel-title">{t("Calques")}</span>
      <div class="row"><span class="thumb text">T</span>{t("Titre")}</div>
      <div class="row on">
        <svg class="clip" viewBox="0 0 24 24"><path d="M6.5 2.5v8.8a4.2 4.2 0 0 0 4.2 4.2H16" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round" /><path d="M15.4 10l6.6 5.5-6.6 5.5z" /></svg>
        <span class="thumb"><span class="half"></span></span>{t("Développement")}
      </div>
      <div class="seam"></div>
      <div class="row">{#if photo}<img class="thumb" src={photo} alt="" />{:else}<span class="thumb tone"></span>{/if}{t("Photo")}</div>
      <div class="row"><span class="thumb white"></span>{t("Arrière-plan")}</div>
      <span class="hint"><kbd>{t("Alt")}</kbd> {t("+ clic entre les deux")}</span>
    </div>
  {:else if figure === "softBrushClicks"}
    <div class="paper">
      {#each clicks as click (click.n)}
        <span class="step">
          <span class="dab" style:background="radial-gradient(circle, rgba(0,0,0,{click.alpha}) 0%, rgba(0,0,0,{click.alpha * 0.85}) 22%, rgba(0,0,0,{click.alpha * 0.4}) 48%, rgba(0,0,0,0) 72%)"></span>
          <span class="label">{click.n > 1 ? t("{n} clics", { n: click.n }) : t("1 clic")} · {Math.round(click.alpha * 100)} %</span>
        </span>
      {/each}
    </div>
  {:else if figure === "multiSelectFrame"}
    <div class="stage">
      <div class="group">
        {#if photo}<img class="piece a" src={photo} alt="" />{:else}<span class="piece a tone"></span>{/if}
        <span class="piece b"></span>
        <span class="piece c"></span>
        <span class="box">
          {#each ["tl", "tm", "tr", "ml", "mr", "bl", "bm", "br"] as handle (handle)}<span class="handle {handle}"></span>{/each}
        </span>
      </div>
    </div>
  {:else if figure === "adjustLive"}
    <div class="stage column">
      {#if photo}<img class="photo strip" src={photo} alt="" />{:else}<span class="photo strip tone"></span>{/if}
      <div class="slider">
        <span class="slider-label">{t("Exposition")}</span><span class="slider-value">+0,80 IL</span>
        <span class="track"><span class="fill"></span><span class="knob"></span></span>
      </div>
    </div>
  {:else if figure === "penCursors"}
    <div class="stage">
      <svg class="path" viewBox="0 0 520 200" aria-hidden="true">
        <path d="M40 150C120 30 220 30 280 110S440 180 480 60" fill="none" stroke="#ec2a3c" stroke-width="2" />
        <rect x="36" y="146" width="8" height="8" fill="#fff" stroke="#ec2a3c" stroke-width="1.5" />
        <rect x="276" y="106" width="8" height="8" fill="#fff" stroke="#ec2a3c" stroke-width="1.5" />
        <rect x="476" y="56" width="8" height="8" fill="#fff" stroke="#ec2a3c" stroke-width="1.5" />
      </svg>
      <span class="cursor" style:left="12%" style:top="68%"><img src={cursorPen} alt="" /><span>{t("Tracer")}</span></span>
      <span class="cursor" style:left="30%" style:top="18%"><img src={cursorAdd} alt="" /><span>{t("Ajouter")}</span></span>
      <span class="cursor" style:left="54%" style:top="50%"><img src={cursorDelete} alt="" /><span>{t("Supprimer")}</span></span>
      <span class="cursor" style:left="86%" style:top="22%"><img src={cursorConvert} alt="" /><span><kbd>{t("Alt")}</kbd> {t("Convertir")}</span></span>
    </div>
  {:else}
    <div class="stage">
      <div class="history">
        <span class="history-title">{t("Historique")}</span>
        {#each ["Ouvrir", "Pinceau", "Développement", "Déplacer", "Effets de calque"] as label, index (index)}
          <span class="state" class:current={index === 4}>{label}</span>
        {/each}
        <span class="saving"><span class="bar"><span></span></span>{t("Copie de secours · historique compris")}</span>
      </div>
    </div>
  {/if}
</div>

<style>
  .figure {
    position: relative;
    height: 320px;
    border-radius: var(--radius-l);
    overflow: hidden;
    background: #0f0f11;
    border: 1px solid rgba(255, 255, 255, 0.08);
    display: flex;
  }
  .stage {
    flex-grow: 1;
    position: relative;
    display: grid;
    place-items: center;
    background: var(--bg-canvas);
  }
  .stage.column {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 22px;
  }
  .tone {
    background: #1a1726;
  }
  .photo {
    width: 420px;
    height: 174px;
    object-fit: cover;
    border-radius: var(--radius-s);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
    display: block;
  }
  .photo.warm {
    filter: saturate(1.5) sepia(0.2) hue-rotate(-10deg);
  }
  .photo.strip {
    width: 460px;
    height: 150px;
    filter: brightness(1.35) saturate(1.1);
  }

  /* panneau Calques (écrêtage) */
  .panel {
    width: 250px;
    flex-shrink: 0;
    box-sizing: border-box;
    padding: 16px 12px;
    background: var(--bg-panel);
    border-left: 1px solid rgba(255, 255, 255, 0.07);
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 13px;
  }
  .panel-title {
    padding: 0 6px 10px;
    font-weight: 600;
  }
  .row {
    height: 42px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    gap: 10px;
    border-radius: var(--radius-m);
  }
  .row.on {
    background: rgba(255, 255, 255, 0.1);
    gap: 8px;
  }
  .thumb {
    width: 40px;
    height: 24px;
    flex-shrink: 0;
    border-radius: var(--radius-s);
    background: #2a2a2f;
    object-fit: cover;
    display: grid;
    place-items: center;
  }
  .thumb.text {
    font-weight: 700;
    font-size: 11px;
  }
  .thumb.white {
    background: #fff;
  }
  .half {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    border: 2.5px solid var(--text);
    box-sizing: border-box;
    background: linear-gradient(90deg, var(--text) 50%, transparent 50%);
  }
  .clip {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    color: var(--accent);
    fill: var(--accent);
  }
  .seam {
    height: 2px;
    margin: 0 8px;
    border-radius: var(--radius-xs);
    background: var(--accent);
  }
  .hint {
    margin-top: auto;
    padding-left: 8px;
    font-size: 12px;
    color: var(--text-3);
  }
  kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 20px;
    padding: 0 6px;
    box-sizing: border-box;
    border-radius: var(--radius-s);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-bottom-width: 2px;
    font: 500 11px "Geist Mono", ui-monospace, monospace;
    color: var(--text);
  }

  /* pinceau flou */
  .paper {
    flex-grow: 1;
    background: #f4f4f5;
    display: flex;
    align-items: center;
    justify-content: space-evenly;
  }
  .step {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
  .dab {
    width: 120px;
    height: 120px;
    border-radius: 50%;
  }
  .label {
    font-size: 13px;
    font-weight: 500;
    color: #3a3a40;
  }

  /* sélection multiple */
  .group {
    position: relative;
    width: 430px;
    height: 220px;
  }
  .piece {
    position: absolute;
    border-radius: var(--radius-s);
    object-fit: cover;
  }
  .piece.a {
    left: 0;
    top: 30px;
    width: 220px;
    height: 150px;
  }
  .piece.b {
    left: 250px;
    top: 0;
    width: 150px;
    height: 100px;
    background: #efeadf;
  }
  .piece.c {
    left: 280px;
    top: 130px;
    width: 150px;
    height: 90px;
    background: #6f6f77;
  }
  .box {
    position: absolute;
    inset: -8px;
    border: 1.5px solid var(--accent);
  }
  .handle {
    position: absolute;
    width: 10px;
    height: 10px;
    background: #fff;
    border: 1.5px solid var(--accent);
    box-sizing: border-box;
    transform: translate(-50%, -50%);
  }
  .tl { left: 0; top: 0; }
  .tm { left: 50%; top: 0; }
  .tr { left: 100%; top: 0; }
  .ml { left: 0; top: 50%; }
  .mr { left: 100%; top: 50%; }
  .bl { left: 0; top: 100%; }
  .bm { left: 50%; top: 100%; }
  .br { left: 100%; top: 100%; }

  /* réglage en direct */
  .slider {
    width: 460px;
    display: grid;
    grid-template-columns: 1fr auto;
    row-gap: 12px;
    font-size: 13px;
  }
  .slider-value {
    color: var(--text-2);
  }
  .track {
    grid-column: 1 / -1;
    position: relative;
    height: 6px;
    border-radius: var(--radius-xs);
    background: rgba(255, 255, 255, 0.14);
  }
  .fill {
    position: absolute;
    left: 50%;
    width: 8%;
    top: 0;
    bottom: 0;
    background: #9dbdff;
  }
  .knob {
    position: absolute;
    left: 58%;
    top: 50%;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    transform: translate(-50%, -50%);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
  }

  /* curseurs de la plume */
  .path {
    position: absolute;
    inset: 40px 60px;
    width: calc(100% - 120px);
    height: calc(100% - 80px);
  }
  .cursor {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-2);
    transform: translate(-6px, -24px);
  }
  .cursor img {
    width: 32px;
    height: 32px;
  }

  /* historique et copie de secours */
  .history {
    width: 300px;
    padding: 14px;
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    border: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 13px;
  }
  .history-title {
    padding: 0 8px 8px;
    font-weight: 600;
  }
  .state {
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    display: flex;
    align-items: center;
    color: var(--text-2);
  }
  .state.current {
    background: rgba(255, 255, 255, 0.1);
    color: var(--text);
  }
  .saving {
    margin-top: 10px;
    padding: 0 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 12px;
    color: var(--text-muted);
  }
  .bar {
    height: 3px;
    border-radius: var(--radius-xs);
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }
  .bar span {
    display: block;
    width: 64%;
    height: 100%;
    background: linear-gradient(90deg, #fff, rgba(120, 140, 255, 0.95));
  }
</style>
