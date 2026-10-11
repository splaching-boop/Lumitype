<script lang="ts">
  import { panelOwnsKeyboard } from "../lib/keymap";
  import { t } from "../lib/i18n.svelte";
  import { provideAdjustPreview } from "../lib/adjust-preview";
  import { flushSync, onMount, untrack } from "svelte";
  import { finishTextEdit } from "../lib/text-edit.svelte";
  import { adopt, edit, editLatest, enqueue, notifyDirty, onDirty, provideFrameSettled, sampleColor, waitForFrame, whenIdle } from "../lib/engine";
  import { DabPath } from "../lib/brush-path";
  import { snapOffset } from "../lib/geometry";
  import { resolveStops } from "../lib/gradient";
  import { providePreview } from "../lib/layer-preview";
  import { SNAP_PIXELS, edgesAndCenter, setSnapObjects, snapAxis, snapPoint } from "../lib/snapping.svelte";
  import { commitTransform, openTransform, positionLocked, sameLayers, transformDragging, transformIsModal, transformIsPending } from "../lib/transform-session";
  import { type Bounds, type CombineMode, type DocumentInfo, type Guide, type SelectionShape, api } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import { DynamicResolution } from "../lib/resolution";
  import { noteDraw } from "../lib/fps.svelte";
  import { type Camera, clampZoom, fitCamera, steppedZoom, zoomAt } from "./camera";
  import { ViewportRenderer } from "./renderer";
  import { gpuMemoryBudgetMb } from "../lib/performance.svelte";
  import CropOverlay from "./CropOverlay.svelte";
  import PathOverlay from "./PathOverlay.svelte";
  import Rulers from "./Rulers.svelte";
  import TextEditorOverlay from "./TextEditorOverlay.svelte";
  import TransformOverlay from "./TransformOverlay.svelte";

  interface ActiveStroke {
    docId: number;
    pending: number[][];
    sending: boolean;
    ended: boolean;
  }

  type Point = { x: number; y: number };
  /** Sélection en cours de tracé, en pixels du document. */
  type Draft =
    | { kind: "box"; shape: "rect" | "ellipse"; mode: CombineMode; start: Point; end: Point; constrain: boolean; centered: boolean }
    | { kind: "path"; closeOnRelease: boolean; mode: CombineMode; points: Point[]; hover: Point | null };

  /** Glisser en cours avec l'outil Déplacement (ou d'une sélection avec un outil de sélection). */
  interface MoveGesture {
    id: string;
    kind: "layers" | "pixels" | "selection";
    start: Point;
    /** Faux tant que la sélection automatique n'a pas répondu : rien n'est envoyé avant. */
    ready: boolean;
    /** Étendue de ce qui est déplacé, pour le magnétisme. Absente tant que le moteur n'a pas répondu. */
    bounds: Bounds | null;
    /**
     * Aperçu local : le calque suit la souris sans solliciter le moteur, qui ne reçoit que le
     * déplacement final. Faux quand l'image ne se décompose pas (modes de fusion, écrêtage…).
     */
    local: boolean;
    /** Vrai une fois l'image passée un cran plus grossier pour ce geste (voie du moteur). */
    draft?: boolean;
    /** La sélection automatique a changé de calque : le cadre à poignées doit être reposé sur le calque saisi. */
    reframe?: boolean;
    dx: number;
    dy: number;
  }

  /** Repère en cours de création (index nul) ou de déplacement. */
  interface GuideDrag {
    orientation: Guide["orientation"];
    index: number | null;
    position: number;
    /** Le pointeur est revenu sur la règle : relâcher supprime (ou abandonne) le repère. */
    discard: boolean;
    snapped: boolean;
  }

  /** Étendue des calques sélectionnés : cadre de l'outil Déplacement, et cible d'aimantation des repères. */
  let selectionBounds = $state<Bounds | null>(null);
  /** Décalage provisoire du cadre pendant qu'on glisse le calque. */
  let boundsShift = $state({ dx: 0, dy: 0 });
  /** Pendant un glisser, le cadre suit la souris : on ne redemande pas l'étendue au moteur (elle s'ajouterait au décalage). */
  let isMoving = $state(false);

  // Avec l'outil Déplacement, les calques sélectionnés portent un cadre à poignées actives. Tirer
  // une poignée transforme ; faire autre chose (cliquer ailleurs, changer d'outil ou de calque)
  // valide tout seul, sans Entrée. Le cadre suit la sélection et toute modification du document.
  $effect(() => {
    const doc = app.active;
    const ids = doc?.selection ?? [];
    // Cadre à poignées : avec l'outil Déplacement, et avec l'outil Forme sur un calque de forme (pour le
    // redimensionner sans changer d'outil).
    const shapeSelected = ids.length === 1 && !!doc?.info.layers.find((l) => l.id === ids[0])?.shape;
    const moveTool = (app.tool === "move" || (app.tool === "shape" && shapeSelected)) && !app.textEdit;
    void doc?.info.revision;
    if (isMoving) return;
    // Poignée du cadre en cours de geste : le cadre tiré n'est ni validé ni remplacé sous la souris.
    if (transformDragging()) return;

    const current = untrack(() => app.transform);
    if (current && !current.auto) return; // Ctrl+T en cours : on n'y touche pas
    const sameTarget = current !== null && moveTool && sameLayers(current.ids, ids);
    if (current?.previewed) {
      // Transformation en attente : elle n'est validée que si on a changé de calque ou d'outil.
      if (sameTarget) return;
      untrack(commitTransform);
    } else if (current && !sameTarget) {
      // Autre calque (ou autre outil) : l'ancien cadre disparaît TOUT DE SUITE. Il restait affiché, à l'ancienne
      // place, le temps que l'étendue du nouveau calque arrive — ou pour de bon si elle n'arrivait pas.
      untrack(() => (app.transform = null));
    }
    if (!doc || ids.length === 0 || !(moveTool || guideDrag !== null)) {
      selectionBounds = null;
      untrack(() => app.transform?.auto && (app.transform = null));
      return;
    }
    let cancelled = false;
    const docId = doc.info.id;
    const wanted = [...ids];
    // Par la file du document : l'étendue est lue APRÈS les modifications déjà demandées (la validation d'une
    // transformation, notamment), jamais celle d'avant.
    enqueue(docId, () => api.contentBounds(docId, wanted)).then(
      (bounds) => {
        if (cancelled) return;
        [selectionBounds, boundsShift] = [bounds, { dx: 0, dy: 0 }];
        if (transformDragging()) return;
        const session = app.transform;
        if (session && (!session.auto || session.previewed)) return;
        // Même cadre qu'affiché : on le garde (le recréer redessinerait le cadre pour rien).
        // (Un calque devenu verrouillé passe par `openTransform`, qui retire alors le cadre.)
        const locked = !!app.active && positionLocked(app.active.info, wanted);
        if (session && bounds && moveTool && !locked && sameLayers(session.ids, wanted) && sameFrame(session.source, bounds)) return;
        if (bounds && moveTool) openTransform(wanted, bounds, true);
        else app.transform = null;
      },
      () => {
        // Étendue illisible : pas de cadre plutôt qu'un cadre périmé.
        if (!cancelled && app.transform?.auto && !app.transform.previewed && !transformDragging()) app.transform = null;
      },
    );
    return () => (cancelled = true);
  });

  /** Le cadre d'origine `source` est exactement le rectangle `bounds`. */
  function sameFrame(source: readonly Point[], [x0, y0, x1, y1]: Bounds): boolean {
    const [a, , c] = source;
    return !!a && !!c && a.x === x0 && a.y === y0 && c.x === x1 && c.y === y1;
  }

  // ---- Cadres bleus : l'étendue du calque survolé dans le panneau Calques, et celle des calques sélectionnés
  // quand l'outil en main n'a pas de cadre à poignées. On voit ainsi à l'écran ce que représente un calque.
  let outlines = $state<Bounds[]>([]);
  $effect(() => {
    const doc = app.active;
    const hovered = app.hoverLayer;
    void doc?.info.revision;
    if (!doc) {
      outlines = [];
      return;
    }
    const framed = app.transform !== null;
    const ids = [...new Set([...(hovered !== null ? [hovered] : []), ...(framed ? [] : doc.selection)])];
    let cancelled = false;
    Promise.all(ids.map((id) => api.contentBounds(doc.info.id, [id]).catch(() => null))).then((all) => {
      if (!cancelled) outlines = all.filter((b): b is Bounds => b !== null);
    });
    return () => (cancelled = true);
  });

  let guideDrag = $state<GuideDrag | null>(null);

  let moving: MoveGesture | null = null;
  /** Identifie l'aperçu local en cours : la fin tardive d'un geste ne doit pas fermer l'aperçu du suivant. */
  let previewSerial = 0;
  let gestureCounter = 0;
  const nextGesture = () => `geste-${Date.now()}-${gestureCounter++}`;


  let canvas = $state<HTMLCanvasElement>();
  let draft = $state<Draft | null>(null);
  let renderer: ViewportRenderer | null = null;
  let frameRequested = false;
  let spaceHeld = $state(false);
  let altHeld = $state(false);
  /** Pipette active : outil Pipette, ou Alt avec un outil de peinture (pinceau, gomme, pot de peinture, dégradé), ou Alt pendant qu'un sélecteur de couleur est ouvert. */
  // En masque rapide, Alt sert à retirer de la sélection : pas de pipette.
  const sampling = $derived(app.tool === "eyedropper" || (app.eyedropperArmed && app.eyedropperTarget !== null) || (altHeld && !app.quickMask && (app.paintTool !== null || app.tool === "fill" || app.eyedropperTarget !== null)));
  let samplingDrag = false;
  /** Dégradé en cours de tracé : ligne du point de départ au pointeur, en pixels du document. */
  let gradient = $state<{ from: Point; to: Point } | null>(null);
  const gradientLine = $derived.by(() => {
    const camera = app.active?.camera;
    if (!gradient || !camera) return null;
    const at = (p: Point) => ({ x: camera.x + p.x * camera.zoom, y: camera.y + p.y * camera.zoom });
    return { from: at(gradient.from), to: at(gradient.to) };
  });
  /** Redimensionnement du pinceau en cours (Alt + clic droit glissé) : valeurs et point de départ. */
  let brushResize = $state<{ x: number; y: number; size: number; hardness: number; at: { x: number; y: number }; axis: "size" | "hardness" | null } | null>(null);
  let drag = $state<{ pointerId: number; lastX: number; lastY: number } | null>(null);
  let stroke: ActiveStroke | null = null;
  /** Position du pointeur dans la zone de travail, pour le curseur du pinceau. */
  let hover = $state<{ x: number; y: number } | null>(null);

  const panning = $derived(spaceHeld || app.tool === "hand");
  const brushDiameter = $derived.by(() => {
    const tool = app.paintTool;
    const zoom = app.active?.camera?.zoom;
    // Un champ numérique vidé vaut `null` : la taille retombe alors à 1 px.
    if (app.tool === "quickSelect" && zoom && !panning) return Math.max(1, app.quickSelect.size || 1) * zoom;
    if (app.tool === "heal" && zoom && !panning) return Math.max(1, app.heal.size || 1) * zoom; // même cercle que le pinceau
    return tool && zoom && !panning ? Math.max(1, app.brushes[tool].size || 1) * zoom : 0;
  });
  /** Repère verrouillé : il reste visible et aimante toujours, mais ne se déplace plus. */
  const guidesLocked = $derived(app.active?.info.guidesLocked ?? false);
  /** Orientation du repère survolé, pour le curseur. */
  let hoveredGuide = $state<Guide["orientation"] | null>(null);

  const cursor = $derived(
    hoveredGuide && !guidesLocked && !sampling
      ? hoveredGuide === "horizontal"
        ? "row-resize"
        : "col-resize"
      : drag
      ? "grabbing"
      : panning
        ? "grab"
        : sampling
          ? "var(--cursor-pipette)"
        : app.tool === "zoom"
          ? altHeld
            ? "zoom-out"
            : "zoom-in"
          : app.paintTool
            ? brushDiameter > 6
              ? "none"
              : "crosshair"
            : app.tool === "move"
              ? "move"
              : app.tool === "text"
                ? "text"
                : "crosshair",
  );

  // ---- Résolution dynamique : pendant un geste, le canevas est plus petit que l'écran (voir lib/resolution.ts).
  const resolution = new DynamicResolution(app.performance.adaptiveResolution !== false);
  $effect(() => resolution.setEnabled(app.performance.adaptiveResolution !== false));
  /** Un bouton de la souris est tenu sur la zone de travail. */
  let pointerHeld = false;
  /** Pixels du canevas par pixel CSS : rapport de pixels de l'écran, réduit par la résolution dynamique. */
  function effectiveDpr(): number {
    return window.devicePixelRatio * resolution.factor;
  }
  function gesturing(): boolean {
    return pointerHeld || animation !== null || wheelZoom !== null || moving !== null || stroke !== null || (renderer?.previewing() ?? false);
  }
  /** Taille du canevas en pixels, d'après sa taille CSS et le facteur courant. */
  function applyCanvasSize(): void {
    if (!canvas) return;
    const dpr = effectiveDpr();
    const width = Math.max(1, Math.round(canvas.clientWidth * dpr));
    const height = Math.max(1, Math.round(canvas.clientHeight * dpr));
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    app.renderScale = resolution.factor;
  }

  /**
   * Une seule boucle par image. Ce qui doit changer avant le dessin (déplacement de la vue cumulé, pas d'animation
   * du zoom, position affichée du pointeur) est confié à `beforeFrame` : ces tâches tournent en tête de l'image, puis
   * l'état de l'interface est appliqué tout de suite (`flushSync` : cadre de transformation, repères, règles) et le
   * canevas est dessiné DANS LE MÊME rappel. Auparavant la caméra changeait dans un rappel et le dessin était
   * redemandé pour l'image suivante : le cadre bleu (DOM) avait une image d'avance sur l'image WebGL, d'où le
   * décalage en déplaçant vite, et une image de latence en plus sur tout.
   */
  const frameTasks = new Set<(now: number) => void>();
  /** Vrai entre le début de l'image et son dessin : un `scheduleDraw` arrivant là est servi par cette même image. */
  let preparingFrame = false;
  function beforeFrame(task: (now: number) => void): void {
    frameTasks.add(task);
    scheduleDraw();
  }

  /** Au plus un rendu par image affichée, et seulement quand quelque chose a changé. */
  function scheduleDraw(): void {
    if (frameRequested || preparingFrame) return;
    frameRequested = true;
    requestAnimationFrame((now) => {
      frameRequested = false;
      preparingFrame = true;
      try {
        const tasks = [...frameTasks];
        frameTasks.clear();
        for (const task of tasks) task(now);
        flushSync();
      } finally {
        preparingFrame = false;
      }
      const changed = gesturing() ? resolution.gestureFrame(now) : resolution.endGesture();
      if (changed) applyCanvasSize();
      if (renderer && app.performance.showFps && !renderer.suspended) {
        // Mesure : temps du code, puis temps de la carte (l'attente la bloque, mais la case est un mode de mesure).
        const started = performance.now();
        renderer.draw(app.active?.camera ?? { zoom: 1, x: 0, y: 0 }, effectiveDpr());
        const js = performance.now() - started;
        noteDraw(js, renderer.waitForGpu());
      } else {
        renderer?.draw(app.active?.camera ?? { zoom: 1, x: 0, y: 0 }, effectiveDpr());
      }
      // Le geste vient de finir à résolution réduite : une image nette suit aussitôt. Une tâche ajoutée pendant
      // l'image (animation qui continue) en demande une autre.
      if ((!gesturing() && resolution.factor !== 1) || frameTasks.size > 0) scheduleDraw();
    });
  }

  function resize(): void {
    if (!canvas) return;
    app.viewport = { width: canvas.clientWidth, height: canvas.clientHeight };
    applyCanvasSize();
    // Vue en « taille écran » : elle suit la fenêtre (agrandissement, plein écran, panneaux…).
    // Dès qu'on a zoomé ou déplacé la vue soi-même, elle n'est plus touchée.
    const active = app.active;
    if (active?.autoFit && active.camera && app.viewport.width > 0) {
      stopCameraAnimation();
      app.setCamera(fitCamera(active.info.width, active.info.height, app.viewport.width, app.viewport.height), true);
    }
    scheduleDraw();
  }

  onMount(() => {
    if (!canvas) return;
    const gray = getComputedStyle(canvas).getPropertyValue("--bg-canvas").trim();
    const channel = (i: number) => Number.parseInt(gray.slice(1 + i * 2, 3 + i * 2), 16) / 255;
    try {
      renderer = new ViewportRenderer(
        canvas,
        (level, x, y) => {
          const id = app.activeId;
          return id === null ? Promise.resolve(new ArrayBuffer(8)) : api.getTile(id, level, x, y);
        },
        (level, x, y) => {
          const id = app.activeId;
          return id === null ? Promise.resolve(new ArrayBuffer(9)) : api.getSelectionTile(id, level, x, y);
        },
        (layers, opaque) => (level, x, y) => {
          const id = app.activeId;
          return id === null ? Promise.resolve(new ArrayBuffer(8)) : api.getTilePart(id, level, x, y, layers, opaque);
        },
        scheduleDraw,
        [channel(0), channel(1), channel(2)],
        (docId, layer, rect, part, limit) =>
          part === "mask" ? api.layerMask(docId, layer, rect, limit) : api.layerPixels(docId, layer, rect, part === "pixels" ? null : part, limit),
        (adjustment) => api.adjustmentGpu(adjustment),
        (docId, layer) => api.layerEffects(docId, layer),
      );
    } catch (error) {
      app.fail(error);
      return;
    }
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    provideFrameSettled((layers) => renderer?.whenSettled(layers) ?? Promise.resolve());
    app.cameraAnimator = (target, fit) => animateCamera(target, 160, fit);
    app.fitToWindow = () => {
      const active = app.active;
      if (active) animateCamera(fitCamera(active.info.width, active.info.height, app.viewport.width, app.viewport.height), 200, true);
    };
    provideAdjustPreview({
      begin: (plan) => (renderer?.beginAdjustPreview(plan), scheduleDraw()),
      setLut: (bytes, size, mix) => (renderer?.setAdjustLut(bytes, size, mix), scheduleDraw()),
      composes: (layer) => renderer?.composesAdjustment(layer) ?? false,
      setValues: (layer, bytes) => (renderer?.setAdjustValues(layer, bytes), scheduleDraw()),
      finish: () => (renderer?.finishAdjustPreview(), scheduleDraw()),
    });
    providePreview({
      draft: (levels) => (renderer?.setDraftLevel(levels), scheduleDraw()),
      composes: () => renderer?.gpuComposes() ?? false,
      begin: (plan, opaque) => (renderer?.beginMovePreview(plan, opaque), scheduleDraw()),
      setAlpha: (alpha) => (renderer?.setPreviewAlpha(alpha), scheduleDraw()),
      end: () => (renderer?.endMovePreview(), scheduleDraw()),
    });
    const unsubscribe = onDirty((docId, dirty, revision, immediate) => {
      if (docId !== app.activeId) return;
      // Description du document après la modification d'abord (l'effet qui la transmet ne passe qu'après) : la
      // composition sur carte graphique place les zones repeintes d'après la position ACTUELLE des calques.
      if (app.active) renderer?.setDocument(app.active.info);
      renderer?.invalidate(dirty, revision, immediate);
      scheduleDraw();
    });
    return () => {
      provideFrameSettled(null);
      app.cameraAnimator = null;
      app.fitToWindow = null;
      stopCameraAnimation();
      providePreview(null);
      unsubscribe();
      observer.disconnect();
      renderer?.dispose();
    };
  });

  // Changement de document, premier cadrage, puis tout mouvement de caméra.
  $effect(() => {
    const active = app.active;
    renderer?.setDocument(active?.info ?? null);
    if (active && !active.camera && app.viewport.width > 0) {
      active.camera = fitCamera(active.info.width, active.info.height, app.viewport.width, app.viewport.height);
    }
    void active?.camera?.zoom;
    void active?.camera?.x;
    void active?.camera?.y;
    scheduleDraw();
  });

  // Le contour de sélection est animé : tant qu'il existe, l'affichage est rafraîchi dix fois par seconde.
  $effect(() => {
    if (!app.active?.info.hasSelection) return;
    const timer = setInterval(scheduleDraw, 100);
    return () => clearInterval(timer);
  });

  // Changer d'outil abandonne un tracé en cours.
  $effect(() => {
    void app.tool;
    void app.marqueeShape;
    void app.lassoShape;
    draft = null;
  });

  // ------------------------------------------------------------- zoom fluide

  /**
   * Transition de caméra. Le zoom est interpolé sur une échelle logarithmique (un passage de
   * 25 % à 100 % paraît régulier), et la position suit de sorte que le point visé reste fixe.
   * Un nouvel ordre pendant une transition repart de l'état courant vers la nouvelle cible.
   */
  let animation: { from: Camera; to: Camera; start: number; duration: number; tick: (now: number) => void } | null = null;
  /**
   * Zoom à la molette ou au pincement : UNE cible (zoom visé, point de l'écran qui doit rester fixe) que chaque
   * événement ajuste, et vers laquelle l'affichage glisse à chaque image, sans jamais être arrêté ni relancé.
   * Auparavant un petit pas (pincement) coupait l'animation d'un grand pas (cran) et repartait d'ailleurs : avec le
   * défilement doux de Chromium, qui découpe chaque cran en petits et grands pas mélangés, le zoom arrière
   * remontait par moments avant de redescendre.
   */
  let wheelZoom: { zoom: number; anchor: { x: number; y: number }; tick: (now: number) => void } | null = null;
  /** Constante de temps du glissement vers la cible : court, pour sentir chaque cran sans à-coup. */
  const WHEEL_SMOOTHING_MS = 45;
  let wheelLast = 0;

  function wheelZoomTo(zoom: number, anchor: { x: number; y: number }): void {
    const camera = app.active?.camera;
    if (!camera) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      app.setCamera(zoomAt(camera, zoom, anchor.x, anchor.y));
      return;
    }
    if (animation) stopCameraAnimation();
    if (wheelZoom) {
      wheelZoom.zoom = zoom;
      wheelZoom.anchor = anchor;
      return;
    }
    wheelLast = performance.now();
    const tick = (now: number) => {
      const run = wheelZoom;
      const current = app.active?.camera;
      if (!run || !current) return;
      const dt = Math.min(100, Math.max(1, now - wheelLast));
      wheelLast = now;
      const remaining = Math.log(run.zoom / current.zoom);
      if (Math.abs(remaining) < 0.002) {
        app.setCamera(zoomAt(current, run.zoom, run.anchor.x, run.anchor.y));
        wheelZoom = null;
        return;
      }
      // Rapprochement exponentiel : même courbe quel que soit le rythme des images.
      const alpha = 1 - Math.exp(-dt / WHEEL_SMOOTHING_MS);
      app.setCamera(zoomAt(current, current.zoom * Math.exp(remaining * alpha), run.anchor.x, run.anchor.y));
      beforeFrame(tick);
    };
    wheelZoom = { zoom, anchor, tick };
    beforeFrame(tick);
  }


  function animateCamera(to: Camera, duration = 160, fit = false): void {
    const from = app.active?.camera;
    if (!from) return;
    if (animation) frameTasks.delete(animation.tick);
    if (wheelZoom) {
      frameTasks.delete(wheelZoom.tick);
      wheelZoom = null;
    }
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      app.setCamera(to, fit);
      return;
    }
    const tick = (now: number) => {
      const run = animation;
      if (!run) return;
      const t = Math.min(1, (now - run.start) / run.duration);
      const eased = 1 - (1 - t) ** 3; // départ franc, arrivée douce
      const zoom = run.from.zoom * (run.to.zoom / run.from.zoom) ** eased;
      // Position interpolée en fonction du zoom atteint : le point d'ancrage ne glisse pas.
      const progress = run.to.zoom === run.from.zoom ? eased : (zoom - run.from.zoom) / (run.to.zoom - run.from.zoom);
      // `fit` : l'arrivée est un ajustement à la fenêtre, qui restera suivi au redimensionnement.
      app.setCamera({ zoom, x: run.from.x + (run.to.x - run.from.x) * progress, y: run.from.y + (run.to.y - run.from.y) * progress }, fit);
      if (t < 1) beforeFrame(tick);
      else animation = null;
    };
    animation = { from: { ...from }, to, start: performance.now(), duration, tick };
    beforeFrame(tick);
  }

  function stopCameraAnimation(): void {
    if (animation) frameTasks.delete(animation.tick);
    if (wheelZoom) frameTasks.delete(wheelZoom.tick);
    [animation, wheelZoom] = [null, null];
  }

  /**
   * Déplacement de la vue au clic molette (ou Espace + clic) par-dessus les calques d'interface :
   * plume, saisie de texte, cadre de transformation. Ces calques recouvrent l'image et recevraient
   * le clic à sa place ; il est donc intercepté avant eux, à la descente de l'événement.
   */
  let overlayPan: { pointerId: number; x: number; y: number } | null = null;

  function onStagePointerDownCapture(event: PointerEvent): void {
    const wantsPan = event.button === 1 || (event.button === 0 && panning);
    if (!wantsPan || event.target === canvas || !app.active?.camera) return; // sur l'image nue, le canevas s'en charge déjà
    event.stopPropagation();
    event.preventDefault();
    stopCameraAnimation();
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    overlayPan = { pointerId: event.pointerId, x: event.clientX, y: event.clientY };
  }

  function onStagePointerMove(event: PointerEvent): void {
    const camera = app.active?.camera;
    if (!overlayPan || overlayPan.pointerId !== event.pointerId || !camera) return;
    queuePan(event.clientX - overlayPan.x, event.clientY - overlayPan.y);
    [overlayPan.x, overlayPan.y] = [event.clientX, event.clientY];
  }

  // ---- Déplacement de la vue (main, Espace, cadre) : les mouvements du pointeur sont cumulés et appliqués UNE fois
  // par image. Une souris à 1 000 Hz envoyait mille positions par seconde, et chacune faisait recalculer règles,
  // cadre et barre d'état : le fil principal saturait et l'affichage bégayait, alors
  // qu'une image ne peut de toute façon montrer qu'une position. Même chose pour la position affichée du pointeur.
  let panDelta: [number, number] = [0, 0];
  let panQueued = false;
  function queuePan(dx: number, dy: number): void {
    panDelta = [panDelta[0] + dx, panDelta[1] + dy];
    if (panQueued) return;
    panQueued = true;
    beforeFrame(() => {
      panQueued = false;
      const camera = app.active?.camera;
      const [px, py] = panDelta;
      panDelta = [0, 0];
      if (camera && (px !== 0 || py !== 0)) app.setCamera({ ...camera, x: camera.x + px, y: camera.y + py });
    });
  }
  let pointerQueued = false;
  function queuePointer(): void {
    if (pointerQueued) return;
    pointerQueued = true;
    // Pas `beforeFrame` : la position du pointeur (barre d'état) ne redessine pas la zone de travail. En passant par la
    // boucle de dessin, chaque mouvement de souris, même sans clic, redessinait toute la zone en 4K (beta90b).
    requestAnimationFrame(() => {
      pointerQueued = false;
      const camera = app.active?.camera;
      if (!hover || !camera) return;
      const next = { x: Math.floor((hover.x - camera.x) / camera.zoom), y: Math.floor((hover.y - camera.y) / camera.zoom) };
      if (app.pointer?.x !== next.x || app.pointer?.y !== next.y) app.pointer = next;
    });
  }

  function localPoint(event: MouseEvent): { x: number; y: number } {
    const rect = canvas?.getBoundingClientRect();
    return { x: event.clientX - (rect?.left ?? 0), y: event.clientY - (rect?.top ?? 0) };
  }

  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    const camera = app.active?.camera;
    if (!camera) return;
    // Alt+molette comme Photoshop ; Ctrl+molette est aussi ce qu'émet un pincement au pavé tactile.
    if (event.altKey || event.ctrlKey) {
      // Chaque pas, petit (pincement, défilement doux) ou grand (cran), déplace la cible ; un cran entier
      // (120 unités) vaut ×0,77 ou ×1,3. Pas de distinction par la taille du pas : elle n'est pas fiable.
      const base = wheelZoom?.zoom ?? camera.zoom;
      wheelZoomTo(clampZoom(base * Math.exp(-event.deltaY * 0.0022)), localPoint(event));
    } else {
      stopCameraAnimation();
      const [dx, dy] = event.shiftKey ? [event.deltaY, event.deltaX] : [event.deltaX, event.deltaY];
      app.setCamera({ ...camera, x: camera.x - dx, y: camera.y - dy });
    }
  }

  // ------------------------------------------------------------------ pinceau

  /** `[x, y, pression]` en pixels du document. La souris vaut une pression de 1. */
  function strokePoint(event: PointerEvent): number[] | null {
    const camera = app.active?.camera;
    if (!camera) return null;
    const { x, y } = localPoint(event);
    const pressure = event.pointerType === "pen" ? event.pressure : 1;
    return [(x - camera.x) / camera.zoom, (y - camera.y) / camera.zoom, pressure];
  }

  /**
   * Stabilisateur : le trait suit le pointeur comme au bout d'un élastique. Chaque point reçu ne
   * rapproche la pointe que d'une fraction de l'écart, ce qui gomme les tremblements de la main et
   * les à-coups de la souris. Plus le lissage est fort, plus la pointe traîne ; elle rattrape le
   * pointeur au relâchement. S'ajoute au lissage par courbes du moteur, qui arrondit les angles.
   */
  let tip: number[] | null = null;

  function stabilize(raw: number[]): number[] {
    const follow = 1 - app.strokeSmoothing * 0.9;
    const [x = 0, y = 0, p = 1] = raw;
    const [tx = x, ty = y, tp = p] = tip ?? raw;
    tip = [tx + (x - tx) * follow, ty + (y - ty) * follow, tp + (p - tp) * follow];
    return tip;
  }

  /** Points qui ramènent la pointe jusqu'au pointeur, pour que le trait finisse là où on a lâché. */
  function catchUp(raw: number[]): number[][] {
    const points: number[][] = [];
    for (let i = 0; i < 12 && tip && Math.hypot((raw[0] ?? 0) - (tip[0] ?? 0), (raw[1] ?? 0) - (tip[1] ?? 0)) > 0.5; i++) points.push(stabilize(raw));
    return [...points, raw];
  }

  // Épreuve CMJN.
  $effect(() => {
    if (!renderer) return;
    renderer.proofProfile = app.proofProfile;
    scheduleDraw();
  });

  // Accélération matérielle : composition des calques par la carte graphique, dans la limite de mémoire réservée
  // (une part de la mémoire vidéo détectée ; voir performance.svelte.ts).
  $effect(() => {
    renderer?.setGpuOptions(app.performance.gpuPreviews, gpuMemoryBudgetMb(app.performance, { vramMb: app.systemInfo?.vramMb, vramShared: app.systemInfo?.vramShared, ramMb: app.systemInfo?.ramMb }));
  });

  // État de la composition GPU, relevé deux fois par seconde (Préférences, barre d'état) : sans effet sur le rendu.
  $effect(() => {
    const timer = setInterval(() => {
      const status = renderer?.gpuStatus();
      if (status && JSON.stringify(status) !== JSON.stringify(app.gpuStatus)) app.gpuStatus = status;
    }, 500);
    return () => clearInterval(timer);
  });

  // Jauge de chargement des calques (barre d'état) : relevée dix fois par seconde, affichée seulement si le
  // chargement dure (plus de 300 ms), pour ne pas clignoter à chaque petit rechargement.
  $effect(() => {
    let since: number | null = null;
    const timer = setInterval(() => {
      const progress = renderer?.loadProgress() ?? null;
      if (progress === null) {
        since = null;
        if (app.loading !== null) app.loading = null;
        return;
      }
      since ??= performance.now();
      if (performance.now() - since < 300) return;
      if (app.loading?.done !== progress.done || app.loading.total !== progress.total) app.loading = progress;
    }, 100);
    return () => clearInterval(timer);
  });

  // Masque rapide : l'affichage de la sélection bascule entre contour animé et rouge translucide.
  $effect(() => {
    if (renderer) renderer.quickMask = app.quickMask;
    scheduleDraw();
  });

  /** Masque rapide : trait qui peint la sélection. Peindre ajoute ; Alt (ou la gomme) retire. */
  function beginSelectionStroke(event: PointerEvent): void {
    const doc = app.active;
    const tool = app.paintTool;
    const first = strokePoint(event);
    if (!doc || !tool || !first || !canvas) return;
    const settings = app.brushes[tool];
    const docId = doc.info.id;
    const erase = tool === "eraser" || event.altKey;
    [tip, lastRaw, dabPath] = [first, first, null];
    const current: ActiveStroke = { docId, pending: [first], sending: false, ended: false };
    stroke = current;
    canvas.setPointerCapture(event.pointerId);
    const brush = { size: Math.max(1, settings.size || 1), hardness: settings.hardness, opacity: settings.opacity, color: [255, 255, 255] as [number, number, number], erase, pressureSize: settings.pressureSize };
    enqueue(docId, () => api.strokeBeginSelection(docId, brush)).then(
      () => pump(current),
      (error: unknown) => {
        current.ended = true;
        if (stroke === current) stroke = null;
        app.fail(error);
      },
    );
  }

  function beginStroke(event: PointerEvent): void {
    if (app.quickMask) {
      beginSelectionStroke(event);
      return;
    }
    const doc = app.active;
    const layer = app.activeLayer;
    const tool = app.paintTool;
    const first = strokePoint(event);
    if (!doc || !tool || !first) return;
    if (!layer || doc.selection.length !== 1 || (layer.isGroup && doc.target !== "mask")) {
      app.error = t("Sélectionnez un seul calque de pixels pour peindre.");
      return;
    }
    const settings = app.brushes[tool];
    const docId = doc.info.id;
    const size = Math.max(1, settings.size || 1);
    tip = first;
    lastRaw = first;
    // L'aperçu local n'est utilisé que si sa vérification date bien de l'état courant du document.
    const local = localPaint?.layerId === layer.id && localPaint.revision === doc.info.revision ? localPaint : null;
    // Pinceau rond flou : empreintes tous les 10 % du diamètre, à un tiers de force, qui s'additionnent (comme le moteur).
    const softRound = app.brushKind.type === "round" && settings.hardness < 1;
    dabPath = local ? new DabPath(size, settings.pressureSize, softRound ? 0.1 : null) : null;
    if (local && dabPath) {
      // Esquisse des autres formes : un rond approchant. Spray : très doux et léger ; matière : franc, un peu allégé.
      const kind = app.brushKind.type;
      const [sketchAlpha, sketchHardness] = kind === "spray" ? [0.35, 0] : kind === "tip" ? [0.7, 0.8] : [1, settings.hardness];
      renderer?.beginStrokePreview(app.foreground, settings.opacity * local.alpha * sketchAlpha, sketchHardness, softRound);
      renderer?.addStrokeDabs(dabPath.add(first));
      scheduleDraw();
    }
    const current: ActiveStroke = { docId, pending: [first], sending: false, ended: false };
    stroke = current;
    enqueue(docId, () =>
      api.strokeBegin(docId, layer.id, doc.target === "mask", {
        ...settings,
        size,
        color: app.foreground,
        erase: tool === "eraser",
        kind: $state.snapshot(app.brushKind),
        // Trait esquissé par la carte graphique : le moteur n'a pas à recomposer le calque pendant le geste.
        deferred: dabPath !== null,
        dynamics: $state.snapshot(app.brushDynamics),
      }),
    ).then(
      () => pump(current),
      (error: unknown) => {
        if (stroke === current) stroke = null;
        app.fail(error);
      },
    );
  }

  // Aperçu du trait par la carte graphique. Le moteur reste la référence (c'est son rendu qui est
  // enregistré), mais il n'est plus sur le chemin de l'affichage : un pinceau de 500 px suit la
  // souris aussi bien qu'un pinceau de 5 px. Possible quand le trait se pose simplement PAR-DESSUS
  // l'image : calque en mode Normal, sans masque, écrêtage, verrou ni sélection, et rien au-dessus.
  // Sinon (gomme, masque, mode de fusion…), l'affichage suit le moteur comme avant.
  let localPaint = $state<{ layerId: number; revision: number; alpha: number } | null>(null);
  let dabPath: DabPath | null = null;
  let lastRaw: number[] | null = null;

  $effect(() => {
    const doc = app.active;
    const layer = app.activeLayer;
    localPaint = null;
    if (!doc || !layer || app.tool !== "brush" || doc.selection.length !== 1 || doc.target === "mask") return;
    // Priorité à la fluidité : pendant le geste, le trait est dessiné par la carte graphique, par-dessus
    // l'image, quel que soit le calque (écrêté, masqué, sous d'autres calques, en mode de fusion, avec une
    // sélection…). C'est une ESQUISSE : elle ignore ces particularités. Au relâchement, le rendu exact du
    // moteur la remplace d'un seul coup. Seul le mélangeur, qui dépend de ce qu'il traverse, n'a pas d'esquisse.
    const sketchable = app.performance.gpuPreviews && app.brushKind.type !== "mixer" && !layer.isGroup && !layer.text && !layer.shape && !layer.adjustment && layer.visible && !layer.locks.pixels && !layer.locks.all;
    if (sketchable) localPaint = { layerId: layer.id, revision: doc.info.revision, alpha: layer.opacity * layer.fill };
  });

  /**
   * Envoie les points accumulés. Un seul lot est en vol à la fois : si le moteur
   * est plus lent que le stylet, les points s'accumulent ici au lieu de saturer l'IPC.
   */
  function pump(current: ActiveStroke): void {
    if (current.sending) return;
    if (current.pending.length === 0) {
      if (current.ended) finishStroke(current);
      return;
    }
    current.sending = true;
    const batch = current.pending.splice(0);
    enqueue(current.docId, () => api.strokePoints(current.docId, batch))
      // Aperçu local : les tuiles du moteur s'accumulent en coulisse. Sinon, elles s'affichent une à une.
      .then(([revision, dirty]) => notifyDirty(current.docId, dirty, revision, dabPath === null))
      .catch((error: unknown) => app.fail(error))
      .finally(() => {
        current.sending = false;
        pump(current);
      });
  }

  function finishStroke(current: ActiveStroke): void {
    if (stroke === current) stroke = null;
    enqueue(current.docId, () => api.strokeEnd(current.docId))
      .then(
        (result) => adopt(result.document, result.dirty),
        (error: unknown) => app.fail(error),
      )
      .finally(() => {
        // Le moteur a fini : son rendu remplace l'aperçu, d'un seul coup.
        if (stroke === null) renderer?.finishStrokePreview();
      });
  }

  // --------------------------------------------------------------- sélections

  function docPoint(event: MouseEvent): Point | null {
    const camera = app.active?.camera;
    if (!camera) return null;
    const { x, y } = localPoint(event);
    return { x: (x - camera.x) / camera.zoom, y: (y - camera.y) / camera.zoom };
  }

  /** Modificateurs au début du geste, comme Photoshop. */
  function combineMode(event: MouseEvent): CombineMode {
    if (event.shiftKey && event.altKey) return "intersect";
    return event.shiftKey ? "add" : event.altKey ? "subtract" : "replace";
  }

  /** Rectangle d'un tracé en boîte, contraintes appliquées (carré / cercle, depuis le centre). */
  function boxOf(box: Extract<Draft, { kind: "box" }>): { x: number; y: number; width: number; height: number } {
    let width = box.end.x - box.start.x;
    let height = box.end.y - box.start.y;
    if (box.constrain) {
      const side = Math.max(Math.abs(width), Math.abs(height));
      width = Math.sign(width || 1) * side;
      height = Math.sign(height || 1) * side;
    }
    return box.centered
      ? { x: box.start.x - width, y: box.start.y - height, width: width * 2, height: height * 2 }
      : { x: box.start.x, y: box.start.y, width, height };
  }

  /** Tracés de sélection envoyés au moteur, encore affichés le temps qu'il réponde. */
  const sent = new WeakSet<object>();

  function commit(shape: SelectionShape, mode: CombineMode): void {
    // Nouvelle sélection qui remplace l'ancienne : l'ancien contour disparaît aussitôt, et le tracé qu'on vient
    // de faire reste affiché jusqu'à ce que le moteur ait rendu la sélection (plus de « flash » de l'ancienne).
    if (mode === "replace") renderer?.dropSelection();
    const pending = draft;
    if (pending) sent.add(pending); // figé : il ne suit plus la souris en attendant
    void edit({ op: "select", shape, mode }).finally(() => {
      if (draft === pending) draft = null;
    });
  }

  function closePath(): void {
    if (draft?.kind !== "path") return;
    const { points, mode } = draft;
    if (points.length >= 3) commit({ kind: "polygon", points: points.map((p) => [p.x, p.y]) }, mode);
    else draft = null;
  }

  // ------------------------------------------------------------------- texte

  const PLACEHOLDER = "Lorem ipsum";

  /**
   * Clic avec l'outil Texte. Sur un texte existant : on y place le curseur. Ailleurs : un
   * calque de texte vide est créé à cet endroit. Dans les deux cas on tape directement sur l'image.
   */
  /** Double-clic sur un texte, avec n'importe quel outil : passage à l'outil Texte et saisie dans ce texte. */
  async function editTextAt(event: MouseEvent): Promise<void> {
    const doc = app.active;
    const at = docPoint(event);
    if (!doc || !at || app.textEdit || transformIsModal()) return;
    const picked = await api.pickTextLayer(doc.info.id, Math.floor(at.x), Math.floor(at.y)).catch(() => null);
    if (picked === null) return;
    if (transformIsPending()) commitTransform();
    app.tool = "text";
    await placeText(at);
  }

  async function placeText(at: Point): Promise<void> {
    const doc = app.active;
    if (!doc) return;
    const id = `saisie-${Date.now()}`;
    try {
      // Un texte existant sous le pointeur est rouvert, quel que soit le calque actif et même s'il est recouvert.
      const picked = await api.pickTextLayer(doc.info.id, Math.floor(at.x), Math.floor(at.y));
      const existing = doc.info.layers.find((l) => l.id === picked && l.text);
      if (existing?.text) {
        doc.selection = [existing.id];
        const caretAt = await api.textHit(doc.info.id, existing.id, at.x, at.y);
        const runs = structuredClone($state.snapshot(existing.text.runs ?? []));
        app.textEdit = { id, docId: doc.info.id, layerId: existing.id, draft: existing.text.content, runs, selection: { start: caretAt, end: caretAt }, isNew: false, caretAt, selectAll: false };
        return;
      }
      // Comme Photoshop : un texte d'exemple donne tout de suite un aperçu de la police et de la
      // taille. Il est sélectionné en entier, donc la première lettre tapée le remplace.
      const text = { ...app.textDefaults, content: PLACEHOLDER };
      if (!(await edit({ op: "addText", x: Math.round(at.x), y: Math.round(at.y), text, above: app.activeLayer?.id ?? null }))) return;
      // Le calque créé est devenu le calque actif.
      const created = app.activeLayer;
      if (created?.text) app.textEdit = { id, docId: doc.info.id, layerId: created.id, draft: PLACEHOLDER, runs: [], selection: { start: 0, end: PLACEHOLDER.length }, isNew: true, caretAt: null, selectAll: true };
    } catch (error) {
      app.fail(error);
    }
  }

  // Changer d'outil, de calque ou de document valide la saisie en cours. La validation ne passe pas par
  // la surcouche de saisie : quand le calque actif change, elle est déjà démontée (sa condition
  // d'affichage ne tient plus) et le calque de texte resté vide n'était alors jamais retiré.
  $effect(() => {
    const editing = app.textEdit;
    if (editing && (app.tool !== "text" || app.activeLayer?.id !== editing.layerId)) untrack(finishTextEdit);
  });

  // ------------------------------------------------------------- déplacement

  /**
   * Alt + glisser : duplique les calques sélectionnés et déplace les copies (Maj en plus : le
   * déplacement reste sur un axe). Les copies deviennent la sélection.
   */
  async function duplicateSelection(): Promise<boolean> {
    const doc = app.active;
    if (!doc || doc.selection.length === 0) return false;
    const before = new Set(doc.info.layers.map((l) => l.id));
    for (const id of [...doc.selection]) {
      if (!(await edit({ op: "duplicate", id }))) return false;
    }
    const created = (app.active?.info.layers ?? []).filter((l) => !before.has(l.id));
    // Seules les copies de plus haut niveau sont sélectionnées : leurs enfants suivent leur groupe.
    const top = created.filter((l) => l.parent === null || before.has(l.parent)).map((l) => l.id);
    if (app.active && top.length > 0) app.active.selection = top;
    return true;
  }

  /**
   * `additive` (Maj + clic) : le calque cliqué s'AJOUTE à la sélection. Tous les calques sélectionnés partagent
   * alors un cadre commun (l'étendue qui les englobe tous) : les déplacer ou les transformer agit sur tous à la fois.
   */
  function beginMove(at: Point, duplicate = false, additive = false): void {
    const doc = app.active;
    if (!doc) return;
    // Une poignée du cadre a été tirée : cliquer ailleurs applique la transformation.
    if (transformIsPending()) commitTransform();
    const gesture: MoveGesture = { id: nextGesture(), kind: doc.info.hasSelection ? "pixels" : "layers", start: at, ready: true, bounds: null, local: false, dx: 0, dy: 0 };
    moving = gesture;
    isMoving = true;
    if (gesture.kind === "layers" && app.autoSelect) {
      gesture.ready = false;
      api.pickLayer(doc.info.id, Math.floor(at.x), Math.floor(at.y)).then(
        (picked) => {
          if (picked === null) {
            // Clic hors de tout contenu (plan de travail, zone vide) : plus aucun calque sélectionné — sauf avec Maj,
            // qui ne fait qu'ajouter.
            if (!additive) doc.selection = [];
            if (moving === gesture) [moving, isMoving] = [null, false];
            return;
          }
          // Maj + clic : le calque rejoint la sélection. Sans Maj, un clic sur un calque déjà sélectionné garde la
          // sélection multiple intacte (pour la déplacer d'un bloc) ; sur un autre, il la remplace.
          const before = [...doc.selection];
          if (additive && !doc.selection.includes(picked)) doc.selection = [...doc.selection, picked];
          else if (!doc.selection.includes(picked)) doc.selection = [picked];
          // Le geste a changé la sélection : le cadre affiché est celui de l'ANCIEN calque (son effet ne se relance pas
          // pendant un glisser). Il est retiré tout de suite, puis reposé sur le calque saisi dès que son étendue
          // arrive (`loadBounds`) — sinon le cadre de A suivait la souris pendant que B se déplaçait.
          if (before.length !== doc.selection.length || before.some((id, i) => id !== doc.selection[i])) {
            gesture.reframe = true;
            selectionBounds = null;
            if (app.transform?.auto) app.transform = null;
          }
          const start = duplicate ? duplicateSelection() : Promise.resolve(true);
          void start.then((ok) => {
            if (!ok || moving !== gesture) return;
            gesture.ready = true;
            loadBounds(gesture);
          });
        },
        (error: unknown) => app.fail(error),
      );
    } else if (gesture.kind === "layers" && !duplicate && positionLocked(doc.info, doc.selection)) {
      // Sélection automatique coupée et calque verrouillé choisi dans le panneau : rien ne bouge, on le dit au lieu de
      // laisser glisser un cadre que le moteur refuserait.
      [moving, isMoving] = [null, false];
      app.error = t("Calque verrouillé : retirez le cadenas pour le déplacer.");
    } else if (duplicate && gesture.kind === "layers") {
      gesture.ready = false;
      void duplicateSelection().then((ok) => {
        if (!ok || moving !== gesture) return;
        gesture.ready = true;
        loadBounds(gesture);
      });
    } else {
      loadBounds(gesture);
    }
  }

  /**
   * Ce que déplace l'outil Déplacement sur un calque à masque : la partie visée seule (pixels, ou masque), ou les
   * deux si les deux vignettes sont visées (Ctrl + clic). Sans masque, avec plusieurs calques, ou sur un dossier :
   * tout. (Un dossier bouge d'un bloc, masque compris : déplacer « ses pixels » seuls n'a pas de sens — il n'en a
   * pas — et faisait recopier son masque à chaque pas du geste, texture rechargée et contenu laissé sur place.)
   */
  function movedParts(): "pixels" | "mask" | "both" {
    const doc = app.active;
    const layer = app.activeLayer;
    if (!doc || !layer?.hasMask || layer.isGroup || doc.selection.length !== 1) return "both";
    return doc.target;
  }

  /**
   * Déplacement prévisualisable par la carte graphique : pas de sélection de pixels (seuls ses pixels bougeraient) et
   * aucun calque verrouillé en position (le moteur refuserait le déplacement).
   */
  function gpuMovable(doc: NonNullable<typeof app.active>): boolean {
    if (doc.info.hasSelection) return false;
    const ids = new Set(withDescendants(doc.info, doc.selection));
    return doc.info.layers.every((l) => !ids.has(l.id) || (!l.locks.position && !l.locks.all));
  }

  /** Calques et tout leur contenu (un groupe déplacé emporte ses calques). */
  function withDescendants(info: DocumentInfo, ids: readonly number[]): number[] {
    const out = new Set(ids);
    let grew = true;
    while (grew) {
      grew = false;
      for (const layer of info.layers) {
        if (layer.parent !== null && out.has(layer.parent) && !out.has(layer.id)) {
          out.add(layer.id);
          grew = true;
        }
      }
    }
    return [...out];
  }

  /** Le magnétisme a besoin de l'étendue de ce qu'on déplace ; sans réponse, on déplace sans aimant. */
  function loadBounds(gesture: MoveGesture): void {
    const doc = app.active;
    if (!doc) return;
    // Document composé par la carte graphique : les calques déplacés (groupes et leur contenu compris) sont décalés à
    // l'écran, pixels et masque ensemble ou l'un des deux seulement, quelle que soit la pile (modes, réglages…).
    if (gesture.kind === "layers" && app.performance.gpuPreviews && gpuMovable(doc)) {
      const ids = withDescendants(doc.info, doc.selection);
      if (gesture.dx === 0 && gesture.dy === 0 && renderer?.beginGpuMovePreview(ids, movedParts())) {
        gesture.local = true;
        previewSerial += 1;
        scheduleDraw();
      }
    }
    // Sinon, l'aperçu local par couches déplace le calque ENTIER : il ne sert pas quand pixels et masque bougent séparément.
    if (!gesture.local && gesture.kind === "layers" && app.performance.gpuPreviews && movedParts() === "both") {
      const [docId, ids] = [doc.info.id, [...doc.selection]];
      // Par la file du document : le plan décrit les calques APRÈS la transformation que ce clic vient de valider.
      enqueue(docId, () => api.movePlan(docId, ids)).then(
        (plan) => {
          // Le geste peut s'être terminé entre-temps, ou avoir déjà commencé par la voie du moteur.
          if (!plan || moving !== gesture || gesture.dx !== 0 || gesture.dy !== 0) return;
          gesture.local = true;
          previewSerial += 1;
          renderer?.beginMovePreview(plan);
          scheduleDraw();
        },
        () => undefined,
      );
    }
    // Les autres objets du document : ce qu'on déplace s'aimante à leurs bords et à leur centre.
    setSnapObjects([]);
    api.otherBounds(doc.info.id, [...doc.selection]).then(setSnapObjects, () => undefined);
    const ids = [...doc.selection];
    const docId = doc.info.id;
    // Par la file du document, comme le plan : une transformation validée par ce clic est appliquée avant la lecture,
    // sinon le cadre se reposait à l'ancienne place (et le magnétisme visait l'ancienne étendue). Les pas du
    // déplacement, mis en file après, ne sont pas comptés dans cette étendue de départ.
    enqueue(docId, () => api.contentBounds(docId, ids)).then(
      (bounds) => {
        gesture.bounds = bounds;
        // Sélection changée par ce geste : le cadre se pose sur le calque saisi, décalé de ce qu'il a déjà parcouru.
        if (!gesture.reframe || moving !== gesture || !bounds) return;
        gesture.reframe = false;
        [selectionBounds, boundsShift] = [bounds, { dx: gesture.dx, dy: gesture.dy }];
        if (!app.transform && (app.tool === "move" || app.tool === "shape")) openTransform(ids, bounds, true);
      },
      () => undefined,
    );
  }

  function updateMove(event: PointerEvent, at: Point): void {
    const doc = app.active;
    if (!moving?.ready || !doc || (moving.kind === "layers" && doc.selection.length === 0)) return;
    let dx = Math.round(at.x - moving.start.x);
    let dy = Math.round(at.y - moving.start.y);
    // Maj : déplacement contraint à l'axe dominant.
    if (event.shiftKey) [dx, dy] = Math.abs(dx) >= Math.abs(dy) ? [dx, 0] : [0, dy];
    const camera = doc.camera;
    // Magnétisme : bords et centre de l'objet déplacé contre bords, centre et repères du document. Ctrl le suspend.
    if (app.snapEnabled && moving.bounds && camera && !event.ctrlKey) {
      const [x0, y0, x1, y1] = moving.bounds;
      const reach = SNAP_PIXELS / camera.zoom;
      const [sx, sy] = [snapAxis(edgesAndCenter(x0 + dx, x1 + dx), "x", reach), snapAxis(edgesAndCenter(y0 + dy, y1 + dy), "y", reach)];
      dx += Math.round(sx.offset);
      dy += Math.round(sy.offset);
      // Ligne d'alignement : montre à quoi l'objet vient de s'accrocher.
      app.smartGuides = { x: sx.line, y: sy.line };
    } else {
      app.smartGuides = { x: null, y: null };
    }
    [moving.dx, moving.dy] = [dx, dy];
    if (moving.kind !== "selection") boundsShift = { dx, dy };
    if (moving.local) {
      renderer?.setMoveOffset(dx, dy);
      scheduleDraw();
    } else {
      // Par le moteur (pile non décomposable, masque seul…) : chaque pas recompose l'image. Le temps du
      // geste, elle est demandée un cran plus grossier — quatre fois moins de pixels à composer, le
      // calque suit la souris ; l'image redevient nette au relâchement, dès que le rendu final est prêt.
      if (moving.kind !== "selection" && !moving.draft) {
        moving.draft = true;
        renderer?.setDraftLevel(1);
      }
      sendMove(moving.kind, dx, dy, moving.id);
    }
  }

  /** Fin d'un aperçu local : le moteur reçoit le déplacement, et l'aperçu reste affiché jusqu'à ce que le vrai rendu soit prêt. */
  function finishLocalMove(gesture: MoveGesture): void {
    const ids = app.active?.selection ?? [];
    const serial = previewSerial;
    const done = () => {
      isMoving = moving !== null;
      if (serial !== previewSerial) return; // un nouveau geste a déjà pris le relais
      renderer?.endMovePreview();
      scheduleDraw();
    };
    if (gesture.dx === 0 && gesture.dy === 0) done();
    else void edit({ op: "translate", ids, dx: gesture.dx, dy: gesture.dy, gesture: gesture.id, parts: movedParts() }).then(() => waitForFrame()).finally(done);
  }

  function sendMove(kind: MoveGesture["kind"], dx: number, dy: number, gesture: string): void {
    const doc = app.active;
    const layer = app.activeLayer;
    if (!doc) return;
    if (kind === "selection") {
      editLatest(gesture, { op: "translateSelection", dx, dy, gesture });
    } else if (kind === "layers") {
      editLatest(gesture, { op: "translate", ids: doc.selection, dx, dy, gesture, parts: movedParts() });
    } else if (layer && !layer.isGroup && doc.selection.length === 1) {
      editLatest(gesture, { op: "movePixels", id: layer.id, dx, dy, gesture });
    } else {
      app.error = t("Sélectionnez un seul calque de pixels pour déplacer le contenu de la sélection.");
      moving = null;
    }
  }

  /** Flèches avec l'outil Déplacement : 1 px, ou 10 px avec Maj. Chaque appui est un geste. */
  function nudge(event: KeyboardEvent): boolean {
    const step = event.shiftKey ? 10 : 1;
    const delta: Record<string, [number, number]> = {
      ArrowLeft: [-step, 0],
      ArrowRight: [step, 0],
      ArrowUp: [0, -step],
      ArrowDown: [0, step],
    };
    const move = delta[event.key];
    if (!move || app.tool !== "move" || !app.active || app.active.selection.length === 0) return false;
    if (transformIsPending()) commitTransform();
    sendMove(app.active.info.hasSelection ? "pixels" : "layers", move[0], move[1], nextGesture());
    return true;
  }

  // ------------------------------------------------------------------ repères

  /** Position d'un repère sous le pointeur, aimantée à 0 %, 50 % et 100 % du document. */
  function guideUnderPointer(event: PointerEvent, orientation: Guide["orientation"]): Omit<GuideDrag, "orientation" | "index"> {
    const camera = app.active?.camera;
    const info = app.active?.info;
    const local = localPoint(event);
    if (!camera || !info) return { position: 0, discard: true, snapped: false };
    const horizontal = orientation === "horizontal";
    const screen = horizontal ? local.y : local.x;
    const raw = (screen - (horizontal ? camera.y : camera.x)) / camera.zoom;
    const extent = horizontal ? info.height : info.width;
    // Cibles : bords et milieu du document, puis bords et milieu du calque sélectionné (un texte, par exemple).
    const targets = [0, extent / 2, extent];
    if (selectionBounds) {
      const [x0, y0, x1, y1] = selectionBounds;
      targets.push(...(horizontal ? edgesAndCenter(y0, y1) : edgesAndCenter(x0, x1)));
    }
    const pull = app.snapEnabled && !event.ctrlKey ? snapOffset([raw], targets, SNAP_PIXELS / camera.zoom) : 0;
    // Hors aimant, un repère tombe sur un pixel entier ; le milieu d'un document impair reste exact.
    return { position: pull !== 0 ? raw + pull : Math.round(raw), discard: screen < 0, snapped: pull !== 0 };
  }

  /** Suit le pointeur jusqu'au relâchement, où qu'il aille dans la fenêtre. */
  function trackGuide(orientation: Guide["orientation"], index: number | null, event: PointerEvent): void {
    if (!app.active?.camera || transformIsModal()) return;
    // Verrouillés : on peut encore en tirer un nouveau depuis la règle, pas bouger ceux qui sont posés.
    if (index !== null && guidesLocked) return;
    event.preventDefault();
    app.guidesVisible = true;
    guideDrag = { orientation, index, ...guideUnderPointer(event, orientation) };
    const move = (e: PointerEvent) => {
      if (guideDrag) guideDrag = { orientation, index, ...guideUnderPointer(e, orientation) };
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const done = guideDrag;
      guideDrag = null;
      if (!done) return;
      if (done.discard) {
        if (done.index !== null) void edit({ op: "removeGuide", index: done.index });
      } else if (done.index === null) {
        void edit({ op: "addGuide", orientation, position: done.position });
      } else {
        void edit({ op: "moveGuide", index: done.index, position: done.position });
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  /** Repère existant sous le pointeur (outil Déplacement), à 4 pixels d'écran près. */
  function guideAt(event: PointerEvent): number {
    const doc = app.active;
    if (!doc?.camera || !app.guidesVisible) return -1;
    const { camera } = doc;
    const local = localPoint(event);
    return doc.info.guides.findIndex((g) => {
      const screen = g.orientation === "horizontal" ? camera.y + g.position * camera.zoom : camera.x + g.position * camera.zoom;
      return Math.abs(screen - (g.orientation === "horizontal" ? local.y : local.x)) <= 4;
    });
  }

  /** Repères à dessiner, en pixels d'écran ; celui qu'on déplace est remplacé par sa position provisoire. */
  const guideLines = $derived.by(() => {
    const doc = app.active;
    if (!doc?.camera || !app.guidesVisible) return [];
    const { camera } = doc;
    const place = (g: Guide) => (g.orientation === "horizontal" ? camera.y : camera.x) + g.position * camera.zoom;
    const fixed = doc.info.guides.filter((_, i) => i !== guideDrag?.index).map((g) => ({ ...g, screen: place(g), active: false, snapped: false }));
    if (!guideDrag || guideDrag.discard) return fixed;
    return [...fixed, { ...guideDrag, screen: place(guideDrag), active: true }];
  });

  /**
   * Coin d'un tracé rectangulaire (sélection, forme) aimanté aux bords et au centre du document,
   * aux repères et aux autres objets. Ctrl suspend, comme partout ailleurs.
   */
  function snapCorner(at: Point, event: PointerEvent | KeyboardEvent): Point {
    const doc = app.active;
    if (!doc?.camera) return at;
    return snapPoint(at, doc.camera.zoom, event.ctrlKey);
  }

  function beginSelection(event: PointerEvent, at: Point): void {
    const mode = combineMode(event);
    // Glisser depuis l'intérieur de la sélection, sans modificateur : on déplace son contour.
    const { x, y } = localPoint(event);
    const dpr = effectiveDpr();
    if (app.tool !== "wand" && app.tool !== "shape" && mode === "replace" && !draft && renderer?.isSelectedAt(x * dpr, y * dpr)) {
      moving = { id: nextGesture(), kind: "selection", start: at, ready: true, bounds: null, local: false, dx: 0, dy: 0 };
      return;
    }
    if (app.tool === "wand") {
      const layer = app.activeLayer;
      if (!layer || layer.isGroup) {
        app.error = t("Sélectionnez un calque de pixels pour la baguette magique.");
        return;
      }
      // Le clic sélectionne ; garder le bouton enfoncé et glisser règle la tolérance en direct (même état d'historique).
      wandDrag = { gesture: `baguette-${nextGesture()}`, layer: layer.id, x: Math.floor(at.x), y: Math.floor(at.y), mode, startX: event.clientX, startTolerance: app.wand.tolerance };
      if (mode === "replace") renderer?.dropSelection();
      canvas?.setPointerCapture(event.pointerId);
      sendWand();
    } else if (app.tool === "shape") {
      // Même tracé que la sélection rectangulaire (Maj : carré, Alt : depuis le centre) ; au relâchement, c'est un calque de forme qui naît.
      // Le tracé en cours s'affiche comme un rectangle (ou une ellipse) : c'est la boîte de la forme.
      const corner = snapCorner(at, event);
      draft = { kind: "box", shape: app.shapeKind === "ellipse" ? "ellipse" : "rect", mode: "replace", start: corner, end: corner, constrain: false, centered: false };
    } else if (app.tool === "marquee") {
      const corner = snapCorner(at, event);
      draft = { kind: "box", shape: app.marqueeShape, mode, start: corner, end: corner, constrain: false, centered: false };
    } else if (app.lassoShape === "free") {
      draft = { kind: "path", closeOnRelease: true, mode, points: [at], hover: null };
    } else if (draft?.kind === "path") {
      draft.points.push(at); // lasso polygonal : chaque clic ajoute un sommet
    } else {
      draft = { kind: "path", closeOnRelease: false, mode, points: [at], hover: null };
    }
  }

  function updateSelection(event: PointerEvent, at: Point): void {
    if (draft && sent.has(draft)) return;
    if (draft?.kind === "box") {
      draft.end = snapCorner(at, event);
      // Une fois le geste commencé en mode « remplacer », Maj contraint et Alt centre.
      if (draft.mode === "replace") {
        draft.constrain = event.shiftKey;
        draft.centered = event.altKey;
      }
    } else if (draft?.kind === "path") {
      if (draft.closeOnRelease) draft.points.push(at);
      else draft.hover = at;
    }
  }

  function endSelection(): void {
    app.smartGuides = { x: null, y: null };
    if (draft?.kind === "box") {
      const box = boxOf(draft);
      // Simple clic sans glisser : désélectionne, comme Photoshop.
      if (app.tool === "shape") {
        const kind = app.shapeKind;
        draft = null;
        const [width, height] = [Math.round(Math.abs(box.width)), Math.round(Math.abs(box.height))];
        if (width >= 2 && height >= 2) {
          const { fill, ...rest } = $state.snapshot(app.shapeDefaults);
          const shape = { kind, sides: Math.min(64, Math.max(3, Math.round(app.shapeSides) || 3)), width, height, ...rest, fill: fill === undefined ? [...app.foreground, 255] : fill };
          void edit({ op: "addShape", x: Math.round(Math.min(box.x, box.x + box.width)), y: Math.round(Math.min(box.y, box.y + box.height)), shape, above: app.activeLayer?.id ?? null });
        }
      } else if (Math.abs(box.width) < 1 || Math.abs(box.height) < 1) {
        const hadSelection = draft.mode === "replace" && app.active?.info.hasSelection;
        draft = null;
        if (hadSelection) void edit({ op: "deselect" });
      } else {
        commit({ kind: draft.shape, ...box }, draft.mode);
      }
    } else if (draft?.kind === "path" && draft.closeOnRelease) {
      closePath();
    }
  }

  /** Tracé de prévisualisation en coordonnées d'écran. */
  const preview = $derived.by(() => {
    const camera = app.active?.camera;
    if (!draft || !camera) return null;
    const sx = (x: number) => camera.x + x * camera.zoom;
    const sy = (y: number) => camera.y + y * camera.zoom;
    if (draft.kind === "box") {
      const box = boxOf(draft);
      const [x, y] = [sx(Math.min(box.x, box.x + box.width)), sy(Math.min(box.y, box.y + box.height))];
      const [w, h] = [Math.abs(box.width) * camera.zoom, Math.abs(box.height) * camera.zoom];
      return draft.shape === "rect"
        ? `M${x} ${y}h${w}v${h}h${-w}Z`
        : `M${x} ${y + h / 2}a${w / 2} ${h / 2} 0 1 0 ${w} 0a${w / 2} ${h / 2} 0 1 0 ${-w} 0Z`;
    }
    const points = draft.hover ? [...draft.points, draft.hover] : draft.points;
    return points.map((p, i) => `${i === 0 ? "M" : "L"}${sx(p.x)} ${sy(p.y)}`).join("");
  });

  // ---------------------------------------------------------------- pointeur

  /** Réglages de l'outil « à pinceau » en main (taille, et dureté s'il en a une), ou rien. */
  function sizedTool(): { size: number; hardness?: number } | null {
    if (app.paintTool) return app.brushes[app.paintTool];
    if (app.tool === "heal") return app.heal;
    if (app.tool === "quickSelect") return app.quickSelect;
    return null;
  }

  // ---- Baguette magique : tolérance réglable en glissant.
  let wandDrag = $state<{ gesture: string; layer: number; x: number; y: number; mode: CombineMode; startX: number; startTolerance: number } | null>(null);
  function sendWand(): void {
    if (!wandDrag) return;
    const { gesture, layer, x, y, mode } = wandDrag;
    editLatest(gesture, { op: "magicWand", layer, x, y, ...$state.snapshot(app.wand), gesture, mode });
  }

  // ---- Sélection rapide : on peint sur le sujet (aperçu vert), la sélection s'étend au relâchement.
  let quickStroke = $state<{ points: Point[]; subtract: boolean } | null>(null);
  const quickPath = $derived.by(() => {
    const camera = app.active?.camera;
    if (!quickStroke || !camera) return "";
    return quickStroke.points.map((p, i) => `${i ? "L" : "M"}${camera.x + p.x * camera.zoom} ${camera.y + p.y * camera.zoom}`).join("");
  });
  function endQuickStroke(): void {
    const stroke = quickStroke;
    const layer = app.activeLayer;
    quickStroke = null;
    if (!stroke || !layer) return;
    void edit({
      op: "quickSelect",
      layer: layer.id,
      points: stroke.points.map((p): [number, number] => [p.x, p.y]),
      size: app.quickSelect.size,
      tolerance: app.quickSelect.tolerance,
      allLayers: app.wand.allLayers,
      // Comme dans Photoshop : chaque trait ajoute à la sélection, Alt retire.
      mode: stroke.subtract ? "subtract" : "add",
    });
  }

  // ---- Correcteur localisé : on peint la zone (aperçu rouge translucide), elle est corrigée au relâchement.
  let healing = $state<Point[] | null>(null);
  const healPath = $derived.by(() => {
    const camera = app.active?.camera;
    if (!healing || !camera) return "";
    return healing.map((p, i) => `${i ? "L" : "M"}${camera.x + p.x * camera.zoom} ${camera.y + p.y * camera.zoom}`).join("");
  });

  function endHealing(): void {
    const points = healing;
    const layer = app.activeLayer;
    healing = null;
    if (!points || points.length === 0 || !layer) return;
    void withWait(edit({ op: "heal", id: layer.id, points: points.map((p): [number, number] => [p.x, p.y]), size: app.heal.size, hardness: app.heal.hardness }));
  }

  /** Curseur d'attente pendant un calcul qui peut durer (grande zone à corriger). */
  async function withWait(work: Promise<unknown>): Promise<void> {
    app.busy = "Correction…";
    try {
      await work;
    } finally {
      app.busy = null;
    }
  }

  /** Clic droit sur l'image : avec une sélection active, les commandes de sélection les plus courantes. */
  let selectionMenu = $state<{ x: number; y: number } | null>(null);
  function onContextMenu(event: MouseEvent): void {
    event.preventDefault();
    if (event.altKey) return; // Alt + clic droit glissé : redimensionnement du pinceau, pas de menu
    // Pinceau ou gomme en main : le menu des pinceaux, comme dans Photoshop ; sinon les commandes de sélection.
    if (app.paintTool) app.brushMenu = { x: event.clientX + 8, y: event.clientY + 8 };
    else if (app.active?.info.hasSelection) selectionMenu = { x: event.clientX, y: event.clientY };
  }
  const selectionEntries = (): MenuEntry[] =>
    (["select.invert", "select.none", "separator", "select.feather", "select.modify", "layer.addMask", "separator", "edit.copy", "edit.cut", "edit.fillForeground"] as const).map((item) =>
      item === "separator" ? item : { label: ACTION_LABELS[item], shortcut: keymap.label(item), disabled: !isActionEnabled(item), run: () => runAction(item) },
    );

  function onPointerDown(event: PointerEvent): void {
    const camera = app.active?.camera;
    if (!camera || !canvas) return;
    pointerHeld = true;
    if (panning || event.button === 1) {
      stopCameraAnimation();
      canvas.setPointerCapture(event.pointerId);
      drag = { pointerId: event.pointerId, lastX: event.clientX, lastY: event.clientY };
    } else if (event.button === 2 && event.altKey && sizedTool()) {
      // Comme Photoshop : Alt + clic droit glissé. Horizontal = taille, vertical = dureté. Vaut pour tous les
      // outils « à pinceau » : pinceau, gomme, correcteur localisé, sélection rapide.
      const settings = sizedTool();
      if (!settings) return;
      canvas.setPointerCapture(event.pointerId);
      brushResize = { x: event.clientX, y: event.clientY, size: settings.size || 1, hardness: settings.hardness ?? 1, at: localPoint(event), axis: null };
    } else if (event.button !== 0) {
      return;
    } else if (sampling) {
      const at = docPoint(event);
      canvas.setPointerCapture(event.pointerId);
      samplingDrag = true;
      if (at) void sampleColor(at.x, at.y, event);
    } else if (app.tool === "zoom") {
      const { x, y } = localPoint(event);
      animateCamera(zoomAt(camera, steppedZoom(camera.zoom, event.altKey ? -1 : 1), x, y));
    } else if (app.paintTool && !stroke) {
      canvas.setPointerCapture(event.pointerId);
      beginStroke(event);
    } else if (app.tool === "text") {
      const at = docPoint(event);
      if (at) void placeText(at);
    } else if (app.tool === "fill") {
      const at = docPoint(event);
      const layer = app.activeLayer;
      if (!at) return;
      if (!layer || layer.isGroup || app.active?.selection.length !== 1) {
        app.error = t("Sélectionnez un seul calque de pixels à remplir.");
      } else if (app.fill.kind === "bucket") {
        const { tolerance, contiguous } = app.fill;
        void edit({ op: "bucketFill", id: layer.id, x: Math.floor(at.x), y: Math.floor(at.y), color: app.foreground, tolerance, contiguous });
      } else {
        canvas.setPointerCapture(event.pointerId);
        gradient = { from: at, to: at };
      }
    } else if (app.tool === "move") {
      const guide = guidesLocked ? -1 : guideAt(event);
      const existing = app.active?.info.guides[guide];
      if (existing) {
        trackGuide(existing.orientation, guide, event);
        return;
      }
      const at = docPoint(event);
      if (!at) return;
      canvas.setPointerCapture(event.pointerId);
      beginMove(at, event.altKey, event.shiftKey);
    } else if (app.tool === "quickSelect") {
      const at = docPoint(event);
      if (!at || !app.activeLayer || app.activeLayer.isGroup) return;
      canvas?.setPointerCapture(event.pointerId);
      quickStroke = { points: [at], subtract: event.altKey };
    } else if (app.tool === "heal") {
      const at = docPoint(event);
      if (!at || !app.activeLayer) return;
      canvas?.setPointerCapture(event.pointerId);
      healing = [at];
    } else if (app.tool === "marquee" || app.tool === "lasso" || app.tool === "wand" || app.tool === "shape") {
      const at = docPoint(event);
      if (!at) return;
      canvas.setPointerCapture(event.pointerId);
      beginSelection(event, at);
    }
  }

  function onPointerMove(event: PointerEvent): void {
    const camera = app.active?.camera;
    hover = localPoint(event);
    // Repère saisissable sous le pointeur : le curseur l'annonce avant même le clic.
    hoveredGuide = app.tool === "move" && !moving && !guideDrag ? (app.active?.info.guides[guideAt(event)]?.orientation ?? null) : null;
    if (!camera) return;
    if (wandDrag) {
      // Un pixel d'écran = un demi-point de tolérance ; les quatre premiers pixels ne comptent pas (simple clic).
      const travel = event.clientX - wandDrag.startX;
      const tolerance = Math.round(Math.min(255, Math.max(0, wandDrag.startTolerance + (Math.abs(travel) < 4 ? 0 : travel * 0.5))));
      if (tolerance !== app.wand.tolerance) {
        app.wand.tolerance = tolerance;
        sendWand();
      }
      return;
    }
    if (quickStroke) {
      const at = docPoint(event);
      const last = quickStroke.points[quickStroke.points.length - 1];
      if (at && last && Math.hypot(at.x - last.x, at.y - last.y) * camera.zoom >= 2) quickStroke.points = [...quickStroke.points, at];
      return;
    }
    if (healing) {
      const at = docPoint(event);
      const last = healing[healing.length - 1];
      // Un point tous les deux pixels d'écran suffit à décrire la zone.
      if (at && last && Math.hypot(at.x - last.x, at.y - last.y) * camera.zoom >= 2) healing = [...healing, at];
      return;
    }
    const resized = brushResize ? sizedTool() : null;
    if (brushResize && resized) {
      const settings = resized;
      const [dx, dy] = [event.clientX - brushResize.x, event.clientY - brushResize.y];
      // Le premier mouvement franc choisit le réglage : on ne dérègle plus la dureté en voulant
      // changer la taille (un geste horizontal n'est jamais parfaitement horizontal).
      if (!brushResize.axis && Math.hypot(dx, dy) > 6) brushResize.axis = Math.abs(dx) >= Math.abs(dy) ? "size" : "hardness";
      if (brushResize.axis === "size") settings.size = Math.round(Math.min(2500, Math.max(1, brushResize.size + dx / camera.zoom)));
      if (brushResize.axis === "hardness" && settings.hardness !== undefined) settings.hardness = Math.min(1, Math.max(0, brushResize.hardness - dy / 200));
      hover = brushResize.at; // le cercle grandit sur place, sous le point de départ
      return;
    }
    if (drag?.pointerId === event.pointerId) {
      queuePan(event.clientX - drag.lastX, event.clientY - drag.lastY);
      // Dernière position mise à jour sur place : rien dans l'affichage ne dépend d'elle, pas de réaction inutile.
      [drag.lastX, drag.lastY] = [event.clientX, event.clientY];
    }
    if (stroke && !stroke.ended) {
      // Les événements fusionnés par le navigateur redonnent la vraie résolution du stylet.
      const events = event.getCoalescedEvents?.() ?? [];
      for (const sample of events.length > 0 ? events : [event]) {
        const raw = strokePoint(sample);
        if (!raw) continue;
        lastRaw = raw;
        const point = stabilize(raw);
        stroke.pending.push(point);
        if (dabPath) renderer?.addStrokeDabs(dabPath.add(point));
      }
      if (dabPath) scheduleDraw();
      pump(stroke);
    }
    const at = docPoint(event);
    if (at && gradient) {
      // Maj : direction contrainte par pas de 45°.
      const [dx, dy] = [at.x - gradient.from.x, at.y - gradient.from.y];
      const angle = event.shiftKey ? Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4) : Math.atan2(dy, dx);
      const length = Math.hypot(dx, dy);
      gradient = { from: gradient.from, to: { x: gradient.from.x + Math.cos(angle) * length, y: gradient.from.y + Math.sin(angle) * length } };
      return;
    }
    if (at && samplingDrag) void sampleColor(at.x, at.y, event); // glisser : la couleur suit le pointeur
    else if (at && moving) updateMove(event, at);
    else if (at) updateSelection(event, at);
    queuePointer();
  }

  function onPointerUp(event: PointerEvent): void {
    pointerHeld = false;
    scheduleDraw(); // retour à la résolution de l'écran
    if (drag?.pointerId === event.pointerId) drag = null;
    if (healing) endHealing();
    if (quickStroke) endQuickStroke();
    wandDrag = null;
    if (gradient) {
      const { from, to } = gradient;
      const layer = app.activeLayer;
      gradient = null;
      if (layer && Math.hypot(to.x - from.x, to.y - from.y) >= 1) {
        const stops = resolveStops($state.snapshot(app.gradient), app.foreground, app.background);
        // Masque de fusion pour cible : le dégradé s'y inscrit (noir = masqué, blanc = visible).
        const onMask = app.active?.target === "mask" && layer.hasMask;
        void edit({ op: "gradientFill", id: layer.id, from: [from.x, from.y], to: [to.x, to.y], stops, radial: app.fill.radial, onMask });
      }
    }
    if (samplingDrag) app.eyedropperArmed = false; // une pipette armée ne sert qu'une fois
    samplingDrag = false;
    app.sample = null;
    brushResize = null;
    if (stroke && !stroke.ended) {
      stroke.ended = true;
      for (const point of lastRaw ? catchUp(lastRaw) : []) {
        stroke.pending.push(point);
        if (dabPath) renderer?.addStrokeDabs(dabPath.add(point));
      }
      if (dabPath) {
        renderer?.addStrokeDabs(dabPath.finish());
        scheduleDraw();
      }
      pump(stroke);
    }
    app.smartGuides = { x: null, y: null };
    if (moving) {
      const finished = moving;
      moving = null;
      if (finished.local) finishLocalMove(finished);
      else {
        isMoving = false;
        // Fin d'un déplacement par le moteur : l'image grossière reste affichée jusqu'au rendu net du dernier pas.
        if (finished.draft) void whenIdle(finished.id).then(() => waitForFrame()).finally(() => (renderer?.setDraftLevel(0), scheduleDraw()));
      }
    } else {
      endSelection();
    }
  }

  function onKey(event: KeyboardEvent): void {
    altHeld = event.altKey;
    // Sous Windows, relâcher Alt seul ouvre le menu système de la fenêtre et avale la frappe suivante.
    if (event.key === "Alt") event.preventDefault();
    if (transformIsModal()) return; // Ctrl+T : le cadre a la main sur le clavier
    // Les flèches et Entrée/Échap du lasso ne s'appliquent pas derrière une boîte ou un panneau flottant ;
    // l'Espace (main temporaire) reste disponible, l'image restant utilisable à la souris.
    if (panelOwnsKeyboard() && event.code !== "Space") return;
    const inControl = event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement || event.target instanceof HTMLTextAreaElement;
    if (event.type === "keydown" && !inControl && nudge(event)) {
      event.preventDefault();
      return;
    }
    if (draft && event.type === "keydown" && !(event.target instanceof HTMLInputElement)) {
      // Lasso polygonal : Entrée ferme, Échap abandonne, Retour arrière retire le dernier sommet.
      if (event.key === "Enter") closePath();
      else if (event.key === "Escape") draft = null;
      else if (event.key === "Backspace" && draft.kind === "path" && draft.points.length > 1) draft.points.pop();
    }
    if (event.code !== "Space" || event.target instanceof HTMLInputElement) return;
    // Espace = main temporaire. preventDefault évite le défilement et l'activation du bouton focalisé.
    event.preventDefault();
    spaceHeld = event.type === "keydown";
  }
  import { ACTION_LABELS } from "../lib/action-catalog";
  import { isActionEnabled, runAction } from "../lib/actions";
  import ContextMenu, { type MenuEntry } from "../components/ContextMenu.svelte";
  import { keymap } from "../state/keymap.svelte";
</script>

<!--
  « Alt enfoncé » est aussi relu sur chaque mouvement et chaque clic de la souris, qui portent l'état réel des
  touches. Sous Linux, le relâchement d'Alt n'arrive pas toujours (après Alt + clic droit glissé pour la taille du
  pinceau, notamment) : la pipette temporaire restait alors affichée. Elle disparaît maintenant au premier
  mouvement, et un clic ne peut jamais prélever une couleur si Alt n'est pas réellement enfoncé.
-->
<svelte:window
  onkeydown={onKey}
  onkeyup={onKey}
  onpointermovecapture={(event) => (altHeld = event.altKey)}
  onpointerdowncapture={(event) => (altHeld = event.altKey)}
  onpointerupcapture={(event) => (altHeld = event.altKey)}
  onfocus={() => (altHeld = false)}
  onblur={() => ((spaceHeld = false), (altHeld = false))}
/>

<div class="viewport" class:rulers={app.rulersVisible}>
  {#if app.rulersVisible}
    <Rulers camera={app.active?.camera ?? null} pointer={app.pointer} onguidestart={(orientation, event) => trackGuide(orientation, null, event)} />
  {/if}
  <div
    class="stage"
    role="presentation"
    onwheel={onWheel}
    onpointerdowncapture={onStagePointerDownCapture}
    onpointermove={onStagePointerMove}
    onpointerup={() => (overlayPan = null)}
    onpointercancel={() => (overlayPan = null)}
  >
    <canvas
      bind:this={canvas}
      style:cursor
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      ondblclick={(event) => {
        if (app.tool === "lasso") closePath();
        // Outil Forme : le double-clic ouvre « Créer une forme » (nombre de côtés, taille), centrée sur le point cliqué.
        else if (app.tool === "shape") {
          const at = docPoint(event);
          if (at) app.shapeDialog = { x: Math.round(at.x), y: Math.round(at.y) };
        }
        // Pinceau, gomme, correcteur et sélection rapide se servent de clics rapprochés : pas de bascule vers le texte.
        else if (app.paintTool === null && app.tool !== "heal" && app.tool !== "quickSelect" && app.tool !== "crop") void editTextAt(event);
      }}
      oncontextmenu={onContextMenu}
      onpointerleave={() => ((app.pointer = null), (hover = null))}
    ></canvas>
    {#each guideLines as line, index (index)}
      <div
        class="guide"
        class:horizontal={line.orientation === "horizontal"}
        class:active={line.active}
        style:transform={line.orientation === "horizontal" ? `translateY(${Math.round(line.screen)}px)` : `translateX(${Math.round(line.screen)}px)`}
      >
        {#if line.active}
          <span class:snapped={line.snapped}>{Math.round(line.position * 10) / 10} px{line.snapped ? t(" · aimanté") : ""}</span>
        {/if}
      </div>
    {/each}
    {#if app.tool === "crop" && app.active?.camera}
      <CropOverlay info={app.active.info} camera={app.active.camera} />
    {/if}
    {#if (app.tool === "pen" || app.tool === "pathSelect") && app.active?.camera}
      <PathOverlay paths={app.active.info.paths} camera={app.active.camera} tool={app.tool} />
    {/if}
    {#if quickStroke && app.active?.camera}
      <svg class="draft" aria-hidden="true">
        <path class="healing quick" class:subtract={quickStroke.subtract} d={quickPath ? `${quickPath}l0.01 0` : "M0 0"} stroke-width={Math.max(1, app.quickSelect.size * app.active.camera.zoom)} />
      </svg>
    {/if}
    {#if wandDrag && hover}
      <div class="wand-readout" style:left="{hover.x + 16}px" style:top="{hover.y + 16}px">{t("Tolérance : {tolerance}", { tolerance: app.wand.tolerance })}</div>
    {/if}
    {#if healing && app.active?.camera}
      <svg class="draft" aria-hidden="true">
        <path class="healing" d={healPath || "M0 0"} stroke-width={Math.max(1, app.heal.size * app.active.camera.zoom)} />
        {#if healing.length === 1 && healing[0]}
          <circle class="healing-dot" cx={app.active.camera.x + healing[0].x * app.active.camera.zoom} cy={app.active.camera.y + healing[0].y * app.active.camera.zoom} r={(app.heal.size * app.active.camera.zoom) / 2} />
        {/if}
      </svg>
    {/if}
    {#if (app.smartGuides.x !== null || app.smartGuides.y !== null) && app.active?.camera}
      {@const camera = app.active.camera}
      <!-- Lignes d'alignement : apparaissent à l'instant où l'objet s'aimante (bord, centre, repère, autre objet). -->
      <svg class="draft smart" aria-hidden="true">
        {#if app.smartGuides.x !== null}
          <line x1={camera.x + app.smartGuides.x * camera.zoom} y1="0" x2={camera.x + app.smartGuides.x * camera.zoom} y2="100%" />
        {/if}
        {#if app.smartGuides.y !== null}
          <line x1="0" y1={camera.y + app.smartGuides.y * camera.zoom} x2="100%" y2={camera.y + app.smartGuides.y * camera.zoom} />
        {/if}
      </svg>
    {/if}
    {#if app.textEdit && app.activeLayer?.text && app.activeLayer.id === app.textEdit.layerId && app.active?.camera}
      <TextEditorOverlay session={app.textEdit} layer={app.activeLayer} text={app.activeLayer.text} camera={app.active.camera} />
    {/if}
    {#if outlines.length > 0 && app.active?.camera}
      {@const cam = app.active.camera}
      <svg class="outlines" aria-hidden="true">
        {#each outlines as [x0, y0, x1, y1], index (index)}
          <rect x={cam.x + x0 * cam.zoom + 0.5} y={cam.y + y0 * cam.zoom + 0.5} width={Math.max(0, (x1 - x0) * cam.zoom - 1)} height={Math.max(0, (y1 - y0) * cam.zoom - 1)} />
        {/each}
      </svg>
    {/if}
    {#if app.transform && app.active?.camera}
      <!-- Une nouvelle séance (autre calque, nouveau Ctrl+T) recrée le cadre : rien de l'ancien geste n'y survit. -->
      {#key app.transform.gesture}
        <TransformOverlay session={app.transform} camera={app.active.camera} passive={app.transform.auto} shift={app.transform.auto ? boundsShift : undefined} />
      {/key}
    {/if}
    {#if gradientLine}
      <svg class="draft" aria-hidden="true">
        <path d="M{gradientLine.from.x} {gradientLine.from.y}L{gradientLine.to.x} {gradientLine.to.y}" class="under" style="stroke-width: 3" />
        <path d="M{gradientLine.from.x} {gradientLine.from.y}L{gradientLine.to.x} {gradientLine.to.y}" class="over" style="stroke-dasharray: none" />
        <circle cx={gradientLine.from.x} cy={gradientLine.from.y} r="4" class="gradient-end" />
        <circle cx={gradientLine.to.x} cy={gradientLine.to.y} r="4" class="gradient-end" />
      </svg>
    {/if}
    {#if preview}
      <svg class="draft" aria-hidden="true">
        <path d={preview} class="under" />
        <path d={preview} class="over" />
      </svg>
    {/if}
    {#if hover && brushDiameter > 6 && !drag && (!sampling || brushResize)}
      <div
        class="brush-cursor"
        style:width="{brushDiameter}px"
        style:height="{brushDiameter}px"
        style:transform="translate({hover.x - brushDiameter / 2}px, {hover.y - brushDiameter / 2}px)"
      ></div>
    {/if}
    {#if !app.active}
      <div class="empty">
        <p>{t("Aucun document ouvert")}</p>
        <p class="hint">{t("Ctrl+O pour ouvrir · Ctrl+N pour créer")}</p>
      </div>
    {/if}
  </div>
</div>

{#if selectionMenu}
  <ContextMenu x={selectionMenu.x} y={selectionMenu.y} entries={selectionEntries()} onclose={() => (selectionMenu = null)} />
{/if}

<style>
  .viewport {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template: "canvas" 1fr / 1fr;
    background: var(--bg-canvas);
  }
  .viewport.rulers {
    grid-template:
      "corner top" var(--ruler)
      "left canvas" 1fr / var(--ruler) 1fr;
  }
  .stage {
    grid-area: canvas;
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
  }
  .guide {
    position: absolute;
    top: 0;
    left: 0;
    width: 1px;
    height: 100%;
    background: #38c6d9;
    pointer-events: none;
  }
  .guide.horizontal {
    width: 100%;
    height: 1px;
  }
  .guide.active {
    background: #fff;
  }
  .guide span {
    position: absolute;
    top: 6px;
    left: 6px;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    background: var(--bg-menu);
    color: var(--text);
    font-size: 11px;
    white-space: nowrap;
  }
  .guide span.snapped {
    background: var(--accent);
    color: #fff;
  }
  .draft {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .gradient-end {
    fill: #fff;
    stroke: #000;
    stroke-width: 1;
  }
  .draft path.healing {
    fill: none;
    stroke: rgba(255, 60, 70, 0.4);
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: none;
    animation: none;
  }
  .draft path.healing.quick {
    stroke: rgba(60, 220, 120, 0.4);
  }
  .draft path.healing.quick.subtract {
    stroke: rgba(255, 60, 70, 0.4);
  }
  .wand-readout {
    position: absolute;
    padding: 3px 8px;
    border-radius: var(--radius-s);
    background: rgba(15, 15, 17, 0.85);
    color: #fff;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
    white-space: nowrap;
  }
  .healing-dot {
    fill: rgba(255, 60, 70, 0.4);
  }
  .smart line {
    stroke: #ff3bd4;
    stroke-width: 1;
  }
  .draft path {
    fill: none;
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  /* Tirets noirs sur trait blanc : lisible sur n'importe quelle image. */
  .draft .under {
    stroke: #fff;
  }
  .draft .over {
    stroke: #000;
    stroke-dasharray: 4 4;
  }
  .brush-cursor {
    position: absolute;
    top: 0;
    left: 0;
    border-radius: 50%;
    /* Double liseré : visible sur fond clair comme sur fond sombre. */
    border: 1px solid rgba(255, 255, 255, 0.9);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.6);
    pointer-events: none;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--text-3);
    pointer-events: none;
  }
  .empty p {
    margin: 0 0 6px;
    font-size: 13px;
  }
  .empty .hint {
    color: var(--text-muted);
    font-size: 12px;
  }
  .outlines {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    overflow: visible;
  }
  .outlines rect {
    fill: none;
    stroke: var(--accent, #ec2a3c);
    stroke-width: 1;
  }
</style>
