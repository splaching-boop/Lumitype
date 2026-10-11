<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { type ActionId, isActionEnabled, runAction } from "../lib/actions";
  import { edit, editLatest, thumbnailOf, waitForFrame } from "../lib/engine";
  import { type PreviewControls, beginOpacityPreview, setDraftLevel } from "../lib/layer-preview";
  import { LAYER_LABELS, type LayerInfo, type LayerLabel, type Locks } from "../lib/ipc";
  import { app } from "../state/app.svelte";
  import { ACTION_LABELS } from "../lib/action-catalog";
  import { keymap } from "../state/keymap.svelte";
  import BlendSelect from "./BlendSelect.svelte";
  import ContextMenu, { type MenuEntry } from "./ContextMenu.svelte";
  import Panel from "./Panel.svelte";

  type DropZone = "above" | "below" | "into";

  let altHeld = $state(false);
  let renaming = $state<{ id: number; value: string } | null>(null);
  let drop = $state<{ id: number; zone: DropZone } | null>(null);
  let menu = $state<{ x: number; y: number; layer: LayerInfo } | null>(null);

  const doc = $derived(app.active);
  const layer = $derived(app.activeLayer);

  /** Calques affichés : ceux dont aucun ancêtre n'est replié. */
  const rows = $derived.by(() => {
    const visible: LayerInfo[] = [];
    let hiddenBelowDepth = Number.POSITIVE_INFINITY;
    for (const item of doc?.info.layers ?? []) {
      if (item.depth > hiddenBelowDepth) continue;
      hiddenBelowDepth = item.isGroup && !item.expanded ? item.depth : Number.POSITIVE_INFINITY;
      visible.push(item);
    }
    return visible;
  });

  /** Couleur d'étiquette effective de chaque calque : la sienne, sinon celle du dossier le plus proche qui en a une. */
  const tints = $derived.by(() => {
    const byId = new Map<number, LayerInfo>((doc?.info.layers ?? []).map((l) => [l.id, l]));
    const out = new Map<number, string>();
    const colorOf = (label: LayerLabel | null) => LAYER_LABELS.find((entry) => entry.label === label)?.color ?? null;
    for (const item of doc?.info.layers ?? []) {
      let node: LayerInfo | undefined = item;
      let color: string | null = null;
      // Les ids déjà parcourus arrêtent la remontée : une parenté incohérente (cycle) ne peut pas boucler sans fin.
      const seen = new Set<number>();
      while (node && color === null && !seen.has(node.id)) {
        seen.add(node.id);
        color = colorOf(node.label);
        node = node.parent === null ? undefined : byId.get(node.parent);
      }
      if (color) out.set(item.id, color);
    }
    return out;
  });

  // ---------------------------------------------------------------- sélection

  /**
   * Un clic dans le panneau y laisse le focus clavier : Suppr supprime alors le calque et non les pixels
   * sélectionnés (voir `edit.deleteKey`). Sous WebKit, un bouton cliqué ne prend pas le focus de lui-même.
   * Un champ de saisie en cours ailleurs (opacité du pinceau, texte…) n'est pas dérangé.
   */
  function keepFocus(event: MouseEvent): void {
    // Seul le bouton principal déplace le focus : un clic droit (menu contextuel) n'y touche pas.
    if (event.button !== 0) return;
    const panel = (event.currentTarget as Element | null)?.closest<HTMLElement>("[data-panel='layers']");
    const active = document.activeElement;
    if (!panel || (active && panel.contains(active))) return;
    const typing = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement || (active instanceof HTMLElement && active.isContentEditable);
    if (typing) return;
    panel.focus({ preventScroll: true });
  }

  function select(event: MouseEvent, target: LayerInfo, part: "pixels" | "mask"): void {
    keepFocus(event);
    if (!doc) return;
    // Ctrl + clic sur l'autre vignette du calque actif (pixels et masque) : les deux sont visés ensemble,
    // et l'outil Déplacement les déplace ensemble. Un nouveau Ctrl + clic retire cette vignette.
    if (event.ctrlKey && target.hasMask && layer?.id === target.id && doc.selection.length === 1) {
      const other = part === "mask" ? "pixels" : "mask";
      doc.target = doc.target === other ? "both" : doc.target === "both" ? other : part;
      return;
    }
    if (event.shiftKey && !event.ctrlKey && layer) {
      // Maj + clic : toute la plage entre le calque actif et celui-ci (comme dans Photoshop).
      const from = rows.findIndex((r) => r.id === layer.id);
      const to = rows.findIndex((r) => r.id === target.id);
      const range = rows.slice(Math.min(from, to), Math.max(from, to) + 1).map((r) => r.id);
      doc.selection = [...range.filter((id) => id !== target.id), target.id];
    } else if (event.ctrlKey) {
      // Ctrl + clic : ce calque rejoint la sélection, ou la quitte s'il y était.
      // Tous les calques sélectionnés partagent le même cadre de transformation.
      const without = doc.selection.filter((id) => id !== target.id);
      doc.selection = without.length === doc.selection.length ? [...doc.selection, target.id] : without;
      if (doc.selection.length === 0) doc.selection = [target.id];
    } else {
      doc.selection = [target.id];
    }
    doc.target = part === "mask" && target.hasMask ? "mask" : "pixels";
  }

  /** Clic sur la miniature du masque : le sélectionne. Maj+clic : l'active ou le désactive. */
  function onMaskClick(event: MouseEvent, target: LayerInfo): void {
    if (event.shiftKey) void edit({ op: "setMaskEnabled", id: target.id, enabled: !target.maskEnabled });
    else select(event, target, "mask");
  }

  // ------------------------------------------------------------- propriétés

  const selectedIds = () => doc?.selection ?? [];

  function setLock(name: keyof Locks): void {
    if (!layer) return;
    void edit({ op: "setProps", ids: selectedIds(), patch: { locks: { ...layer.locks, [name]: !layer.locks[name] } } });
  }

  function setPercent(key: "opacity" | "fill", event: Event & { currentTarget: HTMLInputElement }): void {
    const value = event.currentTarget.valueAsNumber;
    if (Number.isFinite(value)) editLatest(key, { op: "setProps", ids: selectedIds(), patch: { [key]: value / 100 } });
  }

  // Curseur d'opacité : tant qu'on le glisse, l'opacité est appliquée par la carte graphique sur
  // un rendu du calque à 100 %. Le mouvement est continu, et le moteur ne recompose qu'une fois,
  // au relâchement. Si l'image ne s'y prête pas (modes de fusion, écrêtage…), on retombe sur
  // la recomposition par le moteur à chaque pas.
  let opacityPreview: Promise<PreviewControls | null> | null = null;
  let draggedOpacity = $state<number | null>(null);

  function startOpacityDrag(): void {
    const single = doc && layer && doc.selection.length === 1;
    opacityPreview = single ? beginOpacityPreview(doc.info.id, layer.id, layer.isGroup) : Promise.resolve(null);
  }

  function dragOpacity(event: Event & { currentTarget: HTMLInputElement }): void {
    const value = event.currentTarget.valueAsNumber;
    draggedOpacity = value;
    void (opacityPreview ?? Promise.resolve(null)).then((controls) => {
      if (controls) return controls.setAlpha(value / 100);
      // Pile non décomposable (réglage, mode de fusion, écrêtage au-dessus) : le moteur recompose à
      // chaque pas. Pour que le curseur reste fluide, l'image passe un cran plus grossière le temps du geste.
      setDraftLevel(1);
      editLatest("opacity", { op: "setProps", ids: selectedIds(), patch: { opacity: value / 100 } });
    });
  }

  function endOpacityDrag(event: Event & { currentTarget: HTMLInputElement }): void {
    const value = event.currentTarget.valueAsNumber;
    const pending = opacityPreview;
    opacityPreview = null;
    void (pending ?? Promise.resolve(null)).then(async (controls) => {
      if (controls) {
        await edit({ op: "setProps", ids: selectedIds(), patch: { opacity: value / 100 } });
        await waitForFrame(); // l'aperçu ne disparaît qu'une fois le vrai rendu à l'écran
        controls.end();
      } else {
        // Retour au rendu fin, une fois la dernière valeur composée par le moteur.
        await edit({ op: "setProps", ids: selectedIds(), patch: { opacity: value / 100 } });
        setDraftLevel(0);
      }
      draggedOpacity = null;
    });
  }

  function commitRename(): void {
    const current = renaming;
    renaming = null;
    const name = current?.value.trim();
    const original = doc?.info.layers.find((l) => l.id === current?.id)?.name;
    if (current && name && name !== original) void edit({ op: "setProps", ids: [current.id], patch: { name } });
  }

  function focusAndSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  const LOCKS: { name: keyof Locks; label: string; icon: IconName }[] = [
    { name: "transparency", label: t("Verrouiller les pixels transparents"), icon: "lockTransparency" },
    { name: "pixels", label: t("Verrouiller les pixels de l'image"), icon: "lockPixels" },
    { name: "position", label: t("Verrouiller la position"), icon: "lockPosition" },
    { name: "all", label: t("Tout verrouiller"), icon: "lockAll" },
  ];

  // --------------------------------------------------------------- écrêtage

  /**
   * Alt + clic sur la ligne entre deux calques (comme Photoshop) : celui du dessus s'écrête sur celui du dessous.
   * Le geste se lit sur le clic lui-même (Alt enfoncé à cet instant, à 5 px du bord d'une ligne), sans dépendre de la
   * ligne bleue : celle-ci ne s'affiche qu'après une frappe ou un mouvement avec Alt, dont l'état peut manquer selon
   * le moteur d'affichage (Chromium sous X11 : Alt pressé après avoir posé la souris, par exemple).
   */
  function onRowPointerDown(event: PointerEvent, row: LayerInfo): void {
    if (event.button !== 0 || !(event.altKey || event.getModifierState("Alt"))) return;
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const y = event.clientY - rect.top;
    const at = rows.indexOf(row);
    // Bas d'une ligne : elle s'écrête sur la suivante ; haut d'une ligne : c'est la précédente qui s'écrête sur elle.
    const target = y >= rect.height - 4 ? row : y <= 5 ? rows[at - 1] : undefined;
    if (!target || target.index === 0) return;
    event.preventDefault();
    event.stopPropagation();
    void edit({ op: "setProps", ids: [target.id], patch: { clipped: !target.clipped } });
  }

  // ---------------------------------------------------------- glisser-déposer
  //
  // Glisser mené par l'application (voir lib/layer-drag.svelte.ts), sans le glisser natif du HTML qui bloquait la
  // fenêtre sous Linux. Le geste lui-même (seuil, capture, fantôme, défilement, onglets) vit dans ce module ; le
  // panneau garde ses règles : où tombe le calque, ce qui est refusé, et l'opération envoyée au dépôt.

  /** Zones d'une ligne qui gardent leur propre clic et ne lancent pas de glisser. */
  const NO_DRAG = ".eye, .chevron, .boundary, input, select, textarea";

  function onRowDragPointerDown(event: PointerEvent, row: LayerInfo): void {
    if (event.button !== 0 || !doc || renaming?.id === row.id) return;
    const from = event.target instanceof Element ? event.target : null;
    if (!from || from.closest(NO_DRAG)) return;
    const docId = doc.info.id;
    // La vignette du masque glisse le masque seul : déposé sur un autre calque, il y est déplacé (Alt : copié).
    const mask = row.hasMask && from.closest(".thumb.mask") !== null;
    armLayerDrag(
      event,
      () => {
        if (!doc || doc.info.id !== docId) return null;
        if (mask) return { kind: "mask", docId, ids: [row.id], label: t("Masque de fusion") };
        // Comme avant : glisser un calque hors de la sélection le sélectionne seul ; sinon toute la sélection suit.
        if (!doc.selection.includes(row.id)) doc.selection = [row.id];
        const ids = [...doc.selection];
        const label = ids.length === 1 ? t(doc.info.layers.find((l) => l.id === ids[0])?.name ?? row.name) : t("{n} calques", { n: ids.length });
        return { kind: "layers", docId, ids, label };
      },
      { hover: dragHover, drop: dragDrop, clear: () => (drop = null), list: () => list ?? null },
    );
  }

  /**
   * Vrai si `target` est l'un des calques déplacés ou se trouve à l'intérieur de l'un d'eux. Un groupe ne
   * peut pas être rangé dans lui-même ni dans un de ses descendants : la cible est alors refusée (aucun
   * liseré, aucun dépôt). La remontée des parents s'arrête sur un id déjà vu (garde contre les cycles).
   */
  function insideMoved(target: LayerInfo, moved: readonly number[]): boolean {
    if (moved.length === 0) return false;
    const ids = new Set(moved);
    const byId = new Map<number, LayerInfo>((doc?.info.layers ?? []).map((l) => [l.id, l]));
    const seen = new Set<number>();
    let node: LayerInfo | undefined = target;
    while (node && !seen.has(node.id)) {
      if (ids.has(node.id)) return true;
      seen.add(node.id);
      node = node.parent === null ? undefined : byId.get(node.parent);
    }
    return false;
  }

  /** Calques déplacés (et non copiés) par le glisser en cours DANS ce document ; vide sinon. */
  function movedHere(source: DragSource, alt: boolean): readonly number[] {
    if (!doc || alt || source.kind === "mask") return [];
    // Calques venus d'un autre document (copiés, jamais déplacés) : aucune cible n'est interdite.
    if (source.docId !== doc.info.id) return [];
    return source.ids;
  }

  /** Ligne du panneau sous le point (x, y) de l'écran, et le calque qu'elle montre. */
  function rowAt(x: number, y: number): { element: HTMLElement; layer: LayerInfo } | null {
    const element = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-panel='layers'] li[data-layer]");
    const layer = element ? rows.find((r) => r.id === Number(element.dataset.layer)) : undefined;
    return element && layer ? { element, layer } : null;
  }

  /** Survol pendant le glisser : la zone de dépôt (au-dessus, au-dessous, dans le dossier) s'affiche en liseré. */
  function dragHover(x: number, y: number, source: DragSource, alt: boolean): void {
    const hit = rowAt(x, y);
    // Hors des lignes (bas vide de la liste, image, autre panneau), ou cible interdite : pas de liseré, pas de dépôt.
    if (!doc || !hit || insideMoved(hit.layer, movedHere(source, alt))) {
      drop = null;
      return;
    }
    const target = hit.layer;
    const rect = hit.element.getBoundingClientRect();
    const ratio = (y - rect.top) / rect.height;
    const zone: DropZone = target.isGroup
      ? ratio < 0.3
        ? "above"
        : ratio > 0.7
          ? "below"
          : "into"
      : ratio < 0.5
        ? "above"
        : "below";
    drop = { id: target.id, zone };
  }

  function dragDrop(source: DragSource, alt: boolean): void {
    const zone = drop?.zone;
    const target = doc?.info.layers.find((l) => l.id === drop?.id);
    drop = null;
    if (!target) return;
    // Même contrôle qu'au survol : un groupe ne se dépose ni sur lui-même ni dans un de ses descendants.
    if (insideMoved(target, movedHere(source, alt))) return;
    // Masque de fusion glissé sur un autre calque du même document : déplacé, ou copié avec Alt (comme dans Photoshop).
    if (source.kind === "mask") {
      const from = source.ids[0];
      if (doc && source.docId === doc.info.id && from !== undefined && from !== target.id && !target.isGroup) void edit({ op: "copyMask", from, to: target.id, removeSource: !alt });
      return;
    }
    // Calques venus d'un autre document (on a survolé son onglet en glissant) : ils sont copiés ici, à l'endroit du dépôt.
    if (doc && zone && source.docId !== doc.info.id) {
      const at = zone === "into" ? { parent: target.id, index: target.childCount } : { parent: target.parent, index: target.index + (zone === "above" ? 1 : 0) };
      void pasteFromOtherDocument({ docId: source.docId, ids: source.ids }, doc.info.id, at);
      return;
    }
    // Alt + glisser, comme dans Photoshop : les calques (ou dossiers) sont dupliqués à l'endroit du dépôt.
    const duplicate = alt;
    if (!doc || !zone || (!duplicate && doc.selection.includes(target.id))) return;
    // Les indices du moteur vont du bas vers le haut : « au-dessus » = index + 1.
    const destination =
      zone === "into"
        ? { parent: target.id, index: target.childCount }
        : { parent: target.parent, index: target.index + (zone === "above" ? 1 : 0) };
    void edit(duplicate ? { op: "duplicateTo", ids: [...doc.selection], ...destination } : { op: "move", ids: doc.selection, ...destination });
  }

  // --------------------------------------------------------- menu contextuel

  function openMenu(event: MouseEvent, target: LayerInfo): void {
    event.preventDefault();
    if (doc && !doc.selection.includes(target.id)) doc.selection = [target.id];
    menu = { x: event.clientX, y: event.clientY, layer: target };
  }

  const action = (label: string, id: ActionId): MenuEntry => ({
    label,
    shortcut: keymap.label(id),
    disabled: !isActionEnabled(id),
    run: () => runAction(id),
  });

  function menuEntries(target: LayerInfo): MenuEntry[] {
    const mask: MenuEntry[] = target.hasMask
      ? [
          {
            label: target.maskEnabled ? t("Désactiver le masque de fusion") : t("Activer le masque de fusion"),
            shortcut: "Maj+clic",
            run: () => void edit({ op: "setMaskEnabled", id: target.id, enabled: !target.maskEnabled }),
          },
          { label: t("Supprimer le masque de fusion"), run: () => void edit({ op: "removeMask", id: target.id }) },
        ]
      : [
          {
            // Avec une sélection active, le masque la reprend : le reste du calque est caché sans être détruit.
            label: doc?.info.hasSelection ? t("Masque de fusion depuis la sélection") : t("Ajouter un masque de fusion"),
            run: () => void edit({ op: "addMask", id: target.id, revealAll: true }),
          },
          {
            label: t("Ajouter un masque noir (tout masquer)"),
            run: () => void edit({ op: "addMask", id: target.id, revealAll: false }),
          },
        ];
    return [
      action(t("Dupliquer le calque"), "layer.duplicate"),
      action(t("Supprimer le calque"), "layer.delete"),
      ...(doc && doc.selection.length >= 2 && doc.selection.includes(target.id)
        ? [{ label: t("Fusionner les {n} calques", { n: doc.selection.length }), shortcut: keymap.label("layer.mergeGroup"), run: () => void edit({ op: "mergeLayers", ids: [...doc!.selection] }) } satisfies MenuEntry]
        : target.isGroup
          ? [{ label: t("Fusionner le groupe"), shortcut: keymap.label("layer.mergeGroup"), run: () => void edit({ op: "mergeGroup", id: target.id }) } satisfies MenuEntry]
          : []),
      ...(!target.isGroup && !target.adjustment ? [{ label: t("Effets de calque (fx)…"), run: () => void (app.effectsLayer = target.id) } satisfies MenuEntry] : []),
      "separator",
      ...smartEntries(target),
      ...(target.text || target.shape || target.smart
        ? [{ label: t("Pixelliser le calque"), run: () => void edit({ op: "rasterize", id: target.id }) } satisfies MenuEntry]
        : []),
      "separator",
      ...mask,
      {
        label: target.clipped ? t("Annuler le masque d'écrêtage") : t("Créer un masque d'écrêtage"),
        shortcut: keymap.label("layer.toggleClipping"),
        // Le calque le plus bas de sa liste n'a rien sous lui sur quoi s'écrêter.
        disabled: !target.clipped && target.index === 0,
        run: () => void edit({ op: "setProps", ids: [target.id], patch: { clipped: !target.clipped } }),
      },
      "separator",
      action(t("Grouper les calques"), "layer.group"),
      action(t("Importer une image comme calque…"), "file.importLayer"),
      "separator",
      {
        label: t("Couleur"),
        colors: [{ value: null, color: null, name: t("Aucune") }, ...LAYER_LABELS.map((entry) => ({ value: entry.label, color: entry.color, name: entry.name }))],
        current: target.label,
        // La couleur s'applique à tous les calques sélectionnés si le calque visé en fait partie.
        choose: (value) => void edit({ op: "setLabel", ids: doc && doc.selection.includes(target.id) ? [...doc.selection] : [target.id], label: value as LayerLabel | null }),
      },
    ];
  }

  /**
   * Section « objets dynamiques » du menu : Convertir pour tout calque (les calques sélectionnés, si le calque visé en
   * fait partie, deviennent ensemble le contenu) ; le reste seulement sur un objet dynamique.
   */
  function smartEntries(target: LayerInfo): MenuEntry[] {
    const ids = doc && doc.selection.includes(target.id) ? [...doc.selection] : [target.id];
    const convert: MenuEntry = { label: t("Convertir en objet dynamique"), run: () => void edit({ op: "convertToSmart", ids }) };
    const smart = target.smart;
    if (!smart) return [convert];
    return [
      convert,
      { label: t("Modifier le contenu"), shortcut: t("double-clic"), run: () => void openSmartContent(target.id) },
      { label: t("Remplacer le contenu…"), run: () => void replaceSmartContent(target.id) },
      { label: t("Nouvel objet dynamique par copie"), run: () => void smartEdit("newSmartByCopy", target.id) },
      { label: t("Exporter le contenu…"), run: () => void exportSmartContent(target.id) },
      ...(smart.linked
        ? [
            { label: t("Incorporer le fichier lié"), disabled: smart.missing, run: () => void smartEdit("embedSmartLink", target.id) } satisfies MenuEntry,
            { label: t("Relier…"), run: () => void relinkSmart(target.id) } satisfies MenuEntry,
          ]
        : []),
      { label: t("Réinitialiser la transformation"), run: () => void smartEdit("resetSmartTransform", target.id) },
      "separator",
    ];
  }

  /** Libellé discret à droite du nom d'un objet dynamique, et son infobulle. */
  function smartKind(row: LayerInfo): { text: string; title: string; missing: boolean } | null {
    const smart = row.smart;
    if (!smart) return null;
    const shared = smart.instances > 1 ? ` ×${smart.instances}` : "";
    const sharedTitle = smart.instances > 1 ? ` · ${t("Partagé par {n} calques", { n: smart.instances })}` : "";
    if (smart.missing) return { text: t("introuvable"), title: t("Fichier lié introuvable") + (smart.path ? ` : ${smart.path}` : ""), missing: true };
    if (smart.linked) return { text: t("lié") + shared, title: t("Objet dynamique lié") + (smart.path ? ` : ${smart.path}` : "") + sharedTitle, missing: false };
    return { text: t("objet") + shared, title: t("Objet dynamique — double-clic sur la vignette pour modifier le contenu") + sharedTitle, missing: false };
  }

  // --------------------------------------------------------------- miniatures

  /** Dessine la miniature dans le canevas dès qu'elle est disponible ou change. */
  function paint(canvas: HTMLCanvasElement, image: ImageData | undefined) {
    const render = (data: ImageData | undefined) => {
      if (!data) return;
      canvas.width = data.width;
      canvas.height = data.height;
      canvas.getContext("2d")?.putImageData(data, 0, 0);
    };
    render(image);
    return { update: render };
  }

  const percent = (value: number) => Math.round(value * 100);

  /** Côté de la case d'une vignette (carrée, comme dans Photoshop) ; la vignette y prend la forme du document. */
  /** Case des vignettes : 30 px, au format de l'image (lignes de 34 px, comme le panneau de Photoshop). */
  const THUMB_BOX = 30;
  /** Colonne de l'œil, fixe à gauche quelle que soit la profondeur du calque. */
  const EYE_COLUMN = 34;
  /** Retrait d'un niveau : 22 px jusqu'au cinquième, 12 px au-delà, pour que dix groupes imbriqués restent lisibles. */
  const indent = (depth: number) => Math.min(depth, 5) * 22 + Math.max(0, depth - 5) * 12;
  // La miniature est cadrée sur le document : sa forme est celle du document (portrait, paysage, carré),
  // inscrite dans la case. Un document 3 fois plus haut que large donne une vignette 3 fois plus haute que large.
  const thumbStyle = $derived.by(() => {
    const w = doc?.info.width ?? 1;
    const h = doc?.info.height ?? 1;
    const k = THUMB_BOX / Math.max(w, h);
    return `width:${Math.max(6, Math.round(w * k))}px;height:${Math.max(6, Math.round(h * k))}px`;
  });

  const ADJUSTMENT_ICON: IconName = "adjustment";
  const ADJUSTMENTS: ActionId[] = ["adjust.brightnessContrast", "adjust.hueSaturation", "adjust.levels", "adjust.gradientMap", "adjust.invert", "adjust.negative"];

  /** Bouton « calque de réglage » : menu des réglages disponibles, ouvert au-dessus du bouton. */
  let adjustMenu = $state<{ x: number; y: number } | null>(null);
  function openAdjustmentMenu(event: MouseEvent): void {
    const rect = (event.currentTarget as Element).getBoundingClientRect();
    adjustMenu = { x: rect.left, y: rect.top - 16 - ADJUSTMENTS.length * 30 };
  }
  const adjustmentEntries = (): MenuEntry[] =>
    ADJUSTMENTS.map((action) => ({ label: ACTION_LABELS[action].replace("Réglage ", ""), shortcut: keymap.label(action), run: () => runAction(action) }));

  const FOOTER: { label: string; icon: IconName; text?: string; action?: ActionId; menu?: (event: MouseEvent) => void }[] = [
    { label: t("Lier les calques — à venir"), icon: "link" },
    { label: t("Effets de calque (fx) : ombres, lueurs, contour…"), icon: "fx", text: "fx", action: "layer.effects" },
    {
      label: t("Ajouter un masque de fusion"),
      icon: "mask",
      action: "layer.addMask",
    },
    { label: t("Nouveau calque de réglage"), icon: ADJUSTMENT_ICON, menu: openAdjustmentMenu },
    { label: t("Nouveau groupe"), icon: "group", action: "layer.newGroup" },
    { label: t("Nouveau calque (Ctrl+Maj+N)"), icon: "newLayer", action: "layer.new" },
    { label: t("Supprimer (Suppr)"), icon: "trash", action: "layer.delete" },
  ];
  import { CUSTOM_ICONS, type IconName } from "../lib/icons";
  import Icon from "./Icon.svelte";
  import { exportSmartContent, openSmartContent, pasteFromOtherDocument, relinkSmart, replaceSmartContent, smartEdit } from "../lib/actions";
  import { type DragSource, armLayerDrag, cancelLayerDrag } from "../lib/layer-drag.svelte";

  /** Liste des calques : elle défile d'elle-même quand un glisser approche de ses bords. */
  let list = $state<HTMLUListElement>();
  // Panneau retiré en plein glisser (document fermé, disposition changée) : le geste s'arrête avec lui.
  $effect(() => () => cancelLayerDrag());
