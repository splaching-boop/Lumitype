<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { type BrushDynamics, type BrushKind, type BrushTipInfo, MIXER_DYNAMICS, NEUTRAL_DYNAMICS, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Panneau du pinceau : formes de base, matières fournies, pointes importées (.abr, .gbr, images),
  // dynamique de forme, et préréglages personnels enregistrés d'un lancement à l'autre.
  const BASICS = [
    { id: "hard", label: "Net", hint: t("Rond à bord franc"), icon: "M12 5a7 7 0 100 14 7 7 0 000-14z", fill: true, soft: false },
    { id: "soft", label: "Flou", hint: t("Rond à bord fondu"), icon: "M12 8a4 4 0 100 8 4 4 0 000-8z", fill: true, soft: true },
    { id: "spray", label: "Spray", hint: t("Nuage de gouttelettes, pour fondre des couleurs"), icon: "M7 8h.01M11 6h.01M15 8h.01M9 11h.01M13 10h.01M17 12h.01M7 14h.01M11 14h.01M15 15h.01M9 17h.01M13 18h.01", fill: false, soft: false },
    { id: "mixer", label: "Mélangeur", hint: t("Entraîne et mêle les couleurs qu'il traverse"), icon: "M5 16c4-8 6 4 9-3s3-5 5-6", fill: false, soft: false },
  ] as const;

  /** Préréglage personnel. La pointe est désignée par son NOM : les identifiants changent d'un lancement à l'autre. */
  interface Preset {
    name: string;
    kind: "round" | "spray" | "mixer" | "tip";
    tip?: string;
    size: number;
    hardness: number;
    opacity: number;
    dynamics: BrushDynamics;
  }

  // `inline` : le contenu est posé tel quel dans le panneau Propriétés ; sinon, c'est le bouton de la barre
  // d'options, qui ne fait qu'y conduire.
  let { inline = false }: { inline?: boolean } = $props();

  let importing = $state(false);
  let presets = $state<Preset[]>([]);
  let presetName = $state("");

  const settings = $derived(app.brushes[app.paintTool ?? "brush"]);
  const kind = $derived(app.brushKind);
  const tip = $derived(kind.type === "tip" ? app.brushTips.find((t) => t.id === kind.id) : undefined);
  const current = $derived(tip?.name ?? (kind.type === "spray" ? "Spray" : kind.type === "mixer" ? "Mélangeur" : settings.hardness >= 0.5 ? "Net" : "Flou"));
  const builtin = $derived(app.brushTips.filter((t) => t.builtin));
  const imported = $derived(app.brushTips.filter((t) => !t.builtin));

  // Bibliothèque et préréglages : chargés dès le départ, pour qu'un préréglage retrouve sa pointe.
  onMount(() => {
    api.listBrushes().then(adopt, (error: unknown) => app.fail(error));
    api.loadSettings("brush-presets").then(
      (text) => {
        try {
          const parsed: unknown = text ? JSON.parse(text) : [];
          if (Array.isArray(parsed)) presets = parsed as Preset[];
        } catch {
          presets = [];
        }
      },
      () => undefined,
    );
  });

  function adopt(tips: BrushTipInfo[]): void {
    const known = new Set(app.brushTips.map((t) => t.id));
    app.brushTips = [...app.brushTips, ...tips.filter((t) => !known.has(t.id))];
    for (const item of tips) {
      if (app.brushThumbnails[item.id]) continue;
      api.brushThumbnail(item.id).then(
        (png) => (app.brushThumbnails[item.id] = URL.createObjectURL(new Blob([png], { type: "image/png" }))),
        () => undefined,
      );
    }
  }

  /** Changer de forme remet la dynamique à zéro : chaque forme repart de son comportement naturel. */
  function choose(next: BrushKind, hardness?: number): void {
    app.brushKind = next;
    app.brushDynamics = next.type === "mixer" ? { ...MIXER_DYNAMICS } : { ...NEUTRAL_DYNAMICS };
    if (hardness !== undefined) settings.hardness = hardness;
  }

  function chooseBasic(id: (typeof BASICS)[number]["id"]): void {
    if (id === "hard") choose({ type: "round" }, 1);
    else if (id === "soft") choose({ type: "round" }, 0);
    else choose({ type: id });
  }

  async function importFile(): Promise<void> {
    try {
      const path = await open({ multiple: false, filters: [{ name: "Pinceaux", extensions: ["abr", "gbr", "png", "jpg", "jpeg", "webp"] }] });
      if (typeof path !== "string") return;
      importing = true;
      const tips = await api.importBrushes(path);
      adopt(tips);
      if (tips[0]) choose({ type: "tip", id: tips[0].id });
    } catch (error) {
      app.fail(error);
    } finally {
      importing = false;
    }
  }

  function persist(): void {
    void api.saveSettings("brush-presets", JSON.stringify($state.snapshot(presets))).catch((error: unknown) => app.fail(error));
  }

  function savePreset(): void {
    const name = presetName.trim() || `${current} ${Math.round(settings.size)} px`;
    const preset: Preset = { name, kind: kind.type, tip: tip?.name, size: settings.size, hardness: settings.hardness, opacity: settings.opacity, dynamics: { ...$state.snapshot(app.brushDynamics) } };
    presets = [...presets.filter((p) => p.name !== name), preset];
    presetName = "";
    persist();
  }

  function applyPreset(preset: Preset): void {
    const found = preset.kind === "tip" ? app.brushTips.find((t) => t.name === preset.tip) : undefined;
    if (preset.kind === "tip" && !found) {
      app.error = `La pointe « ${preset.tip ?? "?"} » de ce préréglage n'est plus dans la bibliothèque : réimportez son fichier.`;
      return;
    }
    app.brushKind = found ? { type: "tip", id: found.id } : { type: preset.kind === "tip" ? "round" : preset.kind };
    app.brushDynamics = { ...NEUTRAL_DYNAMICS, ...preset.dynamics };
    Object.assign(settings, { size: preset.size, hardness: preset.hardness, opacity: preset.opacity });
  }

  function removePreset(name: string): void {
    presets = presets.filter((p) => p.name !== name);
    persist();
  }

  const percent = (value: number) => `${Math.round(value * 100)} %`;
</script>

{#if !inline}
  <button class="trigger" title={t("Forme et dynamique du pinceau : dans le panneau Propriétés")} onclick={() => (app.leftTab = "properties")}>
    {current}
    <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
  </button>
{/if}

{#snippet tipGrid(tips: BrushTipInfo[])}
  <div class="tips">
    {#each tips as item (item.id)}
      <button title={t("{name} · {width} × {height} px", { name: item.name, width: item.width, height: item.height })} class:on={kind.type === "tip" && kind.id === item.id} onclick={() => choose({ type: "tip", id: item.id })}>
        {#if app.brushThumbnails[item.id]}<img src={app.brushThumbnails[item.id]} alt="" />{/if}
        <span>{t(item.name)}</span>
      </button>
    {/each}
  </div>
{/snippet}

{#snippet slider(label: string, value: number, min: number, max: number, step: number, shown: string, set: (v: number) => void)}
  <label class="slider">
    <span>{label}</span>
    <output>{shown}</output>
    <input type="range" {min} {max} {step} {value} oninput={(event) => set(event.currentTarget.valueAsNumber)} />
  </label>
{/snippet}

{#if inline}
  <div class="panel inline">
    <div class="column">
      <p>{t("Formes")}</p>
      <div class="basics">
        {#each BASICS as basic (basic.id)}
          <button title={t(basic.hint)} class:on={current === basic.label} onclick={() => chooseBasic(basic.id)}>
            <svg viewBox="0 0 24 24" width="26" height="26" aria-hidden="true" class:fill={basic.fill} class:soft={basic.soft}><path d={basic.icon} /></svg>
            {t(basic.label)}
          </button>
        {/each}
      </div>

      <p>{t("Matières")}</p>
      {@render tipGrid(builtin)}

      <p>{t("Importées")}</p>
      {#if imported.length > 0}
        {@render tipGrid(imported)}
      {:else}
        <p class="empty">{t("Aucune pour l'instant.")}</p>
      {/if}
      <button class="wide" disabled={importing} onclick={importFile}>{importing ? "Import…" : t("Importer… (.abr, .gbr, image)")}</button>
    </div>

    <div class="column">
      <p>{t("Dynamique — {current}", { current })}</p>
      {#if kind.type === "round"}
        <p class="empty">{t("Le pinceau rond se règle par sa taille, sa dureté, son opacité et le lissage, dans la barre d'options.")}</p>
      {:else}
        {@render slider("Espacement", app.brushDynamics.spacing, 0, 3, 0.01, app.brushDynamics.spacing > 0 ? percent(app.brushDynamics.spacing) : `Auto${tip ? ` (${percent(tip.spacing)})` : ""}`, (v) => (app.brushDynamics.spacing = v))}
        {@render slider("Dispersion", app.brushDynamics.scatter, 0, 3, 0.01, percent(app.brushDynamics.scatter), (v) => (app.brushDynamics.scatter = v))}
        {#if kind.type === "tip"}
          {@render slider("Angle", app.brushDynamics.angle, -180, 180, 1, `${Math.round(app.brushDynamics.angle)}°`, (v) => (app.brushDynamics.angle = v))}
          {@render slider("Variation de taille", app.brushDynamics.sizeJitter, 0, 1, 0.01, percent(app.brushDynamics.sizeJitter), (v) => (app.brushDynamics.sizeJitter = v))}
          {@render slider("Variation d'angle", app.brushDynamics.angleJitter, 0, 1, 0.01, percent(app.brushDynamics.angleJitter), (v) => (app.brushDynamics.angleJitter = v))}
          <label class="check"><input type="checkbox" bind:checked={app.brushDynamics.followDirection} /> {t("Suivre la direction du trait")}</label>
        {:else if kind.type === "spray"}
          {@render slider("Densité", app.brushDynamics.amount || 1, 0.1, 4, 0.05, percent(app.brushDynamics.amount || 1), (v) => (app.brushDynamics.amount = v))}
        {:else}
          <!-- Les trois réglages du mélange de couleurs de Clip Studio Paint. -->
          {@render slider("Quantité de peinture", app.brushDynamics.paint, 0, 1, 0.01, percent(app.brushDynamics.paint), (v) => (app.brushDynamics.paint = v))}
          {@render slider("Densité de la peinture", app.brushDynamics.amount || 0.7, 0.05, 1, 0.01, percent(app.brushDynamics.amount || 0.7), (v) => (app.brushDynamics.amount = v))}
          {@render slider("Étirement des couleurs", app.brushDynamics.stretch, 0, 0.98, 0.01, percent(app.brushDynamics.stretch), (v) => (app.brushDynamics.stretch = v))}
          <p class="empty">{t("Peinture à 0 % : le pinceau étale et estompe sans rien ajouter. Étirement fort : les couleurs sont traînées loin.")}</p>
        {/if}
        {#if kind.type !== "mixer"}
          {@render slider("Flux", app.brushDynamics.flow || 1, 0.02, 1, 0.01, percent(app.brushDynamics.flow || 1), (v) => (app.brushDynamics.flow = v >= 1 ? 0 : v))}
        {/if}
        <button class="wide" onclick={() => (app.brushDynamics = kind.type === "mixer" ? { ...MIXER_DYNAMICS } : { ...NEUTRAL_DYNAMICS })}>{t("Réinitialiser la dynamique")}</button>
      {/if}

      <p>{t("Mes pinceaux")}</p>
      {#each presets as preset (preset.name)}
        <div class="preset">
          <button class="apply" onclick={() => applyPreset(preset)}>{t(preset.name)}<em>{Math.round(preset.size)} px</em></button>
          <button class="remove" aria-label={t("Supprimer {name}", { name: preset.name })} onclick={() => removePreset(preset.name)}>×</button>
        </div>
      {:else}
        <p class="empty">{t("Enregistrez le pinceau courant pour le retrouver ici.")}</p>
      {/each}
      <div class="save">
        <input type="text" placeholder={t("Nom du pinceau")} bind:value={presetName} onkeydown={(event) => (event.stopPropagation(), event.key === "Enter" && savePreset())} />
        <button onclick={savePreset}>{t("Enregistrer")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* Dans Propriétés : une seule colonne, sans cadre ni ombre, à la largeur du panneau. */
  .panel.inline {
    position: static;
    width: auto;
    max-height: none;
    grid-template-columns: 1fr;
    padding: 12px 14px;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    overflow: visible;
  }
  .panel.inline .tips {
    max-height: 220px;
  }
  .trigger {
    height: 26px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    white-space: nowrap;
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  svg.fill {
    fill: currentColor;
    stroke: none;
  }
  svg.soft {
    filter: blur(2.2px);
  }
  .panel {
    position: fixed;
    z-index: 45;
    width: 640px;
    max-height: calc(100vh - 120px);
    box-sizing: border-box;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 18px;
    padding: 16px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
    overflow-y: auto;
  }
  .column {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  p {
    margin: 6px 0 0;
    color: var(--text-muted);
  }
  p:first-child {
    margin-top: 0;
  }
  .empty {
    margin: 0;
    line-height: 1.5;
  }
  .basics,
  .tips {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .tips {
    max-height: 170px;
    overflow-y: auto;
  }
  .basics button,
  .tips button {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 8px 4px 7px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text-2);
    font-size: 11px;
  }
  .basics button:hover,
  .tips button:hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .basics button.on,
  .tips button.on {
    background: var(--bg-active);
    border-color: rgba(255, 255, 255, 0.22);
    color: #fff;
  }
  .tips img {
    width: 36px;
    height: 36px;
    object-fit: contain;
  }
  .tips span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wide {
    height: 28px;
    flex-shrink: 0;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text);
  }
  .wide:hover {
    border-color: #fff;
  }
  .slider {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px;
    color: var(--text-3);
  }
  .slider input {
    grid-column: 1 / -1;
    width: 100%;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .preset {
    display: flex;
    gap: 4px;
  }
  .apply {
    flex: 1;
    min-width: 0;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text);
    text-align: left;
  }
  .apply:hover {
    border-color: var(--border-strong);
  }
  .apply em {
    color: var(--text-muted);
    font-style: normal;
  }
  .remove {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .remove:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .save {
    display: flex;
    gap: 6px;
  }
  .save input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .save button {
    height: 28px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
</style>
