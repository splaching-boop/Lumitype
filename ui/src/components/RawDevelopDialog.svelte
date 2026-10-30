<script lang="ts">
  // Fenêtre « Développement RAW » : un RAW ouvert ou importé passe ici avant de devenir un document, un calque ou un
  // objet dynamique. Le moteur garde la lumière linéaire du capteur (session `opened.id`) ; chaque réglage redemande
  // l'aperçu réduit, au plus une fois par image d'écran, et seule la dernière demande compte.
  import { onDestroy, onMount } from "svelte";
  import { i18n, t } from "../lib/i18n.svelte";
  import { api, type RawClipView, type RawParams } from "../lib/ipc";
  import { app, type RawDevelopRequest } from "../state/app.svelte";

  const { request }: { request: RawDevelopRequest } = $props();
  // Une fenêtre par RAW (`{#key}` dans App.svelte) : la demande ne change pas pendant la vie du composant.
  // svelte-ignore state_referenced_locally
  const { opened, mode } = request;
  const info = opened.info;
  /**
   * Filtre › Filtre Camera Raw : la session est faite des pixels du calque actif (déjà en sRGB). Pas d'EXIF ni de
   * préréglages d'éclairage : la balance « telle quelle » vaut 6 500 K / 0 (le calque inchangé) et la température se
   * déplace relativement à elle (voir `engine::raw::layer_calibration`).
   */
  const filter = mode === "filter";
  // svelte-ignore state_referenced_locally
  const target = filter ? app.activeLayer : null;
  // svelte-ignore state_referenced_locally
  const inSelection = filter && (app.active?.info.hasSelection ?? false);

  type NumericKey = Exclude<keyof RawParams, "whiteBalance">;
  type Preset = "asShot" | "auto" | "daylight" | "cloudy" | "shade" | "tungsten" | "fluorescent" | "flash" | "custom";
  /** Préréglages de balance : température et teinte (celles de Lightroom, à peu près). */
  const PRESETS: Record<Exclude<Preset, "asShot" | "auto" | "custom">, [number, number]> = {
    daylight: [5500, 10],
    cloudy: [6500, 10],
    shade: [7500, 10],
    tungsten: [2850, 0],
    fluorescent: [3800, 21],
    flash: [5500, 0],
  };

  const NEUTRAL: RawParams = {
    whiteBalance: "asShot",
    temperature: Math.round(info.asShotTemperature),
    tint: Math.round(info.asShotTint),
    exposure: 0,
    contrast: 0,
    highlights: 0,
    shadows: 0,
    whites: 0,
    blacks: 0,
    clarity: 0,
    vibrance: 0,
    saturation: 0,
    noiseLuma: 0,
    noiseColor: 0,
    sharpen: 0,
    vignette: 0,
  };

  /** Réglages de départ : ceux du RAW précédent de la série ; « telle quelle » et « auto » restent propres à ce fichier. */
  function initialParams(): RawParams {
    // svelte-ignore state_referenced_locally
    const from = request.initial;
    if (!from) return { ...NEUTRAL };
    const next = { ...from };
    if (next.whiteBalance === "asShot") [next.temperature, next.tint] = [NEUTRAL.temperature, NEUTRAL.tint];
    if (next.whiteBalance === "auto") [next.temperature, next.tint] = [Math.round(info.autoTemperature), Math.round(info.autoTint)];
    return next;
  }

  function presetOf(p: RawParams): Preset {
    if (p.whiteBalance !== "custom") return p.whiteBalance;
    if (filter) return "custom";
    const found = Object.entries(PRESETS).find(([, [k, n]]) => k === p.temperature && n === p.tint);
    return (found?.[0] as Preset | undefined) ?? "custom";
  }

  let params = $state<RawParams>(initialParams());
  let preset = $state<Preset>(presetOf(params));
  let view = $state<"after" | "before" | "split">("after");
  let clip = $state<RawClipView>("none");
  let picking = $state(false);
  let applyToRest = $state(false);
  /** « Redévelopper plus tard » : objet dynamique qui garde le RAW et ces réglages. */
  // svelte-ignore state_referenced_locally
  let keepRaw = $state(request.keepRaw);
  let afterUrl = $state<string | null>(null);
  let beforeUrl = $state<string | null>(null);
  let histogram = $state<Uint32Array | null>(null);
  let dialog = $state<HTMLDialogElement>();
  let done = false;
  let closed = false;

  // ------------------------------------------------------------------ aperçu

  /** Réponse de `raw_preview` : 8 octets de taille, 768 u32 d'histogramme, puis le JPEG. */
  function parse(buffer: ArrayBuffer): { url: string; histogram: Uint32Array } {
    const histogram = new Uint32Array(buffer.slice(8, 8 + 768 * 4));
    const url = URL.createObjectURL(new Blob([new Uint8Array(buffer, 8 + 768 * 4)], { type: "image/jpeg" }));
    return { url, histogram };
  }

  let dirty = true;
  let inFlight = false;
  let frame = 0;
  function schedule(): void {
    dirty = true;
    if (!frame) frame = requestAnimationFrame(pump);
  }
  /** Une demande à la fois, la plus récente : les réglages intermédiaires d'un glissement rapide sont sautés. */
  async function pump(): Promise<void> {
    frame = 0;
    if (!dirty || inFlight || closed) return;
    dirty = false;
    inFlight = true;
    try {
      const result = parse(await api.rawPreview(opened.id, $state.snapshot(params), clip));
      if (closed) return URL.revokeObjectURL(result.url);
      if (afterUrl) URL.revokeObjectURL(afterUrl);
      afterUrl = result.url;
      // L'histogramme est celui de l'image, pas de la vue des zones écrêtées.
      if (clip === "none") histogram = result.histogram;
    } catch (error) {
      app.fail(error);
    } finally {
      inFlight = false;
      if (dirty) schedule();
    }
  }

  $effect(() => {
    // Toute valeur lue ici (réglages, vue des zones écrêtées) relance l'aperçu.
    JSON.stringify(params);
    void clip;
    schedule();
  });

  onMount(() => {
    dialog?.showModal();
    // « Avant » : le développement par défaut, calculé une fois.
    void api
      .rawPreview(opened.id, NEUTRAL, "none")
      .then((buffer) => {
        const { url } = parse(buffer);
        if (closed) URL.revokeObjectURL(url);
        else beforeUrl = url;
      })
      .catch(() => undefined);
  });

  onDestroy(() => {
    closed = true;
    if (frame) cancelAnimationFrame(frame);
    clearTimeout(detailTimer);
    for (const url of [afterUrl, beforeUrl, details.after?.url, details.before?.url]) if (url) URL.revokeObjectURL(url);
    finish(null); // fermeture imprévue : la série continue, le RAW est libéré par l'appelant
  });

  // ------------------------------------------------------------------ zoom
  //
  // L'aperçu (1 600 px de côté au plus) est affiché à l'échelle `scale` (pixels d'écran CSS par pixel de l'image
  // pleine taille). Quand l'écran demande plus de pixels que l'aperçu n'en a, la zone visible est redéveloppée en
  // pleine définition (`raw_region`) et posée par-dessus : bruit et netteté se jugent alors sur les vrais pixels.

  const W = info.width;
  const H = info.height;
  const previewRatio = opened.previewWidth / W;
  /** Paliers de la liste et des boutons − / + (Ctrl+- / Ctrl++). */
  const LEVELS = [1 / 16, 1 / 8, 1 / 6, 1 / 4, 1 / 3, 1 / 2, 2 / 3, 1, 2, 3, 4, 6, 8];
  const MAX_ZOOM = 8;
  const GAP = 12;
  /** Marges de l'image en mode Ajuster : boutons et histogramme au-dessus. */
  const PAD = { top: 104, side: 24, bottom: 16 };
  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

  /** `null` = Ajuster (suit la taille de la fenêtre). */
  let zoom = $state<number | null>(null);
  /** Point de l'image (0 à 1) au centre de la vue. */
  let cx = $state(0.5);
  let cy = $state(0.5);
  let stageW = $state(0);
  let stageH = $state(0);
  let spaceDown = $state(false);
  let pan = $state<{ x: number; y: number; cx: number; cy: number; moved: boolean } | null>(null);

  const paneW = $derived(Math.max(1, view === "split" ? (stageW - GAP) / 2 : stageW));
  const paneH = $derived(Math.max(1, stageH));
  const fit = $derived(Math.max(0.005, Math.min((paneW - 2 * PAD.side) / W, (paneH - PAD.top - PAD.bottom) / H)));
  const minZoom = $derived(Math.min(fit, 1 / 16));
  const scale = $derived(zoom ?? fit);

  const clampN = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
  /** Rectangle de l'image dans un panneau : centrée si elle tient, sinon bornée pour ne pas laisser de vide. */
  function placement(s: number, x: number, y: number): { left: number; top: number; w: number; h: number } {
    const w = W * s;
    const h = H * s;
    const left = w <= paneW ? (paneW - w) / 2 : clampN(paneW / 2 - x * w, paneW - w, 0);
    const room = paneH - PAD.top - PAD.bottom;
    const top = h <= room ? PAD.top + (room - h) / 2 : h <= paneH ? (paneH - h) / 2 : clampN(paneH / 2 - y * h, paneH - h, 0);
    return { left, top, w, h };
  }
  const geo = $derived(placement(scale, cx, cy));
  const canPan = $derived(geo.w > paneW + 0.5 || geo.h > paneH + 0.5);

  /** Centre ramené à ce que la vue montre vraiment (bornes). */
  function setView(s: number, x: number, y: number): void {
    const g = placement(s, x, y);
    cx = (paneW / 2 - g.left) / g.w;
    cy = (paneH / 2 - g.top) / g.h;
  }
  /** Zoom autour du point `(px, py)` du panneau (le centre par défaut) : ce point de l'image reste sous le pointeur. */
  function zoomTo(next: number | null, px = paneW / 2, py = paneH / 2): void {
    if (next === null || Math.abs(next - fit) < 1e-4) {
      zoom = null;
      cx = cy = 0.5;
      return;
    }
    const s = clampN(next, minZoom, MAX_ZOOM);
    const u = (px - geo.left) / geo.w;
    const v = (py - geo.top) / geo.h;
    const x = (paneW / 2 - (px - u * W * s)) / (W * s);
    const y = (paneH / 2 - (py - v * H * s)) / (H * s);
    zoom = s;
    setView(s, x, y);
  }
  function step(direction: 1 | -1): void {
    const next = direction > 0 ? LEVELS.find((l) => l > scale * 1.001) : [...LEVELS].reverse().find((l) => l < scale * 0.999);
    zoomTo(next ?? (direction > 0 ? MAX_ZOOM : minZoom));
  }
  const percent = (s: number) => `${new Intl.NumberFormat(i18n.info.tag, { maximumFractionDigits: s < 0.995 ? 1 : 0 }).format(s * 100)} %`;
  /** Curseur de zoom : échelle logarithmique de `minZoom` à 800 %. */
  const sliderPos = $derived(Math.round((Math.log(scale / minZoom) / Math.log(MAX_ZOOM / minZoom)) * 1000));
  const fromSlider = (p: number) => minZoom * Math.pow(MAX_ZOOM / minZoom, p / 1000);

  function local(event: MouseEvent): { px: number; py: number } {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    return { px: event.clientX - rect.left, py: event.clientY - rect.top };
  }
  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    const { px, py } = local(event);
    // Molette classique (~100 par cran) : ~15 % par cran ; pavé tactile : progressif.
    const delta = event.deltaMode === 1 ? event.deltaY * 33 : event.deltaY;
    zoomTo(scale * Math.exp(-clampN(delta, -300, 300) * 0.0014), px, py);
  }
  function onPanePointerDown(event: PointerEvent): void {
    const middle = event.button === 1;
    if (event.button !== 0 && !middle) return;
    if (!canPan || (picking && !spaceDown && !middle)) return;
    event.preventDefault();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    pan = { x: event.clientX, y: event.clientY, cx, cy, moved: false };
  }
  function onPanePointerMove(event: PointerEvent): void {
    if (!pan) return;
    const dx = event.clientX - pan.x;
    const dy = event.clientY - pan.y;
    if (Math.abs(dx) + Math.abs(dy) > 3) pan.moved = true;
    setView(scale, pan.cx - dx / geo.w, pan.cy - dy / geo.h);
  }
  let panMoved = false;
  function onPanePointerUp(): void {
    panMoved = pan?.moved ?? false;
    pan = null;
  }
  function onPaneClick(event: MouseEvent): void {
    if (panMoved) return void (panMoved = false);
    if (!picking || spaceDown) return;
    const { px, py } = local(event);
    const u = (px - geo.left) / geo.w;
    const v = (py - geo.top) / geo.h;
    if (u >= 0 && u <= 1 && v >= 0 && v <= 1) void pick(u, v);
  }
  function onPaneDblClick(event: MouseEvent): void {
    if (picking) return;
    const { px, py } = local(event);
    zoomTo(zoom !== null && zoom >= 0.999 ? null : 1, px, py);
  }

  // Navigation : la vignette de l'image et le rectangle visible ; cliquer ou glisser y déplace la vue.
  const NAV = 200;
  const navW = $derived(W >= H ? NAV : Math.round((NAV * W) / H));
  const navH = $derived(W >= H ? Math.round((NAV * H) / W) : NAV);
  const visible = $derived.by(() => {
    const g = geo;
    const x0 = Math.max(0, -g.left / scale);
    const y0 = Math.max(0, -g.top / scale);
    return { x0, y0, x1: Math.min(W, x0 + (paneW - Math.max(0, g.left)) / scale), y1: Math.min(H, y0 + (paneH - Math.max(0, g.top)) / scale) };
  });
  let navDrag = false;
  function navTo(event: PointerEvent): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    setView(scale, clampN((event.clientX - rect.left) / rect.width, 0, 1), clampN((event.clientY - rect.top) / rect.height, 0, 1));
  }

  // Zone en pleine définition, pour chacune des deux images (Après : les réglages ; Avant : le développement par défaut).
  type Kind = "after" | "before";
  interface Detail {
    url: string;
    key: string;
    factor: number;
    x: number;
    y: number;
    w: number;
    h: number;
  }
  let details = $state<Record<Kind, Detail | null>>({ after: null, before: null });
  let pending = $state<Record<Kind, boolean>>({ after: false, before: false });
  const seq: Record<Kind, number> = { after: 0, before: 0 };
  /** Au plus ~6 millions de pixels développés par zone : un zoom proche d'Ajuster reste rapide. */
  const DETAIL_BUDGET = 6e6;
  /**
   * Pixels pleine taille par pixel développé : 1 dès que l'écran montre au moins un pixel d'écran par pixel réel,
   * davantage quand la zone (agrandie d'un quart de chaque côté) dépasserait le budget.
   */
  const factor = $derived.by(() => {
    const area = Math.min(W, (paneW / scale) * 1.5) * Math.min(H, (paneH / scale) * 1.5);
    return Math.max(1, Math.floor(1 / (scale * dpr) + 1e-6), Math.ceil(Math.sqrt(area / DETAIL_BUDGET)));
  });
  /** En mode Ajuster, l'aperçu suffit ; au-delà, dès que l'écran demande plus de pixels qu'il n'en a. */
  const needDetail = $derived(zoom !== null && scale * dpr > previewRatio * 1.05 && 1 / factor > previewRatio * 1.05);
  const afterKey = $derived(JSON.stringify(params) + clip);
  const keyOf = (kind: Kind) => (kind === "after" ? afterKey : "avant");
  const kinds = $derived<Kind[]>(view === "after" ? ["after"] : view === "before" ? ["before"] : ["before", "after"]);
  const shown = (kind: Kind): Detail | null => {
    const d = details[kind];
    return needDetail && d && d.key === keyOf(kind) && d.factor === factor ? d : null;
  };
  const covers = (d: Detail | null) => !!d && d.x <= visible.x0 + 0.5 && d.y <= visible.y0 + 0.5 && d.x + d.w >= visible.x1 - 0.5 && d.y + d.h >= visible.y1 - 0.5;

  let detailTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    // Relu à chaque changement de vue, de zoom ou de réglage ; la demande part quand tout est immobile depuis 140 ms.
    if (!needDetail || stageW === 0) return;
    const wanted = kinds.filter((kind) => !covers(shown(kind)));
    void factor;
    if (!wanted.length) return;
    clearTimeout(detailTimer);
    detailTimer = setTimeout(() => wanted.forEach((kind) => void fetchDetail(kind)), 140);
  });

  async function fetchDetail(kind: Kind): Promise<void> {
    const { x0, y0, x1, y1 } = visible;
    // Un quart de plus de chaque côté : un petit déplacement ne redemande rien.
    const mx = (x1 - x0) / 4;
    const my = (y1 - y0) / 4;
    const x = Math.max(0, Math.floor(x0 - mx));
    const y = Math.max(0, Math.floor(y0 - my));
    const w = Math.min(W, Math.ceil(x1 + mx)) - x;
    const h = Math.min(H, Math.ceil(y1 + my)) - y;
    const key = keyOf(kind);
    const f = factor;
    const id = ++seq[kind];
    pending[kind] = true;
    try {
      const snapshot = kind === "after" ? $state.snapshot(params) : NEUTRAL;
      const buffer = await api.rawRegion(opened.id, snapshot, kind === "after" ? clip : "none", x, y, w, h, f);
      const head = new Uint32Array(buffer.slice(0, 16));
      const url = URL.createObjectURL(new Blob([new Uint8Array(buffer, 16)], { type: "image/jpeg" }));
      // Décodée avant d'être montrée : pas d'image vide le temps du décodage.
      const img = new Image();
      img.src = url;
      await img.decode().catch(() => undefined);
      if (closed || id !== seq[kind]) return URL.revokeObjectURL(url);
      const old = details[kind];
      details[kind] = { url, key, factor: f, x: head[0]!, y: head[1]!, w: head[2]!, h: head[3]! };
      if (old) URL.revokeObjectURL(old.url);
    } catch (error) {
      if (!closed) app.fail(error);
    } finally {
      if (id === seq[kind]) pending[kind] = false;
    }
  }

  const badge = $derived.by(() => {
    if (zoom === null) return "";
    if (!needDetail) return t("{zoom} · aperçu", { zoom: percent(scale) });
    if (kinds.every((kind) => shown(kind))) return factor === 1 ? t("{zoom} · pixels réels développés", { zoom: percent(scale) }) : t("{zoom} · développé en pleine définition", { zoom: percent(scale) });
    return t("{zoom} · développement de la zone…", { zoom: percent(scale) });
  });

  function onZoomKey(event: KeyboardEvent): void {
    const target = event.target as HTMLElement | null;
    const typing = !!target && (target.tagName === "INPUT" || target.tagName === "SELECT" || target.tagName === "TEXTAREA");
    if (event.code === "Space" && !typing) {
      event.preventDefault();
      spaceDown = event.type === "keydown";
      return;
    }
    if (event.type !== "keydown" || !(event.ctrlKey || event.metaKey)) return;
    const action = { Digit0: () => zoomTo(null), Numpad0: () => zoomTo(null), Digit1: () => zoomTo(1), Numpad1: () => zoomTo(1), Equal: () => step(1), NumpadAdd: () => step(1), Minus: () => step(-1), NumpadSubtract: () => step(-1) }[event.code];
    if (!action) return;
    event.preventDefault();
    action();
  }

  // ------------------------------------------------------------------ histogramme

  const histPaths = $derived.by(() => {
    if (!histogram) return [];
    const h = histogram;
    // Racine carrée : les petites populations restent visibles ; les cases extrêmes (souvent énormes) n'écrasent pas l'échelle.
    let max = 1;
    for (let k = 0; k < 3; k++) for (let i = 1; i < 255; i++) max = Math.max(max, h[k * 256 + i]!);
    const top = Math.sqrt(max);
    return [0, 1, 2].map((k) => {
      let d = "M0 80";
      for (let i = 0; i < 256; i++) d += ` L${i} ${(80 - Math.min(1, Math.sqrt(h[k * 256 + i]!) / top) * 76).toFixed(1)}`;
      return `${d} L255 80 Z`;
    });
  });
  const histClip = $derived.by(() => {
    if (!histogram) return { low: false, high: false };
    const h = histogram;
    return { low: [0, 1, 2].some((k) => h[k * 256]! > 0), high: [0, 1, 2].some((k) => h[k * 256 + 255]! > 0) };
  });

  // ------------------------------------------------------------------ curseurs

  interface Slider {
    key: NumericKey;
    label: string;
    min: number;
    max: number;
    step: number;
    /** Valeur d'un double-clic. */
    neutral: () => number;
    track?: string;
    /** Zones écrêtées montrées avec Alt pendant le glissement. */
    clip?: RawClipView;
    /** Position (0 à 1) d'une valeur, et inversement : linéaire par défaut. */
    toPos?: (v: number) => number;
    fromPos?: (p: number) => number;
    format?: (v: number) => string;
    /** Après un changement par l'utilisateur. */
    changed?: () => void;
    reset?: () => void;
  }

  const number = (digits: number, signed: boolean) => new Intl.NumberFormat(i18n.info.tag, { minimumFractionDigits: digits, maximumFractionDigits: digits, signDisplay: signed ? "exceptZero" : "auto" });
  const signed = (v: number) => number(0, true).format(v);
  const KMIN = 2000;
  const KMAX = 25000;
  const toCustom = () => {
    params.whiteBalance = "custom";
    preset = presetOf(params);
  };
  const backToShot = () => choosePreset("asShot");

  const WB: Slider[] = [
    {
      key: "temperature",
      label: t("Température"),
      min: KMIN,
      max: KMAX,
      step: 50,
      neutral: () => NEUTRAL.temperature,
      track: "linear-gradient(90deg, #4a7fd6, #ddd 45%, #e0a83a)",
      // Échelle logarithmique : autant de course de 2 000 à 7 000 K que de 7 000 à 25 000 K, à peu près.
      toPos: (v) => Math.log(v / KMIN) / Math.log(KMAX / KMIN),
      fromPos: (p) => Math.round((KMIN * Math.pow(KMAX / KMIN, p)) / 50) * 50,
      format: (v) => number(0, false).format(v),
      changed: toCustom,
      reset: backToShot,
    },
    { key: "tint", label: t("Teinte"), min: -150, max: 150, step: 1, neutral: () => NEUTRAL.tint, track: "linear-gradient(90deg, #3fae5a, #ddd 50%, #c94fb4)", changed: toCustom, reset: backToShot },
  ];
  const TONE: Slider[] = [
    { key: "exposure", label: t("Exposition"), min: -5, max: 5, step: 0.01, neutral: () => 0, clip: "highlights", format: (v) => number(2, true).format(v) },
    { key: "contrast", label: t("Contraste"), min: -100, max: 100, step: 1, neutral: () => 0, clip: "highlights" },
    { key: "highlights", label: t("Hautes lumières"), min: -100, max: 100, step: 1, neutral: () => 0, clip: "highlights" },
    { key: "shadows", label: t("Ombres"), min: -100, max: 100, step: 1, neutral: () => 0, clip: "shadows" },
    { key: "whites", label: t("Blancs"), min: -100, max: 100, step: 1, neutral: () => 0, clip: "highlights" },
    { key: "blacks", label: t("Noirs"), min: -100, max: 100, step: 1, neutral: () => 0, clip: "shadows" },
  ];
  const PRESENCE: Slider[] = [
    { key: "clarity", label: t("Clarté"), min: -100, max: 100, step: 1, neutral: () => 0 },
    { key: "vibrance", label: t("Vibrance"), min: -100, max: 100, step: 1, neutral: () => 0 },
    { key: "saturation", label: t("Saturation"), min: -100, max: 100, step: 1, neutral: () => 0 },
  ];
  const DETAIL: Slider[] = [
    { key: "noiseLuma", label: t("Bruit (lum.)"), min: 0, max: 100, step: 1, neutral: () => 0, format: (v) => String(Math.round(v)) },
    { key: "noiseColor", label: t("Bruit (couleur)"), min: 0, max: 100, step: 1, neutral: () => 0, format: (v) => String(Math.round(v)) },
    { key: "sharpen", label: t("Netteté"), min: 0, max: 100, step: 1, neutral: () => 0, format: (v) => String(Math.round(v)) },
  ];
  const OPTICS: Slider[] = [{ key: "vignette", label: t("Vignetage"), min: -100, max: 100, step: 1, neutral: () => 0 }];

  const position = (s: Slider, v: number) => Math.min(1, Math.max(0, s.toPos ? s.toPos(v) : (v - s.min) / (s.max - s.min)));
  function valueAt(s: Slider, p: number): number {
    const raw = s.fromPos ? s.fromPos(p) : s.min + p * (s.max - s.min);
    const stepped = Math.round(raw / s.step) * s.step;
    return Math.min(s.max, Math.max(s.min, Number(stepped.toFixed(4))));
  }
  function set(s: Slider, v: number): void {
    if (params[s.key] === v) return;
    params[s.key] = v;
    s.changed?.();
  }
  function reset(s: Slider): void {
    if (s.reset) return s.reset();
    params[s.key] = s.neutral();
  }

  /**
   * Glissement d'un curseur. Sans Maj, la poignée suit le pointeur ; avec Maj (réglage fin), elle avance dix fois moins
   * vite que lui, depuis sa position — Maj peut être pressée ou relâchée en cours de route.
   */
  let dragging: { slider: Slider; track: HTMLElement; x: number; pos: number } | null = null;
  const FINE = 0.1;
  function clipFor(alt: boolean): void {
    clip = alt && dragging?.slider.clip ? dragging.slider.clip : "none";
  }
  function dragTo(event: PointerEvent): void {
    if (!dragging) return;
    const rect = dragging.track.getBoundingClientRect();
    dragging.pos = event.shiftKey ? dragging.pos + ((event.clientX - dragging.x) / rect.width) * FINE : (event.clientX - rect.left) / rect.width;
    dragging.pos = Math.min(1, Math.max(0, dragging.pos));
    dragging.x = event.clientX;
    set(dragging.slider, valueAt(dragging.slider, dragging.pos));
    clipFor(event.altKey);
  }
  function startDrag(event: PointerEvent, s: Slider): void {
    if (event.button !== 0) return;
    event.preventDefault();
    const track = event.currentTarget as HTMLElement;
    track.setPointerCapture(event.pointerId);
    track.focus();
    dragging = { slider: s, track, x: event.clientX, pos: position(s, params[s.key]) };
    // Maj au clic : la poignée ne saute pas sous le pointeur.
    if (!event.shiftKey) dragTo(event);
    else clipFor(event.altKey);
  }

  /** Valeur tapée dans la case d'un curseur (virgule ou point, signe facultatif) ; invalide : la valeur reste. */
  function typed(s: Slider, input: HTMLInputElement): void {
    const value = Number(input.value.replace(/[\s\u202f\u00a0]/g, "").replace(",", ".").replace("−", "-"));
    if (Number.isFinite(value)) set(s, Math.min(s.max, Math.max(s.min, Number((Math.round(value / s.step) * s.step).toFixed(4)))));
    input.value = s.format ? s.format(params[s.key]) : signed(params[s.key]);
  }
  function keyOnValue(event: KeyboardEvent, s: Slider): void {
    const input = event.currentTarget as HTMLInputElement;
    if (event.key === "Enter") {
      typed(s, input);
      input.blur();
    } else if (event.key === "Escape") {
      // Échap annule la saisie sans fermer la fenêtre.
      event.preventDefault();
      event.stopPropagation();
      input.value = s.format ? s.format(params[s.key]) : signed(params[s.key]);
      input.blur();
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      keyOnSlider(event, s);
      queueMicrotask(() => (input.value = s.format ? s.format(params[s.key]) : signed(params[s.key])));
    }
  }
  function endDrag(): void {
    dragging = null;
    clip = "none";
  }
  function keyOnSlider(event: KeyboardEvent, s: Slider): void {
    const delta = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[event.key];
    if (!delta) return;
    event.preventDefault();
    const steps = event.shiftKey ? 10 : 1;
    if (s.toPos) set(s, valueAt(s, position(s, params[s.key]) + delta * steps * 0.005));
    else set(s, Math.min(s.max, Math.max(s.min, Number((params[s.key] + delta * steps * s.step).toFixed(4)))));
  }
  // Alt pressé ou relâché pendant un glissement, sans bouger la souris.
  function onKey(event: KeyboardEvent): void {
    onZoomKey(event);
    if (dragging && event.key === "Alt") {
      event.preventDefault();
      clipFor(event.type === "keydown");
    }
  }

  // ------------------------------------------------------------------ balance des blancs

  function choosePreset(next: Preset): void {
    preset = next;
    if (next === "asShot") Object.assign(params, { whiteBalance: "asShot", temperature: NEUTRAL.temperature, tint: NEUTRAL.tint });
    else if (next === "auto") Object.assign(params, { whiteBalance: "auto", temperature: Math.round(info.autoTemperature), tint: Math.round(info.autoTint) });
    else if (next !== "custom") {
      const [temperature, tint] = PRESETS[next];
      Object.assign(params, { whiteBalance: "custom", temperature, tint });
    } else params.whiteBalance = "custom";
  }

  /** Pipette : le point `(u, v)` de l'image (0 à 1) devient neutre. */
  async function pick(u: number, v: number): Promise<void> {
    if (!picking) return;
    const result = await api.rawPickWhite(opened.id, u, v).catch((error) => (app.fail(error), null));
    picking = false;
    if (!result) return;
    Object.assign(params, { whiteBalance: "custom", temperature: Math.round(result[0] / 10) * 10, tint: Math.round(result[1]) });
    preset = presetOf(params);
  }

  async function autoTone(): Promise<void> {
    try {
      Object.assign(params, await api.rawAutoTone(opened.id, $state.snapshot(params)));
    } catch (error) {
      app.fail(error);
    }
  }

  // ------------------------------------------------------------------ fin

  function finish(result: { params: RawParams; applyToRest: boolean; keepRaw: boolean } | null): void {
    if (done) return;
    done = true;
    request.resolve(result);
  }
  const confirm = () => finish({ params: $state.snapshot(params), applyToRest, keepRaw: keepRaw || mode === "smart" || mode === "redevelop" });
  const cancel = () => finish(null);
  function resetAll(): void {
    Object.assign(params, NEUTRAL);
    preset = "asShot";
  }

  const confirmLabel = $derived(
    mode === "redevelop" ? t("Appliquer") : filter ? (target?.smart ? t("Pixelliser et appliquer") : t("Appliquer")) : mode === "smart" || (mode === "layer" && keepRaw) ? t("Placer l'objet dynamique") : mode === "open" ? t("Ouvrir l'image") : t("Importer le calque"),
  );
  const exif = $derived.by(() => {
    const parts: string[] = [];
    if (info.camera) parts.push(info.camera);
    parts.push(`${number(0, false).format(Math.round((info.width * info.height) / 1e6))} Mpx`);
    if (info.iso) parts.push(`ISO ${info.iso}`);
    if (info.aperture) parts.push(`f/${new Intl.NumberFormat(i18n.info.tag, { maximumFractionDigits: 1 }).format(info.aperture)}`);
    if (info.shutter) parts.push(info.shutter >= 0.3 ? `${new Intl.NumberFormat(i18n.info.tag, { maximumFractionDigits: 1 }).format(info.shutter)} s` : `1/${Math.round(1 / info.shutter)} s`);
    if (info.focalLength) parts.push(`${Math.round(info.focalLength)} mm`);
    return parts.join(" · ");
  });
