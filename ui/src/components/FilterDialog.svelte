<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { untrack } from "svelte";
  import { edit } from "../lib/engine";
  import { specOf } from "../lib/filters";
  import { type BlurPin, type Filter, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";

  // Fenêtre d'un filtre, au centre de l'écran : à gauche un aperçu du calque filtré, à droite les
  // réglages. L'aperçu est calculé par le moteur sur une réduction du calque : il suit les curseurs
  // sans attendre, quelle que soit la taille de l'image, et ne touche pas au document. L'image
  // elle-même n'est filtrée qu'à la validation.
  let { kind: kindProp }: { kind: Filter["type"] } = $props();
  // Figé à l'ouverture : la propriété vient de l'état global, qui repasse à « aucun filtre » au
  // moment même où l'on valide. La lire à ce moment-là envoyait au moteur un filtre sans type,
  // qu'il refusait : rien n'était appliqué.
  const kind = untrack(() => kindProp);

  const PREVIEW_SIDE = 760;
  const MAX_FIELD_BLUR = 100;
  const spec = untrack(() => specOf(kind));
  const docId = untrack(() => app.activeId);
  const layerId = untrack(() => app.activeLayer?.id ?? null);

  let dialog = $state<HTMLDialogElement>();
  let canvas = $state<HTMLCanvasElement>();
  let values = $state<Record<string, number | boolean | string>>(Object.fromEntries([...(spec?.params ?? []), ...(spec?.toggles ?? []), ...(spec?.choices ?? [])].map((p) => [p.key, p.value])));

  // Filtre Verre : texture chargée par l'utilisateur (une image dont la clarté sert de relief).
  let loadedTexture = $state<string | null>(null);
  async function loadTexture(): Promise<void> {
    try {
      const path = await open({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"] }] });
      if (typeof path !== "string") return;
      const [name, width, height] = await api.loadGlassTexture(path);
      loadedTexture = `${name} · ${width} × ${height} px`;
      values.texture = "custom";
      // Même réglage, nouvelle texture : l'aperçu doit être refait.
      textureSerial += 1;
    } catch (error) {
      failure = String(error);
    }
  }
  let textureSerial = $state(0);
  // Flou de profondeur de champ : une pastille au centre pour commencer, comme dans Photoshop.
  let pins = $state<BlurPin[]>([{ x: 0.5, y: 0.5, blur: 15 }]);
  let selectedPin = $state(0);
  let draggingPin = $state(false);
  /** Réglage du flou en tournant la bague de la pastille. */
  let turningRing = $state(false);
  let failure = $state<string | null>(null);
  let size = $state({ width: 3, height: 2 });
  /** Place disponible pour l'aperçu ; le cadre y tient en entier, aux proportions du calque. */
  let room = $state({ width: 0, height: 0 });
  const frame = $derived.by(() => {
    const [w, h] = [Math.max(0, room.width - 36), Math.max(0, room.height - 36)];
    const k = Math.min(w / size.width, h / size.height);
    return { width: Math.floor(size.width * k), height: Math.floor(size.height * k) };
  });

  const filter = $derived((kind === "fieldBlur" ? { type: kind, pins } : { type: kind, ...values }) as Filter);

  $effect(() => {
    dialog?.showModal();
  });

  // ---- Zoom et déplacement dans l'aperçu : molette pour zoomer sous le pointeur, bouton du milieu (ou
  // Espace + glisser) pour se déplacer. En zoomant, l'aperçu est recalculé plus finement.
  let view = $state({ zoom: 1, x: 0, y: 0 });
  let panning: { x: number; y: number } | null = null;
  let spaceHeld = $state(false);
  /** Finesse demandée au moteur : elle suit le zoom, par paliers pour ne pas tout recalculer à chaque cran de molette. */
  /**
   * Finesse demandée au moteur : la taille RÉELLE du cadre à l'écran (pixels physiques) multipliée par le zoom de
   * l'aperçu, arrondie par paliers de 256 px pour ne pas tout recalculer au moindre changement. Un aperçu calculé
   * plus petit que le cadre était agrandi à l'affichage : il paraissait pixélisé, surtout dans la grande fenêtre
   * du flou de profondeur de champ. Plafond : 4 096 px (et jamais plus que le calque lui-même, côté moteur),
   * ramené à 2 048 px pour le flou de profondeur de champ, dont chaque aperçu floute l'image sept fois :
   * au-delà, la mémoire demandée grimpe sans que l'œil y gagne quoi que ce soit.
   */
  const previewSide = $derived.by(() => {
    const shown = Math.max(frame.width, frame.height, 1) * (window.devicePixelRatio || 1) * view.zoom;
    const ceiling = kind === "fieldBlur" ? 2048 : 4096;
    return Math.min(ceiling, Math.max(PREVIEW_SIDE, Math.ceil(shown / 256) * 256));
  });
  let loading = $state(false);

  function zoomAt(factor: number, px: number, py: number): void {
    const zoom = Math.min(8, Math.max(1, view.zoom * factor));
    const k = zoom / view.zoom;
    // Le point sous le pointeur reste sous le pointeur ; à 100 % l'aperçu se recentre.
    view = zoom === 1 ? { zoom: 1, x: 0, y: 0 } : { zoom, x: px - (px - view.x) * k, y: py - (py - view.y) * k };
  }

  function onStageWheel(event: WheelEvent): void {
    event.preventDefault();
    // Flou à pastilles : Alt + molette règle le flou de la pastille sélectionnée ; la molette seule zoome, comme partout.
    if (kind === "fieldBlur" && event.altKey) return onWheel(event);
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    zoomAt(Math.exp(-event.deltaY * 0.0015), event.clientX - rect.left - rect.width / 2, event.clientY - rect.top - rect.height / 2);
  }

  function onStageDown(event: PointerEvent): void {
    if (event.button !== 1 && !(event.button === 0 && spaceHeld)) return;
    event.preventDefault();
    event.stopPropagation();
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    panning = { x: event.clientX, y: event.clientY };
  }

  function onStageMove(event: PointerEvent): void {
    if (panning) {
      view = { ...view, x: view.x + event.clientX - panning.x, y: view.y + event.clientY - panning.y };
      panning = { x: event.clientX, y: event.clientY };
      return;
    }
    dragPin(event);
  }

  // Aperçu dans la fenêtre : **un seul calcul à la fois**. En déplaçant une pastille, l'ancienne
  // façon lançait un calcul par mouvement : les réponses dépassées étaient ignorées, mais le moteur,
  // lui, les calculait toutes en même temps — sur un gros flou de profondeur de champ, plusieurs
  // gigaoctets s'additionnaient et l'application était tuée par le système. Désormais, un mouvement
  // pendant un calcul ne fait que **remplacer la demande en attente** : c'est la dernière qui part.
  type PreviewJob = { filter: Filter; side: number; target: HTMLCanvasElement };
  let queued: PreviewJob | null = null;
  let running = false;

  async function pumpPreview(): Promise<void> {
    if (running || docId === null || layerId === null) return;
    running = true;
    loading = true;
    while (queued) {
      const job = queued;
      queued = null;
      try {
        const buffer = await api.filterPreview(docId, layerId, job.filter, job.side);
        const header = new DataView(buffer);
        const [width, height] = [header.getUint32(0, true), header.getUint32(4, true)];
        job.target.width = width;
        job.target.height = height;
        job.target.getContext("2d")?.putImageData(new ImageData(new Uint8ClampedArray(buffer, 8), width, height), 0, 0);
        [size, failure] = [{ width, height }, null];
      } catch (error: unknown) {
        failure = String(error);
      }
    }
    [running, loading] = [false, false];
  }

  $effect(() => {
    const current = $state.snapshot(filter) as Filter;
    void textureSerial; // une nouvelle texture chargée relance l'aperçu, même à réglages identiques
    const target = canvas;
    const side = previewSide;
    if (docId === null || layerId === null || !target) return;
    queued = { filter: current, side, target };
    void pumpPreview();
  });

  function close(apply: boolean): void {
    // Le filtre est relevé AVANT de fermer la fenêtre (voir plus haut).
    const chosen = $state.snapshot(filter) as Filter;
    app.filterDialog = null;
    if (apply && layerId !== null) void edit({ op: "applyFilter", id: layerId, filter: chosen });
  }

  const format = (value: number, step: number) => (step < 1 ? value.toFixed(1) : String(Math.round(value)));

  // ---- Pastilles du flou de profondeur de champ

  function pinPosition(event: PointerEvent): { x: number; y: number } {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return { x: 0.5, y: 0.5 };
    return { x: Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)), y: Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height)) };
  }

  function addPin(event: PointerEvent): void {
    if (kind !== "fieldBlur" || event.button !== 0) return;
    // La nouvelle pastille reprend la force de la pastille sélectionnée : on ajuste ensuite.
    pins.push({ ...pinPosition(event), blur: pins[selectedPin]?.blur ?? 15 });
    selectedPin = pins.length - 1;
    startDrag(event);
  }

  function startDrag(event: PointerEvent): void {
    draggingPin = true;
    (event.currentTarget as Element).closest(".stage")?.setPointerCapture(event.pointerId);
  }

  function dragPin(event: PointerEvent): void {
    const pin = pins[selectedPin];
    if (!pin) return;
    if (turningRing) pin.blur = blurFromAngle(event, pin);
    else if (draggingPin) Object.assign(pin, pinPosition(event));
  }

  /**
   * Comme dans Photoshop : la bague se tourne. L'angle du pointeur autour de la pastille, compté
   * dans le sens horaire depuis le haut, donne la force du flou (un tour complet = le maximum).
   */
  function blurFromAngle(event: PointerEvent, pin: BlurPin): number {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return pin.blur;
    const [cx, cy] = [rect.left + pin.x * rect.width, rect.top + pin.y * rect.height];
    const turn = (Math.atan2(event.clientX - cx, cy - event.clientY) / (2 * Math.PI) + 1) % 1;
    // Près du haut, l'angle saute de 100 % à 0 % : on reste du côté où l'on était.
    const previous = pin.blur / MAX_FIELD_BLUR;
    const held = Math.abs(turn - previous) > 0.6 ? (previous > 0.5 ? 1 : 0) : turn;
    return Math.round(held * MAX_FIELD_BLUR);
  }

  /** Molette au-dessus de l'aperçu : règle le flou de la pastille sélectionnée, par pas de 2. */
  function onWheel(event: WheelEvent): void {
    const pin = pins[selectedPin];
    if (kind !== "fieldBlur" || !pin) return;
    event.preventDefault();
    pin.blur = Math.min(MAX_FIELD_BLUR, Math.max(0, pin.blur - Math.sign(event.deltaY) * 2));
  }

  function removePin(): void {
    if (pins.length <= 1) return;
    pins.splice(selectedPin, 1);
    selectedPin = Math.max(0, selectedPin - 1);
  }

  /** Arc de la bague : sa longueur montre la force du flou de la pastille. */
  const ring = (blur: number) => {
    const circumference = 2 * Math.PI * 22;
    return `${(Math.min(blur, MAX_FIELD_BLUR) / MAX_FIELD_BLUR) * circumference} ${circumference}`;
  };
