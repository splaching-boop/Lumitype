<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import type { BlendIfChannel } from "../lib/ipc";

  // Rampe de comparaison (options de fusion, « Blend If » de Photoshop) : un dégradé de la couche comparée, et deux
  // triangles (noir à gauche, blanc à droite). Glisser un triangle le déplace ; Alt + glisser le scinde en deux
  // moitiés, entre lesquelles le calque apparaît en fondu. Une moitié scindée se glisse ensuite seule ; ramenée sur
  // l'autre, le triangle se referme.
  type Range = [number, number, number, number];
  let {
    label,
    channel,
    value,
    onchange,
  }: { label: string; channel: BlendIfChannel; value: Range; onchange: (value: Range, done: boolean) => void } = $props();

  const END: Record<BlendIfChannel, string> = { gray: "#ffffff", red: "#ff2a2a", green: "#2aff4a", blue: "#3a6bff" };
  let bar = $state<HTMLDivElement | null>(null);
  /** Indices déplacés par le geste en cours : une moitié ([i]) ou un triangle entier ([i, i + 1]). */
  let drag: { indices: number[]; start: Range } | null = null;

  const pct = (v: number) => `${(v / 255) * 100}%`;
  const valueAt = (clientX: number) => {
    const rect = bar!.getBoundingClientRect();
    return Math.round(Math.min(1, Math.max(0, (clientX - rect.left) / Math.max(1, rect.width))) * 255);
  };

  function down(event: PointerEvent, side: 0 | 2, half: 0 | 1 | null): void {
    event.preventDefault();
    event.stopPropagation();
    const joined = value[side] === value[side + 1];
    let indices: number[];
    if (!joined) indices = [side + (half ?? 0)];
    else if (event.altKey) {
      // Alt : le triangle se scinde, la moitié saisie (gauche ou droite du triangle) part seule.
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
      indices = [side + (event.clientX < rect.left + rect.width / 2 ? 0 : 1)];
    } else indices = [side, side + 1];
    drag = { indices, start: [...value] as Range };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function move(event: PointerEvent, done = false): void {
    if (!drag || !bar) return;
    const v = valueAt(event.clientX);
    const next = [...drag.start] as Range;
    const first = drag.indices[0]!;
    const last = drag.indices[drag.indices.length - 1]!;
    // Bornes : les triangles ne se croisent pas (noir ≤ blanc, début ≤ fin).
    const low = first === 0 ? 0 : next[first - 1]!;
    const high = last === 3 ? 255 : next[last + 1]!;
    for (const i of drag.indices) next[i] = Math.min(high, Math.max(low, v));
    if (next.some((x, i) => x !== value[i]) || done) onchange(next, done);
    if (done) drag = null;
  }

  /** Texte des valeurs, comme Photoshop : « 0 » ou « 30/80 » pour un triangle scindé. */
  const text = (a: number, b: number) => (a === b ? `${a}` : `${a}/${b}`);
</script>

{#snippet handle(side: 0 | 2)}
  {#if value[side] === value[side + 1]}
    <button
      class="tri"
      class:white={side === 2}
      style:left={pct(value[side]!)}
      aria-label={side === 0 ? t("Triangle noir (Alt pour scinder)") : t("Triangle blanc (Alt pour scinder)")}
      onpointerdown={(event) => down(event, side, null)}
      onpointermove={(event) => move(event)}
      onpointerup={(event) => move(event, true)}
    ></button>
  {:else}
    {#each [0, 1] as const as half (half)}
      <button
        class="tri half"
        class:left={half === 0}
        class:right={half === 1}
        class:white={side === 2}
        style:left={pct(value[side + half]!)}
        aria-label={t("Moitié de triangle")}
        onpointerdown={(event) => down(event, side, half)}
        onpointermove={(event) => move(event)}
        onpointerup={(event) => move(event, true)}
      ></button>
    {/each}
  {/if}
{/snippet}

<div class="ramp">
  <div class="head">
    <span>{label}</span>
    <span class="values">{text(value[0], value[1])}<span class="gap"></span>{text(value[2], value[3])}</span>
  </div>
  <div class="bar" bind:this={bar} style:background="linear-gradient(to right, #000, {END[channel]})"></div>
  <div class="track">
    {@render handle(0)}
    {@render handle(2)}
  </div>
</div>

<style>
  .ramp {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-3);
  }
  .head {
    display: flex;
    justify-content: space-between;
  }
  .values {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .gap {
    display: inline-block;
    width: 18px;
  }
  .bar {
    height: 14px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .track {
    position: relative;
    height: 12px;
    margin: 0 0 2px;
  }
  .tri {
    position: absolute;
    top: 0;
    width: 12px;
    height: 11px;
    margin-left: -6px;
    padding: 0;
    border: none;
    background: none;
    overflow: hidden;
    cursor: ew-resize;
    --fill: #0b0b0c;
    --edge: #a0a0a6;
  }
  .tri.white {
    --fill: #f2f2f2;
    --edge: #55555b;
  }
  /* Triangle dessiné par deux couches : le bord, puis le fond un peu en retrait. */
  .tri::before,
  .tri::after {
    content: "";
    position: absolute;
    top: 0;
    left: 0;
    width: 12px;
    height: 11px;
    clip-path: polygon(50% 0, 100% 100%, 0 100%);
    background: var(--edge);
  }
  .tri::after {
    background: var(--fill);
    transform: scale(0.72) translateY(1.5px);
  }
  /* Moitiés d'un triangle scindé : la gauche s'arrête au centre, la droite en part. */
  .tri.half {
    width: 6px;
  }
  .tri.half.right {
    margin-left: 0;
  }
  .tri.half.right::before,
  .tri.half.right::after {
    left: -6px;
  }
  /* Le fond se décale du côté de la coupe : le bord du triangle reste visible le long de la coupe. */
  .tri.half.left::after {
    transform: scale(0.72) translate(-2px, 1.5px);
  }
  .tri.half.right::after {
    transform: scale(0.72) translate(2px, 1.5px);
  }
</style>
