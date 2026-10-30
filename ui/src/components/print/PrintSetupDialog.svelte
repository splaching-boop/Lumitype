<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { edit } from "../../lib/engine";
  import { DEFAULT_PRINT, formatLength, recommendedInk } from "../../lib/print.svelte";
  import { exportSpace } from "../../lib/color-mode.svelte";
  import { profileShort } from "../../lib/color-settings.svelte";
  import { colorModeOf } from "../../lib/color-mode.svelte";
  import { app } from "../../state/app.svelte";
  import PrintFields from "./PrintFields.svelte";

  // Image › Format d'impression… : fonds perdus, zone de sécurité et limite d'encrage d'un document existant. Les
  // fonds perdus agrandissent la zone de travail autour du format fini (qui ne bouge pas).
  let dialog = $state<HTMLDialogElement>();
  let v = $state({ bleedOn: true, bleedMm: DEFAULT_PRINT.bleedMm, safeOn: true, safeMm: DEFAULT_PRINT.safeMm, inkOn: true, inkLimit: DEFAULT_PRINT.inkLimit, extend: true });
  const info = $derived(app.active?.info ?? null);
  const cmyk = $derived(colorModeOf(info).kind === "cmykWorkflow");
  let dpi = $state(300);
  const advice = $derived(info ? t("Conseillé : {advised} % pour {profile}", { advised: recommendedInk(exportSpace(info).profile), profile: profileShort(exportSpace(info).profile) }) : "");
  const trim = $derived<[number, number, number, number]>(info ? (info.trim ?? [0, 0, info.width, info.height]) : [0, 0, 0, 0]);

  $effect(() => {
    if (!app.printSetupDialogOpen) {
      dialog?.close();
      return;
    }
    const p = app.active?.info.print;
    dpi = Math.round(app.active?.info.dpi ?? 300);
    v = p
      ? { bleedOn: p.bleedMm > 0, bleedMm: p.bleedMm || DEFAULT_PRINT.bleedMm, safeOn: p.safeMm > 0, safeMm: p.safeMm || DEFAULT_PRINT.safeMm, inkOn: p.inkLimit > 0, inkLimit: p.inkLimit || DEFAULT_PRINT.inkLimit, extend: true }
      : { bleedOn: true, bleedMm: DEFAULT_PRINT.bleedMm, safeOn: true, safeMm: DEFAULT_PRINT.safeMm, inkOn: true, inkLimit: DEFAULT_PRINT.inkLimit, extend: true };
    dialog?.showModal();
  });

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    app.printSetupDialogOpen = false;
    const any = v.bleedOn || v.safeOn || v.inkOn;
    const setup = any ? { bleedMm: v.bleedOn ? Math.max(0, v.bleedMm) : 0, safeMm: v.safeOn ? Math.max(0, v.safeMm) : 0, inkLimit: v.inkOn ? v.inkLimit : 0 } : null;
    const changedDpi = info && Number.isFinite(dpi) && dpi >= 1 && dpi !== info.dpi ? dpi : null;
    void edit({ op: "setPrintSetup", setup, extend: v.extend, dpi: changedDpi });
  }

  function remove(): void {
    app.printSetupDialogOpen = false;
    void edit({ op: "setPrintSetup", setup: null, extend: false });
  }
</script>

<dialog bind:this={dialog} onclose={() => (app.printSetupDialogOpen = false)}>
  <form onsubmit={submit}>
    <h2>{t("Format d'impression")}</h2>
    {#if info}
      <div class="row">
        <span>{t("Format fini")}</span>
        <span class="muted">{formatLength(trim[2], "mm", dpi || info.dpi)} × {formatLength(trim[3], "mm", dpi || info.dpi)} · {trim[2]} × {trim[3]} px</span>
      </div>
    {/if}
    <PrintFields bind:bleedOn={v.bleedOn} bind:bleedMm={v.bleedMm} bind:safeOn={v.safeOn} bind:safeMm={v.safeMm} bind:inkOn={v.inkOn} bind:inkLimit={v.inkLimit} bind:extend={v.extend} showExtend inkEnabled={true} showDpi bind:dpi inkAdvice={advice} />
    <p class="hint">
      {t("Les fonds perdus agrandissent la zone de travail de chaque côté ; les calques qui touchent le bord y sont prolongés (bord recopié). Le format fini ne bouge pas.")}
      {#if !cmyk}<br />{t("Document en RVB : l'encrage est vérifié avec le profil des Paramètres de couleur.")}{/if}
    </p>
    <div class="actions">
      {#if info?.print}<button type="button" class="ghost left" onclick={remove}>{t("Retirer")}</button>{/if}
      <button type="button" class="ghost" onclick={() => (app.printSetupDialogOpen = false)}>{t("Annuler")}</button>
      <button type="submit" class="pill">{t("Appliquer")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 400px;
    padding: 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    color: var(--text);
    box-shadow: var(--shadow-menu);
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 14px;
    font-weight: 500;
  }
  .row {
    display: flex;
    justify-content: space-between;
    color: var(--text-2);
  }
  .muted,
  .hint {
    color: var(--text-muted);
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    line-height: 1.5;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    height: 28px;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }
  .left {
    margin-right: auto;
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
