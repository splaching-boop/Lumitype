<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { beginFreeTransform } from "../lib/actions";
  import { dropLatest, edit, editLatest, enqueue, sampleColor, whenIdle } from "../lib/engine";
  import { type LayerInfo, type TextData, api } from "../lib/ipc";
  import { finishTextEdit } from "../lib/text-edit.svelte";
  import { adjustRuns } from "../lib/text-runs";
  import { FRAME_HANDLES, type FrameAxes, type FrameHandle, frameAxes, frameQuad, handleCursor, resizeFrame, toFrame } from "../lib/text-frame";
  import { app, type Point, type TextEditSession } from "../state/app.svelte";
  import { type Camera, clientToDoc } from "./camera";

  // Saisie de texte directement sur l'image, façon machine à écrire.
  //
  // La frappe passe par un champ de saisie invisible : c'est lui qui gère correctement les
  // accents (touches mortes ^ ¨ en AZERTY), le presse-papiers, la sélection au clavier et
  // les méthodes de saisie. L'AFFICHAGE, lui, vient entièrement du moteur : le texte est le
  // vrai rendu, et la position du curseur et de la sélection est calculée par la même mise
  // en page. Rien ne peut donc se décaler entre ce qu'on tape et ce qu'on voit.

  let {
    session,
    layer,
    text,
    camera,
    oncontext,
  }: { session: TextEditSession; layer: LayerInfo; text: TextData; camera: Camera; oncontext?: (event: MouseEvent) => void } = $props();

  let field = $state<HTMLTextAreaElement>();
  let caret = $state<[Point, Point] | null>(null);
  let highlights = $state<Point[][]>([]);
  // Texte de paragraphe : coins de son cadre (repère document, confirmés par le moteur) et débordement signalé.
  let frame = $state<Point[] | null>(null);
  let overflow = $state(false);
  /**
   * Poignée du cadre tirée : axes et taille du cadre au début du geste, cadre affiché en attendant le moteur.
   * `ended` : bouton relâché, le cadre affiché reste celui du geste jusqu'à la réponse du moteur (pas de saut).
   */
  let resizing = $state<{ pointerId: number; handle: FrameHandle; axes: FrameAxes; width: number; height: number; gesture: string; quad: Point[]; ended: boolean } | null>(null);
  /** Le cadre a été redimensionné pendant cette séance (voir `cancel`). */
  let resized = false;
  /** Point de départ d'une sélection à la souris, en position de texte. */
  let dragAnchor: number | null = null;
  let altHeld = $state(false);
  let sampling = false;
  const docId = app.activeId;

  const toScreen = (p: Point): Point => ({ x: camera.x + p.x * camera.zoom, y: camera.y + p.y * camera.zoom });
  const caretLine = $derived(caret ? [toScreen(caret[0]), toScreen(caret[1])] : null);
  const polygons = $derived(highlights.map((quad) => quad.map(toScreen).map((p) => `${p.x},${p.y}`).join(" ")));
  /** Cadre affiché, à l'écran : celui du geste en cours, sinon celui du moteur. */
  const frameScreen = $derived((resizing?.quad ?? frame)?.map(toScreen) ?? null);
  const framePoints = $derived(frameScreen?.map((p) => `${p.x},${p.y}`).join(" ") ?? "");
  /** Axes du repère du texte d'après le cadre confirmé : direction des poignées (curseurs). */
  const axes = $derived(frame && text.frame ? frameAxes(frame, text.frame.width, text.frame.height) : null);
  /** Poignées : sur les coins et au milieu des bords du cadre affiché. */
  const handles = $derived.by(() => {
    const quad = frameScreen;
    if (!quad || quad.length !== 4) return [];
    const [a, b, c, d] = quad as [Point, Point, Point, Point];
    const lerp = (p: Point, q: Point, k: number): Point => ({ x: p.x + (q.x - p.x) * k, y: p.y + (q.y - p.y) * k });
    return FRAME_HANDLES.map((handle) => ({ handle, at: lerp(lerp(a, b, handle.hx), lerp(d, c, handle.hx), handle.hy), cursor: axes ? handleCursor(axes, handle) : "move" }));
  });
  /** Indicateur de débordement : juste après le coin bas droit, dans la direction de la diagonale du cadre. */
  const overflowMark = $derived.by(() => {
    const quad = frameScreen;
    if (!overflow || !quad?.[0] || !quad[2]) return null;
    const [start, corner] = [quad[0], quad[2]];
    const length = Math.hypot(corner.x - start.x, corner.y - start.y) || 1;
    return { x: corner.x + ((corner.x - start.x) / length) * 13, y: corner.y + ((corner.y - start.y) / length) * 13 };
  });

  /** Demande au moteur où dessiner le curseur. Placée dans la file du document : toujours après les frappes déjà envoyées. */
  let caretRequest = 0;
  function refreshCaret(): void {
    if (!field || docId === null) return;
    const request = ++caretRequest;
    const [start, end] = field.selectionDirection === "backward" ? [field.selectionEnd, field.selectionStart] : [field.selectionStart, field.selectionEnd];
    // Le panneau Propriétés applique police et couleur à cette plage.
    session.selection = { start: Math.min(start, end), end: Math.max(start, end) };
    enqueue(docId, () => api.textCaret(docId, layer.id, start, end)).then(
      (info) => {
        if (request !== caretRequest) return; // une demande plus récente est partie : cette réponse est déjà périmée
        caret = [{ x: info.caret[0][0], y: info.caret[0][1] }, { x: info.caret[1][0], y: info.caret[1][1] }];
        highlights = info.highlights.map((quad) => quad.map(([x, y]) => ({ x, y })));
        frame = info.frame?.map(([x, y]) => ({ x, y })) ?? null;
        overflow = info.overflow;
        // Geste de poignée terminé : le cadre du moteur (celui du geste, désormais) prend le relais.
        if (resizing?.ended) resizing = null;
      },
      (error: unknown) => app.fail(error),
    );
  }

  function onInput(): void {
    if (!field) return;
    // Les portions stylées suivent la frappe : un mot coloré reste coloré quand on tape devant lui.
    session.runs = adjustRuns(session.runs, session.draft, field.value);
    session.draft = field.value;
    // Taper remplace la sélection : son surlignage (celui du « Lorem ipsum » de départ, notamment) disparaît tout de suite.
    highlights = [];
    // `text` peut dater d'avant un redimensionnement du cadre encore en route : le moteur ignore le cadre reçu par
    // setText (il ne change que par setTextFrame ou une conversion), l'ancien cadre ne revient donc jamais.
    editLatest(session.id, { op: "setText", id: layer.id, text: { ...text, content: field.value, runs: $state.snapshot(session.runs) }, session: session.id });
  }

  // Le texte confirmé par le moteur vient de changer (frappe appliquée, police modifiée,
  // transformation…) : la mise en page a bougé, le curseur aussi.
  $effect(() => {
    void text;
    void layer.offset;
    refreshCaret();
  });

  // Un réglage du panneau Propriétés a pris le focus : on le reprend pour continuer à taper.
  $effect(() => {
    if (!app.focusTextEditor || !field) return;
    app.focusTextEditor = false;
    focusField();
  });

  /**
   * Focus du champ de saisie SANS défilement : sinon le navigateur fait défiler la zone de travail ou cette surcouche
   * (overflow: hidden, mais défilables par script) pour montrer le champ — le surlignage se décalait alors du texte au
   * premier clic, et la position de la souris ensuite (vue tournée surtout, où le coin du cadre est hors de l'écran).
   */
  function focusField(): void {
    field?.focus({ preventScroll: true });
  }

  /** Garde-fou : cette surcouche ne défile jamais (voir `focusField`). */
  function unscroll(): void {
    if (root && (root.scrollLeft !== 0 || root.scrollTop !== 0)) [root.scrollLeft, root.scrollTop] = [0, 0];
  }

  onMount(() => {
    if (!field) return;
    field.value = session.draft;
    focusField();
    const at = session.caretAt ?? field.value.length;
    if (session.selectAll) field.select();
    else field.setSelectionRange(at, at);
    refreshCaret();
    // Le curseur bouge aussi sans frappe : flèches, Origine, Fin, Ctrl+A… Selon la webview, le
    // changement est signalé sur le document ou sur le champ lui-même : on écoute les deux.
    const onSelection = () => document.activeElement === field && refreshCaret();
    document.addEventListener("selectionchange", onSelection);
    field.addEventListener("select", onSelection);
    field.addEventListener("keyup", onSelection);
    const watched = field;
    return () => {
      document.removeEventListener("selectionchange", onSelection);
      watched.removeEventListener("select", onSelection);
      watched.removeEventListener("keyup", onSelection);
    };
  });

  /** Valide la saisie. Un calque resté vide est retiré, comme dans Photoshop. */
  export function commit(): void {
    // Seule la séance de CETTE surcouche est close. `finishTextEdit` lit tout dans un instantané pris
    // avant de vider `app.textEdit` : la prop `session` EST `app.textEdit` et vaut `null` juste après.
    if (!session || app.textEdit?.id !== session.id) return;
    finishTextEdit();
  }

  /** Échap : toute la saisie de cette séance est abandonnée, sans trace dans l'historique. */
  function cancel(): void {
    // Même précaution que `commit` : on garde l'identifiant de la séance et sa nature avant de la clore.
    const current = session;
    if (!current || app.textEdit?.id !== current.id) return;
    // Cadre redimensionné pendant la séance : ces gestes sont des étapes d'historique à part (Ctrl+Z les défait un à
    // un). Annuler seulement la frappe ou retirer le calque laisserait un état mélangé : la saisie est validée.
    if (resized) {
      commit();
      return;
    }
    const { id, isNew } = current;
    app.textEdit = null;
    // La dernière frappe encore en attente (regroupée par `editLatest`) est abandonnée : envoyée après
    // l'annulation, elle réécrirait le texte qu'on vient d'annuler (ou viserait un calque déjà retiré).
    // On attend aussi celle en cours d'envoi, pour que l'annulation la couvre.
    dropLatest(id);
    void whenIdle(id)
      .then(() => edit({ op: "cancelGesture", gesture: id }))
      .then(() => {
        if (isNew) void edit({ op: "undo" });
      });
  }

  function onKeyDown(event: KeyboardEvent): void {
    event.stopPropagation(); // aucune frappe ne doit atteindre les raccourcis de l'application
    if (event.key === "Tab" && !event.ctrlKey && !event.altKey && field) {
      // Tab insère une tabulation dans le texte (taquets réguliers, voir le moteur), au lieu de quitter la saisie.
      field.setRangeText("\t", field.selectionStart, field.selectionEnd, "end");
      onInput();
    } else if (event.key === "Escape") cancel();
    else if (event.key === "Enter" && (event.ctrlKey || event.code === "NumpadEnter")) commit();
    else if (event.ctrlKey && !event.altKey && event.key.toLowerCase() === "t") {
      // Ctrl+T en pleine saisie : le texte est validé, puis son cadre de transformation s'ouvre, du même coup.
      // On attend que la dernière frappe ET toute la file des modifications du document soient appliquées (sans
      // quoi le cadre s'appuyait sur un état intermédiaire et la première pression ne faisait que valider le
      // texte), puis on ouvre le cadre directement, sans repasser par le filtre des raccourcis.
      const docId = app.active?.info.id;
      const gesture = session.id; // lu avant `commit()`, qui vide la séance
      commit();
      void whenIdle(gesture)
        .then(() => (docId === undefined ? undefined : enqueue(docId, () => Promise.resolve(null))))
        .then(() => beginFreeTransform())
        .catch((error: unknown) => app.fail(error));
    } else return;
    event.preventDefault();
  }

  let root = $state<HTMLDivElement>();

  /**
   * Point du document sous le pointeur. À calculer tout de suite dans le gestionnaire : après un
   * `await`, `event.currentTarget` ne vaut plus rien et le clic serait perdu.
   */
  function docPoint(event: PointerEvent): Point {
    // Par la zone de travail : juste aussi quand l'affichage est tourné (outil R).
    const mapped = clientToDoc(event.clientX, event.clientY);
    if (mapped) return mapped;
    const rect = root?.getBoundingClientRect();
    return { x: (event.clientX - (rect?.left ?? 0) - camera.x) / camera.zoom, y: (event.clientY - (rect?.top ?? 0) - camera.y) / camera.zoom };
  }

  const positionAt = (at: Point): Promise<number | null> => (docId === null ? Promise.resolve(null) : api.textHit(docId, layer.id, at.x, at.y));

  async function onPointerDown(event: PointerEvent): Promise<void> {
    if (event.button !== 0 || docId === null) return;
    event.preventDefault(); // le champ de saisie garde le focus
    if (event.altKey && app.eyedropperTarget) {
      // Sélecteur de couleur ouvert : Alt + clic prélève la couleur sous le pointeur.
      const point = docPoint(event);
      root?.setPointerCapture(event.pointerId);
      sampling = true;
      void sampleColor(point.x, point.y, event);
      return;
    }
    const at = docPoint(event);
    root?.setPointerCapture(event.pointerId);
    try {
      // Cliquer hors du texte valide la saisie, comme dans Photoshop. Le texte se reconnaît à
      // son rectangle, même recouvert par un autre calque.
      const picked = session.draft.trim() === "" ? null : await api.pickTextLayer(docId, Math.floor(at.x), Math.floor(at.y));
      if (picked !== layer.id) {
        // Hors de tout calque (plan de travail, zone vide) : on sort de la saisie ET plus rien n'est sélectionné.
        const under = await api.pickLayer(docId, Math.floor(at.x), Math.floor(at.y));
        commit();
        if (under === null && app.active) app.active.selection = [];
        return;
      }
      // Le curseur va à l'endroit cliqué (le moteur ramène un clic un peu à côté sur la lettre la plus proche).
      const index = await positionAt(at);
      if (index === null || !field) return;
      dragAnchor = event.shiftKey ? field.selectionStart : index;
      field.setSelectionRange(Math.min(dragAnchor, index), Math.max(dragAnchor, index), index < dragAnchor ? "backward" : "forward");
      focusField();
      refreshCaret();
    } catch (error) {
      app.fail(error);
    }
  }

  // ------------------------------------------------------------ cadre de paragraphe

  /** Poignée saisie : le geste part du cadre confirmé par le moteur, dans le repère du texte. */
  function startResize(event: PointerEvent, handle: FrameHandle): void {
    if (event.button !== 0 || !frame || !text.frame) return;
    event.preventDefault(); // le champ de saisie garde le focus
    event.stopPropagation();
    const start = frameAxes(frame, text.frame.width, text.frame.height);
    if (!start) return;
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    resizing = { pointerId: event.pointerId, handle, axes: start, width: text.frame.width, height: text.frame.height, gesture: `cadre-texte-${layer.id}-${Date.now()}`, quad: frame, ended: false };
  }

  /** Tirer la poignée : le cadre suit, le texte se recompose en direct (un seul état d'historique pour le geste). */
  function moveResize(event: PointerEvent): void {
    const current = resizing;
    if (!current || current.ended || event.pointerId !== current.pointerId) return;
    event.stopPropagation();
    const next = resizeFrame(current.handle, current.width, current.height, toFrame(current.axes, docPoint(event)), event.shiftKey);
    current.quad = frameQuad(current.axes, next.shift[0], next.shift[1], next.width, next.height);
    resized = true;
    editLatest(current.gesture, { op: "setTextFrame", id: layer.id, frame: { width: next.width, height: next.height }, shift: next.shift, gesture: current.gesture });
  }

  function endResize(event: PointerEvent): void {
    const current = resizing;
    if (!current || event.pointerId !== current.pointerId) return;
    event.stopPropagation();
    const { gesture } = current;
    // Le cadre du geste reste affiché jusqu'à ce que le moteur ait appliqué le dernier pas et rendu le sien.
    void whenIdle(gesture).then(() => {
      if (resizing?.gesture !== gesture) return;
      resizing.ended = true;
      refreshCaret();
    });
    focusField();
  }

  async function onPointerMove(event: PointerEvent): Promise<void> {
    if (sampling) {
      // Bouton maintenu : la couleur suit le pointeur jusqu'au relâchement.
      const point = docPoint(event);
      void sampleColor(point.x, point.y, event);
      return;
    }
    if (dragAnchor === null || !field) return;
    const index = await positionAt(docPoint(event)).catch(() => null);
    if (index === null || dragAnchor === null) return;
    field.setSelectionRange(Math.min(dragAnchor, index), Math.max(dragAnchor, index), index < dragAnchor ? "backward" : "forward");
    refreshCaret();
  }
