<script lang="ts">
  import { panelOwnsKeyboard } from "../lib/keymap";
  import { t } from "../lib/i18n.svelte";
  import { api } from "../lib/ipc";
  import { rotateCursorFor } from "../lib/cursors";
  import { add, centroid, decompose, diagonalScale, dot, isUsableQuad, midpoint, pointInQuad, rotate, scale, snapOffset, sub, unit } from "../lib/geometry";
  import { SNAP_PIXELS, edgesAndCenter, setSnapObjects, snapAxis, snapTargets } from "../lib/snapping.svelte";
  import ContextMenu, { type MenuEntry } from "../components/ContextMenu.svelte";
  import { onDestroy } from "svelte";
  import { cancelTransform, commitTransform, sendTransform, setTransformDragging } from "../lib/transform-session";
  import { app, type Point, type TransformSession } from "../state/app.svelte";
  import type { Camera } from "./camera";

  // Cadre de transformation libre. Toute manipulation se réduit à déplacer les quatre
  // coins du cadre ; la matrice envoyée au moteur est l'homographie « cadre d'origine ->
  // cadre courant ». Échelle, rotation, inclinaison et déformation partagent donc un seul
  // chemin, et le moteur repart toujours de l'état d'avant Ctrl+T (aucun cumul d'erreurs).

  // Deux usages :
  // - Ctrl+T (modal) : tout l'écran appartient au cadre ; cliquer ailleurs valide ;
  // - outil Déplacement (`passive`) : seuls les poignées et les anneaux de rotation réagissent,
  //   le reste de l'image reste à l'outil (déplacer, sélectionner un autre calque).
  let { session, camera, passive, shift = { dx: 0, dy: 0 } }: { session: TransformSession; camera: Camera; passive: boolean; shift?: { dx: number; dy: number } } = $props();

  type Grip = { kind: "corner"; index: number } | { kind: "edge"; index: number } | { kind: "move" } | { kind: "rotate"; index: number };
  let drag = $state<{ grip: Grip; start: Point; quad: Point[] } | null>(null);
  let hoverCursor = $state("default");

  const HANDLE = 10; // demi-zone de saisie des poignées, en pixels d'écran
  const HANDLE_SIZE = 12; // côté visible d'une poignée
  const ROTATE_REACH = 72; // jusqu'où, autour d'un angle, un glisser fait tourner
  const ROTATE_RING = 34; // cadre passif : anneau de rotation autour de chaque angle
  /** Maj pendant une rotation : pas de 15°. */
  const ROTATE_STEP = Math.PI / 12;
  /** Rotation en cours : angle brut continu (sans saut de ±360°). */
  let turn = { raw: 0, previous: 0 };

  // `shift` : pendant qu'on glisse le calque avec l'outil Déplacement, le cadre le suit.
  const toScreen = (p: Point): Point => ({ x: camera.x + (p.x + shift.dx) * camera.zoom, y: camera.y + (p.y + shift.dy) * camera.zoom });
  const screenQuad = $derived(session.quad.map(toScreen));
  const edgeHandles = $derived(screenQuad.map((p, i) => midpoint(p, screenQuad[(i + 1) % 4] ?? p)));
  const center = $derived(centroid(screenQuad));
  /** Identifiant propre à ce cadre : plusieurs documents peuvent en afficher un. */
  const ringClipId = `anneaux-${Math.random().toString(36).slice(2, 9)}`;
  const outline = $derived(screenQuad.map((p, i) => `${i === 0 ? "M" : "L"}${p.x} ${p.y}`).join("") + "Z");

  let root = $state<SVGSVGElement>();

  function docPoint(event: PointerEvent): Point {
    const rect = (root ?? (event.currentTarget as Element)).getBoundingClientRect();
    return { x: (event.clientX - rect.left - camera.x) / camera.zoom, y: (event.clientY - rect.top - camera.y) / camera.zoom };
  }

  function gripAt(event: PointerEvent): Grip | null {
    const rect = (root ?? (event.currentTarget as Element)).getBoundingClientRect();
    const at = { x: event.clientX - rect.left, y: event.clientY - rect.top };
    const near = (p: Point, reach: number) => Math.hypot(p.x - at.x, p.y - at.y) <= reach;
    const corner = screenQuad.findIndex((p) => near(p, HANDLE));
    if (corner >= 0) return { kind: "corner", index: corner };
    const edgeIndex = edgeHandles.findIndex((p) => near(p, HANDLE));
    if (edgeIndex >= 0) return { kind: "edge", index: edgeIndex };
    if (pointInQuad(at, screenQuad)) return { kind: "move" };
    const ring = screenQuad.findIndex((p) => near(p, ROTATE_REACH));
    return ring >= 0 ? { kind: "rotate", index: ring } : null;
  }

  /** Nouveau cadre pour un glisser depuis `from`, selon la poignée et les touches enfoncées. */
  function reshape(grip: Grip, from: Point[], start: Point, now: Point, event: PointerEvent): Point[] {
    const delta = sub(now, start);
    const center = centroid(from);
    const corner = (i: number): Point => from[((i % 4) + 4) % 4] ?? center;

    if (grip.kind === "move") {
      // Déplacement entier : un simple décalage ne doit jamais flouter l'image.
      let d = { x: Math.round(delta.x), y: Math.round(delta.y) };
      if (event.shiftKey) d = Math.abs(d.x) >= Math.abs(d.y) ? { x: d.x, y: 0 } : { x: 0, y: d.y };
      // Magnétisme : bords et milieu du cadre contre repères, bords et milieu du document. Ctrl le suspend.
      if (app.snapEnabled && !event.ctrlKey) {
        const xs = from.map((p) => p.x + d.x);
        const ys = from.map((p) => p.y + d.y);
        const reach = SNAP_PIXELS / camera.zoom;
        d = {
          x: d.x + Math.round(snapOffset(edgesAndCenter(Math.min(...xs), Math.max(...xs)), snapTargets("x"), reach)),
          y: d.y + Math.round(snapOffset(edgesAndCenter(Math.min(...ys), Math.max(...ys)), snapTargets("y"), reach)),
        };
      }
      return from.map((p) => add(p, d));
    }
    if (grip.kind === "rotate") {
      // Angle brut, rendu continu : passer de +180° à -180° ne fait pas faire un tour complet au calque.
      const measured = Math.atan2(now.y - center.y, now.x - center.x) - Math.atan2(start.y - center.y, start.x - center.x);
      let step = measured - turn.previous;
      step -= Math.round(step / (2 * Math.PI)) * 2 * Math.PI;
      turn.previous = measured;
      turn.raw += step;
      let angle = turn.raw;
      // Maj : pas de 15°, comme Photoshop.
      if (event.shiftKey) angle = Math.round(angle / ROTATE_STEP) * ROTATE_STEP;
      return from.map((p) => rotate(p, center, angle));
    }
    const i = grip.index;
    // Ctrl + coin : déformation libre de ce seul coin.
    if (event.ctrlKey && grip.kind === "corner") return from.map((p, k) => (k === i ? add(p, delta) : p));
    // Ctrl + Maj + côté : inclinaison le long de ce côté.
    if (event.ctrlKey && event.shiftKey && grip.kind === "edge") {
      const along = unit(sub(corner(i + 1), corner(i)));
      const slide = scale(along, dot(delta, along));
      return from.map((p, k) => (k === i || k === (i + 1) % 4 ? add(p, slide) : p));
    }

    // Mise à l'échelle dans le repère du cadre : u le long du côté haut, v le long du côté gauche.
    const u = unit(sub(corner(1), corner(0)));
    const v = unit(sub(corner(3), corner(0)));
    const handle = grip.kind === "corner" ? corner(i) : midpoint(corner(i), corner(i + 1));
    const opposite = grip.kind === "corner" ? corner(i + 2) : midpoint(corner(i + 2), corner(i + 3));
    // Règles des touches, comme Photoshop depuis 2019 (image, texte, forme, groupe : tout calque transformé) :
    //   coin, sans touche : proportions CONSERVÉES, point opposé fixe ;
    //   coin + Maj        : proportions libres ;
    //   côté, sans touche : ce côté seul (une dimension) ; côté + Maj : proportions conservées ;
    //   Alt               : depuis le centre (mêmes règles de proportions) ;
    //   Ctrl              : ce côté seul, sans proportions (pour un coin : déformation, traitée plus haut).
    const anchor = event.altKey ? center : opposite;
    // Forme : tirer un côté (avec ou sans Maj) l'allonge dans cette seule direction ; elle est ensuite redessinée,
    // coins intacts (voir shapePlacement). Ailleurs, Maj sur un côté garde les proportions.
    const shapeOnly = session.ids.length === 1 && !!app.active?.info.layers.find((l) => l.id === session.ids[0])?.shape;
    const proportional = !event.ctrlKey && (grip.kind === "corner" ? !event.shiftKey : !shapeOnly && (event.shiftKey || event.altKey));
    const [a0, b0] = decompose(sub(handle, anchor), u, v);
    // Magnétisme du redimensionnement : la poignée tirée s'accroche aux bords et au centre du plan de travail,
    // aux repères et aux autres objets. Seulement pour un cadre droit (non tourné) ; Ctrl le suspend.
    const moved = add(handle, delta);
    const upright = Math.abs(u.y) < 1e-3 && Math.abs(v.x) < 1e-3;
    const lines: { x: number | null; y: number | null } = { x: null, y: null };
    if (app.snapEnabled && upright && !event.ctrlKey) {
      const reach = SNAP_PIXELS / camera.zoom;
      // Un côté ne bouge que sur son axe : haut et bas (0, 2) en vertical, droite et gauche (1, 3) en horizontal.
      if (grip.kind === "corner" || i % 2 === 1) {
        const snap = snapAxis([moved.x], "x", reach);
        [moved.x, lines.x] = [moved.x + snap.offset, snap.line];
      }
      if (grip.kind === "corner" || i % 2 === 0) {
        const snap = snapAxis([moved.y], "y", reach);
        [moved.y, lines.y] = [moved.y + snap.offset, snap.line];
      }
    }
    app.smartGuides = lines;
    const [a1, b1] = decompose(sub(moved, anchor), u, v);
    let su = Math.abs(a0) > 1e-6 ? a1 / a0 : 1;
    let sv = Math.abs(b0) > 1e-6 ? b1 / b0 : 1;
    if (grip.kind === "edge") {
      // Côtés haut et bas (0, 2) : hauteur ; côtés droit et gauche (1, 3) : largeur.
      if (i % 2 === 0) su = proportional ? sv : 1;
      else sv = proportional ? su : 1;
    } else if (proportional) {
      // Coin à proportions conservées : projection de la souris sur la diagonale coin -> point fixe (voir
      // diagonalScale). Le coin suit ainsi la souris au plus près, au lieu de n'écouter que l'axe le plus
      // étiré (le cadre semblait en retard ou en avance sur le geste). Le retournement reste possible :
      // un cadre croisé ou aplati est refusé plus loin (isUsableQuad).
      const s = diagonalScale(anchor, handle, moved);
      [su, sv] = [s, s];
    }
    return from.map((p) => {
      const [a, b] = decompose(sub(p, anchor), u, v);
      return add(anchor, add(scale(u, a * su), scale(v, b * sv)));
    });
  }

  function endDrag(): void {
    drag = null;
    setTransformDragging(false);
    app.smartGuides = { x: null, y: null };
  }
  // Cadre remplacé ou fermé en plein geste : le drapeau de geste ne doit pas lui survivre.
  onDestroy(() => drag && setTransformDragging(false));

  function begin(event: PointerEvent, grip: Grip): void {
    event.stopPropagation();
    turn = { raw: 0, previous: 0 };
    // Les autres objets du document deviennent des cibles du magnétisme pour ce geste.
    const doc = app.active;
    if (doc) {
      setSnapObjects([]);
      api.otherBounds(doc.info.id, [...doc.selection]).then(setSnapObjects, () => undefined);
    }
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    drag = { grip, start: docPoint(event), quad: session.quad.map((p) => ({ ...p })) };
    setTransformDragging(true); // le viewport ne rouvre pas de cadre sous la poignée tirée
    session.steps.push(drag.quad); // état d'avant cette manipulation, pour Ctrl+Z
  }

  /** Curseur de rotation tourné vers l'extérieur du cadre, à l'angle `index`. */
  function cornerCursor(index: number): string {
    const corner = screenQuad[index] ?? center;
    return rotateCursorFor(corner.x - center.x, corner.y - center.y);
  }

  /** Cadre modal : le clic vise une poignée, l'intérieur, la zone de rotation… ou rien : on valide alors. */
  function onPointerDown(event: PointerEvent): void {
    if (event.button !== 0 || passive) return;
    const grip = gripAt(event);
    if (grip) begin(event, grip);
    else commitTransform(); // cliquer ailleurs applique la transformation, sans passer par Entrée
  }

  function onPointerMove(event: PointerEvent): void {
    if (!drag) {
      const grip = gripAt(event);
      hoverCursor = !grip ? "default" : grip.kind === "move" ? "move" : grip.kind === "rotate" ? cornerCursor(grip.index) : "pointer";
      return;
    }
    const next = reshape(drag.grip, drag.quad, drag.start, docPoint(event), event);
    if (!isUsableQuad(next)) return; // cadre aplati ou croisé : on reste sur le dernier cadre valide
    session.quad = next;
    sendTransform(false);
  }

  /** Ctrl+Z pendant la transformation : défait la dernière manipulation ; s'il n'y en a plus, quitte sans rien changer. */
  function stepBack(): void {
    const previous = session.steps.pop();
    if (!previous) {
      cancelTransform();
      return;
    }
    session.quad = previous;
    sendTransform(false);
  }

  // ---- Clic droit dans le cadre : rotations droites et miroirs, comme Photoshop.
  let menu = $state<{ x: number; y: number } | null>(null);

  /**
   * Applique une transformation droite au cadre lui-même : le contenu suit, puisque la matrice
   * envoyée au moteur est toujours « cadre d'origine -> cadre courant ». Aucune perte de qualité,
   * et [Ctrl] + [Z] revient en arrière comme pour n'importe quelle autre manipulation.
   */
  function reframe(move: (p: Point, middle: Point) => Point): void {
    const middle = centroid(session.quad);
    const next = session.quad.map((p) => move(p, middle));
    if (!isUsableQuad(next)) return;
    session.steps.push(session.quad);
    session.quad = next;
    sendTransform(false);
  }

  const menuEntries = (): MenuEntry[] => [
    { label: t("Rotation 90° horaire"), run: () => reframe((p, c) => rotate(p, c, Math.PI / 2)) },
    { label: t("Rotation 90° antihoraire"), run: () => reframe((p, c) => rotate(p, c, -Math.PI / 2)) },
    { label: t("Rotation 180°"), run: () => reframe((p, c) => rotate(p, c, Math.PI)) },
    "separator",
    { label: "Miroir horizontal", run: () => reframe((p, c) => ({ x: 2 * c.x - p.x, y: p.y })) },
    { label: "Miroir vertical", run: () => reframe((p, c) => ({ x: p.x, y: 2 * c.y - p.y })) },
    "separator",
    { label: "Valider", shortcut: "Entrée", run: commitTransform },
    { label: "Annuler", shortcut: "Échap", run: cancelTransform },
  ];

  /**
   * Le menu n'appartient au cadre que si le clic tombe dedans, ET dans la zone de travail : un clic
   * droit sur l'écran d'accueil, un panneau ou une fenêtre ne concerne jamais la transformation.
   */
  function onContextMenu(event: MouseEvent): void {
    const box = root?.getBoundingClientRect();
    if (!box || app.homeVisible) return;
    const inside = event.target instanceof Node && root?.parentElement?.contains(event.target);
    if (!inside) return;
    const local = { x: event.clientX - box.left, y: event.clientY - box.top };
    if (!pointInQuad(local, screenQuad)) return;
    event.preventDefault();
    event.stopPropagation();
    menu = { x: event.clientX, y: event.clientY };
  }

  function onKeyDown(event: KeyboardEvent): void {
    // Cadre passif intact : le clavier reste à l'application (Ctrl+Z annule normalement, par exemple).
    if (passive && !session.previewed) return;
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || panelOwnsKeyboard()) return;
    if (event.key === "Enter") commitTransform();
    else if (event.key === "Escape") cancelTransform();
    else if (event.ctrlKey && !event.shiftKey && event.key.toLowerCase() === "z") stepBack();
    else return;
    event.preventDefault();
    event.stopPropagation();
  }
