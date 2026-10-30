<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { hexToRgb, rgbToHex } from "../lib/color";
  import type { CharStyle, TextCaps } from "../lib/ipc";
  import { dependentsOf } from "../lib/text-styles.svelte";
  import { app } from "../state/app.svelte";
  import FontPicker from "./FontPicker.svelte";

  // Fenêtre d'un style de caractère : chaque réglage laissé vide hérite du style « Basé sur » (ou n'est pas imposé).
  let { style, list, onsave, oncancel }: { style: CharStyle; list: readonly CharStyle[]; onsave: (style: CharStyle) => void; oncancel: () => void } = $props();

  // svelte-ignore state_referenced_locally
  let draft = $state<CharStyle>(JSON.parse(JSON.stringify(style)) as CharStyle);
  let box = $state<HTMLDivElement>();
  /** Styles possibles pour « Basé sur » : ni lui-même, ni un style qui en hérite (pas de boucle). */
  const parents = $derived.by(() => {
    const family = dependentsOf(list, style.id);
    return list.filter((s) => !family.has(s.id));
  });
  const SHORTCUTS = [1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => `Ctrl+Alt+${n}`);
  const takenShortcuts = $derived(new Set(list.filter((s) => s.id !== style.id && s.shortcut).map((s) => s.shortcut)));
  const CAPS: { id: TextCaps | null; label: string }[] = [
    { id: null, label: "—" },
    { id: "normal", label: "Aa" },
    { id: "allCaps", label: "AA" },
    { id: "smallCaps", label: "ᴀᴀ" },
  ];

  /** Nombre saisi, ou rien (champ vidé) : le réglage n'est plus imposé. */
  function numberOr(value: string, scale = 1): number | null {
    const n = Number(value.replace(",", "."));
    return value.trim() === "" || !Number.isFinite(n) ? null : n * scale;
  }
  const styles = $derived(app.fonts?.find((f) => f.family === draft.fontFamily)?.styles ?? []);
  const weights = $derived([...new Set([100, 200, 300, 400, 500, 600, 700, 800, 900, ...styles.map((s) => s.weight)])].sort((a, b) => a - b));

  onMount(() => {
    box?.querySelector<HTMLInputElement>("input.name")?.select();
  });

  function save(): void {
    const name = draft.name.trim();
    if (!name) return;
    onsave({ ...draft, name });
  }
</script>

