<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api, type CharStyle, type TextData } from "../lib/ipc";
  import type { RangeStyle } from "../lib/text-runs";
  import {
    type LibraryStyle,
    fromLibrary,
    libraryNow,
    loadLibrary,
    nextStyleId,
    overridden,
    publishToLibrary,
    removeFromLibrary,
    resolveStyle,
    saveStyles,
    styleFromText,
    updatesAvailable,
    usageOf,
  } from "../lib/text-styles.svelte";
  import { app } from "../state/app.svelte";
  import CharStyleDialog from "./CharStyleDialog.svelte";
  import ContextMenu, { type MenuEntry } from "./ContextMenu.svelte";

  // Styles de caractère, dans les propriétés du texte : la liste du document (aperçu, nombre de textes qui
  // l'utilisent), le style de la sélection (« + » s'il a été retouché), et la bibliothèque partagée entre projets.
  // Modifier un style déjà utilisé demande si les textes en place doivent suivre.
  let {
    text,
    shown,
    wholeLayer,
    onapply,
  }: {
    text: TextData;
    shown: RangeStyle;
    /** Vrai quand rien n'est sélectionné dans le texte : le style vise le calque entier. */
    wholeLayer: boolean;
    /** Applique le style `id` (ou aucun) à la sélection ou au calque. */
    onapply: (id: number | null) => void;
  } = $props();

  const list = $derived(app.active?.info.textStyles ?? []);
  const layers = $derived(app.active?.info.layers ?? []);
  const currentId = $derived(shown.style);
  const current = $derived(typeof currentId === "number" ? (list.find((s) => s.id === currentId) ?? null) : null);
  const plus = $derived(current ? overridden(shown, text, resolveStyle(list, current.id), wholeLayer) : false);
  let editor = $state<{ style: CharStyle; isNew: boolean } | null>(null);
  /** Modification en attente : faut-il mettre à jour les textes en place ? */
  let pending = $state<{ list: CharStyle[]; id: number; name: string; count: number; label: string; share: boolean } | null>(null);
  let menu = $state<{ x: number; y: number } | null>(null);
  let entries = $state<LibraryStyle[]>([]);

  onMount(() => {
    void loadLibrary().then((loaded) => (entries = loaded));
  });

  /** Entrées de la bibliothèque absentes du document : proposées en dessous, un clic les importe et les applique. */
  const fromKit = $derived(entries.filter((e) => !list.some((s) => s.library?.uid === e.uid)));
  const updates = $derived(updatesAvailable(list, entries));

  function previewStyle(style: CharStyle): string {
    const r = resolveStyle(list, style.id) ?? {};
    const css = [`font-family:'${r.fontFamily ?? text.fontFamily}'`, `font-weight:${r.weight ?? 400}`, r.italic ? "font-style:italic" : "", r.color ? `color:rgb(${r.color.join(",")})` : ""];
    return css.filter(Boolean).join(";");
  }
  const caseOf = (style: CharStyle) => ((resolveStyle(list, style.id)?.caps ?? "normal") === "allCaps" ? "AA" : "Aa");

  /**
   * Enregistre `next` (liste entière). `id` : style modifié ; s'il est déjà utilisé, on demande si les textes en place
   * suivent. `share` : publier ensuite dans la bibliothèque.
   */
  function commit(next: CharStyle[], id: number, label: string, share = false): void {
    const count = usageOf(layers, list, id);
    const name = next.find((s) => s.id === id)?.name ?? "";
    if (count > 0) pending = { list: next, id, name, count, label, share };
    else void finish(next, id, false, label, share);
  }

  async function finish(next: CharStyle[], id: number, update: boolean, label: string, share: boolean): Promise<void> {
    pending = null;
    let saved = next;
    const style = next.find((s) => s.id === id);
    if (style && (share || style.library)) {
      // Style partagé : la bibliothèque reçoit la nouvelle version (les autres projets verront la mise à jour).
      const link = await publishToLibrary(next, style);
      saved = next.map((s) => (s.id === id ? { ...s, library: link } : s));
      entries = libraryNow();
    }
    await saveStyles(saved, update ? [id] : [], label);
    // Saisie en cours sur un texte mis à jour : ses portions sont relues depuis le moteur.
    const editing = app.textEdit;
    const layer = app.active?.info.layers.find((l) => l.id === editing?.layerId);
    if (update && editing && layer?.text) editing.runs = structuredClone($state.snapshot(layer.text.runs ?? []));
  }

  function create(): void {
    const id = nextStyleId(list);
    const style = styleFromText(id, t("Style {n}", { n: list.length + 1 }), text, shown);
    editor = { style, isNew: true };
  }

  function saveEdited(style: CharStyle): void {
    const isNew = editor?.isNew ?? false;
    editor = null;
    if (isNew) {
      void finish([...list, style], style.id, false, "Nouveau style de caractère", false).then(() => onapply(style.id));
      return;
    }
    commit(
      list.map((s) => (s.id === style.id ? style : s)),
      style.id,
      "Modifier le style de caractère",
    );
  }

  /** Redéfinir : le style reprend l'aspect du texte sélectionné. */
  function redefine(): void {
    if (!current) return;
    const fresh = styleFromText(current.id, current.name, text, shown);
    const next = { ...current, ...fresh, basedOn: current.basedOn, shortcut: current.shortcut, library: current.library, lineHeight: wholeLayer ? text.lineHeight : current.lineHeight };
    commit(
      list.map((s) => (s.id === current.id ? next : s)),
      current.id,
      "Redéfinir le style de caractère",
    );
  }

  function remove(style: CharStyle): void {
    // Les textes gardent leur aspect ; le moteur retire seulement le lien vers le style.
    const next = list.filter((s) => s.id !== style.id).map((s) => (s.basedOn === style.id ? { ...s, basedOn: style.basedOn ?? null } : s));
    void saveStyles(next, [], "Supprimer le style de caractère");
  }

  function duplicate(style: CharStyle): void {
    const copy: CharStyle = { ...structuredClone($state.snapshot(style)), id: nextStyleId(list), name: t("{name} copie", { name: style.name }), library: null, shortcut: null };
    void saveStyles([...list, copy], [], "Dupliquer le style de caractère");
  }

  async function toggleShare(style: CharStyle): Promise<void> {
    if (style.library) {
      await removeFromLibrary(style.library.uid);
      entries = libraryNow();
      await saveStyles(
        list.map((s) => (s.id === style.id ? { ...s, library: null } : s)),
        [],
        "Ne plus partager le style",
      );
    } else await finish(list, style.id, false, "Partager le style", true);
  }

  async function importKit(entry: LibraryStyle): Promise<void> {
    const style = fromLibrary(entry, nextStyleId(list));
    await saveStyles([...list, style], [], "Importer un style de la bibliothèque");
    onapply(style.id);
  }

  /** Mises à jour de la bibliothèque : le style du projet reprend l'entrée ; on demande pour les textes en place. */
  function takeUpdate(style: CharStyle, entry: LibraryStyle): void {
    const updated: CharStyle = { ...fromLibrary(entry, style.id), basedOn: null, shortcut: style.shortcut ?? entry.shortcut ?? null };
    commit(
      list.map((s) => (s.id === style.id ? updated : s)),
      style.id,
      "Mettre à jour le style depuis la bibliothèque",
    );
  }

  async function importFromProject(): Promise<void> {
    const path = await open({ multiple: false, filters: [{ name: t("Projet Lumitype"), extensions: ["graf"] }] });
    if (typeof path !== "string") return;
    try {
      const found = await api.projectTextStyles(path);
      if (found.length === 0) return app.fail(t("Ce projet n'a aucun style de caractère."));
      // Identifiants renumérotés ; « Basé sur » suit la renumérotation ; un nom déjà pris est remplacé.
      let id = nextStyleId(list);
      const map = new Map(found.map((s) => [s.id, id++]));
      const imported = found.map((s) => ({ ...s, id: map.get(s.id)!, basedOn: s.basedOn != null ? (map.get(s.basedOn) ?? null) : null }));
      const names = new Set(imported.map((s) => s.name));
      void saveStyles([...list.filter((s) => !names.has(s.name)), ...imported], [], "Importer des styles de caractère");
    } catch (error) {
      app.fail(error);
    }
  }

  function moreEntries(): MenuEntry[] {
    return [
      { label: t("Modifier le style…"), disabled: !current, run: () => current && (editor = { style: current, isNew: false }) },
      { label: t("Dupliquer le style"), disabled: !current, run: () => current && duplicate(current) },
      { label: t("Supprimer le style"), disabled: !current, run: () => current && remove(current) },
      "separator",
      { label: current?.library ? t("Ne plus partager avec le kit de marque") : t("Partager avec le kit de marque"), disabled: !current, run: () => current && void toggleShare(current) },
      { label: t("Importer d'un projet…"), run: () => void importFromProject() },
    ];
  }

  /** Raccourcis Ctrl+Alt+1…9 des styles : appliqués à la sélection (ou au calque). */
  function onKey(event: KeyboardEvent): void {
    if (!(event.ctrlKey && event.altKey) || !/^Digit[1-9]$/.test(event.code)) return;
    const shortcut = `Ctrl+Alt+${event.code.slice(5)}`;
    const style = list.find((s) => s.shortcut === shortcut);
    if (!style) return;
    event.preventDefault();
    event.stopPropagation();
    onapply(style.id);
  }
