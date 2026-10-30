<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { onMount, untrack } from "svelte";
  import { edit } from "../lib/engine";
  import { api, type Rgb } from "../lib/ipc";
  import { type Format, formatCategories, labelOf } from "../lib/formats";
  import { describe, formatHistory, loadFormatHistory } from "../lib/recent-formats";
  import { artboardsOf } from "../lib/artboards.svelte";
  import { app } from "../state/app.svelte";
  import CategoryIcon from "./CategoryIcon.svelte";

  // Fenêtre Plans de travail (menu de la barre, comme la Galerie de filtres) : à gauche la planche, à droite les
  // formats. Un clic sur un format le pose en pointillés à son emplacement par défaut (à droite du dernier plan) ;
  // on peut le glisser, ou glisser un format directement sur la planche. Deux plans ne se chevauchent jamais et
  // gardent une marge entre eux : un plan relâché trop près est poussé au plus proche emplacement libre.
  // Rien ne change dans le document avant « Appliquer » (un seul état d'historique).

  interface Draft {
    key: string;
    /** Plan existant, ou `null` (nouveau ; ou le document lui-même quand il n'a pas encore de plan). */
    id: number | null;
    name: string;
    x: number;
    y: number;
    w: number;
    h: number;
    background: Rgb | null;
    clip: boolean;
    /** Position du contenu dans l'aperçu du document (plans existants et document). */
    origin: { x: number; y: number } | null;
    /** Posé en pointillés, pas encore placé par l'utilisateur. */
    pending: boolean;
  }
  type Rect = { x: number; y: number; w: number; h: number };

  const doc = untrack(() => app.active);
  const docW = doc?.info.width ?? 1;
  const docH = doc?.info.height ?? 1;
  const existing = untrack(() => artboardsOf(doc?.info.layers ?? []));
  let seq = 0;
  const nextKey = () => `n${++seq}`;

  let drafts = $state<Draft[]>(
    untrack(() =>
      existing.length > 0
        ? existing.map((l) => ({ key: `b${l.id}`, id: l.id, name: l.name, x: l.artboard.x, y: l.artboard.y, w: l.artboard.width, h: l.artboard.height, background: l.artboard.background, clip: l.artboard.clip, origin: { x: l.artboard.x, y: l.artboard.y }, pending: false }))
        : [{ key: "doc", id: null, name: doc?.info.name || t("Plan de travail 1"), x: 0, y: 0, w: docW, h: docH, background: [255, 255, 255] as Rgb, clip: true, origin: { x: 0, y: 0 }, pending: false }],
    ),
  );
  let removed = $state<number[]>([]);
  /** Plan dont le nom est en cours de saisie dans la liste. */
  let renamingKey = $state<string | null>(null);
  /** Champ de renommage : prêt à la frappe dès qu'il paraît (`autofocus` n'agit pas sur un champ ajouté après coup). */
  function focusAndSelect(node: HTMLInputElement): void {
    queueMicrotask(() => {
      node.focus();
      node.select();
    });
  }
  let selectedKey = $state<string | null>(untrack(() => drafts[0]?.key ?? null));
  let margin = $state(100);
  const selected = $derived(drafts.find((d) => d.key === selectedKey) ?? null);

  let dialog = $state<HTMLDialogElement>();
  let stage = $state<HTMLDivElement>();
  let previewUrl = $state<string | null>(null);
  let view = $state({ zoom: 0.2, x: 40, y: 40 });

  onMount(() => {
    dialog?.showModal();
    void loadFormatHistory();
    if (doc)
      void api.documentPreview(doc.info.id, 1600).then((png) => {
        previewUrl = URL.createObjectURL(new Blob([png], { type: "image/png" }));
      });
    queueMicrotask(fit);
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
  });

  // ------------------------------------------------------------------ géométrie de la planche

  function collides(r: Rect, other: Rect): boolean {
    return r.x < other.x + other.w + margin && other.x < r.x + r.w + margin && r.y < other.y + other.h + margin && other.y < r.y + r.h + margin;
  }
  const others = (key: string | null) => drafts.filter((d) => d.key !== key);
  const free = (r: Rect, key: string | null) => others(key).every((o) => !collides(r, o));

  /** Emplacement libre le plus proche de `r` : collé à un côté d'un plan (aligné sur son bord), marge comprise. */
  function nearestFree(r: Rect, key: string | null): Rect {
    if (free(r, key)) return r;
    const list = others(key);
    const candidates: Rect[] = [];
    for (const o of list) {
      for (const [x, y] of [
        [o.x + o.w + margin, o.y],
        [o.x + o.w + margin, o.y + o.h - r.h],
        [o.x - margin - r.w, o.y],
        [o.x - margin - r.w, o.y + o.h - r.h],
        [o.x, o.y + o.h + margin],
        [o.x + o.w - r.w, o.y + o.h + margin],
        [o.x, o.y - margin - r.h],
        [o.x + o.w - r.w, o.y - margin - r.h],
        [r.x, o.y + o.h + margin],
        [r.x, o.y - margin - r.h],
        [o.x + o.w + margin, r.y],
        [o.x - margin - r.w, r.y],
      ] as [number, number][]) {
        const c = { ...r, x: Math.round(x), y: Math.round(y) };
        if (free(c, key)) candidates.push(c);
      }
    }
    const dist = (c: Rect) => Math.hypot(c.x - r.x, c.y - r.y);
    return candidates.sort((a, b) => dist(a) - dist(b))[0] ?? { ...r, x: Math.max(...list.map((o) => o.x + o.w)) + margin, y: r.y };
  }

  /** Emplacement par défaut : à droite du dernier plan placé, aligné en haut. */
  function defaultPlace(w: number, h: number, key: string | null): Rect {
    const placed = drafts.filter((d) => !d.pending && d.key !== key);
    const last = placed.at(-1);
    if (!last) return { x: 0, y: 0, w, h };
    return nearestFree({ x: last.x + last.w + margin, y: last.y, w, h }, key);
  }

  /** Aimantation aux bords et aux alignements des autres plans (8 px à l'écran) ; repères affichés. */
  let guides = $state<{ x: number | null; y: number | null }>({ x: null, y: null });
  function snap(r: Rect, key: string | null): Rect {
    const tol = 8 / view.zoom;
    let [bx, by, gx, gy] = [r.x, r.y, null as number | null, null as number | null];
    let [dx, dy] = [tol, tol];
    for (const o of others(key)) {
      for (const [c, line] of [
        [o.x, o.x],
        [o.x + o.w - r.w, o.x + o.w],
        [o.x + o.w + margin, o.x + o.w + margin],
        [o.x - margin - r.w, o.x - margin],
      ] as [number, number][]) {
        if (Math.abs(r.x - c) < dx) [dx, bx, gx] = [Math.abs(r.x - c), c, line];
      }
      for (const [c, line] of [
        [o.y, o.y],
        [o.y + o.h - r.h, o.y + o.h],
        [o.y + o.h + margin, o.y + o.h + margin],
        [o.y - margin - r.h, o.y - margin],
      ] as [number, number][]) {
        if (Math.abs(r.y - c) < dy) [dy, by, gy] = [Math.abs(r.y - c), c, line];
      }
    }
    guides = { x: gx, y: gy };
    return { ...r, x: Math.round(bx), y: Math.round(by) };
  }

  // ------------------------------------------------------------------ vue

  const bounds = $derived.by(() => {
    if (drafts.length === 0) return { x: 0, y: 0, w: docW, h: docH };
    const x0 = Math.min(...drafts.map((d) => d.x));
    const y0 = Math.min(...drafts.map((d) => d.y));
    const x1 = Math.max(...drafts.map((d) => d.x + d.w));
    const y1 = Math.max(...drafts.map((d) => d.y + d.h));
    return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
  });

  function fit(): void {
    if (!stage) return;
    const r = stage.getBoundingClientRect();
    const b = bounds;
    // Un peu de place à droite pour le prochain plan, sans écraser l'ensemble quand il y en a beaucoup.
    const pad = 50;
    const room = drafts.length > 1 ? 1.12 : 1.5;
    const zoom = Math.min((r.width - pad * 2) / (b.w * room), (r.height - pad * 2) / (b.h * 1.1), 1);
    view = { zoom, x: pad - b.x * zoom, y: pad + 20 - b.y * zoom };
  }
  const toScreen = (r: Rect) => ({ left: view.x + r.x * view.zoom, top: view.y + r.y * view.zoom, width: r.w * view.zoom, height: r.h * view.zoom });
  function toPlanche(clientX: number, clientY: number): { x: number; y: number } {
    const r = stage!.getBoundingClientRect();
    return { x: (clientX - r.left - view.x) / view.zoom, y: (clientY - r.top - view.y) / view.zoom };
  }
  function zoomBy(k: number, cx?: number, cy?: number): void {
    const r = stage?.getBoundingClientRect();
    if (!r) return;
    const [px, py] = [cx ?? r.width / 2, cy ?? r.height / 2];
    const zoom = Math.min(4, Math.max(0.01, view.zoom * k));
    view = { zoom, x: px - ((px - view.x) / view.zoom) * zoom, y: py - ((py - view.y) / view.zoom) * zoom };
  }
  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    const r = stage!.getBoundingClientRect();
    zoomBy(Math.exp(-event.deltaY * 0.0015), event.clientX - r.left, event.clientY - r.top);
  }

  // ------------------------------------------------------------------ gestes

  /** Glisser en cours : un plan de la planche, un format venu de la liste, ou la planche elle-même. */
  let drag = $state<
    | { kind: "board"; key: string; start: { x: number; y: number }; from: Rect; moved: boolean }
    | { kind: "format"; format: { w: number; h: number; name: string }; at: { x: number; y: number } | null; startX: number; startY: number; moved: boolean }
    | { kind: "pan"; startX: number; startY: number; from: { x: number; y: number } }
    | null
  >(null);
  /** Rectangle survolé pendant un glisser (affiché) et s'il est libre. */
  let hover = $state<(Rect & { ok: boolean }) | null>(null);

  function boardDown(event: PointerEvent, d: Draft): void {
    if (event.button !== 0) return;
    event.stopPropagation();
    selectedKey = d.key;
    drag = { kind: "board", key: d.key, start: toPlanche(event.clientX, event.clientY), from: { x: d.x, y: d.y, w: d.w, h: d.h }, moved: false };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
  }
  function stageDown(event: PointerEvent): void {
    if (event.button !== 0 && event.button !== 1) return;
    drag = { kind: "pan", startX: event.clientX, startY: event.clientY, from: { x: view.x, y: view.y } };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
  }
  function formatDown(event: PointerEvent, format: { w: number; h: number; name: string }): void {
    if (event.button !== 0) return;
    event.preventDefault();
    drag = { kind: "format", format, at: null, startX: event.clientX, startY: event.clientY, moved: false };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp, { once: true });
  }

  function overStage(event: PointerEvent): boolean {
    const r = stage?.getBoundingClientRect();
    return !!r && event.clientX >= r.left && event.clientX <= r.right && event.clientY >= r.top && event.clientY <= r.bottom;
  }

  function onMove(event: PointerEvent): void {
    if (!drag) return;
    if (drag.kind === "pan") {
      view = { ...view, x: drag.from.x + event.clientX - drag.startX, y: drag.from.y + event.clientY - drag.startY };
      return;
    }
    if (drag.kind === "board") {
      const p = toPlanche(event.clientX, event.clientY);
      const r = snap({ ...drag.from, x: drag.from.x + p.x - drag.start.x, y: drag.from.y + p.y - drag.start.y }, drag.key);
      if (Math.hypot(r.x - drag.from.x, r.y - drag.from.y) * view.zoom > 2) drag.moved = true;
      const key = drag.key;
      const d = drafts.find((x) => x.key === key);
      if (d) [d.x, d.y] = [r.x, r.y];
      hover = { ...r, ok: free(r, drag.key) };
      return;
    }
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > 4) drag.moved = true;
    if (!overStage(event)) {
      drag.at = null;
      hover = null;
      return;
    }
    const p = toPlanche(event.clientX, event.clientY);
    const r = snap({ x: p.x - drag.format.w / 2, y: p.y - drag.format.h / 2, w: drag.format.w, h: drag.format.h }, null);
    drag.at = { x: event.clientX, y: event.clientY };
    hover = { ...r, ok: free(r, null) };
  }

  function onUp(event: PointerEvent): void {
    window.removeEventListener("pointermove", onMove);
    const ended = drag;
    drag = null;
    guides = { x: null, y: null };
    const target = hover;
    hover = null;
    if (!ended) return;
    if (ended.kind === "board") {
      const d = drafts.find((x) => x.key === ended.key);
      if (!d) return;
      if (ended.moved) d.pending = false;
      // Relâché trop près d'un autre plan : poussé au plus proche emplacement libre.
      const r = nearestFree({ x: d.x, y: d.y, w: d.w, h: d.h }, d.key);
      [d.x, d.y] = [r.x, r.y];
      return;
    }
    if (ended.kind === "pan") {
      // Clic sur la planche (sans la faire glisser) : l'aperçu en pointillés est abandonné, jamais créé en douce.
      if (Math.hypot(event.clientX - ended.startX, event.clientY - ended.startY) <= 4) dropPending();
      return;
    }
    if (ended.kind !== "format") return;
    if (!ended.moved) return clickFormat(ended.format);
    // Un autre format glissé sur la planche : l'aperçu en attente disparaît, seul le plan posé compte.
    dropPending();
    if (target && overStage(event)) {
      // Glissé sur la planche : posé là (ou au plus proche emplacement libre), comme un plan placé.
      const r = nearestFree(target, null);
      const draft: Draft = { key: nextKey(), id: null, name: ended.format.name, ...r, background: [255, 255, 255], clip: true, origin: null, pending: false };
      drafts.push(draft);
      selectedKey = draft.key;
    }
  }

  /** Dernier clic sur un format (nom, instant) : un second clic rapide sur le même est un double-clic. */
  let lastFormatClick: { name: string; at: number } | null = null;
  const DOUBLE_CLICK_MS = 400;

  /** Clic : aperçu en pointillés. Double-clic : le plan est créé tout de suite, plein et sélectionné. */
  function clickFormat(format: { w: number; h: number; name: string }): void {
    const now = performance.now();
    const double = lastFormatClick?.name === format.name && now - lastFormatClick.at < DOUBLE_CLICK_MS;
    lastFormatClick = double ? null : { name: format.name, at: now };
    choosePending(format);
    if (double) confirmPending();
  }

  /** L'aperçu en attente devient un plan (posé sur une place libre). */
  function confirmPending(): void {
    const d = drafts.find((x) => x.pending);
    if (!d) return;
    d.pending = false;
    const r = nearestFree({ x: d.x, y: d.y, w: d.w, h: d.h }, d.key);
    [d.x, d.y] = [r.x, r.y];
    selectedKey = d.key;
  }

  /** Retire l'aperçu en attente, s'il y en a un. */
  function dropPending(): void {
    const d = drafts.find((x) => x.pending);
    if (d) remove(d.key);
  }

  /** Clic sur un format : un plan en pointillés à l'emplacement par défaut (le précédent en attente est remplacé). */
  function choosePending(format: { w: number; h: number; name: string }): void {
    const current = drafts.find((d) => d.pending);
    const r = defaultPlace(format.w, format.h, current?.key ?? null);
    if (current) Object.assign(current, { name: format.name, ...r });
    else drafts.push({ key: nextKey(), id: null, name: format.name, ...r, background: [255, 255, 255], clip: true, origin: null, pending: true });
    selectedKey = drafts.find((d) => d.pending)?.key ?? selectedKey;
  }

  function remove(key: string): void {
    const d = drafts.find((x) => x.key === key);
    if (!d) return;
    if (d.id !== null) removed.push(d.id);
    drafts = drafts.filter((x) => x.key !== key);
    if (selectedKey === key) selectedKey = drafts[0]?.key ?? null;
  }

  /** Taille changée dans les réglages : le plan reste en place s'il le peut, sinon il est poussé. */
  function resize(patch: { w?: number; h?: number }): void {
    const d = selected;
    if (!d) return;
    d.w = Math.min(30000, Math.max(1, Math.round(patch.w ?? d.w)));
    d.h = Math.min(30000, Math.max(1, Math.round(patch.h ?? d.h)));
    const r = nearestFree({ x: d.x, y: d.y, w: d.w, h: d.h }, d.key);
    [d.x, d.y] = [r.x, r.y];
  }

  function onKey(event: KeyboardEvent): void {
    if (event.target instanceof HTMLInputElement) return;
    if ((event.key === "Delete" || event.key === "Backspace") && selectedKey) {
      event.preventDefault();
      remove(selectedKey);
    }
  }

  async function applyAll(): Promise<void> {
    const list = $state.snapshot(drafts) as Draft[];
    const ops = list.map((d) => ({
      id: d.id,
      name: d.name.trim() || t("Plan de travail"),
      board: { x: d.x, y: d.y, width: d.w, height: d.h, background: d.background, clip: d.clip, preset: null, backgroundLayer: null },
    }));
    close();
    const ok = await edit({ op: "applyArtboards", boards: ops, removed: $state.snapshot(removed) as number[] });
    if (ok) setTimeout(() => app.fitToWindow?.(), 80);
  }

  function close(): void {
    app.artboardDialog = false;
  }

  // ------------------------------------------------------------------ formats

  interface Tile {
    key: string;
    label: string;
    w: number;
    h: number;
    name: string;
  }
  let category = $state("recent");
  let search = $state("");
  const recentTiles = $derived<Tile[]>(
    $formatHistory.map(describe).map((q) => ({ key: `r-${q.key}`, label: q.format ? labelOf(q.format, t) : `${q.width} × ${q.height}`, w: q.width, h: q.height, name: q.format ? labelOf(q.format, t) : `${q.width} × ${q.height}` })),
  );
  const tilesOf = (categoryLabel: string, formats: readonly Format[], id: string): Tile[] =>
    formats.map((f) => ({ key: `${id}-${f.id}`, label: labelOf(f, t), w: f.width, h: f.height, name: `${categoryLabel} · ${labelOf(f, t)}` }));
  const shown = $derived.by<{ title: string; tiles: Tile[] }[]>(() => {
    const q = search.trim().toLowerCase();
    if (q) {
      const all = $formatCategories.flatMap((c) => tilesOf(labelOf(c, t), c.formats, c.id));
      return [{ title: t("Résultats"), tiles: all.filter((tile) => `${tile.name} ${tile.w} ${tile.h} ${tile.w}x${tile.h}`.toLowerCase().includes(q)) }];
    }
    const cat = $formatCategories.find((c) => c.id === category);
    const groups = cat ? [{ title: labelOf(cat, t), tiles: tilesOf(labelOf(cat, t), cat.formats, cat.id) }] : [];
    if (category === "recent" || recentTiles.length > 0) groups.push({ title: t("Récents"), tiles: recentTiles });
    return category === "recent" ? groups.filter((g) => g.title === t("Récents")) : groups;
  });
  const tileBox = (w: number, h: number) => {
    const k = Math.min(58 / w, 40 / h);
    return `width:${Math.max(6, w * k)}px;height:${Math.max(6, h * k)}px`;
  };

  // Mini-carte : la planche entière et la zone visible.
  const mini = $derived.by(() => {
    const b = bounds;
    const k = Math.min(140 / Math.max(1, b.w), 84 / Math.max(1, b.h));
    return { k, b };
  });
  let stageSize = $state({ w: 800, h: 600 });
  $effect(() => {
    if (!stage) return;
    const observer = new ResizeObserver(() => stage && (stageSize = { w: stage.clientWidth, h: stage.clientHeight }));
    observer.observe(stage);
    return () => observer.disconnect();
  });
