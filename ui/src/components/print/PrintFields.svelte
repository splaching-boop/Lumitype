<script lang="ts">
  import { t } from "../../lib/i18n.svelte";

  // Réglages d'impression en rangées alignées : libellé | valeur (largeur fixe) | interrupteur (largeur fixe).
  // Partagé par la fenêtre Nouveau document et Image › Format d'impression….
  interface Props {
    bleedOn: boolean;
    bleedMm: number;
    safeOn: boolean;
    safeMm: number;
    inkOn: boolean;
    inkLimit: number;
    /** L'encrage n'a de sens qu'en CMJN : grisé sinon. */
    inkEnabled?: boolean;
    /** Ligne « Étendre le contenu dans les fonds perdus » (document existant). */
    extend?: boolean;
    showExtend?: boolean;
    /** Encrage conseillé (profil), rappelé quand la limite est éteinte. */
    inkAdvice?: string;
    /** Résolution du document (ppp), en tête des réglages ; absente : pas de ligne. */
    dpi?: number;
    showDpi?: boolean;
  }
  let { bleedOn = $bindable(), bleedMm = $bindable(), safeOn = $bindable(), safeMm = $bindable(), inkOn = $bindable(), inkLimit = $bindable(), inkEnabled = true, extend = $bindable(false), showExtend = false, inkAdvice = "", dpi = $bindable(300), showDpi = false }: Props = $props();
</script>

{#snippet toggle(on: boolean, label: string, set: (v: boolean) => void, disabled = false)}
  <button type="button" class="switch" class:on role="switch" aria-checked={on} aria-label={label} {disabled} onclick={() => set(!on)}></button>
{/snippet}

<div class="rows">
  {#if showDpi}
    <div class="row">
      <span>{t("Résolution")}</span>
      <label class="value"><input type="number" min="1" max="9600" step="1" bind:value={dpi} aria-label={t("Résolution")} /><em>{t("ppp")}</em></label>
      <span></span>
    </div>
  {/if}
  <div class="row" class:off={!bleedOn}>
    <span>{t("Fonds perdus")}</span>
    <label class="value"><input type="number" min="0" max="50" step="0.5" bind:value={bleedMm} disabled={!bleedOn} aria-label={t("Fonds perdus")} /><em>mm</em></label>
    {@render toggle(bleedOn, t("Fonds perdus"), (v) => (bleedOn = v))}
  </div>
  <div class="row" class:off={!safeOn}>
    <span>{t("Zone de sécurité")}</span>
    <label class="value"><input type="number" min="0" max="100" step="0.5" bind:value={safeMm} disabled={!safeOn} aria-label={t("Zone de sécurité")} /><em>mm</em></label>
    {@render toggle(safeOn, t("Zone de sécurité"), (v) => (safeOn = v))}
  </div>
  <div class="row" class:off={!inkOn || !inkEnabled} class:disabled={!inkEnabled} title={inkEnabled ? undefined : t("En CMJN seulement")}>
    <span>{t("Encrage max.")}</span>
    <label class="value"><input type="number" min="150" max="400" step="5" bind:value={inkLimit} disabled={!inkOn || !inkEnabled} aria-label={t("Encrage max.")} /><em>%</em></label>
    {@render toggle(inkOn && inkEnabled, t("Encrage max."), (v) => (inkOn = v), !inkEnabled)}
  </div>
  {#if inkAdvice && inkEnabled && !inkOn}<p class="advice">{inkAdvice}</p>{/if}
  {#if showExtend}
    <div class="row" class:off={!extend || !bleedOn}>
      <span>{t("Étendre le contenu dans les fonds perdus")}</span>
      <span></span>
      {@render toggle(extend && bleedOn, t("Étendre le contenu dans les fonds perdus"), (v) => (extend = v), !bleedOn)}
    </div>
  {/if}
</div>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .advice {
    margin: -2px 0 2px;
    font-size: 11.5px;
    color: var(--text-muted);
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 96px 34px;
    gap: 10px;
    align-items: center;
    min-height: 30px;
  }
  .row > span:first-child {
    color: var(--text-2);
  }
  .row.disabled > span:first-child {
    opacity: 0.5;
  }
  .value {
    position: relative;
    display: block;
  }
  .value input {
    width: 100%;
    height: 30px;
    padding: 0 30px 0 8px;
    text-align: right;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .value em {
    position: absolute;
    right: 8px;
    top: 50%;
    transform: translateY(-50%);
    font-style: normal;
    font-size: 11px;
    color: var(--text-muted);
    pointer-events: none;
  }
  .row.off .value input {
    opacity: 0.45;
  }
  .switch {
    justify-self: end;
    position: relative;
    width: 34px;
    height: 18px;
    border-radius: 9px;
    background: var(--border-strong);
    transition: background 0.15s;
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-muted);
    transition: left 0.15s;
  }
  .switch.on {
    background: var(--accent);
  }
  .switch.on::after {
    left: 19px;
    background: #fff;
  }
  .switch:disabled {
    opacity: 0.45;
  }
</style>