</script>

<svelte:window onkeydown={onKeyDown} oncontextmenucapture={onContextMenu} />

<svg
  bind:this={root}
  class="overlay"
  class:passive
  role="application"
  aria-label={t("Cadre de transformation : Entrée pour valider, Échap pour annuler")}
  style:cursor={passive ? undefined : drag ? "grabbing" : hoverCursor}
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={endDrag}
  onpointercancel={endDrag}
  ondblclick={() => !passive && commitTransform()}
>
  <path d={outline} class="under" />
  <path d={outline} class="over" />
  {#if passive}
    <!-- Anneaux de rotation, sous les poignées : tirer juste à côté d'un angle fait tourner. Ils sont découpés au
         contour du cadre (règle « evenodd » : tout l'écran moins le cadre) : centrés sur l'angle, ils débordaient à
         l'intérieur, et sur un petit objet l'intérieur entier montrait le curseur de rotation. La découpe vaut aussi
         pour le clic. -->
    <defs>
      <clipPath id={ringClipId}>
        <path clip-rule="evenodd" d="M-100000 -100000H100000V100000H-100000Z{outline}" />
      </clipPath>
    </defs>
    {#each screenQuad as corner, index (index)}
      <circle cx={corner.x} cy={corner.y} r={ROTATE_RING} class="ring" clip-path="url(#{ringClipId})" style:cursor={cornerCursor(index)} role="presentation" onpointerdown={(event) => event.button === 0 && begin(event, { kind: "rotate", index })} />
    {/each}
  {/if}
  {#each screenQuad as corner, index (index)}
    {@const out = unit(sub(corner, center))}
    {@const side = { x: -out.y, y: out.x }}
    {@const tip = add(corner, scale(out, 26))}
    <!-- Arc à l'extérieur de chaque angle : la zone de rotation. -->
    <path class="rotate-hint" d="M{tip.x - side.x * 8 - out.x * 3} {tip.y - side.y * 8 - out.y * 3}Q{tip.x + out.x * 2} {tip.y + out.y * 2} {tip.x + side.x * 8 - out.x * 3} {tip.y + side.y * 8 - out.y * 3}" />
  {/each}
  {#each screenQuad as handle, index (index)}
    <rect x={handle.x - HANDLE_SIZE / 2} y={handle.y - HANDLE_SIZE / 2} width={HANDLE_SIZE} height={HANDLE_SIZE} class="handle" role="presentation" onpointerdown={(event) => passive && event.button === 0 && begin(event, { kind: "corner", index })} />
  {/each}
  {#each edgeHandles as handle, index (index)}
    <rect x={handle.x - HANDLE_SIZE / 2} y={handle.y - HANDLE_SIZE / 2} width={HANDLE_SIZE} height={HANDLE_SIZE} class="handle" role="presentation" onpointerdown={(event) => passive && event.button === 0 && begin(event, { kind: "edge", index })} />
  {/each}
  <path d="M{center.x - 5} {center.y}h10M{center.x} {center.y - 5}v10" class="over" />
</svg>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} entries={menuEntries()} onclose={() => (menu = null)} />
{/if}

<style>
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
  }
  /* Cadre passif : l'image dessous reste cliquable, seuls poignées et anneaux captent la souris. */
  .overlay.passive {
    pointer-events: none;
  }
  .overlay.passive .handle,
  .overlay.passive .ring {
    pointer-events: all;
  }
  .overlay.passive .handle {
    cursor: pointer;
  }
  .ring {
    fill: transparent;
  }
  /* Petit arc visible à chaque angle : on voit où saisir pour faire tourner. */
  .rotate-hint {
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.5;
    stroke-linecap: round;
    opacity: 0.85;
    pointer-events: none;
  }
  path {
    fill: none;
    stroke-width: 1;
  }
  .under {
    stroke: rgba(0, 0, 0, 0.55);
    stroke-width: 3;
  }
  .over {
    stroke: var(--accent);
  }
  .handle {
    fill: #fff;
    stroke: var(--accent);
    stroke-width: 1;
  }
</style>