</script>

<svelte:window onkeydowncapture={onKey} />

<section>
  <h4>{t("Styles de caractère")}</h4>
  {#if updates.length > 0}
    <div class="update">
      <span>{updates.length === 1 ? t("« {name} » a été modifié dans le kit de marque.", { name: updates[0]!.style.name }) : t("{n} styles ont été modifiés dans le kit de marque.", { n: updates.length })}</span>
      <button type="button" onclick={() => updates[0] && takeUpdate(updates[0].style, updates[0].entry)}>{t("Mettre à jour")}</button>
    </div>
  {/if}
  <div class="styles" role="listbox" aria-label={t("Styles de caractère")}>
    <button type="button" class="st" class:on={currentId === undefined} role="option" aria-selected={currentId === undefined} onclick={() => onapply(null)}>
      <span class="pv none">Aa</span><span class="n">{t("Aucun style")}</span>
    </button>
    {#each list as style (style.id)}
      <button type="button" class="st" class:on={currentId === style.id} role="option" aria-selected={currentId === style.id} title={style.shortcut ?? ""} onclick={() => onapply(style.id)} ondblclick={() => (editor = { style, isNew: false })}>
        <span class="pv" style={previewStyle(style)}>{caseOf(style)}</span>
        <span class="n">{style.name}{#if currentId === style.id && plus}<b class="mod" title={t("Retouché par rapport au style")}> +</b>{/if}{#if style.library}<span class="kit" title={t("Partagé avec le kit de marque")}> ◆</span>{/if}</span>
        <span class="m">{resolveStyle(list, style.id)?.size ? `${Math.round(resolveStyle(list, style.id)?.size ?? 0)} px · ` : ""}{usageOf(layers, list, style.id)}</span>
      </button>
    {/each}
    {#if fromKit.length > 0}
      <div class="sub">{t("Kit de marque")}</div>
      {#each fromKit as entry (entry.uid)}
        <button type="button" class="st kitrow" title={t("Importer dans le projet et appliquer")} onclick={() => void importKit(entry)}>
          <span class="pv" style="font-family:'{entry.fontFamily ?? text.fontFamily}';font-weight:{entry.weight ?? 400}{entry.color ? `;color:rgb(${entry.color.join(',')})` : ''}">{entry.caps === "allCaps" ? "AA" : "Aa"}</span>
          <span class="n">{entry.name}</span><span class="m">＋</span>
        </button>
      {/each}
    {/if}
  </div>
  <div class="icons">
    <button type="button" onclick={create}>＋ {t("Nouveau")}</button>
    <button type="button" disabled={!current || !plus} title={t("Le style reprend l'aspect de la sélection")} onclick={redefine}>{t("Redéfinir")}</button>
    <button type="button" disabled={!current || !plus} title={t("La sélection reprend son style")} onclick={() => current && onapply(current.id)}>{t("Effacer +")}</button>
    <button type="button" class="more" aria-label={t("Plus d'options")} onclick={(event) => { const r = event.currentTarget.getBoundingClientRect(); menu = { x: r.left, y: r.bottom + 4 }; }}>⋯</button>
  </div>
</section>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} entries={moreEntries()} onclose={() => (menu = null)} />
{/if}

{#if editor}
  <CharStyleDialog style={editor.style} {list} onsave={saveEdited} oncancel={() => (editor = null)} />
{/if}

{#if pending}
  {@const p = pending}
  <div class="backdrop" role="presentation">
    <div class="ask" role="alertdialog" aria-modal="true" aria-label={t("Mettre à jour les textes")}>
      <h3>{t("Mettre à jour les textes en place ?")}</h3>
      <p>{p.count === 1 ? t("Un texte utilise « {name} ».", { name: p.name }) : t("{n} textes utilisent « {name} ».", { n: p.count, name: p.name })} {t("Ils peuvent prendre le nouvel aspect du style, ou garder celui qu'ils ont (ils seront alors marqués « + »).")}</p>
      <div class="buttons">
        <button type="button" class="ghost" onclick={() => (pending = null)}>{t("Annuler")}</button>
        <button type="button" class="ghost" onclick={() => void finish(p.list, p.id, false, p.label, p.share)}>{t("Garder tels quels")}</button>
        <button type="button" class="pill" onclick={() => void finish(p.list, p.id, true, p.label, p.share)}>{t("Mettre à jour les textes")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
  }
  h4 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .styles {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg-field);
    max-height: 220px;
    overflow-y: auto;
  }
  .st {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 30px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
    text-align: left;
    color: var(--text-2);
  }
  .st:last-child {
    border-bottom: none;
  }
  .st:hover {
    background: var(--bg-hover);
  }
  .st.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .pv {
    width: 30px;
    text-align: center;
    font-size: 14px;
    color: var(--text);
    flex-shrink: 0;
  }
  .pv.none {
    font-style: italic;
    color: var(--text-muted);
  }
  .n {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .m {
    color: var(--text-muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .mod {
    color: var(--accent, #e5343e);
  }
  .kit {
    color: var(--text-muted);
    font-size: 10px;
  }
  .sub {
    padding: 4px 10px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }
  .kitrow .n {
    color: var(--text-3);
  }
  .icons {
    display: flex;
    gap: 4px;
  }
  .icons button {
    flex: 1;
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
    white-space: nowrap;
    font-size: 12px;
  }
  .icons button.more {
    flex: 0 0 30px;
  }
  .icons button:disabled {
    opacity: 0.4;
  }
  .update {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-m);
    border: 1px solid color-mix(in srgb, var(--accent) 45%, transparent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--text-2);
    font-size: 12px;
    line-height: 1.35;
  }
  .update button {
    flex-shrink: 0;
    padding: 3px 8px;
    border-radius: var(--radius-s);
    background: var(--pill-bg);
    color: var(--pill-text);
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 75;
    display: grid;
    place-items: center;
    background: rgba(5, 5, 6, 0.55);
  }
  .ask {
    width: 420px;
    padding: 16px 18px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.6);
  }
  .ask h3 {
    margin: 0 0 8px;
    font-size: 14px;
    color: var(--text);
  }
  .ask p {
    margin: 0 0 14px;
    color: var(--text-2);
    line-height: 1.5;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .ghost,
  .pill {
    height: 30px;
    padding: 0 14px;
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
</style>
