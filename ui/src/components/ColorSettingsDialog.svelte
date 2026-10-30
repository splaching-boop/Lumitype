<script lang="ts">
  // Édition › Paramètres de couleur… (Maj+Ctrl+K), comme Photoshop : un seul CMJN pour tout Lumitype (épreuve Ctrl+Y,
  // Image › Mode › Couleurs CMJN, export CMJN d'un document RVB). Voir lib/color-settings.svelte.ts.
  //
  // Profils proposés : les cinq livrés, ceux chargés dans Lumitype, ceux installés sur l'ordinateur (dont ceux de
  // Photoshop, par exemple « U.S. Web Coated (SWOP) v2 »), et « Charger un profil ICC… ».
  import { printPrefs, setPref } from "../lib/print.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { describeError } from "../lib/errors";
  import { t } from "../lib/i18n.svelte";
  import { DEFAULT_SPACE, applyColorSettings, colorSettings, importProfile, refreshProfiles } from "../lib/color-settings.svelte";
  import type { CmykSpace } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  /** Usage de chaque profil livré, à la manière de Photoshop. */
  const HINTS: Record<string, string> = {
    fogra39: "Offset couché, Europe — défaut de Photoshop en Europe",
    fogra51: "Offset couché, Europe — norme ISO 12647-2:2013",
    fogra52: "Offset non couché, Europe",
    gracol2006: "Offset feuilles couché n° 1, Amérique du Nord",
    swop2006c3: "Rotative couchée n° 3, Amérique du Nord",
  };
  const LOAD = "__charger";

  let dialog = $state<HTMLDialogElement>();
  let draft = $state<CmykSpace>({ ...DEFAULT_SPACE });
  /** Noirs à l'impression (export PDF/X-4), voir lib/print.svelte.ts. */
  let blacks = $state({ textBlack: true, richBlack: true });
  let error = $state("");
  let loading = $state(false);

  const builtin = $derived(colorSettings.profiles.filter((p) => p.origin === "builtin"));
  const user = $derived(colorSettings.profiles.filter((p) => p.origin === "user"));
  const system = $derived(colorSettings.profiles.filter((p) => p.origin === "system"));
  const chosen = $derived(colorSettings.profiles.find((p) => p.id === draft.profile));
  const hint = $derived(chosen ? (chosen.origin === "builtin" ? t(HINTS[chosen.id] ?? "") : chosen.origin === "system" ? t("Installé sur cet ordinateur : {path}", { path: chosen.path ?? "" }) : t("Chargé dans Lumitype")) : "");

  $effect(() => {
    if (colorSettings.dialogOpen) {
      draft = { ...colorSettings.cmyk };
      blacks = { textBlack: printPrefs.textBlack, richBlack: printPrefs.richBlack };
      error = "";
      dialog?.showModal();
      // Un profil installé depuis le dernier lancement (Photoshop vient d'être installé…) apparaît sans redémarrer.
      void refreshProfiles();
    } else dialog?.close();
  });

  function close(): void {
    colorSettings.dialogOpen = false;
  }

  async function pick(event: Event): Promise<void> {
    const select = event.currentTarget as HTMLSelectElement;
    if (select.value !== LOAD) {
      draft.profile = select.value;
      return;
    }
    select.value = draft.profile;
    const path = await open({ multiple: false, filters: [{ name: t("Profils ICC"), extensions: ["icc", "icm", "ICC", "ICM"] }] });
    if (typeof path !== "string") return;
    loading = true;
    try {
      draft.profile = await importProfile(path);
      error = "";
    } catch (failure) {
      error = describeError(failure);
    } finally {
      loading = false;
    }
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    applyColorSettings(draft);
    setPref("textBlack", blacks.textBlack);
    setPref("richBlack", blacks.richBlack);
    close();
  }

  const docInCmyk = $derived(app.documents.some((d) => d.info.colorMode?.kind === "cmykWorkflow"));
</script>