</script>

<svelte:window
  onkeydown={(event) => (altHeld = event.altKey)}
  onkeyup={(event) => (altHeld = event.altKey)}
  onpointermovecapture={(event) => (altHeld = event.altKey)}
  onblur={() => (altHeld = false)}
/>

{#snippet icon(name: IconName, size = 15)}
  <!-- Toutes les icônes passent par le catalogue : un fichier ui/src/assets/icons/<nom>.svg ou .png les remplace. -->
  <Icon {name} {size} />
{/snippet}

<Panel title={t("Calques")} grow>
  {#if doc}
    <!-- Focalisable (sans entrer dans l'ordre de tabulation) : un clic sur un calque laisse le focus ici, voir keepFocus. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="layout" data-panel="layers" tabindex="-1">
      <div class="controls">
        <div class="line">
          <BlendSelect value={layer?.blend ?? "normal"} ids={selectedIds()} group={layer?.isGroup ?? false} disabled={!layer} />
          <label>
            {t("Opacité")}
            <input type="number" min="0" max="100" disabled={!layer} value={draggedOpacity ?? percent(layer?.opacity ?? 1)} onchange={(event) => setPercent("opacity", event)} />
          </label>
        </div>
        <input
          class="slider"
          type="range"
          min="0"
          max="100"
          aria-label={t("Opacité")}
          disabled={!layer}
          value={draggedOpacity ?? percent(layer?.opacity ?? 1)}
          onpointerdown={startOpacityDrag}
          oninput={dragOpacity}
          onchange={endOpacityDrag}
        />
        <div class="line">
          <span class="muted">{t("Verrou")}</span>
          {#each LOCKS as lock (lock.name)}
            <button class="tool" class:on={layer?.locks[lock.name]} disabled={!layer} title={t(lock.label)} aria-label={t(lock.label)} aria-pressed={layer?.locks[lock.name] ?? false} onclick={() => setLock(lock.name)}>
              {@render icon(lock.icon, 14)}
            </button>
          {/each}
          <label class="push">
            {t("Fond")}
            <input type="number" min="0" max="100" disabled={!layer} value={percent(layer?.fill ?? 1)} onchange={(event) => setPercent("fill", event)} />
          </label>
        </div>
      </div>

      <ul class:alt={altHeld} bind:this={list}>
        {#each rows as row (row.id)}
          {@const selected = doc.selection.includes(row.id)}
          {@const active = selected && row.id === layer?.id}
          <li
            data-layer={row.id}
            onpointerenter={() => (app.hoverLayer = row.id)}
            onpointerleave={() => app.hoverLayer === row.id && (app.hoverLayer = null)}
            onpointerdowncapture={(event) => onRowPointerDown(event, row)}
            onpointerdown={(event) => onRowDragPointerDown(event, row)}
            class:selected
            class:drop-above={drop?.id === row.id && drop.zone === "above"}
            class:drop-below={drop?.id === row.id && drop.zone === "below"}
            class:drop-into={drop?.id === row.id && drop.zone === "into"}
            style:padding-left="{EYE_COLUMN + 4 + indent(row.depth)}px"
            style:--tint={tints.get(row.id) ?? null}
            class:tinted={tints.has(row.id)}
            oncontextmenu={(event) => openMenu(event, row)}
          >
            <button class="eye" class:off={!row.visible} aria-label={row.visible ? t("Masquer le calque") : t("Afficher le calque")} title={t("Alt + clic : n'afficher que ce calque (refaire pour tout rallumer)")} onclick={(event) => (event.altKey ? edit({ op: "solo", id: row.id }) : edit({ op: "setProps", ids: [row.id], patch: { visible: !row.visible } }))}>
              {@render icon("eye")}
            </button>

            {#if row.isGroup}
              <button class="chevron" class:open={row.expanded} aria-label={row.expanded ? t("Replier le groupe") : t("Déplier le groupe")} onclick={() => edit({ op: "setExpanded", id: row.id, expanded: !row.expanded })}>
                {@render icon("chevron", 12)}
              </button>
            {/if}
            {#if row.clipped}
              <span class="clip" title={t("Écrêté sur le calque du dessous")}>{@render icon("clip", 14)}</span>
            {/if}

            <button
              class="thumb"
              class:group={row.isGroup || row.text !== null || row.shape !== null || row.adjustment !== null}
              class:target={active && (doc.target === "pixels" || doc.target === "both")}
              aria-label={row.smart ? t("Objet dynamique — double-clic : modifier le contenu") : t("Pixels du calque")}
              title={row.smart ? (row.smart.missing ? t("Fichier lié introuvable") : t("Double-clic : modifier le contenu")) : undefined}
              onclick={(event) => select(event, row, "pixels")}
              ondblclick={() => row.smart && void openSmartContent(row.id)}
            >
              {#if row.isGroup}
                {@render icon("group")}
              {:else if row.text}
                {@render icon("textLayer")}
              {:else if row.shape}
                {@render icon(row.shape.kind === "rect" ? "shapeRect" : row.shape.kind === "ellipse" ? "shapeEllipse" : "shapePolygon")}
              {:else if row.adjustment}
                {@render icon(ADJUSTMENT_ICON)}
              {:else}
                {#if row.smart}
                  <span class="smart-box">
                    <canvas style={thumbStyle} use:paint={thumbnailOf(doc.info.id, row.id, false)}></canvas>
                    <!-- Pastille de l'objet dynamique : carré = incorporé, chaîne = lié, rouge = fichier lié introuvable. -->
                    <span class="smart-badge" class:missing={row.smart.missing}>
                      {#if row.smart.linked}
                        <svg viewBox="0 0 10 10" width="9" height="9" aria-hidden="true"><path d="M4.2 5.8 5.8 4.2M3.6 4.6 2.4 5.8a1.5 1.5 0 0 0 2.1 2.1l1.2-1.2M6.4 5.4l1.2-1.2a1.5 1.5 0 0 0-2.1-2.1L4.3 3.3" fill="none" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" /></svg>
                      {:else}
                        <svg viewBox="0 0 10 10" width="9" height="9" aria-hidden="true"><rect x="1.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.1" /><rect x="3.6" y="3.6" width="2.8" height="2.8" fill="currentColor" /></svg>
                      {/if}
                    </span>
                  </span>
                {:else}
                  <canvas style={thumbStyle} use:paint={thumbnailOf(doc.info.id, row.id, false)}></canvas>
                {/if}
              {/if}
            </button>

            {#if row.hasMask}
              <!-- Glissée (voir onRowDragPointerDown), la vignette emporte le masque seul vers un autre calque. -->
              <button
                class="thumb mask" class:target={active && (doc.target === "mask" || doc.target === "both")} class:disabled={!row.maskEnabled} aria-label={t("Masque de fusion")} title={t("Clic : viser le masque (peinture, déplacement) · Ctrl+clic : viser le masque ET les pixels, pour les déplacer ensemble · Maj+clic : activer ou désactiver · glisser vers un autre calque : déplacer le masque (Alt : le copier)")} onclick={(event) => onMaskClick(event, row)}>
                <canvas style={thumbStyle} use:paint={thumbnailOf(doc.info.id, row.id, true)}></canvas>
              </button>
            {/if}

            {#if renaming?.id === row.id}
              <input
                class="rename"
                bind:value={renaming.value}
                use:focusAndSelect
                onblur={commitRename}
                onkeydown={(event) => {
                  event.stopPropagation();
                  if (event.key === "Enter") commitRename();
                  if (event.key === "Escape") renaming = null;
                }}
              />
            {:else}
              <button class="name" onclick={(event) => select(event, row, "pixels")} ondblclick={() => (renaming = { id: row.id, value: row.name })}>
                {t(row.name)}
                {#if row.effects.length > 0}
                  <!-- Le calque porte des effets : double-clic pour les rouvrir, Alt + clic pour les couper ou les rétablir tous. -->
                  <span
                    class="fx badge"
                    class:muted={!row.effects.some((e) => e.enabled)}
                    role="presentation"
                    title={t("Double-clic : régler les effets · Alt + clic : les désactiver ou les réactiver")}
                    onclick={(event) => {
                      if (!event.altKey) return;
                      event.stopPropagation();
                      const on = !row.effects.some((e) => e.enabled);
                      void edit({ op: "setEffects", id: row.id, effects: $state.snapshot(row.effects).map((e) => ({ ...e, enabled: on })) });
                    }}
                    ondblclick={(event) => (event.stopPropagation(), (app.effectsLayer = row.id))}
                  >
                    {#if CUSTOM_ICONS.fx}<Icon name="fx" size={12} />{:else}{t("fx")}{/if}
                  </span>
                {/if}
              </button>
            {/if}

            {#if row.smart}
              {@const kind = smartKind(row)}
              {#if kind}<span class="smart-kind" class:missing={kind.missing} title={kind.title}>{kind.text}</span>{/if}
            {/if}

            {#if row.locks.all || row.locks.pixels || row.locks.position || row.locks.transparency}
              <span class="locked" title={t("Calque verrouillé")}>{@render icon("lockAll", 13)}</span>
            {/if}

            {#if row.index > 0}
              <!-- Ligne entre deux calques, visible Alt enfoncé : Alt+clic dessus (géré par onRowPointerDown) écrête celui du dessus sur celui du dessous. -->
              <span class="boundary"></span>
            {/if}
          </li>
        {/each}
      </ul>

      <div class="footer">
        {#each FOOTER as item (item.label)}
          <button
            class="tool"
            title={t(item.label)}
            aria-label={t(item.label)}
            disabled={item.menu ? !doc : !item.action || !isActionEnabled(item.action)}
            onclick={(event) => (item.menu ? item.menu(event) : item.action && runAction(item.action))}
          >
            {#if item.text && !CUSTOM_ICONS[item.icon]}<span class="fx">{item.text}</span>{:else}{@render icon(item.icon)}{/if}
          </button>
        {/each}
      </div>
    </div>
  {:else}
    <p class="empty">{t("Aucun document")}</p>
  {/if}
</Panel>

{#if adjustMenu}
  <ContextMenu x={adjustMenu.x} y={adjustMenu.y} entries={adjustmentEntries()} onclose={() => (adjustMenu = null)} />
{/if}
{#if menu}
  <ContextMenu x={menu.x} y={menu.y} entries={menuEntries(menu.layer)} onclose={() => (menu = null)} />
{/if}

<style>
  .layout {
    height: 100%;
    display: flex;
    flex-direction: column;
    outline: none; /* focalisé par programme seulement : pas de liseré */
  }
  .controls {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 12px 8px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .muted,
  label {
    color: var(--text-3);
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: 8px;
  }
  .push {
    margin-left: auto;
  }
  input[type="number"] {
    height: 26px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
  input[type="number"] {
    width: 52px;
    padding: 0 6px;
    text-align: right;
    user-select: text;
  }
  .slider {
    width: 100%;
  }
  .tool {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    color: var(--text-3);
  }
  .tool:disabled {
    color: var(--text-disabled);
  }
  .tool:not(:disabled):hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .tool.on {
    background: var(--bg-active);
    color: #fff;
  }

  ul {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  /* Lignes jointives séparées d'un filet, œil dans une colonne fixe à gauche, contenu seul en retrait. */
  li {
    position: relative;
    height: 34px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    padding-right: 8px;
    border-radius: 0;
    border: 1px solid transparent;
    border-bottom-color: var(--border);
    font-size: 11.5px;
  }
  /* Étiquette de couleur : c'est la cellule de l'œil qui prend la teinte, comme dans Photoshop. */
  li.tinted .eye {
    background: color-mix(in srgb, var(--tint) 55%, #141313);
    color: #fff;
  }
  li:hover {
    background: var(--bg-hover);
  }
  li.selected {
    background: var(--bg-active);
  }
  li.drop-into {
    border-color: var(--accent);
  }
  li.drop-above::before,
  li.drop-below::after {
    content: "";
    position: absolute;
    left: 36px;
    right: 4px;
    height: 2px;
    border-radius: var(--radius-xs);
    background: var(--accent);
  }
  li.drop-above::before {
    top: -2px;
  }
  li.drop-below::after {
    bottom: -2px;
  }
  .eye,
  .chevron {
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  /* Colonne de l'œil : cellule pleine hauteur, neutre ; seule l'étiquette de couleur du calque la teinte. */
  .eye {
    position: absolute;
    left: 0;
    top: 0;
    width: 34px;
    height: 100%;
    border-right: 1px solid var(--border);
  }
  .eye.off {
    color: var(--text-disabled);
  }
  li.tinted .eye.off {
    color: rgba(255, 255, 255, 0.45);
  }
  .chevron {
    width: 14px;
    color: var(--text-muted);
    transition: transform 120ms var(--ease);
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .clip {
    display: grid;
    color: var(--accent);
  }
  /* Case carrée ; la vignette elle-même a la forme du document (angles vifs, comme dans Photoshop). */
  .thumb {
    width: 30px;
    height: 30px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  /* Dossier, texte, forme, réglage : l'icône seule, sans case. */
  .thumb.group {
    width: 22px;
    background: transparent;
    border: 0;
  }
  /* La pastille s'accroche au coin de la vignette elle-même (qui a la forme du document), pas à sa case. */
  .smart-box {
    position: relative;
    display: block;
  }
  /* Pastille carrée en bas à droite de la vignette : l'objet dynamique se reconnaît d'un coup d'œil. */
  .smart-badge {
    position: absolute;
    right: -4px;
    bottom: -4px;
    width: 12px;
    height: 12px;
    display: grid;
    place-items: center;
    background: var(--bg-panel);
    border: 1px solid var(--border-strong);
    color: var(--text);
    pointer-events: none;
  }
  .smart-badge.missing {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .smart-kind {
    flex-shrink: 0;
    color: var(--text-muted);
    font-size: 10.5px;
    white-space: nowrap;
  }
  .smart-kind.missing {
    color: var(--accent);
  }
  .thumb.mask.disabled {
    opacity: 0.35;
  }
  .thumb canvas {
    display: block;
    box-sizing: border-box;
    border: 1px solid var(--border-strong);
    image-rendering: auto;
    /* Damier : la transparence du calque reste lisible dans la miniature. */
    background: repeating-conic-gradient(#bdbdbd 0% 25%, #ffffff 0% 50%) 0 0 / 6px 6px;
  }
  .thumb.mask canvas {
    background: #000;
  }
  .thumb.target canvas,
  .thumb.group.target {
    outline: 1.5px solid #fff;
    outline-offset: 1px;
  }
  .name {
    flex: 1;
    min-width: 0;
    height: 100%;
    overflow: hidden;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rename {
    flex: 1;
    min-width: 0;
    height: 24px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent);
    background: var(--bg-field);
    user-select: text;
  }
  .locked {
    display: grid;
    color: var(--text-muted);
  }
  .boundary {
    position: absolute;
    left: 36px;
    right: 0;
    bottom: -5px;
    z-index: 1;
    height: 9px;
    display: none;
    cursor: s-resize;
  }
  ul.alt .boundary {
    display: block;
  }
  ul.alt .boundary:hover {
    background: linear-gradient(transparent 3px, var(--accent) 3px 5px, transparent 5px);
  }
  .footer {
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    padding: 0 10px;
    border-top: 1px solid var(--border);
  }
  .empty {
    margin: 0;
    padding: 14px;
    color: var(--text-muted);
  }
  .fx {
    font-style: italic;
    font-weight: 600;
    font-size: 12px;
    letter-spacing: 0.02em;
  }
  .fx.badge.muted {
    opacity: 0.4;
    text-decoration: line-through;
  }
  .fx.badge {
    margin-left: 6px;
    padding: 0 5px;
    border-radius: var(--radius-s);
    font-size: 10px;
    color: var(--text-2);
    border: 1px solid var(--border-strong);
  }
</style>
