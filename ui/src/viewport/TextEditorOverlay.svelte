<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount } from "svelte";
  import { beginFreeTransform } from "../lib/actions";
  import { dropLatest, edit, editLatest, enqueue, sampleColor, whenIdle } from "../lib/engine";
  import { type LayerInfo, type TextData, api } from "../lib/ipc";
  import { finishTextEdit } from "../lib/text-edit.svelte";
  import { adjustRuns } from "../lib/text-runs";
  import { app, type Point, type TextEditSession } from "../state/app.svelte";
  import type { Camera } from "./camera";

  // Saisie de texte directement sur l'image, façon machine à écrire.
  //
  // La frappe passe par un champ de saisie invisible : c'est lui qui gère correctement les
  // accents (touches mortes ^ ¨ en AZERTY), le presse-papiers, la sélection au clavier et
  // les méthodes de saisie. L'AFFICHAGE, lui, vient entièrement du moteur : le texte est le
  // vrai rendu, et la position du curseur et de la sélection est calculée par la même mise
  // en page. Rien ne peut donc se décaler entre ce qu'on tape et ce qu'on voit.

  let { session, layer, text, camera }: { session: TextEditSession; layer: LayerInfo; text: TextData; camera: Camera } = $props();

  let field = $state<HTMLTextAreaElement>();
  let caret = $state<[Point, Point] | null>(null);
  let highlights = $state<Point[][]>([]);
  /** Point de départ d'une sélection à la souris, en position de texte. */
  let dragAnchor: number | null = null;
  let altHeld = $state(false);
  let sampling = false;
  const docId = app.activeId;

  const toScreen = (p: Point): Point => ({ x: camera.x + p.x * camera.zoom, y: camera.y + p.y * camera.zoom });
  const caretLine = $derived(caret ? [toScreen(caret[0]), toScreen(caret[1])] : null);
  const polygons = $derived(highlights.map((quad) => quad.map(toScreen).map((p) => `${p.x},${p.y}`).join(" ")));

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
    field.focus();
  });

  onMount(() => {
    if (!field) return;
    field.value = session.draft;
    field.focus();
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
      field.focus();
      refreshCaret();
    } catch (error) {
      app.fail(error);
    }
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
  ondblclick={() => {
    // Double-clic : tout le texte est sélectionné, prêt à être retapé ou remis en forme.
    field?.focus();
    field?.select();
    refreshCaret();
  }}
>
  <svg aria-hidden="true">
    {#each polygons as points, index (index)}
      <polygon {points} />
    {/each}
    {#if caretLine?.[0] && caretLine[1]}
      {#key `${caretLine[0].x}-${caretLine[0].y}`}
        <line x1={caretLine[0].x} y1={caretLine[0].y} x2={caretLine[1].x} y2={caretLine[1].y} class="under" />
        <line x1={caretLine[0].x} y1={caretLine[0].y} x2={caretLine[1].x} y2={caretLine[1].y} class="caret" />
      {/key}
    {/if}
  </svg>
  <!-- Placé sous le curseur : la fenêtre de suggestions d'une méthode de saisie s'ouvre au bon endroit. -->
  <textarea
    bind:this={field}
    aria-label={t("Contenu du texte")}
    spellcheck="false"
    autocomplete="off"
    style:left="{caretLine?.[1]?.x ?? 0}px"
    style:top="{caretLine?.[1]?.y ?? 0}px"
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
</style>