<dialog bind:this={dialog} onclose={close} aria-label={t("Paramètres de couleur")}>
  <form onsubmit={submit}>
    <h2>{t("Paramètres de couleur")}</h2>
    <p class="intro">{t("Un seul CMJN pour tout Lumitype : épreuve écran (Ctrl+Y), mode Couleurs CMJN et export utilisent ce profil.")}{#if docInCmyk}{" "}{t("Les projets déjà en CMJN gardent le leur (Image › Mode › Couleurs CMJN pour le changer).")}{/if}</p>

    <div class="section">{t("Espaces de travail")}</div>
    <label class="row">
      <span>{t("RVB")}</span>
      <select disabled><option>sRGB IEC61966-2.1</option></select>
    </label>
    <label class="row">
      <span>{t("CMJN")}</span>
      <select value={draft.profile} onchange={pick} disabled={loading} aria-describedby="cmyk-hint">
        <optgroup label={t("Livrés avec Lumitype")}>
          {#each builtin as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
        </optgroup>
        {#if user.length}
          <optgroup label={t("Chargés dans Lumitype")}>
            {#each user as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
          </optgroup>
        {/if}
        {#if system.length}
          <optgroup label={t("Installés sur cet ordinateur")}>
            {#each system as item (item.id)}<option value={item.id}>{item.name}</option>{/each}
          </optgroup>
        {/if}
        <option value={LOAD}>{t("Charger un profil ICC…")}</option>
      </select>
    </label>
    <p class="hint" id="cmyk-hint">{loading ? t("Lecture du profil…") : error || hint}</p>
    <label class="row">
      <span>{t("Niveaux de gris")}</span>
      <select disabled><option>{t("Gris sRGB (sans profil)")}</option></select>
    </label>

    <div class="section">{t("Options de conversion")}</div>
    <label class="row">
      <span>{t("Intention")}</span>
      <select bind:value={draft.intent}>
        <option value="relative">{t("Colorimétrie relative")}</option>
        <option value="perceptual">{t("Perceptive")}</option>
      </select>
    </label>
    <label class="check"><input type="checkbox" bind:checked={draft.bpc} /> {t("Compensation du point noir")}</label>

    <div class="section">{t("Noirs à l'impression")}</div>
    <label class="check"><input type="checkbox" bind:checked={blacks.textBlack} /> {t("Texte et filets noirs → 100 % N seul")}</label>
    <label class="check"><input type="checkbox" bind:checked={blacks.richBlack} /> {t("Aplats noirs → noir riche C60 M40 J40 N100")}</label>
    <p class="hint">{t("Appliqué à l'export PDF/X-4. Le noir pur (#000) s'imprime en noir seul, net même si les plaques sont un peu décalées ; l'intérieur des grands aplats reçoit le noir riche (240 %), plus profond, leur bord reste en noir seul.")}</p>

    <div class="actions">
      <button type="button" class="ghost left" onclick={() => (draft = { ...DEFAULT_SPACE })}>{t("Réglages Photoshop « Europe »")}</button>
      <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
      <button type="submit" class="pill">OK</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 560px;
    padding: 18px 20px 20px;
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
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .intro {
    margin: 0;
    color: var(--text-3);
    line-height: 1.45;
  }
  .section {
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .row {
    display: grid;
    grid-template-columns: 150px minmax(0, 1fr);
    align-items: center;
    gap: 12px;
  }
  .row span {
    color: var(--text-3);
  }
  select {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    background: var(--bg-field);
    color: var(--text);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
  }
  select:disabled {
    color: var(--text-2);
  }
  .hint {
    margin: -6px 0 0 162px;
    min-height: 15px;
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .check {
    display: flex;
    gap: 8px;
    align-items: center;
    padding-left: 162px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions .left {
    margin-right: auto;
  }
  .actions button {
    height: 32px;
    padding: 0 16px;
    border-radius: var(--radius-m);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--accent, #ec2a3c);
    color: var(--on-accent);
    font-weight: 500;
  }
</style>
