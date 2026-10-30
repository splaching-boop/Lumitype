<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { IDENTITY, curveAt, normalized } from "../lib/curves";
  import type { CurvePoint } from "../lib/ipc";

  // Éditeur de courbe du réglage Courbes, comme celui de Photoshop : un carré (entrée en abscisse, sortie en
  // ordonnée, 0 à 255). Cliquer ajoute un point et le saisit ; glisser un point le déplace ; le tirer hors du
  // carré, ou Suppr, le retire (il en reste toujours deux). Entrée et Sortie du point choisi se saisissent aussi.
  let {
    points,
    color,
    ghosts = [],
    onchange,
    onsettle,
  }: {
    points: CurvePoint[];
    /** Couleur de la courbe (blanc pour la composite RVB, sinon celle de la couche). */
    color: string;
    /** Courbes des autres couches, dessinées en léger. */
    ghosts?: { points: CurvePoint[]; color: string }[];
    onchange: (points: CurvePoint[]) => void;
    /** Fin d'un geste (relâchement, saisie validée) : un état d'historique. */
    onsettle: () => void;
  } = $props();

  /** Distance (pixels d'écran) en deçà de laquelle un clic saisit un point existant. */
  const GRAB = 9;
  /** Au-delà de cette distance hors du carré, un point tiré est retiré. */
  const REMOVE = 24;

  let svg = $state<SVGSVGElement>();
  let selected = $state<number | null>(null);
  /** Geste en cours : le point saisi, les points au début du geste (le point compris), le pointeur. */
  let drag: { index: number; base: CurvePoint[]; removing: boolean; pointer: number } | null = null;
  /** Point retiré en cours de geste : masqué tant que le pointeur reste dehors. */
  let removing = $state(false);

  const current = $derived(points.length >= 2 ? normalized(points) : IDENTITY);
  const active = $derived(selected !== null && selected < current.length ? current[selected] : null);

  function pathOf(pts: readonly CurvePoint[]): string {
    let d = "";
    for (let x = 0; x <= 255; x += 1.5) d += `${x === 0 ? "M" : "L"}${x.toFixed(1)} ${(255 - curveAt(pts, x)).toFixed(2)}`;
    return `${d}L255 ${(255 - curveAt(pts, 255)).toFixed(2)}`;
  }

  /** Position du pointeur en valeurs de courbe (0 à 255), et sa distance hors du carré en pixels d'écran. */
  function locate(event: PointerEvent): { x: number; y: number; outside: number } {
    const rect = svg!.getBoundingClientRect();
    const px = event.clientX - rect.left;
    const py = event.clientY - rect.top;
    const outside = Math.max(-px, px - rect.width, -py, py - rect.height, 0);
    const x = Math.min(255, Math.max(0, (px / rect.width) * 255));
    const y = Math.min(255, Math.max(0, 255 - (py / rect.height) * 255));
    return { x: Math.round(x), y: Math.round(y), outside };
  }

  function onDown(event: PointerEvent): void {
    if (event.button !== 0 || !svg) return;
    event.preventDefault();
    svg.focus();
    const rect = svg.getBoundingClientRect();
    const scale = rect.width / 255;
    const { x, y } = locate(event);
    let index = current.findIndex(([px, py]) => Math.hypot((px - x) * scale, (py - y) * scale) <= GRAB);
    let next = current;
    if (index < 0) {
      // Nouveau point à l'endroit cliqué, inséré à sa place (une entrée déjà prise est refusée).
      if (current.some(([px]) => Math.abs(px - x) < 1)) return;
      next = [...current, [x, y] as CurvePoint].sort((a, b) => a[0] - b[0]);
      index = next.findIndex(([px, py]) => px === x && py === y);
      onchange(next);
    }
    selected = index;
    drag = { index, base: next, removing: false, pointer: event.pointerId };
    svg.setPointerCapture(event.pointerId);
  }

  function onMove(event: PointerEvent): void {
    if (!drag || drag.pointer !== event.pointerId) return;
    const { x, y, outside } = locate(event);
    const pts = drag.base;
    const index = drag.index;
    // Hors du carré, assez loin : le point sera retiré (il en reste toujours deux).
    drag.removing = outside > REMOVE && pts.length > 2;
    removing = drag.removing;
    if (drag.removing) {
      onchange(pts.filter((_, i) => i !== index));
      return;
    }
    // Le point reste entre ses voisins : l'ordre des entrées ne change jamais en cours de geste.
    const low = index > 0 ? pts[index - 1]![0] + 1 : 0;
    const high = index < pts.length - 1 ? pts[index + 1]![0] - 1 : 255;
    onchange(pts.map((p, i): CurvePoint => (i === index ? [Math.min(high, Math.max(low, x)), y] : p)));
  }

  /** Le point retiré pendant le geste est déjà absent de `points` : le geste n'a plus qu'à se conclure. */
  function onUp(event: PointerEvent): void {
    if (!drag || drag.pointer !== event.pointerId) return;
    if (drag.removing) selected = null;
    drag = null;
    removing = false;
    onsettle();
  }

  function remove(): void {
    if (selected === null || current.length <= 2) return;
    const index = selected;
    selected = null;
    onchange(current.filter((_, i) => i !== index));
    onsettle();
  }

  function onKey(event: KeyboardEvent): void {
    if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      event.stopPropagation();
      remove();
    }
  }

  /** Saisie de l'entrée ou de la sortie du point choisi. */
  function setField(which: 0 | 1, value: number): void {
    if (selected === null || !Number.isFinite(value)) return;
    const index = selected;
    const pts = current;
    const v = Math.round(Math.min(255, Math.max(0, value)));
    const low = index > 0 ? pts[index - 1]![0] + 1 : 0;
    const high = index < pts.length - 1 ? pts[index + 1]![0] - 1 : 255;
    onchange(pts.map((p, i): CurvePoint => (i === index ? (which === 0 ? [Math.min(high, Math.max(low, v)), p[1]] : [p[0], v]) : p)));
    onsettle();
  }