</script>

<svelte:window onkeydown={(e) => (altHeld = e.altKey)} onkeyup={(e) => (altHeld = e.altKey)} />

<div
  bind:this={root}
  class="overlay"
  class:pipette={altHeld && app.eyedropperTarget !== null}
  role="application"
  aria-label={t("Saisie du texte : Ctrl+Entrée pour valider, Échap pour annuler")}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={() => ((dragAnchor = null), (sampling = false), (app.sample = null))}
  onpointercancel={() => ((dragAnchor = null), (sampling = false), (app.sample = null))}
  onscroll={unscroll}
  oncontextmenu={(event) => {
    // Clic droit sur le texte en cours de saisie : le menu du texte (conversion paragraphe / point), par la vue.
    event.preventDefault();
    oncontext?.(event);
  }}
  ondblclick={() => {
    // Double-clic : tout le texte est sélectionné, prêt à être retapé ou remis en forme.
    focusField();
    field?.select();
    refreshCaret();
  }}
>
  <svg aria-hidden="true">
    {#if framePoints}
      <!-- Cadre du texte de paragraphe : pointillés gris foncé sur un liseré clair (lisible sur toute image), bien
           distinct du cadre bleu de la transformation (Ctrl+T). -->
      <polygon class="frame-under" points={framePoints} />
      <polygon class="frame-over" points={framePoints} />
    {/if}
    {#each polygons as points, index (index)}
      <polygon {points} />
    {/each}
    {#if caretLine?.[0] && caretLine[1]}
      {#key `${caretLine[0].x}-${caretLine[0].y}`}
        <line x1={caretLine[0].x} y1={caretLine[0].y} x2={caretLine[1].x} y2={caretLine[1].y} class="under" />
        <line x1={caretLine[0].x} y1={caretLine[0].y} x2={caretLine[1].x} y2={caretLine[1].y} class="caret" />
      {/key}
    {/if}
    {#if overflowMark}
      <!-- Texte masqué sous le cadre : petit ⊞ près du coin bas droit, comme Photoshop (sans son gros carré noir). -->
      <g class="overflow" transform="translate({overflowMark.x} {overflowMark.y})">
        <title>{t("Texte masqué : agrandissez le cadre pour l'afficher")}</title>
        <rect x="-5.5" y="-5.5" width="11" height="11" />
        <path d="M-3 0H3M0 -3V3" />
      </g>
    {/if}
  </svg>
  {#each handles as { handle, at, cursor }, index (index)}
    <div
      class="handle"
      role="presentation"
      style:left="{at.x}px"
      style:top="{at.y}px"
      style:cursor
      onpointerdown={(event) => startResize(event, handle)}
      onpointermove={moveResize}
      onpointerup={endResize}
      onpointercancel={endResize}
    ></div>
  {/each}
  <!-- Placé sous le curseur : la fenêtre de suggestions d'une méthode de saisie s'ouvre au bon endroit. Tant que la
       position du curseur n'est pas connue, il attend au centre du cadre (toujours à l'écran, même vue tournée), et
       non plus au coin du cadre, hors de l'écran : un navigateur qui voudrait le montrer n'a rien à faire défiler. -->
  <textarea
    bind:this={field}
    class:pending={!caretLine?.[1]}
    aria-label={t("Contenu du texte")}
    spellcheck="false"
    autocomplete="off"
    style:left={caretLine?.[1] ? `${caretLine[1].x}px` : undefined}
    style:top={caretLine?.[1] ? `${caretLine[1].y}px` : undefined}
    oninput={onInput}
    onkeydown={onKeyDown}
    onkeyup={(event) => event.stopPropagation()}
  ></textarea>
</div>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    overflow: hidden;
    cursor: text;
    touch-action: none;
  }
  /* Jamais défilable, même par script (voir `focusField`) ; « hidden » reste pour une webview qui ignorerait « clip ». */
  @supports (overflow: clip) {
    .overlay {
      overflow: clip;
    }
  }
  .overlay.pipette {
    cursor: var(--cursor-pipette);
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  polygon {
    fill: rgba(77, 141, 255, 0.35);
  }
  /* Cadre de paragraphe (saisie) : pointillés GRIS FONCÉ, fins, sans couleur — la transformation, elle, est bleue. */
  polygon.frame-under,
  polygon.frame-over {
    fill: none;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }
  polygon.frame-under {
    stroke: rgba(255, 255, 255, 0.75);
  }
  polygon.frame-over {
    stroke: #3a3a3a;
    stroke-dasharray: 3 3;
  }
  /* Poignées carrées blanches, liseré gris foncé, comme Photoshop pendant la saisie. */
  .handle {
    position: absolute;
    width: 7px;
    height: 7px;
    transform: translate(-50%, -50%);
    box-sizing: border-box;
    background: #fff;
    border: 1px solid #3a3a3a;
    touch-action: none;
  }
  .handle::before {
    /* Zone de saisie plus large que le carré dessiné. */
    content: "";
    position: absolute;
    inset: -4px;
  }
  .overflow {
    pointer-events: all;
    cursor: default;
  }
  .overflow rect {
    fill: #fff;
    stroke: #3a3a3a;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }
  .overflow path {
    stroke: #3a3a3a;
    stroke-width: 1.2;
  }
  line {
    stroke-linecap: butt;
  }
  .under {
    stroke: rgba(0, 0, 0, 0.6);
    stroke-width: 3;
  }
  .caret {
    stroke: #fff;
    stroke-width: 1.5;
  }
  @media (prefers-reduced-motion: no-preference) {
    .under,
    .caret {
      animation: blink 1.06s steps(1) infinite;
    }
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  textarea {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    border: 0;
    opacity: 0;
    resize: none;
    overflow: hidden;
    white-space: pre;
    pointer-events: none;
  }
  /* Position du curseur pas encore connue : invisible (découpé en entier, mais focalisable), au centre du cadre. */
  textarea.pending {
    left: 50%;
    top: 50%;
    clip-path: inset(50%);
  }
</style>