</script>

<svelte:window onkeydown={onKey} />

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close())}>
  <header><h2>{t("Plans de travail")}</h2></header>
  <div class="body">
    <div class="stage" bind:this={stage} role="application" aria-label={t("Planche")} onpointerdown={stageDown} onwheel={onWheel}>
      {#each drafts as d (d.key)}
        {@const s = toScreen(d)}
        <div
          class="board"
          class:pending={d.pending}
          class:sel={selectedKey === d.key}
          class:none={d.background === null}
          role="button"
          tabindex="-1"
          style:left="{s.left}px"
          style:top="{s.top}px"
          style:width="{s.width}px"
          style:height="{s.height}px"
          style:background-color={d.pending ? "transparent" : d.background ? `rgb(${d.background.join(",")})` : ""}
          style:background-image={!d.pending && previewUrl && d.origin ? `url(${previewUrl})` : ""}
          style:background-size="{docW * view.zoom}px {docH * view.zoom}px"
          style:background-position="{-(d.origin?.x ?? 0) * view.zoom}px {-(d.origin?.y ?? 0) * view.zoom}px"
          onpointerdown={(event) => boardDown(event, d)}
        >
          {#if d.pending}<span class="ghost-label">{d.name}<em>{d.w} × {d.h}</em></span>{/if}
        </div>
        <span class="label" class:sel={selectedKey === d.key} style:left="{s.left}px" style:top="{s.top - 18}px" style:max-width="{Math.max(24, s.width)}px" title="{d.name} · {d.w} × {d.h}">{d.name}<em> · {d.w} × {d.h}</em></span>
      {/each}
      {#if hover && (drag?.kind === "format" || !hover.ok)}
        {@const s = toScreen(hover)}
        <div class="drop" class:bad={!hover.ok} style:left="{s.left}px" style:top="{s.top}px" style:width="{s.width}px" style:height="{s.height}px">{#if !hover.ok}⊘{/if}</div>
      {/if}
      {#if guides.x !== null}<div class="guide v" style:left="{view.x + guides.x * view.zoom}px"></div>{/if}
      {#if guides.y !== null}<div class="guide h" style:top="{view.y + guides.y * view.zoom}px"></div>{/if}

      <div class="tools" role="toolbar" tabindex="-1" onpointerdown={(event) => event.stopPropagation()}>
        <button type="button" onclick={() => zoomBy(1 / 1.25)}>−</button>
        <output>{Math.round(view.zoom * 100)} %</output>
        <button type="button" onclick={() => zoomBy(1.25)}>+</button>
        <button type="button" class="text" onclick={fit}>{t("Ajuster")}</button>
      </div>
      <svg class="mini" width="150" height="94" role="presentation" onpointerdown={(event) => event.stopPropagation()}>
        {#each drafts as d (d.key)}
          <rect x={5 + (d.x - mini.b.x) * mini.k} y={5 + (d.y - mini.b.y) * mini.k} width={Math.max(1, d.w * mini.k)} height={Math.max(1, d.h * mini.k)} class:pending={d.pending} class:sel={selectedKey === d.key} />
        {/each}
        <rect class="viewport" x={5 + (-view.x / view.zoom - mini.b.x) * mini.k} y={5 + (-view.y / view.zoom - mini.b.y) * mini.k} width={(stageSize.w / view.zoom) * mini.k} height={(stageSize.h / view.zoom) * mini.k} />
      </svg>
    </div>

    <nav class="cats" aria-label={t("Catégories de formats")}>
      <input class="search" type="search" placeholder={t("Rechercher…")} bind:value={search} />
      <button type="button" class:on={category === "recent" && !search} onclick={() => ((category = "recent"), (search = ""))}>
        <svg width="18" height="18" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4a8 8 0 1 1-7.4 5M4 4v5h5M12 8v4l3 2" fill="none" stroke="currentColor" stroke-width="1.6" /></svg>
        <span>{t("Récents")}</span><em>{recentTiles.length}</em>
      </button>
      {#each $formatCategories as c, index (c.id)}
        {#if index > 0 && c.group !== $formatCategories[index - 1]?.group}<span class="rule"></span>{/if}
        <button type="button" class:on={category === c.id && !search} onclick={() => ((category = c.id), (search = ""))}>
          <CategoryIcon id={c.id} />
          <span>{labelOf(c, t)}</span><em>{c.formats.length}</em>
        </button>
      {/each}
    </nav>

    <div class="formats">
      {#each shown as group (group.title)}
        <h3>{group.title}<em>{group.tiles.length}</em></h3>
        <div class="grid">
          {#each group.tiles as tile (tile.key)}
            <button type="button" class="tile" class:picked={drafts.some((d) => d.pending && d.name === tile.name)} title={t("Clic : aperçu · double-clic : créer le plan · glisser : sur la planche")} onpointerdown={(event) => formatDown(event, { w: tile.w, h: tile.h, name: tile.name })}>
              <i style={tileBox(tile.w, tile.h)}></i>
              <b>{tile.label}</b>
              {#if tile.label !== `${tile.w} × ${tile.h}`}<em>{tile.w} × {tile.h}</em>{/if}
            </button>
          {/each}
        </div>
      {/each}
    </div>

    <aside>
      {#if selected}
        {@const d = selected}
        <div class="row">
          <span>{t("Taille")}</span>
          <input type="number" min="1" max="30000" value={d.w} onchange={(e) => resize({ w: e.currentTarget.valueAsNumber })} aria-label={t("Largeur")} />
          <span class="x">×</span>
          <input type="number" min="1" max="30000" value={d.h} onchange={(e) => resize({ w: d.w, h: e.currentTarget.valueAsNumber })} aria-label={t("Hauteur")} />
          <button type="button" class="swap" title={t("Intervertir largeur et hauteur")} onclick={() => resize({ w: d.h, h: d.w })}>⇄</button>
        </div>
        <div class="row">
          <span>{t("Fond")}</span>
          <button type="button" class="sw" class:on={d.background?.join() === "255,255,255"} style:background="#fff" title={t("Blanc")} aria-label={t("Blanc")} onclick={() => (d.background = [255, 255, 255])}></button>
          <button type="button" class="sw" class:on={d.background?.join() === "0,0,0"} style:background="#000" title={t("Noir")} aria-label={t("Noir")} onclick={() => (d.background = [0, 0, 0])}></button>
          <button type="button" class="sw checker" class:on={d.background === null} title={t("Sans fond")} aria-label={t("Sans fond")} onclick={() => (d.background = null)}></button>
        </div>
        <label class="check"><input type="checkbox" bind:checked={d.clip} /> {t("Découper le contenu")}</label>
      {/if}
      <div class="row">
        <span>{t("Marge")}</span>
        <input type="number" min="0" max="2000" bind:value={margin} />
        <span class="x">px</span>
      </div>
      <h4>{t("Plans du document")}</h4>
      <div class="list">
        {#each drafts as d (d.key)}
          <div class="li" class:on={selectedKey === d.key} class:pending={d.pending} role="button" tabindex="-1" onclick={() => (selectedKey = d.key)} ondblclick={() => (renamingKey = d.key)} onkeydown={() => undefined}>
            {#if renamingKey === d.key}
              <!-- svelte-ignore a11y_autofocus -->
              <input class="rename" type="text" use:focusAndSelect bind:value={d.name} spellcheck="false" aria-label={t("Nom")} onclick={(event) => event.stopPropagation()} onkeydown={(event) => (event.stopPropagation(), (event.key === "Enter" || event.key === "Escape") && (renamingKey = null))} onblur={() => (renamingKey = null)} />
            {:else}
              <span class="n">{d.name}</span>
            {/if}
            <em>{d.w} × {d.h}</em>
            <button type="button" class="ic" title={t("Renommer")} aria-label={t("Renommer")} onclick={(event) => (event.stopPropagation(), (selectedKey = d.key), (renamingKey = d.key))}>
              <svg viewBox="0 0 24 24"><path d="M4 20h4L19 9l-4-4L4 16zM14 6l4 4" /></svg>
            </button>
            <button type="button" class="ic del-ic" title={t("Supprimer")} aria-label={t("Supprimer")} onclick={(event) => (event.stopPropagation(), remove(d.key))}>
              <svg viewBox="0 0 24 24"><path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13" /></svg>
            </button>
          </div>
        {/each}
      </div>
    </aside>
  </div>
  <footer>
    <span>{t("{n} plans", { n: drafts.length })}</span>
    <div class="spacer"></div>
    <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
    <button type="button" class="pill" onclick={() => void applyAll()}>{t("Appliquer")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: calc(100vw - 64px);
    max-width: 1680px;
    height: calc(100vh - 64px);
    max-height: 1040px;
    margin: auto;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
    flex-direction: column;
  }
  dialog[open] {
    display: flex;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header {
    height: 50px;
    display: flex;
    align-items: center;
    padding: 0 20px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: var(--bg-well);
    touch-action: none;
    cursor: grab;
  }
  .board {
    position: absolute;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5);
    background-repeat: no-repeat;
    cursor: move;
  }
  .board.none {
    background-color: transparent;
  }
  .board.none:not(.pending) {
    box-shadow: inset 0 0 0 1px rgb(var(--ink) / 0.15);
  }
  .board.sel {
    outline: 2px solid var(--accent, #e5343e);
  }
  .board.pending {
    box-shadow: none;
    border: 2px dashed #9a9aa2;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .board.pending.sel {
    outline: none;
    border-color: var(--accent, #e5343e);
  }
  .ghost-label {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    color: #d0d0d6;
    font-size: 12px;
    text-align: center;
    pointer-events: none;
  }
  .ghost-label em {
    font-style: normal;
    color: #8d8d95;
    font-size: 11px;
  }
  .label {
    position: absolute;
    font-size: 11px;
    /* Lisible sur la planche claire comme sombre (couleurs du thème, pas de gris fixe). */
    color: var(--text-muted);
    white-space: nowrap;
    /* Jamais plus large que son plan : deux plans voisins ne mêlent pas leurs étiquettes. */
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }
  .label.sel {
    color: var(--text-strong);
  }
  .label em {
    font-style: normal;
    color: var(--text-muted);
    opacity: 0.8;
  }
  .drop {
    position: absolute;
    border: 2px solid var(--accent, #e5343e);
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    pointer-events: none;
    display: grid;
    place-items: center;
    color: #f0a0a0;
  }
  .drop.bad {
    border-style: dashed;
    border-color: #8a3035;
    background: repeating-linear-gradient(45deg, color-mix(in srgb, var(--accent) 18%, transparent) 0 6px, transparent 6px 12px);
  }
  .guide {
    position: absolute;
    background: var(--accent, #e5343e);
    pointer-events: none;
  }
  .guide.v {
    top: 0;
    bottom: 0;
    width: 1px;
  }
  .guide.h {
    left: 0;
    right: 0;
    height: 1px;
  }
  .tools {
    position: absolute;
    left: 14px;
    bottom: 14px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border-strong);
    background: var(--bg-card);
    cursor: default;
  }
  .tools button {
    min-width: 28px;
    height: 26px;
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .tools .text {
    padding: 0 10px;
  }
  .tools output {
    min-width: 48px;
    text-align: center;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .mini {
    position: absolute;
    right: 14px;
    bottom: 14px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--bg-well);
    cursor: default;
  }
  .mini rect {
    fill: #d8d8dc;
  }
  .mini rect.pending {
    fill: none;
    stroke: #9a9aa2;
    stroke-dasharray: 2 2;
  }
  .mini rect.sel {
    fill: var(--accent, #e5343e);
  }
  .mini rect.viewport {
    fill: none;
    stroke: var(--accent, #e5343e);
  }
  .cats {
    width: 200px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 10px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  .search {
    height: 32px;
    margin-bottom: 8px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
  }
  .cats button {
    height: 36px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    color: var(--text-3);
    white-space: nowrap;
    text-align: left;
  }
  .cats button span {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cats button em {
    font-style: normal;
    font-size: 11px;
    color: var(--text-muted);
  }
  .cats button:hover {
    background: var(--bg-hover);
  }
  .cats button.on {
    background: var(--bg-active);
    color: var(--text-strong);
    box-shadow: inset 3px 0 0 var(--accent, #e5343e);
  }
  .rule {
    height: 1px;
    flex-shrink: 0;
    margin: 6px 4px;
    background: var(--border);
  }
  .formats {
    width: 320px;
    flex-shrink: 0;
    padding: 12px 14px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  .formats h3 {
    margin: 4px 0 10px;
    font-size: 13px;
    font-weight: 600;
    color: var(--text-strong);
    display: flex;
    gap: 8px;
  }
  .formats h3 em {
    font-style: normal;
    font-weight: 400;
    color: var(--text-muted);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    margin-bottom: 18px;
  }
  .tile {
    height: 98px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: 3px;
    padding: 6px 4px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-card);
    cursor: grab;
    min-width: 0;
  }
  .tile:hover {
    border-color: var(--border-strong);
  }
  .tile.picked {
    border-color: var(--accent, #e5343e);
    background: var(--accent-shadow);
  }
  .tile i {
    display: block;
    margin: auto 0 3px;
    background: #e8e8ec;
    border-radius: 1px;
  }
  .tile b {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text);
  }
  .tile em {
    font-style: normal;
    font-size: 10.5px;
    color: var(--text-muted);
  }
  aside {
    width: 260px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 14px 16px;
    border-left: 1px solid var(--border);
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .row > span:first-child {
    width: 52px;
    flex-shrink: 0;
    color: var(--text-3);
  }
  .row .x {
    color: var(--text-muted);
  }
  aside input[type="text"],
  aside input[type="number"] {
    height: 28px;
    min-width: 0;
    padding: 0 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    font: inherit;
  }
  aside input[type="number"] {
    width: 74px;
  }
  .swap {
    width: 26px;
    height: 26px;
    color: var(--text-2);
  }
  .sw {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    border: 1px solid #555;
    padding: 0;
  }
  .sw.on {
    outline: 2px solid var(--accent, #e5343e);
    outline-offset: 2px;
  }
  .checker {
    background: conic-gradient(#fff 0 25%, #b5b5bb 0 50%, #fff 0 75%, #b5b5bb 0) 0 0 / 14px 14px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  h4 {
    margin: 6px 0 0;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-3);
  }
  .list {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .li {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    cursor: pointer;
  }
  .li:last-child {
    border-bottom: none;
  }
  .li.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  .li.pending .n {
    color: var(--accent, #e5343e);
  }
  .li .n {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .li em {
    font-style: normal;
    font-size: 11px;
    color: var(--text-muted);
    white-space: nowrap;
  }
  footer {
    height: 60px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 20px;
    border-top: 1px solid var(--border);
  }
  footer span {
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .ghost,
  .pill {
    height: 34px;
    padding: 0 18px;
    border-radius: var(--radius-s);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .li .ic {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    flex-shrink: 0;
    color: var(--text-muted);
  }
  .li .ic:hover {
    color: var(--text-strong);
    background: var(--bg-hover);
  }
  .li .del-ic:hover {
    color: var(--danger);
  }
  .li .ic svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .li .rename {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border: 1px solid var(--accent);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
</style>