</script>

<div class="curve">
  <!-- Zone de dessin : se pilote au pointeur, et au clavier (Suppr) une fois le point choisi. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <svg
    bind:this={svg}
    viewBox="-4 -4 263 263"
    role="application"
    aria-label={t("Courbe : cliquer pour ajouter un point, glisser pour le déplacer, le tirer dehors ou Suppr pour le retirer")}
    tabindex="0"
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
    onkeydown={onKey}
  >
    <rect class="frame" x="0" y="0" width="255" height="255" />
    {#each [64, 128, 191] as v (v)}
      <line class="grid" x1={v} y1="0" x2={v} y2="255" />
      <line class="grid" x1="0" y1={v} x2="255" y2={v} />
    {/each}
    <line class="diagonal" x1="0" y1="255" x2="255" y2="0" />
    {#each ghosts as ghost, i (i)}
      <path class="ghost" d={pathOf(ghost.points)} style:stroke={ghost.color} />
    {/each}
    <path class="line" d={pathOf(current)} style:stroke={color} />
    {#each current as point, index (index)}
      {#if !(removing && index === selected)}
        <rect class="point" class:on={index === selected} x={point[0] - 3.5} y={255 - point[1] - 3.5} width="7" height="7" style:--c={color} />
      {/if}
    {/each}
  </svg>
  <div class="fields">
    <label>
      <span>{t("Entrée")}</span>
      <input type="number" min="0" max="255" disabled={!active} value={active ? Math.round(active[0]) : ""} onkeydown={(event) => event.stopPropagation()} onchange={(event) => setField(0, event.currentTarget.valueAsNumber)} />
    </label>
    <label>
      <span>{t("Sortie")}</span>
      <input type="number" min="0" max="255" disabled={!active} value={active ? Math.round(active[1]) : ""} onkeydown={(event) => event.stopPropagation()} onchange={(event) => setField(1, event.currentTarget.valueAsNumber)} />
    </label>
    <button class="remove" disabled={!active || current.length <= 2} title={t("Retirer le point (Suppr)")} onclick={remove}>{t("Retirer")}</button>
  </div>
</div>

<style>
  .curve {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  svg {
    width: 100%;
    aspect-ratio: 1;
    display: block;
    background: var(--bg-field);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    touch-action: none;
    cursor: crosshair;
    outline: none;
  }
  svg:focus-visible {
    border-color: var(--accent);
  }
  .frame {
    fill: none;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 0.8;
  }
  .diagonal {
    stroke: var(--border-strong);
    stroke-width: 0.8;
    stroke-dasharray: 3 3;
  }
  .ghost {
    fill: none;
    stroke-width: 1;
    opacity: 0.45;
  }
  .line {
    fill: none;
    stroke-width: 1.6;
  }
  .point {
    fill: var(--bg-panel);
    stroke: var(--c);
    stroke-width: 1.3;
  }
  .point.on {
    fill: var(--c);
  }
  .fields {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  .fields label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    flex: 1;
    min-width: 0;
    color: var(--text-3);
  }
  .fields input {
    width: 100%;
    box-sizing: border-box;
  }
  .remove {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text-2);
  }
  .remove:disabled {
    color: var(--text-disabled);
  }
</style>
