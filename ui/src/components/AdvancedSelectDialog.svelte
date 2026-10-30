<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount, untrack } from "svelte";
  import { describeError } from "../lib/errors";
  import { edit } from "../lib/engine";
  import { api, type AdvancedDab, type AdvancedInfo, type AdvancedOutput, type AdvancedParams } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Sélection avancée (menu Sélection, Ctrl+Alt+R) : la méthode des graphistes pour les arbres et les cheveux. On part
  // de la couche où le sujet contraste le plus avec le fond (choisie toute seule), on la pousse en noir et blanc par
  // des niveaux (seuil doux), puis on affine le contour (rayon intelligent, pinceau Affiner) et on corrige au pinceau.
  // L'aperçu est calculé par le moteur sur une image réduite ; OK applique les mêmes réglages à l'image entière.
  const CHANNEL_NAMES = ["Rouge", "Vert", "Bleu", "Luminosité"] as const;
  const MIXES: [number, number, number][] = [
    [100, 0, 0],
    [0, 100, 0],
    [0, 0, 100],
    [30, 59, 11],
  ];
  type View = "original" | "channel" | "cut" | "mask" | "overlay";
  type Tool = "refine" | "add" | "remove" | "pick";

  const doc = untrack(() => app.active);
  const layerChoice = untrack(() => (app.activeLayer && !app.activeLayer.isGroup && !app.activeLayer.adjustment ? app.activeLayer : null));
  let dialog = $state<HTMLDialogElement>();
  let canvas = $state<HTMLCanvasElement>();
  let thumbs = $state<HTMLCanvasElement[]>([]);
  let info = $state<AdvancedInfo | null>(null);
  let image: Uint8ClampedArray | null = null;
  let maskBytes: Uint8Array | null = null;
  let channelBytes: Uint8Array | null = null;
  let failure = $state<string | null>(null);
  let busy = $state(false);
  let wholeDocument = $state(layerChoice === null);
  let view = $state<View>("cut");
  let backdrop = $state<"checker" | "black" | "white">("checker");
  let tool = $state<Tool>("refine");
  let brush = $state(40);
  let channelIndex = $state<number | "custom">(2);
  let output = $state<AdvancedOutput>(layerChoice ? "layerMask" : "selection");
  let params = $state<AdvancedParams>({
    method: "channel",
    mix: [0, 0, 100],
    black: 0,
    gamma: 1,
    white: 255,
    invert: false,
    colors: [],
    fuzziness: 40,
    toneLow: 0,
    toneHigh: 85,
    toneSoft: 20,
    radius: 4,
    smooth: 0,
    feather: 0,
    shift: 0,
    decontaminate: false,
    amount: 0.7,
    dabs: [],
  });
  /** Position de l'image dans le canevas (aperçu ajusté à la zone). */
  const FRAME = { width: 660, height: 440 };
  const fit = $derived(info ? Math.min(FRAME.width / info.previewWidth, FRAME.height / info.previewHeight) : 1);
  /** Zoom de l'aperçu (1 = ajusté au cadre) et décalage de l'image dans le cadre, en pixels d'écran. */
  let zoom = $state(1);
  let pan = $state({ x: 0, y: 0 });
  let frameEl = $state<HTMLDivElement>();
  let spaceHeld = $state(false);
  let panning: { x: number; y: number; from: { x: number; y: number } } | null = null;
  const shownW = $derived(info ? info.previewWidth * fit * zoom : 0);
  const shownH = $derived(info ? info.previewHeight * fit * zoom : 0);
  /** Image centrée tant qu'elle tient dans le cadre ; sinon déplacée, sans laisser de vide au bord. */
  function clampPan(p: { x: number; y: number }): { x: number; y: number } {
    const cx = (FRAME.width - shownW) / 2;
    const cy = (FRAME.height - shownH) / 2;
    return { x: shownW <= FRAME.width ? cx : Math.min(0, Math.max(FRAME.width - shownW, p.x)), y: shownH <= FRAME.height ? cy : Math.min(0, Math.max(FRAME.height - shownH, p.y)) };
  }
  function zoomAt(next: number, fx = FRAME.width / 2, fy = FRAME.height / 2): void {
    const z = Math.min(16, Math.max(1, next));
    const k = z / zoom;
    zoom = z;
    pan = clampPan({ x: fx - (fx - pan.x) * k, y: fy - (fy - pan.y) * k });
  }
  function onWheel(event: WheelEvent): void {
    if (!frameEl) return;
    event.preventDefault();
    const r = frameEl.getBoundingClientRect();
    zoomAt(zoom * Math.exp(-event.deltaY * 0.002), event.clientX - r.left, event.clientY - r.top);
  }
  // Recadre le décalage quand l'image ou le zoom change ; `pan` est lu hors suivi (sinon l'effet se relancerait
  // sur sa propre écriture) et n'est réécrit que s'il change vraiment.
  $effect(() => {
    void info;
    void shownW;
    void shownH;
    const current = untrack(() => pan);
    const next = clampPan(current);
    if (next.x !== current.x || next.y !== current.y) pan = next;
  });
  const best = $derived(info ? info.scores.reduce((b, s, i, all) => (s.score > (all[b]?.score ?? 0) ? i : b), 0) : 2);

  onMount(() => {
    dialog?.showModal();
    void load();
    return () => void api.advancedSelectionClose().catch(() => undefined);
  });

  /** Analyse de l'image (calque seul ou tout le visible) ; la meilleure couche et ses niveaux sont proposés. */
  async function load(): Promise<void> {
    if (!doc) return;
    busy = true;
    try {
      info = await api.advancedSelectionOpen(doc.info.id, wholeDocument ? null : (layerChoice?.id ?? null), 1600);
      image = new Uint8ClampedArray(await api.advancedSelectionImage());
      drawThumbs();
      chooseChannel(best);
    } catch (error) {
      failure = describeError(error);
    } finally {
      busy = false;
    }
  }

  function chooseChannel(index: number | "custom"): void {
    channelIndex = index;
    if (index === "custom" || !info) return;
    const s = info.scores[index];
    params.mix = [...MIXES[index]!] as [number, number, number];
    if (s) [params.black, params.white, params.gamma, params.invert] = [Math.round(s.black), Math.round(s.white), 1, s.invert];
  }

  function drawThumbs(): void {
    if (!info || !image) return;
    MIXES.forEach((mix, i) => {
      const c = thumbs[i];
      if (!c || !info || !image) return;
      c.width = info.previewWidth;
      c.height = info.previewHeight;
      const out = new ImageData(info.previewWidth, info.previewHeight);
      for (let p = 0; p < out.data.length; p += 4) {
        const v = (image[p]! * mix[0] + image[p + 1]! * mix[1] + image[p + 2]! * mix[2]) / 100;
        out.data[p] = out.data[p + 1] = out.data[p + 2] = v;
        out.data[p + 3] = 255;
      }
      c.getContext("2d")?.putImageData(out, 0, 0);
    });
  }

  /** Histogramme de la couche mélangée, avant les niveaux. */
  const histogram = $derived.by(() => {
    void info;
    const mix = params.mix;
    const bins = new Array<number>(64).fill(0);
    if (!image) return bins;
    for (let p = 0; p < image.length; p += 16) {
      const v = Math.max(0, Math.min(255, (image[p]! * mix[0] + image[p + 1]! * mix[1] + image[p + 2]! * mix[2]) / 100));
      bins[Math.min(63, v >> 2)]!++;
    }
    const max = Math.max(1, ...bins);
    return bins.map((n) => n / max);
  });

  // Aperçu : recalculé (avec un court délai) à chaque réglage.
  let generation = 0;
  $effect(() => {
    const snapshot = $state.snapshot(params) as AdvancedParams;
    if (!info) return;
    const mine = ++generation;
    const timer = setTimeout(() => {
      api
        .advancedSelectionPreview(snapshot)
        .then((buffer) => {
          if (mine !== generation || !info) return;
          const n = info.previewWidth * info.previewHeight;
          maskBytes = new Uint8Array(buffer, 0, n);
          channelBytes = new Uint8Array(buffer, n, n);
          paint();
        })
        .catch((error: unknown) => (failure = describeError(error)));
    }, 60);
    return () => clearTimeout(timer);
  });
  $effect(() => {
    void view;
    void backdrop;
    paint();
  });

  function paint(): void {
    if (!canvas || !info || !image) return;
    const [w, h] = [info.previewWidth, info.previewHeight];
    canvas.width = w;
    canvas.height = h;
    const out = new ImageData(w, h);
    const d = out.data;
    const mask = maskBytes;
    const channel = channelBytes;
    const bg = backdrop === "black" ? 0 : 255;
    for (let i = 0, p = 0; i < w * h; i++, p += 4) {
      const m = mask ? mask[i]! / 255 : 1;
      if (view === "mask" || (view === "channel" && channel)) {
        const v = view === "mask" ? (mask?.[i] ?? 255) : channel![i]!;
        d[p] = d[p + 1] = d[p + 2] = v;
        d[p + 3] = 255;
      } else if (view === "overlay") {
        // Calque rouge : le fond est recouvert de rouge à 55 %.
        const k = (1 - m) * 0.55;
        d[p] = image[p]! * (1 - k) + 229 * k;
        d[p + 1] = image[p + 1]! * (1 - k) + 52 * k;
        d[p + 2] = image[p + 2]! * (1 - k) + 62 * k;
        d[p + 3] = 255;
      } else if (view === "cut") {
        if (backdrop === "checker") {
          d[p] = image[p]!;
          d[p + 1] = image[p + 1]!;
          d[p + 2] = image[p + 2]!;
          d[p + 3] = image[p + 3]! * m;
        } else {
          for (let c = 0; c < 3; c++) d[p + c] = image[p + c]! * m + bg * (1 - m);
          d[p + 3] = 255;
        }
      } else {
        d[p] = image[p]!;
        d[p + 1] = image[p + 1]!;
        d[p + 2] = image[p + 2]!;
        d[p + 3] = image[p + 3]!;
      }
    }
    canvas.getContext("2d")?.putImageData(out, 0, 0);
  }

  // ---- pinceaux et pipette (coordonnées du document)

  let stroking = false;
  /** Début de chaque coup de pinceau dans `params.dabs` (pour annuler le dernier). */
  let strokeStarts = $state<number[]>([]);
  let cursor = $state<{ x: number; y: number } | null>(null);
  function docPoint(event: PointerEvent): { x: number; y: number } | null {
    if (!canvas || !info) return null;
    const rect = canvas.getBoundingClientRect();
    const k = info.previewWidth / rect.width;
    return { x: ((event.clientX - rect.left) * k) / info.scale, y: ((event.clientY - rect.top) * k) / info.scale };
  }
  function dab(event: PointerEvent): void {
    const point = docPoint(event);
    if (!point || tool === "pick") return;
    const radius = brush / 2;
    const step = Math.max(1, radius * 0.35);
    // Dans un même coup de pinceau, les points sont reliés : des empreintes régulières entre deux positions.
    const start = strokeStarts.at(-1) ?? 0;
    const last = params.dabs.length > start ? params.dabs.at(-1) : undefined;
    if (!last) {
      params.dabs.push({ x: point.x, y: point.y, radius, mode: tool } satisfies AdvancedDab);
      return;
    }
    const distance = Math.hypot(point.x - last.x, point.y - last.y);
    if (distance < step) return;
    const count = Math.floor(distance / step);
    const added: AdvancedDab[] = [];
    for (let i = 1; i <= count; i++) {
      const k = (i * step) / distance;
      added.push({ x: last.x + (point.x - last.x) * k, y: last.y + (point.y - last.y) * k, radius, mode: tool });
    }
    params.dabs.push(...added);
  }
  function pick(event: PointerEvent): void {
    if (!info || !image || !canvas) return;
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor(((event.clientX - rect.left) / rect.width) * info.previewWidth);
    const y = Math.floor(((event.clientY - rect.top) / rect.height) * info.previewHeight);
    const p = (y * info.previewWidth + x) * 4;
    const color: [number, number, number] = [image[p] ?? 0, image[p + 1] ?? 0, image[p + 2] ?? 0];
    params.colors = event.shiftKey ? [...params.colors, color] : [color];
  }
  function down(event: PointerEvent): void {
    // Espace + glisser, ou bouton du milieu : déplacer l'aperçu agrandi.
    if (event.button === 1 || (event.button === 0 && spaceHeld)) {
      event.preventDefault();
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
      panning = { x: event.clientX, y: event.clientY, from: pan };
      return;
    }
    if (event.button !== 0) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    if (params.method === "color" && (tool === "pick" || event.altKey)) return pick(event);
    stroking = true;
    strokeStarts.push(params.dabs.length);
    dab(event);
  }
  function moved(event: PointerEvent): void {
    if (panning) {
      pan = clampPan({ x: panning.from.x + event.clientX - panning.x, y: panning.from.y + event.clientY - panning.y });
      return;
    }
    if (frameEl) {
      const rect = frameEl.getBoundingClientRect();
      cursor = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    }
    if (stroking) dab(event);
  }
  const brushOnScreen = $derived(info ? brush * info.scale * fit * zoom : brush);

  function reset(): void {
    params.dabs = [];
    strokeStarts = [];
    params.radius = 4;
    params.smooth = 0;
    params.feather = 0;
    params.shift = 0;
    params.decontaminate = false;
    chooseChannel(best);
  }

  function close(): void {
    app.advancedSelectOpen = false;
  }

  async function confirm(): Promise<void> {
    const snapshot = $state.snapshot(params) as AdvancedParams;
    const layer = wholeDocument ? null : (layerChoice?.id ?? null);
    close();
    try {
      app.exportProgress = { progress: 0.3, label: t("Sélection avancée") };
      await edit({ op: "advancedSelection", layer, params: snapshot, output });
    } finally {
      app.exportProgress = null;
    }
  }

  // ---- rampe des niveaux

  let ramp = $state<HTMLDivElement>();
  function dragLevel(event: PointerEvent, which: "black" | "gamma" | "white"): void {
    event.preventDefault();
    const target = event.currentTarget as HTMLElement;
    target.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      if (!ramp) return;
      const rect = ramp.getBoundingClientRect();
      const v = Math.round(Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width)) * 255);
      if (which === "black") params.black = Math.min(v, params.white - 1);
      else if (which === "white") params.white = Math.max(v, params.black + 1);
      else {
        // Gamma : la position du triangle gris entre noir et blanc (au milieu = 1).
        const t01 = Math.min(0.98, Math.max(0.02, (v - params.black) / Math.max(1, params.white - params.black)));
        params.gamma = Math.round(Math.min(9.99, Math.max(0.1, Math.log(0.5) / Math.log(t01))) * 100) / 100;
      }
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  }
  const gammaAt = $derived(params.black + (params.white - params.black) * Math.pow(0.5, 1 / params.gamma));
  const pct = (v: number) => `${(v / 255) * 100}%`;
  const hex = (c: [number, number, number]) => `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
</script>

{#snippet slider(label: string, key: "radius" | "smooth" | "feather" | "shift" | "fuzziness" | "toneLow" | "toneHigh" | "toneSoft", min: number, max: number, step: number, unit: string)}
  <label class="line">
    <span>{t(label)}</span>
    <input type="range" {min} {max} {step} bind:value={params[key]} />
    <output>{String(params[key]).replace(".", ",")}{unit}</output>
  </label>
{/snippet}

<svelte:window onkeydown={(event) => event.code === "Space" && !(event.target instanceof HTMLInputElement) && ((spaceHeld = true), event.preventDefault())} onkeyup={(event) => event.code === "Space" && (spaceHeld = false)} />

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close())}>
  <header>
    <h2>{t("Sélection avancée")}</h2>
    <span class="muted">{wholeDocument ? t("Tous les calques visibles") : t("Calque « {name} »", { name: layerChoice?.name ?? "" })}</span>
    <div class="spacer"></div>
    <button type="button" class="icon" aria-label={t("Fermer")} onclick={close}>✕</button>
  </header>
  <div class="body">
    <div class="tools" role="toolbar" aria-label={t("Outils")}>
      <button type="button" class:on={tool === "refine"} title={t("Pinceau Affiner : recalcule le contour sous le pinceau (cheveux, feuillage)")} onclick={() => (tool = "refine")}>✎</button>
      <button type="button" class:on={tool === "add"} title={t("Pinceau + : force le sujet")} onclick={() => (tool = "add")}>＋</button>
      <button type="button" class:on={tool === "remove"} title={t("Pinceau − : force le fond")} onclick={() => (tool = "remove")}>－</button>
      {#if params.method === "color"}
        <button type="button" class:on={tool === "pick"} title={t("Pipette : prélever une couleur (Maj : en ajouter)")} onclick={() => (tool = "pick")}>⌖</button>
      {/if}
      <div class="sep"></div>
      <button type="button" disabled={params.dabs.length === 0} title={t("Annuler le dernier coup de pinceau")} onclick={() => (params.dabs = params.dabs.slice(0, strokeStarts.pop() ?? 0))}>↶</button>
    </div>

    <div class="view">
      <div class="line top">
        <div class="segmented">
          {#each [["original", "Original"], ["channel", "Couche"], ["cut", "Détouré"], ["mask", "Masque"], ["overlay", "Calque rouge"]] as const as [id, label] (id)}
            <button type="button" class:on={view === id} onclick={() => (view = id)}>{t(label)}</button>
          {/each}
        </div>
        <div class="spacer"></div>
        {#if view === "cut"}
          <select bind:value={backdrop} aria-label={t("Fond")}>
            <option value="checker">{t("Fond : damier")}</option>
            <option value="black">{t("Fond : noir")}</option>
            <option value="white">{t("Fond : blanc")}</option>
          </select>
        {/if}
        <label class="brush">{t("Pinceau")} <input type="range" min="4" max="400" bind:value={brush} /> <output>{brush} px</output></label>
      </div>
      <div class="frame" bind:this={frameEl} style:width="{FRAME.width}px" style:height="{FRAME.height}px" onwheel={onWheel} role="presentation">
        {#if info}
          <canvas
            bind:this={canvas}
            class:checker={view === "cut" && backdrop === "checker"}
            class:pixelated={zoom * fit > 1.5}
            class:grab={spaceHeld}
            style:left="{pan.x}px"
            style:top="{pan.y}px"
            style:width="{shownW}px"
            style:height="{shownH}px"
            onpointerdown={down}
            onpointermove={moved}
            onpointerup={() => ((stroking = false), (panning = null))}
            onpointerleave={() => (cursor = null)}
          ></canvas>
          {#if cursor && tool !== "pick" && !spaceHeld}
            <div class="cursor" style:left="{cursor.x}px" style:top="{cursor.y}px" style:width="{brushOnScreen}px" style:height="{brushOnScreen}px"></div>
          {/if}
          <div class="zoombar" role="toolbar" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
            <button type="button" aria-label={t("Zoom arrière")} onclick={() => zoomAt(zoom / 1.5)}>−</button>
            <output>{Math.round(zoom * fit * 100)} %</output>
            <button type="button" aria-label={t("Zoom avant")} onclick={() => zoomAt(zoom * 1.5)}>+</button>
            <button type="button" class="text" onclick={() => ((zoom = 1), (pan = clampPan({ x: 0, y: 0 })))}>{t("Ajuster")}</button>
          </div>
        {/if}
        {#if busy}<p class="state">{t("Analyse de l'image…")}</p>{/if}
        {#if failure}<p class="state error">{failure}</p>{/if}
      </div>
      <div class="channels">
        {#each CHANNEL_NAMES as name, i (name)}
          <button type="button" class="ch" class:on={params.method === "channel" && channelIndex === i} onclick={() => ((params.method = "channel"), chooseChannel(i))}>
            <canvas bind:this={thumbs[i]}></canvas>
            <span>{t(name)}{#if i === best} <b class="star" title={t("Meilleur contraste")}>★</b>{/if}</span>
            <span class="score">{info ? `${Math.round((info.scores[i]?.score ?? 0) * 100)} %` : ""}</span>
          </button>
        {/each}
      </div>
    </div>

    <aside>
      <section>
        <h4>{t("Image analysée")}</h4>
        <select value={wholeDocument ? "all" : "layer"} onchange={(e) => ((wholeDocument = e.currentTarget.value === "all"), (params.dabs = []), (strokeStarts = []), void load())}>
          <option value="layer" disabled={!layerChoice}>{t("Calque actif")}{layerChoice ? ` · ${layerChoice.name}` : ""}</option>
          <option value="all">{t("Tous les calques visibles")}</option>
        </select>
      </section>
      <section>
        <h4>{t("Méthode")}</h4>
        <div class="segmented">
          <button type="button" disabled title={t("Détourage automatique par intelligence artificielle : dans une prochaine version")}>{t("Sujet")}</button>
          <button type="button" class:on={params.method === "color"} onclick={() => ((params.method = "color"), (tool = "pick"))}>{t("Couleur")}</button>
          <button type="button" class:on={params.method === "channel"} onclick={() => ((params.method = "channel"), tool === "pick" && (tool = "refine"))}>{t("Couche")}</button>
          <button type="button" class:on={params.method === "tones"} onclick={() => ((params.method = "tones"), tool === "pick" && (tool = "refine"))}>{t("Plage de tons")}</button>
        </div>
      </section>

      {#if params.method === "channel"}
        <section>
          <h4>{t("Couche")} {#if info}<span class="auto">· {t("auto : {name}, meilleur contraste", { name: t(CHANNEL_NAMES[best]!) })}</span>{/if}</h4>
          <label class="line">
            <span>{t("Couche source")}</span>
            <select value={String(channelIndex)} onchange={(e) => chooseChannel(e.currentTarget.value === "custom" ? "custom" : Number(e.currentTarget.value))}>
              {#each CHANNEL_NAMES as name, i (name)}<option value={String(i)}>{t(name)}{i === best ? ` (${t("auto")})` : ""}</option>{/each}
              <option value="custom">{t("Mélange personnalisé")}</option>
            </select>
          </label>
          {#if channelIndex === "custom"}
            {#each ["R", "V", "B"] as letter, c (letter)}
              <label class="line">
                <span>{letter}</span>
                <input type="range" min="-200" max="200" step="1" bind:value={params.mix[c]} />
                <output>{params.mix[c]} %</output>
              </label>
            {/each}
          {/if}
        </section>
        <section>
          <h4>{t("Niveaux / seuil doux")}</h4>
          <div class="hist">
            {#each histogram as v, i (i)}<i style:height="{v * 100}%"></i>{/each}
          </div>
          <div class="ramp" bind:this={ramp}>
            <div class="gradient"></div>
            <button type="button" class="tri black" style:left={pct(params.black)} aria-label={t("Point noir")} onpointerdown={(e) => dragLevel(e, "black")}></button>
            <button type="button" class="tri gray" style:left={pct(gammaAt)} aria-label={t("Gamma")} onpointerdown={(e) => dragLevel(e, "gamma")}></button>
            <button type="button" class="tri white" style:left={pct(params.white)} aria-label={t("Point blanc")} onpointerdown={(e) => dragLevel(e, "white")}></button>
          </div>
          <div class="line">
            <span>{t("Noir / Blanc")}</span>
            <output class="left">{params.black} · {String(params.gamma).replace(".", ",")} · {params.white}</output>
          </div>
          <label class="check"><input type="checkbox" bind:checked={params.invert} /> {t("Inverser (sujet sombre)")}</label>
        </section>
      {:else if params.method === "color"}
        <section>
          <h4>{t("Couleurs prélevées")}</h4>
          <div class="swatches">
            {#each params.colors as color, i (i)}<button type="button" class="swatch" style:background={hex(color)} title={t("Retirer")} onclick={() => (params.colors = params.colors.filter((_, j) => j !== i))}></button>{/each}
            {#if params.colors.length === 0}<span class="muted">{t("Cliquez dans l'image avec la pipette (Maj : ajouter).")}</span>{/if}
          </div>
          {@render slider("Tolérance", "fuzziness", 1, 200, 1, "")}
          <label class="check"><input type="checkbox" bind:checked={params.invert} /> {t("Inverser")}</label>
        </section>
      {:else}
        <section>
          <h4>{t("Plage de tons")}</h4>
          <div class="presets">
            <button type="button" onclick={() => ([params.toneLow, params.toneHigh] = [0, 85])}>{t("Ombres")}</button>
            <button type="button" onclick={() => ([params.toneLow, params.toneHigh] = [85, 170])}>{t("Tons moyens")}</button>
            <button type="button" onclick={() => ([params.toneLow, params.toneHigh] = [170, 255])}>{t("Hautes lumières")}</button>
          </div>
          {@render slider("De", "toneLow", 0, 255, 1, "")}
          {@render slider("À", "toneHigh", 0, 255, 1, "")}
          {@render slider("Adoucir", "toneSoft", 0, 100, 1, "")}
          <label class="check"><input type="checkbox" bind:checked={params.invert} /> {t("Inverser")}</label>
        </section>
      {/if}

      <section>
        <h4>{t("Affiner les contours")}</h4>
        {@render slider("Rayon intelligent", "radius", 0, 60, 1, " px")}
        {@render slider("Lissage", "smooth", 0, 10, 1, "")}
        {@render slider("Contour progressif", "feather", 0, 40, 0.5, " px")}
        {@render slider("Décaler le contour", "shift", -100, 100, 1, " %")}
        <label class="check"><input type="checkbox" bind:checked={params.decontaminate} disabled={output !== "newLayer"} /> {t("Décontaminer les couleurs")}</label>
        {#if params.decontaminate && output === "newLayer"}
          <label class="line">
            <span>{t("Intensité")}</span>
            <input type="range" min="0" max="1" step="0.05" bind:value={params.amount} />
            <output>{Math.round(params.amount * 100)} %</output>
          </label>
        {/if}
      </section>

      <section>
        <h4>{t("Sortie")}</h4>
        <select bind:value={output}>
          <option value="selection">{t("Sélection")}</option>
          <option value="layerMask" disabled={wholeDocument || !layerChoice}>{t("Masque de calque")}</option>
          <option value="newLayer">{t("Nouveau calque avec masque")}</option>
        </select>
        {#if output !== "newLayer"}<p class="muted small">{t("La décontamination des couleurs s'applique à un nouveau calque.")}</p>{/if}
      </section>
    </aside>
  </div>
  <footer>
    <span class="muted">{t("Aperçu en direct sur une image réduite · OK : image entière")}</span>
    <div class="spacer"></div>
    <button type="button" class="ghost" onclick={reset}>{t("Réinitialiser")}</button>
    <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
    <button type="button" class="pill" disabled={!info} onclick={() => void confirm()}>OK</button>
  </footer>
</dialog>

<style>
  dialog {
    width: min(1180px, calc(100vw - 32px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 16px;
  }
  header {
    height: 46px;
    border-bottom: 1px solid var(--border);
  }
  footer {
    height: 54px;
    border-top: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  h4 {
    margin: 0 0 8px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .auto {
    text-transform: none;
    letter-spacing: 0;
    color: var(--accent, #e5343e);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    margin: 6px 0 0;
    font-size: 11px;
  }
  .spacer {
    flex: 1;
  }
  .icon {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .body {
    display: flex;
    max-height: calc(100vh - 140px);
  }
  .tools {
    width: 44px;
    padding: 8px 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    border-right: 1px solid var(--border);
  }
  .tools button {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-m);
    color: var(--text-2);
    font-size: 15px;
  }
  .tools button.on {
    background: var(--accent, #e5343e);
    color: var(--on-accent);
  }
  .tools button:disabled {
    opacity: 0.35;
  }
  .tools .sep {
    width: 22px;
    height: 1px;
    background: var(--border);
  }
  .view {
    flex: 1;
    min-width: 0;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .line > span:first-child {
    width: 128px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  .line input[type="range"] {
    flex: 1;
    min-width: 0;
  }
  .line output {
    width: 54px;
    text-align: right;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .line output.left {
    width: auto;
    text-align: left;
    color: var(--text);
  }
  .brush {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-3);
  }
  .brush input {
    width: 100px;
  }
  .frame canvas {
    position: absolute;
  }
  canvas.pixelated {
    image-rendering: pixelated;
  }
  canvas.grab {
    cursor: grab !important;
  }
  .zoombar {
    position: absolute;
    right: 8px;
    bottom: 8px;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: rgba(20, 20, 22, 0.9);
    border: 1px solid var(--border-strong);
  }
  .zoombar button {
    min-width: 26px;
    height: 24px;
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .zoombar .text {
    padding: 0 8px;
  }
  .zoombar output {
    min-width: 46px;
    text-align: center;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .frame {
    position: relative;
    border-radius: var(--radius-m);
    background: var(--bg-well);
    overflow: hidden;
    max-width: 100%;
  }
  canvas.checker {
    background: repeating-conic-gradient(#3a3a3e 0% 25%, #2a2a2e 0% 50%) 0 0 / 14px 14px;
  }
  .frame canvas {
    cursor: none;
    touch-action: none;
  }
  .cursor {
    position: absolute;
    border-radius: 50%;
    border: 1.5px solid #fff;
    box-shadow: 0 0 0 1px #000;
    transform: translate(-50%, -50%);
    pointer-events: none;
  }
  .state {
    position: absolute;
    margin: 0;
    color: var(--text-2);
  }
  .state.error {
    bottom: 10px;
    color: var(--danger, #ff6b6b);
  }
  .channels {
    display: flex;
    gap: 10px;
  }
  .ch {
    width: 150px;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--bg-field);
    text-align: left;
  }
  .ch.on {
    border-color: var(--accent, #e5343e);
  }
  .ch canvas {
    width: 100%;
    height: 84px;
    object-fit: cover;
  }
  .ch span {
    padding: 2px 8px;
    font-size: 11px;
    color: var(--text-3);
  }
  .ch .score {
    padding-top: 0;
    padding-bottom: 4px;
    color: var(--text-muted);
  }
  .star {
    color: var(--accent, #e5343e);
  }
  aside {
    width: 400px;
    overflow-y: auto;
    border-left: 1px solid var(--border);
  }
  section {
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  section select {
    width: 100%;
  }
  .line select {
    flex: 1;
    width: auto;
  }
  select {
    height: 28px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-l);
    background: var(--bg-well);
    border: 1px solid var(--border);
  }
  .segmented button {
    flex: 1;
    height: 28px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .segmented button.on {
    background: var(--accent, #e5343e);
    color: var(--on-accent);
  }
  .segmented button:disabled {
    opacity: 0.4;
  }
  .hist {
    height: 50px;
    display: flex;
    align-items: flex-end;
    gap: 1px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg-well);
  }
  .hist i {
    flex: 1;
    background: #6a6a72;
  }
  .ramp {
    position: relative;
    height: 26px;
    margin-top: 4px;
  }
  .gradient {
    height: 10px;
    border-radius: 3px;
    background: linear-gradient(90deg, #000, #fff);
    border: 1px solid #333;
  }
  .tri {
    position: absolute;
    top: 12px;
    width: 12px;
    height: 11px;
    margin-left: -6px;
    padding: 0;
    clip-path: polygon(50% 0, 100% 100%, 0 100%);
    cursor: ew-resize;
    background: #aaa;
  }
  .tri.black {
    background: var(--bg-well);
    filter: drop-shadow(0 0 1px #ccc);
  }
  .tri.white {
    background: #f2f2f2;
  }
  .tri.gray {
    background: #888;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
    color: var(--text-2);
  }
  .swatches {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }
  .swatch {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid rgb(var(--ink) / 0.4);
  }
  .presets {
    display: flex;
    gap: 6px;
    margin-bottom: 6px;
  }
  .presets button {
    flex: 1;
    height: 26px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    color: var(--text-2);
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
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .pill:disabled {
    opacity: 0.5;
  }
</style>
