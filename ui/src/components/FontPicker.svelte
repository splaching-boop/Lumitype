<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { type FontFamily, api } from "../lib/ipc";
  import { ensureBundledFont } from "../lib/fonts";

  // Champ de police façon Photoshop : on tape pour chercher, les flèches parcourent la liste et
  // chaque police survolée ou parcourue s'applique tout de suite au texte. Entrée valide,
  // Échap remet la police d'avant.
  let {
    value,
    fonts,
    onpreview,
    onrestore,
    ondone,
  }: {
    /** `null` : plusieurs polices dans la plage visée, le champ affiche « ? ». */
    value: string | null;
    fonts: FontFamily[];
    /**
     * Applique une police au texte. `temporary` : simple survol de la liste, à défaire par
     * `onrestore` si l'on quitte la liste sans choisir ; sinon c'est un choix (clic, flèches, frappe).
     */
    onpreview: (family: string, temporary: boolean) => void;
    /** Remet le texte comme avant le survol. */
    onrestore: () => void;
    /** Validation ou abandon : le champ rend la main (à la saisie du texte, par exemple). */
    ondone: () => void;
  } = $props();

  let input = $state<HTMLInputElement>();
  let list = $state<HTMLDivElement>();
  let open = $state(false);
  /** Texte tapé ; `null` tant que l'utilisateur ne cherche pas (le champ affiche alors la police courante). */
  let query = $state<string | null>(null);
  let original: string | null = "";

  const label = (family: string | null) => (family === null ? "?" : family || "Par défaut");
  const bundled = $derived(new Set(fonts.filter((f) => f.bundled).map((f) => f.family)));
  /** Noms dans l'ordre d'affichage : « Par défaut », les polices incluses avec Lumitype, puis celles du système. */
  const ordered = $derived(["", ...fonts.filter((f) => f.bundled).map((f) => f.family), ...fonts.filter((f) => !f.bundled).map((f) => f.family)]);
  const matches = $derived.by(() => {
    const needle = (query ?? "").trim().toLowerCase();
    if (!needle) return ordered;
    // Les polices qui COMMENCENT par la recherche passent avant celles qui la contiennent.
    const starts = ordered.filter((n) => label(n).toLowerCase().startsWith(needle));
    const contains = ordered.filter((n) => !label(n).toLowerCase().startsWith(needle) && label(n).toLowerCase().includes(needle));
    return [...starts, ...contains];
  });
  /** Titre de section à afficher au-dessus de cette ligne, hors recherche : « Incluses » ou « Système ». */
  function heading(family: string, index: number): string | null {
    if (query?.trim()) return null;
    if (family === "") return null;
    const previous = matches[index - 1];
    const section = bundled.has(family) ? "Incluses" : "Système";
    const previousSection = previous === undefined || previous === "" ? null : bundled.has(previous) ? "Incluses" : "Système";
    return section === previousSection ? null : section;
  }

  /** Police incluse : son fichier est chargé dans la webview (une fois) dès que sa ligne devient visible. */
  function sampleFont(node: HTMLElement, family: string) {
    if (!bundled.has(family)) return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        void ensureBundledFont(family, () => api.bundledFontFile(family));
        observer.disconnect();
      }
    });
    observer.observe(node);
    return { destroy: () => observer.disconnect() };
  }

  /** Position de la liste à l'écran. Elle est fixe : sortie du panneau, elle ne peut ni être rognée par lui ni le faire défiler. */
  let place = $state({ left: 0, top: 0, height: 420 });

  function show(): void {
    if (open) return;
    original = value;
    const rect = input?.getBoundingClientRect();
    if (rect) place = { left: rect.left, top: rect.bottom + 4, height: Math.max(160, Math.min(420, window.innerHeight - rect.bottom - 16)) };
    open = true;
    queueMicrotask(scrollToCurrent);
  }

  function scrollToCurrent(): void {
    list?.querySelector(".current")?.scrollIntoView({ block: "nearest" });
  }

  /** Vrai tant que la police affichée ne vient que d'un survol. */
  let hovering = false;

  function close(restore: boolean): void {
    if (hovering) onrestore();
    else if (restore && value !== original && original !== null) onpreview(original, false);
    hovering = false;
    open = false;
    query = null;
  }

  /**
   * Faire défiler la liste au clavier la déplace sous une souris immobile, et le navigateur signale alors
   * l'entrée du pointeur dans la ligne qui passe dessous : ce survol-là est ignoré. Seul un vrai
   * mouvement de la souris (coordonnées différentes) rend la main au survol.
   */
  let pointerAt: { x: number; y: number } | null = null;
  let pointerMoved = true;

  function onListPointerMove(event: PointerEvent): void {
    if (pointerAt === null || pointerAt.x !== event.clientX || pointerAt.y !== event.clientY) pointerMoved = true;
    pointerAt = { x: event.clientX, y: event.clientY };
  }

  function hover(family: string): void {
    if (!pointerMoved || family === value) return;
    hovering = true;
    onpreview(family, true);
  }

  function leaveList(): void {
    if (!hovering) return;
    hovering = false;
    onrestore();
  }

  function choose(family: string): void {
    hovering = false;
    onpreview(family, false);
  }

  function step(direction: 1 | -1): void {
    const pool = matches.length > 0 ? matches : [""];
    const index = value === null ? -1 : pool.indexOf(value);
    const next = pool[index < 0 ? 0 : Math.min(pool.length - 1, Math.max(0, index + direction))];
    if (next !== undefined && next !== value) choose(next);
    pointerMoved = false;
    queueMicrotask(scrollToCurrent);
  }

  function onKeyDown(event: KeyboardEvent): void {
    event.stopPropagation(); // ni raccourcis de l'application, ni déplacement de calque aux flèches
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      show();
      step(event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Enter") {
      // Recherche en cours : la première police trouvée est retenue, comme dans Photoshop.
      const first = matches[0];
      if (query !== null && first !== undefined && (value === null || !matches.includes(value))) choose(first);
      close(false);
      ondone();
    } else if (event.key === "Escape") {
      close(true);
      ondone();
    } else {
      return;
    }
    event.preventDefault();
  }

  function onInput(): void {
    query = input?.value ?? "";
    show();
    // La meilleure correspondance s'applique pendant la frappe : on voit tout de suite le résultat.
    const first = matches[0];
    if (query.trim() !== "" && first !== undefined && first !== value) choose(first);
  }