</script>

<svelte:window
  onkeydown={(event) => event.code === "Space" && !(event.target instanceof HTMLInputElement) && ((spaceHeld = true), event.preventDefault())}
  onkeyup={(event) => event.code === "Space" && (spaceHeld = false)}
/>

<dialog bind:this={dialog} class:large={kind === "fieldBlur"} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <header>
      <h2>{spec?.label ?? "Filtre"}</h2>
      <span>{spec?.hint ?? ""}</span>
    </header>

    <div class="body">
      <div
        class="stage"
        class:pins={kind === "fieldBlur"}
        class:grab={spaceHeld}
        role="presentation"
        bind:clientWidth={room.width}
        bind:clientHeight={room.height}
        onpointerdowncapture={onStageDown}
        onpointermove={onStageMove}
        onpointerup={() => ((draggingPin = false), (turningRing = false), (panning = null))}
        onpointercancel={() => ((draggingPin = false), (turningRing = false), (panning = null))}
        onwheel={onStageWheel}
      >
        <!-- Barre de chargement discrète, la même que celle de la bannière de lancement. -->
        <div class="loading" class:on={loading} aria-hidden="true"><div></div></div>
        <div class="frame" style:width="{frame.width * view.zoom}px" style:height="{frame.height * view.zoom}px" style:transform="translate({view.x}px, {view.y}px)">
          <canvas bind:this={canvas} onpointerdown={addPin}></canvas>
          {#if kind === "fieldBlur"}
            {#each pins as pin, index (index)}
              <button
                type="button"
                class="pin"
                draggable="false"
                ondragstart={(event) => event.preventDefault()}
                class:on={index === selectedPin}
                style:left="{pin.x * 100}%"
                style:top="{pin.y * 100}%"
                aria-label="Pastille {index + 1}, flou {Math.round(pin.blur)} pixels"
                aria-pressed={index === selectedPin}
                onpointerdown={(event) => {
                  event.stopPropagation();
                  selectedPin = index;
                  startDrag(event);
                }}
                onkeydown={(event) => (event.key === "Delete" || event.key === "Backspace") && removePin()}
              >
                {#if index === selectedPin}
                  <!-- Bague : on la saisit et on tourne pour régler le flou. -->
                  <svg viewBox="0 0 56 56" aria-hidden="true">
                    <circle
                      cx="28"
                      cy="28"
                      r="22"
                      class="ring-back"
                      role="presentation"
                      onpointerdown={(event) => {
                        event.stopPropagation();
                        turningRing = true;
                        (event.currentTarget as Element).closest(".stage")?.setPointerCapture(event.pointerId);
                      }}
                    />
                    <circle cx="28" cy="28" r="22" class="ring" stroke-dasharray={ring(pin.blur)} />
                  </svg>
                  <span class="badge">Flou : {Math.round(pin.blur)}</span>
                {/if}
              </button>
            {/each}
          {/if}
        </div>
        <div class="zoombar" role="presentation" onpointerdown={(event) => event.stopPropagation()}>
          <button type="button" aria-label={t("Zoom arrière")} onclick={() => zoomAt(0.5, 0, 0)}>−</button>
          <output>{Math.round(view.zoom * 100)} %</output>
          <button type="button" aria-label={t("Zoom avant")} onclick={() => zoomAt(2, 0, 0)}>+</button>
          <button type="button" class="text" onclick={() => (view = { zoom: 1, x: 0, y: 0 })}>{t("Ajuster")}</button>
        </div>
        {#if failure}<p class="failure">{failure}</p>{/if}
      </div>

      <aside>
        {#if kind === "fieldBlur"}
          {@const pin = pins[selectedPin]}
          {#if pin}
            <label>
              <span>{t("Flou de la pastille")}</span>
              <output>{Math.round(pin.blur)} px</output>
              <input type="range" min="0" max={MAX_FIELD_BLUR} step="1" value={pin.blur} oninput={(event) => (pin.blur = event.currentTarget.valueAsNumber)} />
            </label>
          {/if}
          <button type="button" class="ghost" disabled={pins.length <= 1} onclick={removePin}>{t("Supprimer la pastille")}</button>
          <p>{t("Cliquez dans l'aperçu pour poser une pastille, glissez-la pour la déplacer. Tournez sa bague (ou la molette de la souris) pour régler le flou. Une seule pastille floute toute l'image ; avec plusieurs, le flou varie progressivement de l'une à l'autre. Une pastille à 0 garde sa zone nette.")}</p>
        {/if}
        {#each spec?.params ?? [] as param (param.key)}
          <label>
            <span>{t(param.label)}</span>
            <output>{format(Number(values[param.key]), param.step)}{param.unit}</output>
            <input type="range" min={param.min} max={param.max} step={param.step} value={Number(values[param.key])} oninput={(event) => (values[param.key] = event.currentTarget.valueAsNumber)} />
          </label>
        {/each}
        {#if app.activeLayer?.shape || app.activeLayer?.text}
          <p class="notice">Ce calque est {app.activeLayer.shape ? t("une forme") : t("un texte")} : il sera pixellisé pour recevoir le filtre, et ne sera plus réglable ensuite. Ctrl+Z annule le tout.</p>
        {/if}
        {#each spec?.choices ?? [] as choice (choice.key)}
          <label class="choice">
            <span>{t(choice.label)}</span>
            <select bind:value={values[choice.key]}>
              {#each choice.options as option (option.value)}
                <option value={option.value}>{t(option.label)}</option>
              {/each}
            </select>
          </label>
        {/each}
        {#if kind === "glass"}
          <button type="button" class="ghost load" onclick={loadTexture}>{t("Charger une texture…")}</button>
          <p>{loadedTexture ?? t("Toute image convient : ses zones claires forment les bosses du verre, ses zones sombres les creux. Elle est répétée pour couvrir l'image.")}</p>
        {/if}
        {#each spec?.toggles ?? [] as toggle (toggle.key)}
          <label class="check"><input type="checkbox" checked={values[toggle.key] === true} onchange={(event) => (values[toggle.key] = event.currentTarget.checked)} /> {t(toggle.label)}</label>
        {/each}
        <div class="spacer"></div>
        <p>{app.active?.info.hasSelection ? t("Sera appliqué à la zone sélectionnée du calque actif.") : t("Sera appliqué à tout le calque actif.")}</p>
      </aside>
    </div>

    <footer>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill">OK</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: min(1040px, calc(100vw - 48px));
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
    height: min(560px, calc(100vh - 220px));
  }
  .stage {
    flex: 1;
    min-width: 0;
    display: grid;
    place-items: center;
    padding: 18px;
    background: var(--bg-canvas);
    touch-action: none;
  }
  /* Le cadre épouse les proportions du calque et tient dans la zone d'aperçu. */
  .frame {
    position: relative;
    background: repeating-conic-gradient(#d0d0d0 0% 25%, #ffffff 0% 50%) 0 0 / 16px 16px;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.5);
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
  }
  .stage.pins canvas {
    cursor: copy;
  }
  .pin {
    position: absolute;
    width: 16px;
    height: 16px;
    margin: -8px 0 0 -8px;
    padding: 0;
    border-radius: 50%;
    background: #fff;
    border: 2px solid var(--accent);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.5);
    cursor: grab;
  }
  .pin.on {
    background: var(--accent);
    border-color: #fff;
  }
  .pin svg {
    position: absolute;
    left: -22px;
    top: -22px;
    width: 56px;
    height: 56px;
    transform: rotate(-90deg);
    fill: none;
    pointer-events: none;
    overflow: visible;
  }
  /* Piste de la bague : large pour être saisie facilement ; seul son trait réagit, pas son centre. */
  .ring-back {
    stroke: rgba(0, 0, 0, 0.45);
    stroke-width: 9;
    pointer-events: stroke;
    cursor: ew-resize;
  }
  .ring {
    stroke: #fff;
    stroke-width: 3.5;
    stroke-linecap: round;
    pointer-events: none;
  }
  .badge {
    position: absolute;
    left: 50%;
    bottom: 42px;
    transform: translateX(-50%);
    padding: 3px 8px;
    border-radius: var(--radius-s);
    background: rgba(0, 0, 0, 0.72);
    color: #fff;
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
  aside {
    width: 290px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px 20px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px;
    color: var(--text-3);
  }
  label.check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  input[type="range"] {
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
  .ghost:disabled {
    opacity: 0.35;
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
  .choice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    color: var(--text-3);
  }
  .choice select {
    height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  .load {
    align-self: flex-start;
    height: 28px;
    padding: 0 12px;
  }
  .notice {
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: rgba(255, 170, 60, 0.12);
    border: 1px solid rgba(255, 170, 60, 0.4);
    color: #ffd9a8;
  }
  .stage {
    position: relative;
    overflow: hidden;
  }
  .stage.grab {
    cursor: grab;
  }
  .frame {
    flex-shrink: 0;
  }
  .pin {
    -webkit-user-drag: none;
    user-select: none;
  }
  .loading {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    overflow: hidden;
    opacity: 0;
    transition: opacity 150ms var(--ease) 120ms; /* les calculs très courts ne font pas clignoter la barre */
    z-index: 2;
  }
  .loading.on {
    opacity: 1;
    background: rgba(255, 255, 255, 0.14);
  }
  .loading div {
    width: 40%;
    height: 100%;
    background: linear-gradient(90deg, transparent, #fff, rgba(120, 140, 255, 0.95), transparent);
    animation: sweep 1.1s linear infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
  .zoombar {
    position: absolute;
    left: 50%;
    bottom: 12px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px 6px 3px 3px;
    border-radius: var(--radius-pill);
    background: rgba(15, 15, 17, 0.85);
    border: 1px solid var(--border);
    color: var(--text-2);
    z-index: 2;
    white-space: nowrap;
  }
  .zoombar button {
    width: 26px;
    height: 24px;
    border-radius: var(--radius-pill);
    color: var(--text);
  }
  .zoombar button.text {
    width: auto;
    padding: 0 10px;
    color: var(--text-2);
  }
  .zoombar button:hover {
    background: var(--bg-hover);
  }
  .zoombar output {
    min-width: 52px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  /* Flou de profondeur de champ : on place les pastilles au pixel près, la fenêtre prend presque tout l'écran,
     centrée (marges égales tout autour). */
  dialog.large {
    width: calc(100vw - 64px);
    max-width: none;
    margin: auto;
  }
  dialog.large .body {
    height: calc(100vh - 64px - 54px - 58px);
  }
</style>