<div class="backdrop" role="presentation" onpointerdown={(event) => event.target === event.currentTarget && oncancel()}>
  <div class="box" role="dialog" aria-modal="true" aria-label={t("Style de caractère")} tabindex="-1" bind:this={box} onkeydown={(event) => (event.key === "Escape" ? (event.stopPropagation(), oncancel()) : event.key === "Enter" && !(event.target instanceof HTMLSelectElement) && (event.preventDefault(), save()))}>
    <header>{t("Style de caractère « {name} »", { name: draft.name || t("Sans nom") })}</header>
    <div class="grid">
      <span>{t("Nom")}</span>
      <input class="name" type="text" bind:value={draft.name} spellcheck="false" />

      <span>{t("Basé sur")}</span>
      <select value={draft.basedOn ?? ""} onchange={(e) => (draft.basedOn = e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}>
        <option value="">{t("Aucun")}</option>
        {#each parents as parent (parent.id)}<option value={parent.id}>{parent.name}</option>{/each}
      </select>

      <span>{t("Police")}</span>
      <div class="pair">
        <div class="grow"><FontPicker value={draft.fontFamily ?? ""} fonts={app.fonts ?? []} onpreview={(family, temporary) => !temporary && (draft.fontFamily = family)} onrestore={() => undefined} ondone={() => undefined} /></div>
        <button type="button" class="clear" title={t("Hériter")} disabled={draft.fontFamily == null} onclick={() => (draft.fontFamily = null)}>✕</button>
      </div>

      <span>{t("Graisse")}</span>
      <div class="pair">
        <select class="grow" value={draft.weight ?? ""} onchange={(e) => (draft.weight = e.currentTarget.value === "" ? null : Number(e.currentTarget.value))}>
          <option value="">—</option>
          {#each weights as w (w)}<option value={w}>{w}</option>{/each}
        </select>
        <label class="inline"><input type="checkbox" checked={draft.italic === true} indeterminate={draft.italic == null} onchange={(e) => (draft.italic = e.currentTarget.checked)} /> {t("Italique")}</label>
      </div>

      <span>{t("Taille (px)")}</span>
      <input type="text" inputmode="decimal" placeholder="—" value={draft.size ?? ""} onchange={(e) => (draft.size = numberOr(e.currentTarget.value))} />

      <span>{t("Interlignage (%)")}</span>
      <input type="text" inputmode="decimal" placeholder="—" value={draft.lineHeight == null ? "" : Math.round(draft.lineHeight * 100)} onchange={(e) => (draft.lineHeight = numberOr(e.currentTarget.value, 0.01))} />

      <span>{t("Approche")}</span>
      <input type="text" inputmode="decimal" placeholder="—" value={draft.tracking ?? ""} onchange={(e) => (draft.tracking = numberOr(e.currentTarget.value))} />

      <span>{t("Casse")}</span>
      <div class="segmented">
        {#each CAPS as option (option.label)}
          <button type="button" class:on={(draft.caps ?? null) === option.id} title={option.id === null ? t("Hériter") : option.id === "normal" ? t("Telle que tapée") : option.id === "allCaps" ? t("Tout en capitales") : t("Petites capitales")} onclick={() => (draft.caps = option.id)}>{option.label}</button>
        {/each}
      </div>

      <span>{t("Couleur")}</span>
      <div class="pair">
        <input type="color" value={draft.color ? rgbToHex(draft.color) : "#000000"} oninput={(e) => (draft.color = hexToRgb(e.currentTarget.value) ?? draft.color)} />
        <span class="muted">{draft.color ? rgbToHex(draft.color) : t("héritée")}</span>
        <button type="button" class="clear" title={t("Hériter")} disabled={draft.color == null} onclick={() => (draft.color = null)}>✕</button>
      </div>

      <span>{t("Raccourci")}</span>
      <select value={draft.shortcut ?? ""} onchange={(e) => (draft.shortcut = e.currentTarget.value || null)}>
        <option value="">{t("Aucun")}</option>
        {#each SHORTCUTS as shortcut (shortcut)}<option value={shortcut} disabled={takenShortcuts.has(shortcut)}>{shortcut}</option>{/each}
      </select>
    </div>
    <p class="note">{t("Les champs laissés vides (—) héritent du style « Basé sur » : changer la police de celui-ci change aussi ce style.")}</p>
    <footer>
      <button type="button" class="ghost" onclick={oncancel}>{t("Annuler")}</button>
      <button type="button" class="pill" disabled={!draft.name.trim()} onclick={save}>{t("Enregistrer")}</button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: grid;
    place-items: center;
    background: rgba(5, 5, 6, 0.55);
  }
  .box {
    width: 380px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
    color: var(--text-2);
  }
  header {
    padding: 12px 14px;
    font-weight: 600;
    color: var(--text);
    border-bottom: 1px solid var(--border);
  }
  .grid {
    display: grid;
    grid-template-columns: 110px 1fr;
    gap: 8px 10px;
    align-items: center;
    padding: 12px 14px;
  }
  .grid > span {
    color: var(--text-3);
  }
  input[type="text"],
  select {
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    min-width: 0;
  }
  .pair {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .inline {
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .clear {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
  }
  .clear:disabled {
    opacity: 0.3;
  }
  input[type="color"] {
    width: 30px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border);
    background: none;
  }
  .muted {
    color: var(--text-muted);
    flex: 1;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-well);
    border: 1px solid var(--border);
    width: max-content;
  }
  .segmented button {
    min-width: 34px;
    height: 24px;
    border-radius: var(--radius-s);
    color: var(--text-muted);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .note {
    margin: 0;
    padding: 0 14px 10px;
    color: var(--text-muted);
    font-size: 11px;
    line-height: 1.45;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
  }
  .ghost,
  .pill {
    height: 30px;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .pill:disabled {
    opacity: 0.5;
  }
</style>
