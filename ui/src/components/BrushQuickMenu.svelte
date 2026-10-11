<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onDestroy, onMount } from "svelte";
  import { dismissGuard } from "../lib/menu-dismiss";
  import { type BrushKind, MIXER_DYNAMICS, NEUTRAL_DYNAMICS, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Clic droit sur l'image avec le pinceau (ou la gomme) en main, comme dans Photoshop : taille, dureté
  // et choix de la forme, sous le pointeur. Un clic ailleurs ou Échap le referme.
  let { x, y }: { x: number; y: number } = $props();

  let panel = $state<HTMLDivElement>();
  const settings = $derived(app.brushes[app.paintTool ?? "brush"]);
  const kind = $derived(app.brushKind);
  const BASICS = [
    { id: "hard", label: "Net" },
    { id: "soft", label: "Flou" },
    { id: "spray", label: "Spray" },
    { id: "mixer", label: "Mélangeur" },
  ] as const;
  /** Vignette du spray : un semis de points fixe (même dessin à chaque ouverture). */
  const SPRAY_DOTS: [number, number, number][] = Array.from({ length: 34 }, (_, i) => {
    const angle = i * 2.399963; // angle d'or : répartition régulière sans motif visible
    const distance = 12 * Math.sqrt((i + 0.5) / 34);
    return [15 + Math.cos(angle) * distance, 15 + Math.sin(angle) * distance, i % 3 === 0 ? 1.3 : 0.9];
  });
  const activeBasic = $derived(kind.type === "tip" ? null : kind.type === "spray" ? "spray" : kind.type === "mixer" ? "mixer" : settings.hardness >= 0.5 ? "hard" : "soft");
  // Le menu reste dans la fenêtre.
  const left = $derived(Math.max(8, Math.min(x, window.innerWidth - 348)));
  const top = $derived(Math.max(8, Math.min(y, window.innerHeight - 420)));

  // Le clic droit qui ouvre ce menu ne doit pas le refermer (appui livré après l'ouverture sous CEF/X11).
  const guard = dismissGuard();
  onDestroy(() => guard.dispose());

  onMount(() => {
    // Vignettes manquantes (le panneau Propriétés n'a peut-être jamais été ouvert).
    for (const tip of app.brushTips) {
      if (app.brushThumbnails[tip.id]) continue;
      api.brushThumbnail(tip.id).then(
        (png) => (app.brushThumbnails[tip.id] = URL.createObjectURL(new Blob([png], { type: "image/png" }))),
        () => undefined,
      );
    }
  });

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
</script>

<svelte:window
  onpointerdowncapture={(event) => !(event.target instanceof Node && panel?.contains(event.target)) && guard.pointerDownCloses(event) && (app.brushMenu = null)}
  onkeydown={(event) => event.key === "Escape" && (app.brushMenu = null)}
/>

<div class="menu" role="dialog" tabindex="-1" aria-label={t("Pinceau")} bind:this={panel} style:left="{left}px" style:top="{top}px" oncontextmenu={(event) => event.preventDefault()}>
  <label>
    <span>{t("Taille")}</span>
    <input type="number" min="1" max="2500" bind:value={settings.size} onkeydown={(event) => event.stopPropagation()} /> px
    <input type="range" min="1" max="500" bind:value={settings.size} />
  </label>
  <label>
    <span>{t("Dureté")}</span>
    <output>{Math.round(settings.hardness * 100)} %</output>
    <input type="range" min="0" max="1" step="0.01" bind:value={settings.hardness} />
  </label>
  <!-- Toutes les formes présentées de la même façon : une vignette et un nom. Les quatre formes de base ont
       une vignette dessinée ici ; les pointes (fournies ou importées) ont celle calculée par le moteur. -->
  <div class="tips">
    {#each BASICS as basic (basic.id)}
      <button title={t(basic.label)} class:on={activeBasic === basic.id} onclick={() => chooseBasic(basic.id)}>
        <svg viewBox="0 0 30 30" aria-hidden="true">
          {#if basic.id === "hard"}
            <circle cx="15" cy="15" r="11" fill="#fff" />
          {:else if basic.id === "soft"}
            <defs>
              <radialGradient id="quick-soft">
                <stop offset="0" stop-color="#fff" stop-opacity="0.95" />
                <stop offset="0.45" stop-color="#fff" stop-opacity="0.55" />
                <stop offset="1" stop-color="#fff" stop-opacity="0" />
              </radialGradient>
            </defs>
            <circle cx="15" cy="15" r="14" fill="url(#quick-soft)" />
          {:else if basic.id === "spray"}
            {#each SPRAY_DOTS as [x, y, r], i (i)}<circle cx={x} cy={y} r={r} fill="#fff" />{/each}
          {:else}
            <path d="M4 20c4-9 9-12 14-9s7 3 8-1" fill="none" stroke="#fff" stroke-width="5" stroke-linecap="round" opacity="0.55" />
            <path d="M6 22c4-7 8-9 12-6" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round" />
          {/if}
        </svg>
        <span>{t(basic.label)}</span>
      </button>
    {/each}
    {#each app.brushTips as tip (tip.id)}
      <button title={t("{name} · {width} × {height} px", { name: tip.name, width: tip.width, height: tip.height })} class:on={kind.type === "tip" && kind.id === tip.id} onclick={() => choose({ type: "tip", id: tip.id })}>
        {#if app.brushThumbnails[tip.id]}<img src={app.brushThumbnails[tip.id]} alt="" />{/if}
        <span>{t(tip.name)}</span>
      </button>
    {/each}
  </div>
  <button class="more" onclick={() => ((app.leftTab = "properties"), (app.brushMenu = null))}>{t("Dynamique, import et préréglages : panneau Propriétés")}</button>
</div>

<style>
  .menu {
    position: fixed;
    z-index: 60;
    width: 340px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow-menu);
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto auto;
    align-items: center;
    gap: 4px 6px;
    color: var(--text-3);
  }
  label input[type="range"] {
    grid-column: 1 / -1;
    width: 100%;
  }
  label input[type="number"] {
    width: 64px;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    text-align: right;
    user-select: text;
  }
  output {
    grid-column: 2 / -1;
    justify-self: end;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .tips button {
    min-width: 0;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    color: var(--text-2);
    font-size: 11px;
  }
  .tips button.on {
    background: var(--bg-active);
    border-color: rgba(255, 255, 255, 0.22);
    color: #fff;
  }
  .tips {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 4px;
    max-height: 190px;
    overflow-y: auto;
  }
  .tips button {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 6px 2px 5px;
  }
  .tips svg {
    width: 30px;
    height: 30px;
  }
  .tips img {
    width: 30px;
    height: 30px;
    object-fit: contain;
  }
  .tips span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    height: 26px;
    border-radius: var(--radius-pill);
    color: var(--text-muted);
  }
  .more:hover {
    color: var(--text);
  }
</style>