</script>

<svelte:window onkeydown={onKey} onkeyup={onKey} />

{#snippet slider(s: Slider, disabled = false)}
  <div class="sl" class:disabled>
    <span class="label">{s.label}</span>
    <div
      class="track"
      style:background={s.track}
      role="slider"
      tabindex={disabled ? -1 : 0}
      aria-label={s.label}
      aria-valuemin={s.min}
      aria-valuemax={s.max}
      aria-valuenow={params[s.key]}
      title={t("Double-clic : réinitialiser · Maj en glissant : réglage fin")}
      onpointerdown={(e) => !disabled && startDrag(e, s)}
      onpointermove={dragTo}
      onpointerup={endDrag}
      onpointercancel={endDrag}
      ondblclick={() => !disabled && reset(s)}
      onkeydown={(e) => !disabled && keyOnSlider(e, s)}
    >
      <i style:left="{position(s, params[s.key]) * 100}%"></i>
    </div>
    <input
      class="num"
      type="text"
      inputmode="decimal"
      spellcheck="false"
      aria-label={s.label}
      disabled={disabled}
      value={s.format ? s.format(params[s.key]) : signed(params[s.key])}
      onfocus={(e) => {
        // Tout sélectionné à l'entrée (taper remplace la valeur) ; au clic, la sélection se fait après le relâchement.
        const input = e.currentTarget as HTMLInputElement;
        input.select();
        input.dataset.fresh = "1";
      }}
      onmouseup={(e) => {
        // Le relâchement du clic qui a donné le focus placerait le curseur et perdrait la sélection.
        const input = e.currentTarget as HTMLInputElement;
        if (!input.dataset.fresh) return;
        e.preventDefault();
        delete input.dataset.fresh;
        setTimeout(() => input.select(), 0);
      }}
      onchange={(e) => typed(s, e.currentTarget as HTMLInputElement)}
      onkeydown={(e) => keyOnValue(e, s)}
    />
  </div>
{/snippet}

<dialog
  bind:this={dialog}
  class="raw"
  oncancel={(e) => {
    e.preventDefault();
    cancel();
  }}
>
  <header>
    {#if filter}
      <span class="tag">{t("Camera Raw")}</span>
      <b class="name">{t(info.fileName)}</b>
      <span class="exif">{inSelection ? t("Calque actif · sélection seulement") : t("Calque actif")}</span>
    {:else}
      <span class="tag">RAW</span>
      <b class="name">{info.fileName}</b>
      <span class="exif">{exif}</span>
    {/if}
    {#if request.count > 1}<span class="series">{t("{index} sur {count}", { index: request.index, count: request.count })}</span>{/if}
    <button class="x" type="button" aria-label={t("Annuler")} onclick={cancel}>✕</button>
  </header>

  <div class="mid">
    <div class="view" class:picking>
      <div class="modes" role="group" aria-label={t("Affichage")}>
        <button type="button" class:on={view === "after"} onclick={() => (view = "after")}>{t("Après")}</button>
        <button type="button" class:on={view === "before"} onclick={() => (view = "before")}>{t("Avant")}</button>
        <button type="button" class:on={view === "split"} onclick={() => (view = "split")}>{t("Avant | Après")}</button>
      </div>
      <svg class="hist" viewBox="0 0 256 80" preserveAspectRatio="none" aria-label={t("Histogramme")}>
        {#each histPaths as d, k (k)}
          <path {d} class="c{k}" />
        {/each}
        {#if histClip.low}<rect x="0" y="0" width="5" height="5" class="warn" />{/if}
        {#if histClip.high}<rect x="251" y="0" width="5" height="5" class="warn" />{/if}
      </svg>

      <div class="stage" class:split={view === "split"} bind:clientWidth={stageW} bind:clientHeight={stageH}>
        {#each kinds as kind (kind)}
          {@const url = kind === "after" ? afterUrl : beforeUrl}
          {@const d = shown(kind)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div
            class="pane"
            class:grab={canPan && (spaceDown || !picking)}
            class:grabbing={pan !== null}
            onwheel={onWheel}
            onpointerdown={onPanePointerDown}
            onpointermove={onPanePointerMove}
            onpointerup={onPanePointerUp}
            onpointercancel={onPanePointerUp}
            onclick={onPaneClick}
            ondblclick={onPaneDblClick}
          >
            {#if url}
              <div class="canvas" style:left="{geo.left}px" style:top="{geo.top}px" style:width="{geo.w}px" style:height="{geo.h}px">
                <img src={url} alt={kind === "after" ? t("Après") : t("Avant")} draggable="false" />
                {#if d}
                  <img
                    class="detail"
                    class:pixels={scale * dpr >= 2}
                    src={d.url}
                    alt=""
                    draggable="false"
                    style:left="{(d.x / W) * 100}%"
                    style:top="{(d.y / H) * 100}%"
                    style:width="{(d.w / W) * 100}%"
                    style:height="{(d.h / H) * 100}%"
                  />
                {/if}
              </div>
            {:else}
              <span class="wait">{t("Développement de l'aperçu…")}</span>
            {/if}
            {#if view === "split"}<span class="caption">{kind === "after" ? t("Après") : t("Avant")}</span>{/if}
          </div>
        {/each}
      </div>

      {#if badge}<div class="badge">{badge}</div>{/if}

      {#if canPan && (afterUrl || beforeUrl)}
        <div
          class="nav"
          style:width="{navW}px"
          style:height="{navH}px"
          role="slider"
          tabindex="-1"
          aria-label={t("Navigation")}
          aria-valuenow={Math.round(cx * 100)}
          onpointerdown={(e) => {
            e.preventDefault();
            (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
            navDrag = true;
            navTo(e);
          }}
          onpointermove={(e) => navDrag && navTo(e)}
          onpointerup={() => (navDrag = false)}
          onpointercancel={() => (navDrag = false)}
        >
          <img src={view === "before" ? beforeUrl : (afterUrl ?? beforeUrl)} alt="" draggable="false" />
          <i style:left="{(visible.x0 / W) * 100}%" style:top="{(visible.y0 / H) * 100}%" style:width="{((visible.x1 - visible.x0) / W) * 100}%" style:height="{((visible.y1 - visible.y0) / H) * 100}%"></i>
          <span>{t("Navigation")}</span>
        </div>
      {/if}

      <div class="zoombar">
        <button type="button" class:on={zoom === null} onclick={() => zoomTo(null)}>{t("Ajuster")}</button>
        <button type="button" class:on={zoom !== null && Math.abs(zoom - 1) < 1e-3} onclick={() => zoomTo(1)}>100 %</button>
        <button type="button" class:on={zoom !== null && Math.abs(zoom - 2) < 1e-3} onclick={() => zoomTo(2)}>200 %</button>
        <select
          value=""
          aria-label={t("Zoom")}
          onchange={(e) => {
            const el = e.currentTarget as HTMLSelectElement;
            zoomTo(Number(el.value));
            el.value = "";
          }}
        >
          <option value="" hidden>{percent(scale)}</option>
          {#each LEVELS.filter((l) => l >= minZoom - 1e-6) as level (level)}<option value={level}>{percent(level)}</option>{/each}
        </select>
        <button type="button" class="pm" aria-label={t("Zoom arrière")} onclick={() => step(-1)}>−</button>
        <input type="range" min="0" max="1000" value={sliderPos} aria-label={t("Zoom")} oninput={(e) => zoomTo(fromSlider(Number((e.currentTarget as HTMLInputElement).value)))} />
        <button type="button" class="pm" aria-label={t("Zoom avant")} onclick={() => step(1)}>+</button>
        <p class="hint">
          {#if picking}{t("Cliquez sur un gris neutre de l'image")}{:else}{t("Molette : zoom · Espace ou glisser : déplacer · Double-clic : Ajuster ↔ 100 % · Alt : zones écrêtées")}{/if}
        </p>
      </div>
    </div>

    <aside class="side">
      <section>
        <h4>{t("Balance des blancs")}</h4>
        {#if info.monochrome}
          <p class="note">{t("Capteur monochrome : pas de balance des blancs.")}</p>
        {:else}
          <div class="wb">
            <select value={preset} onchange={(e) => choosePreset((e.currentTarget as HTMLSelectElement).value as Preset)} aria-label={t("Balance des blancs")}>
              <option value="asShot">{t("Telle quelle ({k} K)", { k: number(0, false).format(NEUTRAL.temperature) })}</option>
              <option value="auto">{t("Auto")}</option>
              <!-- Un calque n'a pas d'éclairage de prise de vue connu : seuls « telle quelle », Auto et Personnalisée ont un sens. -->
              {#if !filter}
                <option value="daylight">{t("Lumière du jour")}</option>
                <option value="cloudy">{t("Nuageux")}</option>
                <option value="shade">{t("Ombre")}</option>
                <option value="tungsten">{t("Tungstène")}</option>
                <option value="fluorescent">{t("Fluorescent")}</option>
                <option value="flash">{t("Flash")}</option>
              {/if}
              <option value="custom">{t("Personnalisée")}</option>
            </select>
            <button type="button" class="pip" class:on={picking} title={t("Pipette : cliquer sur un gris de l'image")} aria-label={t("Pipette de balance des blancs")} aria-pressed={picking} onclick={() => ((picking = !picking), picking && view === "before" && (view = "after"))}>
              <svg viewBox="0 0 24 24"><path d="M14 4l6 6-9 9H5v-6z" /></svg>
            </button>
          </div>
          {#each WB as s (s.key)}{@render slider(s)}{/each}
        {/if}
      </section>
      <section>
        <h4>{t("Tonalité")}<button type="button" class="auto" onclick={autoTone}>{t("Auto")}</button></h4>
        {#each TONE as s (s.key)}{@render slider(s)}{/each}
      </section>
      <section>
        <h4>{t("Présence")}</h4>
        {#each PRESENCE as s (s.key)}{@render slider(s)}{/each}
      </section>
      <section>
        <h4>{t("Détail")}</h4>
        {#each DETAIL as s (s.key)}{@render slider(s)}{/each}
        <p class="note">{t("Netteté et réduction du bruit se jugent à 100 % : au-delà de la taille de l'aperçu, la zone visible est développée en pleine définition.")}</p>
      </section>
      <section class="last">
        <h4>{t("Optique")}</h4>
        {#each OPTICS as s (s.key)}{@render slider(s)}{/each}
      </section>
    </aside>
  </div>

  <footer>
    <button type="button" class="btn" onclick={resetAll}>{t("Réinitialiser")}</button>
    {#if mode === "open" || mode === "layer"}
      <label class="chk"><input type="checkbox" bind:checked={keepRaw} />{t("Ouvrir en objet dynamique (redévelopper plus tard)")}</label>
    {:else if mode === "redevelop"}
      <span class="chk">{t("Objet dynamique : le nouveau développement remplace celui de toutes ses instances.")}</span>
    {:else if filter && target?.smart}
      <span class="chk">{t("Objet dynamique : il sera pixellisé et filtré en une seule étape (un Ctrl+Z rend l'objet).")}</span>
    {:else if filter && (target?.text || target?.shape || target?.fillLayer)}
      <span class="chk">{t("Ce calque sera pixellisé par le filtre, comme pour les autres filtres.")}</span>
    {/if}
    {#if request.count > 1 && request.index < request.count}
      <label class="chk"><input type="checkbox" bind:checked={applyToRest} />{t("Appliquer à tous les suivants ({n})", { n: request.count - request.index })}</label>
    {/if}
    <span class="sp"></span>
    <button type="button" class="btn" onclick={cancel}>{t("Annuler")}</button>
    <button type="button" class="btn primary" onclick={confirm}>{confirmLabel}</button>
  </footer>
</dialog>

<style>
  dialog.raw {
    width: min(1360px, 96vw);
    height: min(840px, 94vh);
    max-width: none;
    max-height: none;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 80px rgba(0, 0, 0, 0.7);
    overflow: hidden;
  }
  dialog.raw[open] {
    display: grid;
    grid-template-rows: 48px minmax(0, 1fr) 56px;
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.6);
  }
  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 18px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .tag {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: #ff8a94;
    font-size: 10.5px;
    font-weight: 700;
    padding: 2px 7px;
    letter-spacing: 0.05em;
  }
  .name {
    font-size: 14px;
    white-space: nowrap;
  }
  .exif {
    color: var(--text-muted);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .series {
    color: var(--text-2);
    font-size: 12px;
    white-space: nowrap;
  }
  .x {
    margin-left: auto;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: 14px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--text);
  }
  .mid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 360px;
    min-height: 0;
  }
  .view {
    position: relative;
    background: color-mix(in srgb, var(--bg-app) 60%, #000);
    overflow: hidden;
    min-height: 0;
  }
  .modes {
    position: absolute;
    left: 14px;
    top: 12px;
    display: flex;
    gap: 6px;
    z-index: 1;
  }
  .modes button {
    font-size: 11.5px;
    padding: 5px 9px;
    border: 1px solid var(--border-strong);
    background: rgba(20, 20, 20, 0.8);
    color: var(--text-2);
    cursor: pointer;
  }
  .modes button.on {
    border-color: var(--accent);
    color: var(--text-strong);
  }
  .hist {
    position: absolute;
    right: 14px;
    top: 12px;
    width: 220px;
    height: 80px;
    background: rgba(20, 20, 20, 0.85);
    border: 1px solid var(--border-strong);
    z-index: 1;
  }
  .hist path {
    mix-blend-mode: screen;
  }
  .hist .c0 {
    fill: rgba(255, 90, 90, 0.55);
  }
  .hist .c1 {
    fill: rgba(90, 220, 120, 0.5);
  }
  .hist .c2 {
    fill: rgba(90, 140, 255, 0.55);
  }
  .hist .warn {
    fill: #fff;
  }
  .stage {
    position: absolute;
    inset: 0 0 40px;
    display: flex;
    gap: 12px;
  }
  .pane {
    position: relative;
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    touch-action: none;
  }
  .pane.grab {
    cursor: grab;
  }
  .pane.grabbing {
    cursor: grabbing;
  }
  .canvas {
    position: absolute;
    box-shadow: 0 10px 40px rgba(0, 0, 0, 0.6);
  }
  .canvas img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    user-select: none;
    pointer-events: none;
  }
  .canvas img.detail {
    inset: auto;
  }
  .canvas img.pixels {
    image-rendering: pixelated;
  }
  .caption {
    position: absolute;
    left: 50%;
    bottom: 6px;
    transform: translateX(-50%);
    padding: 2px 8px;
    background: rgba(20, 20, 20, 0.8);
    font-size: 11px;
    color: var(--text-2);
    pointer-events: none;
  }
  .wait {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: 12px;
  }
  .badge {
    position: absolute;
    top: 12px;
    left: 50%;
    transform: translateX(-50%);
    padding: 5px 10px;
    background: rgba(20, 20, 20, 0.85);
    border: 1px solid var(--border);
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    z-index: 1;
    pointer-events: none;
  }
  .nav {
    position: absolute;
    right: 14px;
    top: 104px;
    border: 1px solid var(--border-strong);
    background: var(--bg-well);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
    cursor: pointer;
    overflow: hidden;
    z-index: 1;
    touch-action: none;
  }
  .nav img {
    width: 100%;
    height: 100%;
    display: block;
    pointer-events: none;
    user-select: none;
  }
  .nav i {
    position: absolute;
    border: 1.5px solid var(--accent);
    box-shadow: 0 0 0 999px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }
  .nav span {
    position: absolute;
    left: 6px;
    bottom: 4px;
    font-size: 10.5px;
    color: var(--text-strong);
    text-shadow: 0 1px 2px #000;
    pointer-events: none;
  }
  .zoombar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 40px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 14px;
    border-top: 1px solid var(--border);
    background: var(--bg-panel);
    font-size: 12px;
  }
  .zoombar button {
    height: 26px;
    padding: 0 9px;
    border: 1px solid transparent;
    background: none;
    color: var(--text-2);
    cursor: pointer;
    white-space: nowrap;
  }
  .zoombar button:hover {
    color: var(--text);
  }
  .zoombar button.on {
    border-color: var(--border-strong);
    background: var(--bg-active);
    color: var(--text);
  }
  .zoombar .pm {
    padding: 0 6px;
    color: var(--text-muted);
  }
  .zoombar select {
    height: 26px;
    margin-left: 4px;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    color: var(--text);
    font-size: 12px;
    padding: 0 4px;
  }
  .zoombar input[type="range"] {
    width: 150px;
    accent-color: var(--accent);
  }
  .view.picking .pane:not(.grabbing) {
    cursor: var(--cursor-pipette);
  }
  .hint {
    margin: 0 0 0 auto;
    min-width: 0;
    padding-left: 12px;
    color: var(--text-muted);
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .side {
    border-left: 1px solid var(--border);
    overflow-y: auto;
    padding: 4px 0;
  }
  section {
    padding: 12px 18px;
    border-bottom: 1px solid var(--border);
  }
  section.last {
    border-bottom: none;
  }
  h4 {
    margin: 0 0 10px;
    font-size: 11px;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-muted);
    font-weight: 600;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  h4 .auto {
    border: none;
    background: none;
    padding: 0;
    text-transform: none;
    letter-spacing: 0;
    font-size: 11.5px;
    color: var(--text-2);
    cursor: pointer;
  }
  h4 .auto:hover {
    color: var(--text);
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  .sl + .note {
    margin-top: 12px;
    font-size: 11.5px;
    line-height: 1.45;
  }
  .sl {
    display: grid;
    grid-template-columns: 100px 1fr 58px;
    align-items: center;
    gap: 6px 10px;
    margin-bottom: 9px;
    font-size: 12px;
    color: var(--text-2);
  }
  .sl.disabled {
    opacity: 0.4;
  }
  .sl .num {
    width: 100%;
    height: 22px;
    padding: 0 4px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    text-align: right;
    font: inherit;
    font-weight: 500;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .sl .num:hover {
    border-color: var(--border);
  }
  .sl .num:focus {
    outline: none;
    border-color: var(--accent);
    background: var(--bg-field);
  }
  .track {
    height: 3px;
    background: #3a3838;
    position: relative;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  /* Zone de saisie plus haute que la piste. */
  .track::before {
    content: "";
    position: absolute;
    inset: -8px 0;
  }
  .track i {
    position: absolute;
    top: -4px;
    width: 11px;
    height: 11px;
    margin-left: -6px;
    background: #ddd;
    border-radius: 50%;
    pointer-events: none;
  }
  .track:focus-visible i {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .wb {
    display: flex;
    gap: 6px;
    margin-bottom: 10px;
  }
  .wb select {
    flex: 1;
    height: 28px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text);
    padding: 0 6px;
    font-size: 12px;
  }
  .pip {
    width: 28px;
    height: 28px;
    border: 1px solid var(--border-strong);
    background: var(--bg-field);
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
  }
  .pip.on {
    border-color: var(--accent);
  }
  .pip svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: var(--text-2);
    stroke-width: 1.7;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 18px;
    border-top: 1px solid var(--border);
  }
  .btn {
    height: 32px;
    padding: 0 16px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    cursor: pointer;
  }
  .btn:hover {
    background: var(--bg-hover);
  }
  .btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .btn.primary:hover {
    background: var(--accent-strong);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-2);
  }
  .chk input {
    accent-color: var(--accent);
  }
  .sp {
    margin-left: auto;
  }
</style>