</script>

<div class="picker">
  <input
    bind:this={input}
    type="text"
    role="combobox"
    aria-label={t("Police")}
    aria-expanded={open}
    aria-controls="font-list"
    aria-autocomplete="list"
    spellcheck="false"
    autocomplete="off"
    value={query ?? (open ? "" : label(value))}
    placeholder={label(value)}
    onfocus={show}
    onblur={(event) => {
      // Clic dans la liste (sa barre de défilement, notamment) : le champ garde la main, la liste reste ouverte.
      if (list && event.relatedTarget instanceof Node && list.contains(event.relatedTarget)) {
        input?.focus();
        return;
      }
      close(false);
    }}
    onselect={(event) => {
      // Le texte du champ ne se sélectionne pas (ni double-clic, ni glisser, ni Ctrl+A) : rien à glisser par mégarde.
      const field = event.currentTarget;
      const end = field.value.length;
      if (field.selectionStart !== end || field.selectionEnd !== end) field.setSelectionRange(end, end);
    }}
    ondragstart={(event) => event.preventDefault()}
    oninput={onInput}
    onkeydown={onKeyDown}
  />
  {#if open}
    <div class="list" id="font-list" role="listbox" tabindex="-1" bind:this={list} onmousedown={(event) => event.preventDefault()} onpointerleave={leaveList} onpointermove={onListPointerMove} style:left="{place.left}px" style:top="{place.top}px" style:max-height="{place.height}px">
      {#each matches as family, index (family)}
        {@const title = heading(family, index)}
        {#if title}<h4>{t(title)}</h4>{/if}
        <!-- `pointerdown` et non `click` : le choix doit passer avant la perte de focus du champ. -->
        <button
          use:sampleFont={family}
          role="option"
          tabindex="-1"
          aria-selected={family === value}
          class:current={family === value}
          onpointerenter={() => hover(family)}
          onpointerdown={(event) => {
            event.preventDefault();
            choose(family);
            close(false);
            ondone();
          }}
        >
          <span class="name">{label(family)}</span>
          <!-- Échantillon dans la police elle-même : les polices du système sont accessibles à la webview par leur nom,
               les incluses sont chargées depuis leur fichier (voir sampleFont). -->
          <span class="sample" style:font-family={family ? `"${family}"` : "inherit"}>{t("Lorem")}</span>
        </button>
      {:else}
        <p>{t("Aucune police ne correspond")}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
  }
  input {
    width: 100%;
    height: 26px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
    user-select: none;
  }
  input::placeholder {
    /* Ouverte, la liste vide le champ pour la recherche : la police courante y reste lisible, en clair. */
    color: var(--text);
  }
  .list {
    position: fixed;
    z-index: 50;
    width: 320px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 6px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  button {
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    text-align: left;
    color: var(--text-2);
    /* Des centaines de polices : seules les lignes visibles sont mises en page et chargent leur police. */
    content-visibility: auto;
    contain-intrinsic-size: auto 34px;
  }
  h4 {
    margin: 6px 0 2px;
    padding: 0 10px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  h4:first-child {
    margin-top: 2px;
  }
  .name,
  .sample {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    width: 170px;
    flex-shrink: 0;
  }
  .sample {
    flex: 1;
    font-size: 17px;
    color: var(--text);
  }
  button:hover {
    background: var(--bg-hover);
  }
  button.current {
    background: var(--bg-active);
    color: #fff;
  }
  p {
    margin: 0;
    padding: 8px 10px;
    color: var(--text-muted);
  }
</style>
