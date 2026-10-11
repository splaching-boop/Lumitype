<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount, untrack } from "svelte";
  import { edit } from "../lib/engine";
  import { api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Fluidité : on pousse, gonfle, pince ou rétablit l'image au pinceau. La déformation est une
  // grille de décalages (vers l'image d'origine, en fraction de sa taille) : ici elle s'applique
  // en direct à une réduction du calque ; à la validation, le moteur applique la même grille à
  // l'image en pleine résolution. Rien n'est envoyé au moteur pendant qu'on travaille.
  const PREVIEW_SIDE = 900;
  const CELL = 6; // pas de la grille, en pixels de l'aperçu
  const docId = untrack(() => app.activeId);
  const layerId = untrack(() => app.activeLayer?.id ?? null);

  type Tool = "push" | "bloat" | "pucker" | "reconstruct";
  const TOOLS: { id: Tool; label: string; hint: string }[] = [
    { id: "push", label: t("Déformation avant"), hint: t("Pousse la matière dans le sens du geste.") },
    { id: "bloat", label: "Dilatation", hint: t("Gonfle sous le pinceau tant que le bouton est enfoncé.") },
    { id: "pucker", label: "Contraction", hint: t("Resserre vers le centre du pinceau.") },
    { id: "reconstruct", label: "Reconstruction", hint: t("Rétablit peu à peu l'image d'origine.") },
  ];

  let dialog = $state<HTMLDialogElement>();
  let canvas = $state<HTMLCanvasElement>();
  let tool = $state<Tool>("push");
  let size = $state(120);
  let strength = $state(60);
  let failure = $state<string | null>(null);
  let ready = $state(false);
  let changed = $state(false);
  let pointer = $state<{ x: number; y: number } | null>(null);
  let room = $state({ width: 0, height: 0 });
  let dims = $state({ width: 3, height: 2 });
  const frame = $derived.by(() => {
    const k = Math.min(Math.max(0, room.width - 36) / dims.width, Math.max(0, room.height - 36) / dims.height);
    return { width: Math.floor(dims.width * k), height: Math.floor(dims.height * k), scale: k };
  });

  // État non réactif : tableaux volumineux, modifiés des milliers de fois par seconde.
  let source = new Uint8ClampedArray(0);
  let [w, h, cols, rows] = [0, 0, 2, 2];
  let dx = new Float32Array(4);
  let dy = new Float32Array(4);
  let output: ImageData | null = null;
  let dirty = false;
  let held: { x: number; y: number } | null = null;
  let last: { x: number; y: number } | null = null;
  let raf = 0;

  $effect(() => {
    dialog?.showModal();
  });

  onMount(() => {
    if (docId === null || layerId === null) return;
    // Une grille nulle ne déforme rien : on récupère ainsi la réduction du calque telle quelle.
    api.filterPreview(docId, layerId, { type: "liquify", cols: 2, rows: 2, dx: [0, 0, 0, 0], dy: [0, 0, 0, 0] }, PREVIEW_SIDE).then(
      (buffer) => {
        const header = new DataView(buffer);
        [w, h] = [header.getUint32(0, true), header.getUint32(4, true)];
        source = new Uint8ClampedArray(buffer, 8);
        [cols, rows] = [Math.ceil(w / CELL) + 1, Math.ceil(h / CELL) + 1];
        [dx, dy] = [new Float32Array(cols * rows), new Float32Array(cols * rows)];
        output = new ImageData(w, h);
        dims = { width: w, height: h };
        [ready, dirty] = [true, true];
        raf = requestAnimationFrame(tick);
      },
      (error: unknown) => (failure = String(error)),
    );
    return () => cancelAnimationFrame(raf);
  });

  /** Boucle d'affichage : applique les outils « continus » puis redessine si la grille a bougé. */
  function tick(): void {
    if (held && (tool === "bloat" || tool === "pucker" || tool === "reconstruct")) brush(held.x, held.y, 0, 0);
    if (dirty) render();
    raf = requestAnimationFrame(tick);
  }

  /** Applique l'outil autour de `(x, y)` (pixels de l'aperçu) ; `(mx, my)` : déplacement du pinceau. */
  function brush(x: number, y: number, mx: number, my: number): void {
    const radius = size / 2;
    const k = strength / 100;
    const [i0, i1] = [Math.max(0, Math.floor((x - radius) / CELL)), Math.min(cols - 1, Math.ceil((x + radius) / CELL))];
    const [j0, j1] = [Math.max(0, Math.floor((y - radius) / CELL)), Math.min(rows - 1, Math.ceil((y + radius) / CELL))];
    for (let j = j0; j <= j1; j++) {
      for (let i = i0; i <= i1; i++) {
        const [nx, ny] = [i * CELL - x, j * CELL - y];
        const d2 = (nx * nx + ny * ny) / (radius * radius);
        if (d2 >= 1) continue;
        const weight = (1 - d2) * (1 - d2) * k; // doux au bord, plein au centre
        const n = j * cols + i;
        if (tool === "push") {
          // La matière avance de (mx, my) : le point d'arrivée va chercher sa couleur d'autant en arrière.
          dx[n] = (dx[n] ?? 0) - (mx * weight) / w;
          dy[n] = (dy[n] ?? 0) - (my * weight) / h;
        } else if (tool === "reconstruct") {
          dx[n] = (dx[n] ?? 0) * (1 - weight * 0.12);
          dy[n] = (dy[n] ?? 0) * (1 - weight * 0.12);
        } else {
          // Dilatation : on va chercher plus près du centre (la zone grossit) ; contraction : plus loin.
          const sign = tool === "bloat" ? -1 : 1;
          dx[n] = (dx[n] ?? 0) + (sign * nx * weight * 0.03) / w;
          dy[n] = (dy[n] ?? 0) + (sign * ny * weight * 0.03) / h;
        }
      }
    }
    [dirty, changed] = [true, true];
  }

  /** Redessine l'aperçu : pour chaque pixel, décalage interpolé dans la grille puis lecture bilinéaire de la source. */
  function render(): void {
    dirty = false;
    const target = canvas?.getContext("2d");
    if (!output || !target || !canvas) return;
    if (canvas.width !== w) [canvas.width, canvas.height] = [w, h];
    const out = output.data;
    for (let y = 0; y < h; y++) {
      const v = y / CELL;
      const j = Math.min(rows - 2, v | 0);
      const tv = v - j;
      for (let x = 0; x < w; x++) {
        const u = x / CELL;
        const i = Math.min(cols - 2, u | 0);
        const tu = u - i;
        const [a, b, c, d] = [j * cols + i, j * cols + i + 1, (j + 1) * cols + i, (j + 1) * cols + i + 1];
        const ox = ((dx[a] ?? 0) * (1 - tu) + (dx[b] ?? 0) * tu) * (1 - tv) + ((dx[c] ?? 0) * (1 - tu) + (dx[d] ?? 0) * tu) * tv;
        const oy = ((dy[a] ?? 0) * (1 - tu) + (dy[b] ?? 0) * tu) * (1 - tv) + ((dy[c] ?? 0) * (1 - tu) + (dy[d] ?? 0) * tu) * tv;
        const [sx, sy] = [x + ox * w, y + oy * h];
        const o = (y * w + x) * 4;
        if (sx < 0 || sy < 0 || sx > w - 1 || sy > h - 1) {
          out[o + 3] = 0;
          continue;
        }
        const [x0, y0] = [sx | 0, sy | 0];
        const [fx, fy] = [sx - x0, sy - y0];
        const [x1, y1] = [Math.min(w - 1, x0 + 1), Math.min(h - 1, y0 + 1)];
        const [p00, p10, p01, p11] = [(y0 * w + x0) * 4, (y0 * w + x1) * 4, (y1 * w + x0) * 4, (y1 * w + x1) * 4];
        for (let ch = 0; ch < 4; ch++) {
          const top = (source[p00 + ch] ?? 0) * (1 - fx) + (source[p10 + ch] ?? 0) * fx;
          const bottom = (source[p01 + ch] ?? 0) * (1 - fx) + (source[p11 + ch] ?? 0) * fx;
          out[o + ch] = top * (1 - fy) + bottom * fy;
        }
      }
    }
    target.putImageData(output, 0, 0);
  }

  function position(event: PointerEvent): { x: number; y: number } {
    const rect = canvas?.getBoundingClientRect();
    return rect ? { x: ((event.clientX - rect.left) / rect.width) * w, y: ((event.clientY - rect.top) / rect.height) * h } : { x: 0, y: 0 };
  }

  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || !ready) return;
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    held = last = position(event);
  }

  function onPointerMove(event: PointerEvent): void {
    const at = position(event);
    pointer = at;
    if (!held || !last) return;
    held = at;
    if (tool === "push") {
      // Geste découpé en petits pas : un mouvement rapide déforme aussi régulièrement qu'un lent.
      const [mx, my] = [at.x - last.x, at.y - last.y];
      const steps = Math.max(1, Math.ceil(Math.hypot(mx, my) / (size / 8)));
      for (let s = 1; s <= steps; s++) brush(last.x + (mx * s) / steps, last.y + (my * s) / steps, mx / steps, my / steps);
    }
    last = at;
  }

  function reset(): void {
    dx.fill(0);
    dy.fill(0);
    [dirty, changed] = [true, false];
  }

  function close(apply: boolean): void {
    app.liquifyOpen = false;
    if (apply && changed && layerId !== null) void edit({ op: "applyFilter", id: layerId, filter: { type: "liquify", cols, rows, dx: [...dx], dy: [...dy] } });
  }
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <header>
      <h2>{t("Fluidité")}</h2>
      <span>{TOOLS.find((t) => t.id === tool)?.hint}</span>
    </header>
    <div class="body">
      <div class="stage" bind:clientWidth={room.width} bind:clientHeight={room.height}>
        <div class="frame" style:width="{frame.width}px" style:height="{frame.height}px">
          <canvas
            bind:this={canvas}
            onpointerdown={onPointerDown}
            onpointermove={onPointerMove}
            onpointerup={() => (held = last = null)}
            onpointercancel={() => (held = last = null)}
            onpointerleave={() => (pointer = null)}
          ></canvas>
          {#if pointer && ready}
            <!-- Cercle du pinceau, à l'échelle de l'aperçu. -->
            <div class="brush" style:left="{(pointer.x / dims.width) * 100}%" style:top="{(pointer.y / dims.height) * 100}%" style:width="{size * frame.scale}px" style:height="{size * frame.scale}px"></div>
          {/if}
        </div>
        {#if failure}<p class="failure">{failure}</p>{:else if !ready}<p>{t("Préparation de l'aperçu…")}</p>{/if}
      </div>
      <aside>
        <div class="tools" role="radiogroup" aria-label={t("Outil")}>
          {#each TOOLS as item (item.id)}
            <button type="button" role="radio" aria-checked={tool === item.id} class:on={tool === item.id} onclick={() => (tool = item.id)}>{t(item.label)}</button>
          {/each}
        </div>
        <label>
          <span>{t("Taille")}</span>
          <output>{t("{size} px", { size })}</output>
          <input type="range" min="10" max="500" bind:value={size} />
        </label>
        <label>
          <span>{t("Intensité")}</span>
          <output>{strength} %</output>
          <input type="range" min="5" max="100" bind:value={strength} />
        </label>
        <button type="button" class="ghost" disabled={!changed} onclick={reset}>{t("Tout rétablir")}</button>
        <div class="spacer"></div>
        <p>{t("L'aperçu est une réduction du calque ; la déformation est appliquée en pleine résolution à la validation. La taille du pinceau est exprimée dans l'aperçu.")}</p>
      </aside>
    </div>
    <footer>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill" disabled={!changed}>OK</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(1180px, calc(100vw - 48px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 22px;
  }
  header {
    height: 54px;
    border-bottom: 1px solid var(--border);
  }
  header span {
    color: var(--text-muted);
  }
  footer {
    height: 58px;
    justify-content: flex-end;
    gap: 8px;
    border-top: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    display: flex;
    height: min(640px, calc(100vh - 200px));
  }
  .stage {
    flex: 1;
    min-width: 0;
    display: grid;
    place-items: center;
    background: var(--bg-canvas);
  }
  .frame {
    position: relative;
    overflow: hidden;
    background: repeating-conic-gradient(#d0d0d0 0% 25%, #ffffff 0% 50%) 0 0 / 16px 16px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: none;
    touch-action: none;
  }
  .brush {
    position: absolute;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    border: 1px solid #fff;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.6),
      inset 0 0 0 1px rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  aside {
    width: 270px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 20px;
    border-left: 1px solid var(--border);
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .tools button {
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    text-align: left;
    color: var(--text-3);
  }
  .tools button:hover {
    background: var(--bg-hover);
  }
  .tools button.on {
    background: var(--bg-active);
    color: #fff;
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px;
    color: var(--text-3);
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  input {
    grid-column: 1 / -1;
    width: 100%;
  }
  .spacer {
    flex: 1;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  .failure {
    color: var(--danger, #ff8080);
  }
  .ghost,
  .pill {
    height: 32px;
    padding: 0 18px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  aside .ghost {
    align-self: flex-start;
    height: 28px;
    padding: 0 12px;
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .ghost:disabled,
  .pill:disabled {
    opacity: 0.35;
  }
</style>
