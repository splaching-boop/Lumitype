import { t } from "./i18n.svelte";
// Actions de l'application. Menus, raccourcis et boutons passent tous par `runAction`.
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { app, type OpenDocument } from "../state/app.svelte";
import { centerCamera, fitCamera, steppedZoom, zoomAt } from "../viewport/camera";
import { BLEND_ORDER } from "./blend";
import { avifPathToPng, avifToPng, isAvif } from "./avif";
import { forgetRecovery } from "./autosave.svelte";
import { recordRecent } from "./recents.svelte";
import { adopt, edit, editLatest, enqueue, refreshThumbnails, settlePendingEdits } from "./engine";
import { IMAGE_EXTENSIONS, IMAGE_MIME_TYPES, LEGACY_PROJECT_EXTENSION, PROJECT_EXTENSION, api, type DevelopValues, type Adjustment, type LayerInfo } from "./ipc";
import { commitTransform, openTransform, positionLocked, sameLayers, transformIsPending } from "./transform-session";
import { chooseProofProfile } from "./proof-profiles";

export type ActionId =
  | "file.new"
  | "file.home"
  | "file.open"
  | "file.importLayer"
  | "file.placeSmart"
  | "file.placeLinked"
  | "file.save"
  | "file.saveAs"
  | "file.exportPng"
  | "file.close"
  | "edit.undo"
  | "edit.redo"
  | "edit.toggleLastState"
  | "edit.fillForeground"
  | "edit.fillBackground"
  | "help.about"
  | "edit.preferences"
  | "copyright.metadata"
  | "layer.effects"
  | "view.guideLayout"
  | "image.invert"
  | "select.quickMask"
  | "tool.crop"
  | "tool.shape"
  | "tool.shapeCycle"
  | "filter.liquify"
  | "edit.copy"
  | "edit.cut"
  | "edit.paste"
  | "edit.freeTransform"
  | "edit.shortcuts"
  | "select.all"
  | "select.none"
  | "select.invert"
  | "select.feather"
  | "layer.new"
  | "layer.newGroup"
  | "layer.group"
  | "layer.mergeGroup"
  | "layer.duplicate"
  | "layer.delete"
  | "edit.deleteKey"
  | "layer.toggleClipping"
  | "layer.addMask"
  | "layer.blendNext"
  | "layer.blendPrevious"
  | "layer.convertToSmart"
  | "layer.editSmartContent"
  | "layer.replaceSmartContent"
  | "layer.newSmartByCopy"
  | "layer.exportSmartContent"
  | "layer.embedSmartLink"
  | "layer.relinkSmart"
  | "layer.resetSmartTransform"
  | "layer.rasterize"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.fit"
  | "view.actualSize"
  | "view.toggleRulers"
  | "view.toggleGuides"
  | "view.toggleSnap"
  | "view.proofCmyk"
  | "view.proofFogra39"
  | "view.proofFogra51"
  | "view.proofFogra52"
  | "view.proofGracol2006"
  | "view.proofSwop2006c3"
  | "view.lockGuides"
  | "doc.next"
  | "doc.previous"
  | "view.clearGuides"
  | "color.swap"
  | "color.reset"
  | "brush.smaller"
  | "brush.larger"
  | "tool.move"
  | "tool.marquee"
  | "tool.marqueeCycle"
  | "tool.lasso"
  | "tool.lassoCycle"
  | "tool.wand"
  | "adjust.brightnessContrast"
  | "adjust.hueSaturation"
  | "adjust.levels"
  | "adjust.invert"
  | "adjust.gradientMap"
  | "adjust.negative"
  | "filter.gaussianBlur"
  | "filter.fieldBlur"
  | "filter.motionBlur"
  | "filter.lensBlur"
  | "filter.sharpen"
  | "filter.noise"
  | "filter.dither"
  | "filter.presence"
  | "filter.glass"
  | "filter.gaussianNoise"
  | "tool.heal"
  | "tool.wandCycle"
  | "select.modify"
  | "filter.colorDither"
  | "filter.chromaticAberration"
  | "filter.ascii"
  | "adjust.develop"
  | "tool.fill"
  | "tool.fillCycle"
  | "tool.pen"
  | "tool.pathSelect"
  | "path.toSelection"
  | "path.fill"
  | "path.stroke"
  | "path.clear"
  | "tool.eyedropper"
  | "tool.text"
  | "tool.brush"
  | "tool.eraser"
  | "tool.hand"
  | "tool.zoom";

/** Exécute une opération longue en tenant l'utilisateur informé ; les erreurs sont affichées, jamais avalées. */
async function withBusy(label: string, work: () => Promise<void>): Promise<void> {
  app.busy = label;
  try {
    await work();
  } catch (error) {
    app.fail(error);
  } finally {
    app.busy = null;
  }
}

const IMAGE_FILTER = { name: "Images", extensions: IMAGE_EXTENSIONS };

async function openDocuments(): Promise<void> {
  const selection = await open({
    multiple: true,
    filters: [
      { name: t("Images et projets"), extensions: [PROJECT_EXTENSION, LEGACY_PROJECT_EXTENSION, ...IMAGE_EXTENSIONS] },
      { name: t("Projets Lumitype"), extensions: [PROJECT_EXTENSION, LEGACY_PROJECT_EXTENSION] },
      IMAGE_FILTER,
    ],
  });
  if (!selection) return;
  await openPaths(selection);
}

/**
 * Calques glissés d'un document vers un autre : copiés (groupes, masques, effets et texte compris) puis
 * collés dans `targetDocId`, à l'emplacement `at` ou, à défaut, tout en haut de la pile.
 */
export async function pasteFromOtherDocument(origin: { docId: number; ids: number[] }, targetDocId: number, at?: { parent: number | null; index: number }): Promise<void> {
  app.layerDrag = null;
  settlePendingEdits(); // une transformation en attente est validée avant le collage
  try {
    // Par la file du document d'origine : la copie voit la transformation qui vient d'être validée.
    await enqueue(origin.docId, () => api.copyLayers(origin.docId, origin.ids));
    app.lastCopy = "layers";
    const top = app.documents.find((d) => d.info.id === targetDocId)?.info.layers.filter((l) => l.parent === null).length ?? 0;
    const result = await enqueue(targetDocId, () => api.pasteLayers(targetDocId, null, at ?? { parent: null, index: top }));
    adopt(result.document, result.dirty);
  } catch (error) {
    app.fail(error);
  }
}

/**
 * Fichiers locaux glissés dans la fenêtre, connus par leur chemin. Sur un document ouvert, chaque image devient un
 * calque (comme un dépôt ordinaire) ; sinon, ou pour un projet, chacun s'ouvre en document.
 */
export async function importLocalPaths(paths: readonly string[], asSmartObject = false): Promise<void> {
  for (const path of paths) {
    const extension = path.split(".").pop()?.toLowerCase() ?? "";
    const isProject = extension === PROJECT_EXTENSION || extension === LEGACY_PROJECT_EXTENSION;
    if (!isProject && !IMAGE_EXTENSIONS.includes(extension)) {
      app.error = `Format non pris en charge : ${path.split(/[\\/]/).pop()}`;
      continue;
    }
    if (isProject || !app.active || app.homeVisible) {
      await openPaths([path]);
      continue;
    }
    if (isAvif(path)) {
      try {
        await importDroppedFiles([await avifPathToPng(path)], asSmartObject);
      } catch (error) {
        app.fail(error);
      }
      continue;
    }
    // Alt au dépôt : objet dynamique incorporé, plutôt qu'un calque de pixels.
    const op = asSmartObject ? ({ op: "placeSmartObject", path, linked: false, above: activeId() } as const) : ({ op: "importLayer", path, above: activeId() } as const);
    await withBusy("Import…", async () => void (await edit(op)));
  }
}

/** Ouvre des fichiers désignés par leur chemin : double-clic dans l'explorateur, au lancement ou pendant que l'application tourne. */
export async function openPaths(paths: readonly string[]): Promise<void> {
  for (const path of paths) {
    if (isAvif(path)) {
      await withBusy("Ouverture…", async () => {
        const file = await avifPathToPng(path);
        const { document } = await api.importDropped(new Uint8Array(await file.arrayBuffer()), file.name, null, null);
        app.add(document);
        refreshThumbnails(document, []);
        recordRecent(path, document);
      });
      continue;
    }
    await withBusy("Ouverture…", async () => {
      const info = await api.openPath(path);
      app.add(info);
      refreshThumbnails(info, []);
      recordRecent(path, info);
    });
  }
  if (paths.length > 0) app.newDocumentDialogOpen = false; // inutile de proposer un nouveau document
}

async function importLayer(): Promise<void> {
  if (!app.active) return;
  // Une ou plusieurs images d'un coup : chacune devient un calque.
  const paths = await open({ multiple: true, filters: [IMAGE_FILTER] });
  if (!paths) return;
  for (const path of paths) {
    if (isAvif(path)) {
      try {
        await importDroppedFiles([await avifPathToPng(path)]);
      } catch (error) {
        app.fail(error);
      }
      continue;
    }
    await withBusy("Import…", async () => void (await edit({ op: "importLayer", path, above: activeId() })));
  }
}

async function saveProject(forcePrompt: boolean): Promise<void> {
  const active = app.active;
  if (!active) return;
  // Contenu d'un objet dynamique (« Modifier le contenu ») : Enregistrer met à jour l'objet dans son document, rien d'autre.
  if (!forcePrompt && active.info.smartParent) return commitSmartContent(active);
  const path =
    (!forcePrompt && active.info.projectPath) ||
    (await save({
      defaultPath: await inSaveFolder(`${active.info.name}.${PROJECT_EXTENSION}`),
      filters: [{ name: t("Projet Lumitype"), extensions: [PROJECT_EXTENSION] }],
    }));
  if (!path) return;
  await withBusy("Enregistrement…", async () => {
    const info = await api.saveProject(active.info.id, path);
    app.update(info);
    // Contenu d'un objet dynamique : « Enregistrer sous » n'en écrit qu'une copie à part, le parent n'en sait rien. Le
    // document reste donc « modifié » tant que l'objet n'a pas été mis à jour (Ctrl+S), et sa fermeture le propose.
    if (!info.smartParent) {
      active.savedRevision = info.revision;
      forgetRecovery(active); // enregistré : la copie de secours ne sert plus
    }
    recordRecent(path, info);
  });
}

/**
 * Nouveau calque de réglage, écrêté par défaut sur le calque sélectionné : il ne corrige que lui
 * (Ctrl+Alt+G le libère pour corriger tout ce qui est dessous). Ses curseurs s'affichent dans Propriétés.
 */
function addAdjustment(adjustment: Adjustment, clipped = true): Promise<boolean> {
  const above = activeId();
  return edit({ op: "addAdjustment", adjustment, above, clipped: clipped && above !== null });
}

/** Calque au-dessus duquel placer un nouveau calque (le calque actif, ou rien : tout en haut). */
export const activeLayerIdForNewLayer = (): number | null => app.activeLayer?.id ?? null;

export const NEUTRAL_DEVELOP: DevelopValues = { temperature: 0, tint: 0, exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, clarity: 0, dehaze: 0, vibrance: 0, saturation: 0, redHue: 0, greenHue: 0, blueHue: 0 };

/**
 * Pousse les pixels copiés vers le presse-papiers du système, en PNG. Sans gravité si la webview
 * le refuse (permission, plateforme) : le copier-coller interne fonctionne toujours.
 */
async function shareClipboard(): Promise<void> {
  app.ownClipboard = true;
  try {
    const png = await api.clipboardPng();
    if (png.byteLength > 0) await navigator.clipboard.write([new ClipboardItem({ "image/png": new Blob([png], { type: "image/png" }) })]);
  } catch {
    // Presse-papiers du système indisponible : rien à faire de plus.
  }
  await markClipboard();
}

/** Relève l'empreinte du presse-papiers du système juste après une copie faite ici (voir `app.clipboardMark`). */
async function markClipboard(): Promise<void> {
  app.clipboardMark = await api.clipboardFingerprint().catch(() => 0);
}

/**
 * Ctrl+V, et Édition > Coller. Le plus récent gagne : si rien n'a été copié ailleurs depuis notre dernière copie,
 * on colle celle-ci (pixels ou calques) ; sinon, l'image du presse-papiers du système (capture d'écran, copie
 * depuis un navigateur…) devient un nouveau calque. `domImages` : images livrées par l'événement de collage de
 * la webview, en dernier recours (sous Linux, elle ne livre pas toujours celles copiées ailleurs).
 */
export async function pasteFromClipboard(domImages: File[] = []): Promise<void> {
  const fingerprint = await api.clipboardFingerprint().catch(() => 0);
  const ours = app.clipboardMark !== null && fingerprint === app.clipboardMark;
  if (!ours) {
    const png = await api.clipboardImage().catch(() => new ArrayBuffer(0));
    const images = png.byteLength > 0 ? [new File([png], "Image collée.png", { type: "image/png" })] : domImages;
    if (images.length > 0) return void (await importDroppedFiles(images));
  }
  if (app.clipboardMark === null && app.lastCopy === "pixels") {
    app.error = t("Le presse-papiers ne contient pas d'image : copiez-en une (ou faites une capture d'écran), puis Ctrl+V.");
    return;
  }
  await pasteInternal();
}

/** Colle la dernière copie faite ici : des pixels, ou des calques entiers. */
async function pasteInternal(): Promise<void> {
  const doc = app.active;
  if (!doc || app.homeVisible) return; // l'accueil ne colle que des images du presse-papiers
  if (app.lastCopy !== "layers") return void (await edit({ op: "paste", above: activeId() }));
  const docId = doc.info.id;
  settlePendingEdits(); // une transformation en attente est validée avant le collage
  try {
    const result = await enqueue(docId, () => api.pasteLayers(docId, activeId()));
    adopt(result.document, result.dirty);
  } catch (error) {
    app.fail(error);
  }
}

/** Image venue d'une adresse (glissée depuis un navigateur) : calque du document actif, ou nouveau document. */
export async function importFromUrl(url: string): Promise<void> {
  const target = app.homeVisible ? null : app.active; // depuis l'accueil : nouveau document
  await withBusy(t("Téléchargement…"), async () => {
    const run = () => api.importUrl(url, target?.info.id ?? null, activeId());
    if (target) {
      settlePendingEdits();
      const result = await enqueue(target.info.id, run);
      adopt(result.document, result.dirty);
    } else {
      const { document } = await run();
      app.add(document);
      refreshThumbnails(document, []);
    }
  });
}

/**
 * Chemin proposé par une boîte d'enregistrement : dans le dossier par défaut des préférences s'il
 * existe encore ; sinon le nom seul, et le système part de son emplacement habituel (dossier
 * renommé, disque débranché : on n'échoue pas, on ne bloque pas).
 */
export async function inSaveFolder(fileName: string): Promise<string> {
  const folder = app.saveFolder.trim();
  if (!folder) return fileName;
  const exists = await api.directoryExists(folder).catch(() => false);
  if (!exists) return fileName;
  const separator = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return `${folder.replace(/[\\/]+$/, "")}${separator}${fileName}`;
}

/** Demande confirmation avant de perdre quelque chose. Boutons Oui / Non. */
export function confirmLoss(message: string): Promise<boolean> {
  return ask(message, { title: "Lumitype", kind: "warning", okLabel: "Oui", cancelLabel: "Non" });
}

/**
 * Avant de fermer un document modifié : propose de l'enregistrer. Retourne vrai si la fermeture
 * peut se poursuivre (enregistré, ou abandon voulu), faux si l'utilisateur annule — y compris en
 * refermant la boîte « Enregistrer sous ».
 */
export async function settleUnsaved(doc: OpenDocument): Promise<boolean> {
  if (!app.isModified(doc)) return true;
  app.activeId = doc.info.id; // on montre le document dont il est question
  // Contenu d'un objet dynamique : « Oui » met à jour l'objet (voir saveProject) ; « Non » laisse le parent intact.
  const choice = await new Promise<"save" | "discard" | "cancel">((resolve) => (app.savePrompt = { name: documentTitle(doc.info), resolve }));
  if (choice === "cancel") return false;
  if (choice === "discard") return true;
  await saveProject(false);
  return !app.isModified(doc);
}

async function closeActive(): Promise<void> {
  const active = app.active;
  if (!active) return;
  if (!(await settleUnsaved(active))) return;
  await withBusy("Fermeture…", async () => {
    await api.closeDocument(active.info.id);
    app.remove(active.info.id);
  });
}

// ---------------------------------------------------------------------- calques

const activeId = () => app.activeLayer?.id ?? null;
const selection = () => app.active?.selection ?? [];

/** Ctrl+Alt+G : écrête le calque actif sur celui du dessous, ou annule l'écrêtage. */
function toggleClipping(): void {
  const layer = app.activeLayer;
  if (layer) void edit({ op: "setProps", ids: [layer.id], patch: { clipped: !layer.clipped } });
}

function addMask(): void {
  const layer = app.activeLayer;
  if (layer && !layer.hasMask) void edit({ op: "addMask", id: layer.id, revealAll: true });
}

function fill(color: "foreground" | "background"): void {
  const doc = app.active;
  const layer = app.activeLayer;
  if (doc && layer) void edit({ op: "fill", id: layer.id, color: app[color], onMask: doc.target === "mask" });
}

/** Ctrl+T : ouvre le cadre de transformation sur le contenu des calques sélectionnés (ou de la sélection). */
export async function beginFreeTransform(): Promise<void> {
  const doc = app.active;
  if (!doc) return;
  if (app.transform) {
    // Le cadre de l'outil Déplacement est déjà là, sur les calques sélectionnés : Ctrl+T le rend simplement modal.
    if (sameLayers(app.transform.ids, doc.selection) && !positionLocked(doc.info, doc.selection)) {
      app.transform.auto = false;
      return;
    }
    // Cadre resté sur d'autres calques (sélection changée entre-temps) : on le ferme — validé s'il a été tiré —
    // et on en ouvre un neuf sur la sélection. Le reprendre transformait l'ancien calque, cadre à l'ancienne place.
    if (transformIsPending()) commitTransform();
    else app.transform = null;
  }
  if (positionLocked(doc.info, doc.selection)) {
    app.error = t("Calque verrouillé : retirez le cadenas pour le transformer.");
    return;
  }
  try {
    const [docId, ids] = [doc.info.id, [...doc.selection]];
    // Par la file du document : l'étendue lue est celle d'après la validation ci-dessus.
    const bounds = await enqueue(docId, () => api.contentBounds(docId, ids));
    if (!bounds) {
      app.error = t("Rien à transformer : le calque est vide.");
      return;
    }
    // Sélection changée pendant la lecture : ce cadre ne la décrirait plus.
    if (app.active !== doc || !sameLayers(ids, doc.selection)) return;
    openTransform(ids, bounds, false);
  } catch (error) {
    app.fail(error);
  }
}

/** Maj+Alt++ / - : parcourt les modes de fusion du calque actif. */
function cycleBlend(direction: 1 | -1): void {
  const layer = app.activeLayer;
  if (!layer) return;
  const current = BLEND_ORDER.indexOf(layer.blend);
  const next = BLEND_ORDER[(Math.max(current, 0) + direction + BLEND_ORDER.length) % BLEND_ORDER.length];
  if (next) void edit({ op: "setProps", ids: [layer.id], patch: { blend: next } });
}

/** Touches numériques, comme Photoshop : 1 = 10 % … 9 = 90 %, 0 = 100 %. Outil de peinture : son opacité ; sinon : celle du calque. */
export function setOpacityFromDigit(digit: number): void {
  const opacity = digit === 0 ? 1 : digit / 10;
  const tool = app.paintTool;
  if (tool) {
    app.brushes[tool].opacity = opacity;
  } else if (app.activeLayer) {
    editLatest("opacity", { op: "setProps", ids: selection(), patch: { opacity } });
  }
}

/** Touches [ et ] : pas croissant avec la taille, comme Photoshop. */
function resizeBrush(direction: 1 | -1): void {
  const step = (size: number) => (size < 10 ? 1 : size < 50 ? 5 : size < 100 ? 10 : size < 200 ? 25 : 50);
  if (app.tool === "heal") {
    app.heal.size = Math.min(1000, Math.max(1, app.heal.size + direction * step(app.heal.size)));
    return;
  }
  if (app.tool === "quickSelect") {
    app.quickSelect.size = Math.min(300, Math.max(4, app.quickSelect.size + direction * step(app.quickSelect.size)));
    return;
  }
  const tool = app.paintTool;
  if (!tool) return;
  const size = app.brushes[tool].size;
  app.brushes[tool].size = Math.min(2500, Math.max(1, size + direction * step(size)));
}

// ------------------------------------------------------------------------- vue

/** Zoom par paliers autour du centre de la zone de travail, comme Ctrl++ dans Photoshop. */
function stepZoom(direction: 1 | -1): void {
  const camera = app.active?.camera;
  if (!camera) return;
  const { width, height } = app.viewport;
  app.moveCamera(zoomAt(camera, steppedZoom(camera.zoom, direction), width / 2, height / 2));
}

function frame(mode: "fit" | "actual"): void {
  const active = app.active;
  if (!active) return;
  const { width, height } = app.viewport;
  const { width: docW, height: docH } = active.info;
  app.moveCamera(mode === "fit" ? fitCamera(docW, docH, width, height) : centerCamera(docW, docH, width, height, 1), mode === "fit");
}

const HANDLERS: Record<ActionId, () => void | Promise<unknown>> = {
  "file.new": () => void (app.newDocumentDialogOpen = true),
  // Accueil : toujours visible sans document ; avec des documents ouverts, cette commande l'affiche ou le referme.
  "file.home": () => void (app.homeOpen = app.documents.length > 0 && !app.homeOpen),
  "file.open": openDocuments,
  "file.importLayer": importLayer,
  "file.placeSmart": () => placeSmartObjects(false),
  "file.placeLinked": () => placeSmartObjects(true),
  "file.save": () => saveProject(false),
  "file.saveAs": () => saveProject(true),
  // L'identifiant date du temps où seul le PNG existait ; il est conservé pour ne pas casser les raccourcis enregistrés.
  "file.exportPng": () => void (app.exportOpen = true),
  "file.close": closeActive,
  "edit.undo": () => edit({ op: "undo" }),
  "edit.redo": () => edit({ op: "redo" }),
  // Ctrl+Alt+Z : bascule entre le dernier état et le précédent.
  "edit.toggleLastState": () => edit({ op: app.active?.info.canRedo ? "redo" : "undo" }),
  "edit.fillForeground": () => fill("foreground"),
  "edit.fillBackground": () => fill("background"),
  // Presse-papiers interne : pixels sélectionnés du calque actif (tout le calque sans sélection).
  // Ctrl+I, comme Photoshop : inverse directement les pixels du calque (dans la sélection s'il y en a une).
  "help.about": () => void (app.aboutOpen = true),
  "edit.preferences": () => void (app.preferencesOpen = true),
  "copyright.metadata": () => void (app.copyrightOpen = true),
  "layer.effects": () => void (app.effectsLayer = app.activeLayer?.id ?? null),
  "view.guideLayout": () => void (app.guideLayoutOpen = true),
  "select.quickMask": () => void (app.quickMask = !app.quickMask),
  "tool.crop": () => void (app.tool = "crop"),
  "tool.shape": () => void (app.tool = "shape"),
  "tool.shapeCycle": () => {
    // Maj+U : rectangle, ellipse, triangle, puis de nouveau rectangle.
    const next = { rect: "ellipse", ellipse: "polygon", polygon: "rect" } as const;
    app.shapeKind = app.tool === "shape" ? next[app.shapeKind] : "rect";
    if (app.shapeKind === "polygon") app.shapeSides = 3;
    app.tool = "shape";
  },
  "filter.liquify": () => void (app.liquifyOpen = true),
  // Ctrl+I : sur le masque de fusion s'il est la cible (ce qui était masqué se révèle), sinon sur les pixels.
  "image.invert": () => {
    const id = activeId();
    if (id === null) return undefined;
    return edit(app.active?.target === "mask" && app.activeLayer?.hasMask ? { op: "invertMask", id } : { op: "invert", id });
  },
  // Copier et couper alimentent aussi le presse-papiers du système, pour coller dans un autre logiciel.
  // Avec une sélection : les pixels sélectionnés. Sans sélection : les calques sélectionnés eux-mêmes, groupes
  // et contenu compris (texte, effets, masques), à coller ici ou dans un autre document.
  "edit.copy": async () => {
    const doc = app.active;
    const id = activeId();
    if (!doc || id === null) return;
    if (!doc.info.hasSelection) {
      // Une transformation en attente est validée, et la copie (par la file du document) la voit appliquée.
      settlePendingEdits();
      try {
        const [docId, ids] = [doc.info.id, [...doc.selection]];
        await enqueue(docId, () => api.copyLayers(docId, ids));
        app.lastCopy = "layers";
        // Une marque dans le presse-papiers du système : une capture d'écran faite ensuite la remplacera,
        // et c'est alors elle qu'on collera (le plus récent gagne).
        await api.clipboardSetText("Lumitype : calques copiés").catch(() => undefined);
        await markClipboard();
      } catch (error) {
        app.fail(error);
      }
      return;
    }
    if (await edit({ op: "copy", id })) {
      app.lastCopy = "pixels";
      await shareClipboard();
    }
  },
  "edit.cut": () => {
    const id = activeId();
    return id === null ? undefined : edit({ op: "cut", id }).then((ok) => (ok ? shareClipboard() : undefined));
  },
  "edit.paste": () => pasteFromClipboard(),
  // Ctrl+E : plusieurs calques sélectionnés (formes, textes compris) → un seul calque de pixels ; un groupe seul → aplati.
  "layer.mergeGroup": () => {
    const doc = app.active;
    const layer = app.activeLayer;
    if (doc && doc.selection.length >= 2) return edit({ op: "mergeLayers", ids: [...doc.selection] });
    return layer?.isGroup ? edit({ op: "mergeGroup", id: layer.id }) : undefined;
  },
  "edit.freeTransform": beginFreeTransform,
  "edit.shortcuts": () => void (app.shortcutsDialogOpen = true),
  "layer.new": () => edit({ op: "addLayer", above: activeId() }),
  "layer.newGroup": () => edit({ op: "addGroup", above: activeId() }),
  "layer.group": async () => {
    if (!(await edit({ op: "group", ids: selection() }))) return;
    // Le groupe qui vient de naître est sélectionné : on s'assure qu'il s'affiche déplié, avec ses calques visibles.
    const group = app.activeLayer;
    if (group?.isGroup && !group.expanded) await edit({ op: "setExpanded", id: group.id, expanded: true });
  },
  "layer.duplicate": () => {
    const id = activeId();
    return id === null ? undefined : edit({ op: "duplicate", id });
  },
  // Comme Photoshop : avec une sélection active, Suppr efface les pixels ; sinon, le calque.
  // Bouton corbeille et menus : toujours le calque (le moteur refuse un calque verrouillé par le cadenas).
  "layer.delete": () => edit({ op: "delete", ids: selection() }),
  // Touche Suppr : dans le panneau Calques, c'est le calque ; sur l'image avec une sélection, ce sont les pixels
  // sélectionnés (ou, le masque de fusion étant visé, la sélection y est masquée) ; sans sélection, le calque.
  // Un calque caché se supprime donc comme les autres depuis le panneau.
  "edit.deleteKey": () => {
    const id = activeId();
    const inLayersPanel = Boolean(document.activeElement?.closest("[data-panel='layers']"));
    if (inLayersPanel || !app.active?.info.hasSelection || id === null) return edit({ op: "delete", ids: selection() });
    const onMask = app.active.target === "mask" && app.activeLayer?.hasMask === true;
    return edit({ op: "clearPixels", id, onMask });
  },
  // Ctrl+A : le contenu du calque actif (ses pixels opaques), pas tout le plan de travail.
  "select.all": () => edit({ op: "selectAll", layer: app.activeLayer && !app.activeLayer.isGroup ? app.activeLayer.id : undefined }),
  "select.none": () => edit({ op: "deselect" }),
  "select.invert": () => edit({ op: "invertSelection" }),
  "select.feather": () => void (app.featherDialogOpen = true),
  "layer.toggleClipping": toggleClipping,
  "layer.addMask": addMask,
  "layer.convertToSmart": () => edit({ op: "convertToSmart", ids: selection() }),
  "layer.editSmartContent": () => openSmartContent(),
  "layer.replaceSmartContent": () => replaceSmartContent(),
  "layer.newSmartByCopy": () => smartEdit("newSmartByCopy"),
  "layer.exportSmartContent": () => exportSmartContent(),
  "layer.embedSmartLink": () => smartEdit("embedSmartLink"),
  "layer.relinkSmart": () => relinkSmart(),
  "layer.resetSmartTransform": () => smartEdit("resetSmartTransform"),
  "layer.rasterize": () => {
    const layer = app.activeLayer;
    return layer && (layer.text || layer.shape || layer.smart) ? edit({ op: "rasterize", id: layer.id }) : undefined;
  },
  "layer.blendNext": () => cycleBlend(1),
  "layer.blendPrevious": () => cycleBlend(-1),
  "view.zoomIn": () => stepZoom(1),
  "view.zoomOut": () => stepZoom(-1),
  "view.fit": () => frame("fit"),
  "view.actualSize": () => frame("actual"),
  "view.toggleRulers": () => void (app.rulersVisible = !app.rulersVisible),
  "view.toggleGuides": () => void (app.guidesVisible = !app.guidesVisible),
  "view.toggleSnap": () => void (app.snapEnabled = !app.snapEnabled),
  // Ctrl+Y : allume l'épreuve avec le dernier profil choisi, ou l'éteint.
  "view.proofCmyk": () => void (app.proofCmyk = !app.proofCmyk),
  "view.proofFogra39": () => chooseProofProfile("fogra39"),
  "view.proofFogra51": () => chooseProofProfile("fogra51"),
  "view.proofFogra52": () => chooseProofProfile("fogra52"),
  "view.proofGracol2006": () => chooseProofProfile("gracol2006"),
  "view.proofSwop2006c3": () => chooseProofProfile("swop2006c3"),
  "view.lockGuides": () => edit({ op: "setGuidesLocked", locked: !app.active?.info.guidesLocked }),
  "view.clearGuides": () => edit({ op: "clearGuides" }),
  "doc.next": () => stepDocument(1),
  "doc.previous": () => stepDocument(-1),
  "color.swap": () => void ([app.foreground, app.background] = [app.background, app.foreground]),
  "color.reset": () => void ([app.foreground, app.background] = [[0, 0, 0], [255, 255, 255]]),
  "brush.smaller": () => resizeBrush(-1),
  "brush.larger": () => resizeBrush(1),
  "tool.move": () => void (app.tool = "move"),
  "tool.marquee": () => void (app.tool = "marquee"),
  "tool.marqueeCycle": () => {
    app.marqueeShape = app.tool === "marquee" && app.marqueeShape === "rect" ? "ellipse" : "rect";
    app.tool = "marquee";
  },
  "tool.lasso": () => void (app.tool = "lasso"),
  "tool.lassoCycle": () => {
    app.lassoShape = app.tool === "lasso" && app.lassoShape === "free" ? "polygon" : "free";
    app.tool = "lasso";
  },
  "tool.wand": () => void (app.tool = "wand"),
  // Calques de réglage : créés au-dessus du calque actif, avec des paramètres neutres.
  "adjust.brightnessContrast": () => addAdjustment({ type: "brightnessContrast", brightness: 0, contrast: 0 }),
  // Ctrl+U : le calque de réglage est créé ; ses curseurs sont dans Propriétés (l'onglet ne change pas tout seul).
  "adjust.hueSaturation": () => addAdjustment({ type: "hueSaturation", hue: 0, saturation: 0, lightness: 0 }),
  "adjust.levels": () => addAdjustment({ type: "levels", black: 0, gamma: 1, white: 255 }),
  "adjust.invert": () => addAdjustment({ type: "invert" }),
  // Transfert de dégradé : du noir au blanc au départ (neutre à l'œil sur une image en noir et blanc) ; l'éditeur est dans Propriétés.
  "adjust.gradientMap": () =>
    addAdjustment({ type: "gradientMap", stops: [{ position: 0, color: [0, 0, 0, 255] }, { position: 1, color: [255, 255, 255, 255] }], reverse: false, dither: false, smoothness: 100 }),
  "adjust.develop": () => addAdjustment({ type: "develop", ...NEUTRAL_DEVELOP }),
  "filter.gaussianBlur": () => void (app.filterDialog = "gaussianBlur"),
  "filter.fieldBlur": () => void (app.filterDialog = "fieldBlur"),
  "filter.motionBlur": () => void (app.filterDialog = "motionBlur"),
  "filter.lensBlur": () => void (app.filterDialog = "lensBlur"),
  "filter.sharpen": () => void (app.filterDialog = "sharpen"),
  "filter.noise": () => void (app.filterDialog = "noise"),
  "filter.dither": () => void (app.filterDialog = "dither"),
  "filter.presence": () => void (app.filterDialog = "presence"),
  "filter.glass": () => void (app.filterDialog = "glass"),
  "tool.heal": () => void (app.tool = "heal"),
  "tool.wandCycle": () => void (app.tool = app.tool === "wand" ? "quickSelect" : "wand"),
  "select.modify": () => void (app.modifySelectionOpen = true),
  "filter.gaussianNoise": () => void (app.filterDialog = "gaussianNoise"),
  "filter.colorDither": () => void (app.filterDialog = "colorDither"),
  "filter.chromaticAberration": () => void (app.filterDialog = "chromaticAberration"),
  "filter.ascii": () => void (app.filterDialog = "ascii"),
  // Négatif : même courbe qu'Inverser, mais sur toute l'image (calque libre, non écrêté) : le fond aussi devient négatif.
  "adjust.negative": () => addAdjustment({ type: "negative" }, false),
  "tool.fill": () => void (app.tool = "fill"),
  "tool.fillCycle": () => {
    app.fill.kind = app.tool === "fill" && app.fill.kind === "gradient" ? "bucket" : "gradient";
    app.tool = "fill";
  },
  "tool.pen": () => void (app.tool = "pen"),
  "tool.pathSelect": () => void (app.tool = "pathSelect"),
  "path.toSelection": () => edit({ op: "pathToSelection", mode: "replace" }),
  "path.fill": () => {
    const id = activeId();
    return id === null ? undefined : edit({ op: "fillPath", id, color: app.foreground });
  },
  // Le contour utilise le pinceau tel qu'il est réglé (taille, dureté, opacité) et la couleur de premier plan.
  "path.stroke": () => {
    const id = activeId();
    return id === null ? undefined : edit({ op: "strokePath", id, brush: { ...app.brushes.brush, size: app.brushes.brush.size || 1, color: app.foreground, erase: false, pressureSize: false } });
  },
  "path.clear": () => edit({ op: "setPath", paths: [], label: t("Effacer le tracé") }),
  "tool.eyedropper": () => void (app.tool = "eyedropper"),
  "tool.text": () => void (app.tool = "text"),
  "tool.brush": () => void (app.tool = "brush"),
  "tool.eraser": () => void (app.tool = "eraser"),
  "tool.hand": () => void (app.tool = "hand"),
  "tool.zoom": () => void (app.tool = "zoom"),
};

const NO_DOCUMENT_NEEDED: ReadonlySet<ActionId> = new Set([
  "file.home",
  "help.about",
  "edit.preferences",
  "copyright.metadata",
  "file.new",
  "file.open",
  "edit.shortcuts",
  "view.toggleRulers",
  "view.toggleGuides",
  "view.toggleSnap",
  "view.proofCmyk",
  "view.proofFogra39",
  "view.proofFogra51",
  "view.proofFogra52",
  "view.proofGracol2006",
  "view.proofSwop2006c3",
  "color.swap",
  "color.reset",
  "brush.smaller",
  "brush.larger",
  "tool.move",
  "tool.marquee",
  "tool.marqueeCycle",
  "tool.lasso",
  "tool.lassoCycle",
  "tool.wand",
  "tool.heal",
  "tool.wandCycle",
  "tool.fill",
  "tool.fillCycle",
  "tool.pen",
  "tool.pathSelect",
  "tool.eyedropper",
  "tool.text",
  "tool.brush",
  "tool.eraser",
  "tool.hand",
  "tool.zoom",
]);

/** État d'une option à bascule, pour la coche du menu ; `null` pour une action ordinaire. */
/** Document suivant ou précédent, en tournant en boucle : les onglets hors champ restent joignables. */
function stepDocument(step: number): void {
  const docs = app.documents;
  if (docs.length < 2) return;
  const here = docs.findIndex((d) => d.info.id === app.activeId);
  const next = docs[(((here < 0 ? 0 : here) + step) % docs.length + docs.length) % docs.length];
  if (next) app.activeId = next.info.id;
}

export function isActionChecked(id: ActionId): boolean | null {
  switch (id) {
    case "view.toggleSnap":
      return app.snapEnabled;
    case "view.proofCmyk":
      return app.proofCmyk;
    case "view.proofFogra39":
      return app.proofProfile === "fogra39";
    case "view.proofFogra51":
      return app.proofProfile === "fogra51";
    case "view.proofFogra52":
      return app.proofProfile === "fogra52";
    case "view.proofGracol2006":
      return app.proofProfile === "gracol2006";
    case "view.proofSwop2006c3":
      return app.proofProfile === "swop2006c3";
    case "view.toggleRulers":
      return app.rulersVisible;
    case "view.toggleGuides":
      return app.guidesVisible;
    case "view.lockGuides":
      return app.active?.info.guidesLocked ?? false;
    case "select.quickMask":
      return app.quickMask;
    default:
      return null;
  }
}

export function isActionEnabled(id: ActionId): boolean {
  if (app.busy !== null) return false;
  if (NO_DOCUMENT_NEEDED.has(id)) return true;
  const doc = app.active;
  if (!doc) return false;
  switch (id) {
    case "edit.undo":
      return doc.info.canUndo;
    case "edit.redo":
      return doc.info.canRedo;
    case "select.modify":
      return doc.info.hasSelection;
    case "layer.mergeGroup":
      return doc.selection.length >= 2 || (doc.selection.length === 1 && app.activeLayer?.isGroup === true);
    case "layer.delete":
    case "edit.deleteKey":
      // Un calque verrouillé par le cadenas ne se supprime pas : le déverrouiller d'abord.
      return doc.selection.length > 0 && !doc.info.layers.some((l) => doc.selection.includes(l.id) && l.locks.all);
    case "layer.effects": {
      const layer = app.activeLayer;
      return doc.selection.length === 1 && !!layer && !layer.isGroup && !layer.adjustment;
    }
    case "filter.gaussianBlur":
    case "filter.liquify":
    case "filter.fieldBlur":
    case "filter.motionBlur":
    case "filter.lensBlur":
    case "filter.noise":
    case "filter.glass":
    case "filter.presence":
    case "filter.gaussianNoise":
    case "filter.colorDither":
    case "filter.chromaticAberration":
    case "filter.dither":
    case "filter.ascii":
    case "filter.sharpen": {
      // Un filtre agit sur les pixels d'UN calque ; une forme ou un texte sont pixellisés au passage. Un objet
      // dynamique, lui, refuse (le moteur le dit) : on pixellise ou on modifie son contenu d'abord.
      const layer = app.activeLayer;
      return doc.selection.length === 1 && !!layer && !layer.isGroup && !layer.adjustment && !layer.smart;
    }
    case "layer.convertToSmart":
      return doc.selection.length > 0;
    case "layer.editSmartContent":
    case "layer.replaceSmartContent":
    case "layer.newSmartByCopy":
    case "layer.exportSmartContent":
    case "layer.resetSmartTransform":
      return !!app.activeLayer?.smart;
    case "layer.embedSmartLink":
    case "layer.relinkSmart":
      return app.activeLayer?.smart?.linked === true;
    case "layer.rasterize": {
      const layer = app.activeLayer;
      return !!layer && (!!layer.text || !!layer.shape || !!layer.smart);
    }
    case "path.toSelection":
    case "path.fill":
    case "path.stroke":
    case "path.clear":
      return doc.info.paths.some((sub) => sub.anchors.length > 0);
    case "view.clearGuides":
      return doc.info.guides.length > 0;
    case "select.none":
    case "select.feather":
      return doc.info.hasSelection;
    case "layer.addMask":
      return app.activeLayer !== null && !app.activeLayer.hasMask;
    default:
      return true;
  }
}

export function runAction(id: ActionId): void {
  if (!isActionEnabled(id)) return;
  void HANDLERS[id]();
}

/**
 * Fichiers déposés dans la fenêtre. Sur un document ouvert, une image devient un
 * calque au-dessus du calque actif (comme Photoshop) ; sinon chaque fichier s'ouvre
 * comme nouveau document. Un projet `.iep` s'ouvre toujours à part.
 */
export async function importDroppedFiles(files: File[], asSmartObject = false): Promise<void> {
  for (const original of files) {
    // AVIF : décodé par la webview, puis confié au moteur en PNG.
    let file = original;
    if (isAvif(original.name, original.type)) {
      try {
        file = await avifToPng(original, original.name || "Image.avif");
      } catch (error) {
        app.fail(error);
        continue;
      }
    }
    const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
    // Une image collée ou glissée depuis un navigateur n'a pas toujours d'extension : son type suffit.
    const isImage = IMAGE_MIME_TYPES.includes(file.type);
    const isProject = extension === PROJECT_EXTENSION || extension === LEGACY_PROJECT_EXTENSION;
    if (!isProject && !IMAGE_EXTENSIONS.includes(extension) && !isImage) {
      app.error = `Format non pris en charge : ${file.name}`;
      continue;
    }
    // Depuis l'accueil (même avec des documents ouverts dessous), une image ouvre un nouveau document.
    const target = isProject || app.homeVisible ? null : app.active;
    await withBusy("Import…", async () => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      // Objet dynamique (Alt au dépôt, Importer comme objet dynamique) : seulement dans un document ouvert.
      const run = () => api.importDropped(bytes, file.name, target?.info.id ?? null, activeId(), asSmartObject && target !== null && !isProject);
      if (target) {
        settlePendingEdits();
        const result = await enqueue(target.info.id, run);
        adopt(result.document, result.dirty);
      } else {
        const { document } = await run();
        app.add(document);
        refreshThumbnails(document, []);
      }
    });
  }
}

export async function createDocument(params: Parameters<typeof api.newDocument>[0]): Promise<void> {
  await withBusy(t("Création…"), async () => {
    const info = await api.newDocument(params);
    app.add(info);
    refreshThumbnails(info, []);
  });
}

// ------------------------------------------------------------ objets dynamiques

/** Ce qu'on peut placer comme objet dynamique : une image ou un projet entier. */
const placeFilters = () => [
  { name: t("Images et projets"), extensions: [...IMAGE_EXTENSIONS, PROJECT_EXTENSION, LEGACY_PROJECT_EXTENSION] },
  IMAGE_FILTER,
  { name: t("Projets Lumitype"), extensions: [PROJECT_EXTENSION, LEGACY_PROJECT_EXTENSION] },
];

/** Nom affiché d'un document : celui d'un contenu d'objet dynamique dit d'où il vient (« Logo.graf — contenu de « Logo » »). */
export function documentTitle(info: { name: string; smartParent: { name: string } | null }): string {
  return info.smartParent ? t("{name}.graf — contenu de « {parent} »", { name: info.name, parent: info.smartParent.name }) : info.name;
}

/** L'objet dynamique visé : `id` donné (menu contextuel d'un calque) ou, à défaut, le calque actif. */
function smartLayer(id?: number): LayerInfo | null {
  const layer = id === undefined ? app.activeLayer : (app.active?.info.layers.find((l) => l.id === id) ?? null);
  return layer?.smart ? layer : null;
}

/** Opération simple sur un objet dynamique (incorporer, réinitialiser, nouvel objet par copie). */
export function smartEdit(op: "embedSmartLink" | "resetSmartTransform" | "newSmartByCopy", id?: number): Promise<boolean> | undefined {
  const layer = smartLayer(id);
  return layer ? edit({ op, id: layer.id }) : undefined;
}

/** Fichier › Importer comme objet dynamique… (incorporé) ou Importer lié… (`linked` : le contenu suit le fichier). */
async function placeSmartObjects(linked: boolean): Promise<void> {
  if (!app.active) return;
  const paths = await open({ multiple: true, filters: placeFilters() });
  if (!paths) return;
  for (const path of paths) {
    if (isAvif(path)) {
      // AVIF : décodé par la webview, il ne peut être qu'incorporé (le moteur ne sait pas le relire).
      if (linked) app.error = t("Image AVIF : elle est incorporée, un lien ne pourrait pas être relu.");
      try {
        await importDroppedFiles([await avifPathToPng(path)], true);
      } catch (error) {
        app.fail(error);
      }
      continue;
    }
    await withBusy(t("Import…"), async () => void (await edit({ op: "placeSmartObject", path, linked, above: activeId() })));
  }
}

/**
 * « Modifier le contenu » (double-clic sur la vignette) : le contenu s'ouvre dans un nouvel onglet, document à part
 * relié à l'objet. Déjà ouvert : son onglet est simplement activé.
 */
export async function openSmartContent(id?: number): Promise<void> {
  const doc = app.active;
  const layer = smartLayer(id);
  if (!doc || !layer) return;
  const docId = doc.info.id;
  settlePendingEdits(); // une transformation en attente est validée : le contenu ouvert est celui d'après
  await withBusy(t("Ouverture du contenu…"), async () => {
    const info = await enqueue(docId, () => api.smartContentOpen(docId, layer.id));
    if (app.documents.some((d) => d.info.id === info.id)) {
      app.homeOpen = false;
      app.activeId = info.id;
    } else {
      app.add(info);
      refreshThumbnails(info, []);
    }
  });
}

/**
 * Ctrl+S dans l'onglet d'un contenu : l'objet dynamique (et toutes ses instances) est mis à jour dans son document,
 * sans quitter l'onglet. Le contenu est alors « enregistré » ; le parent, lui, devient modifié.
 */
async function commitSmartContent(doc: OpenDocument): Promise<void> {
  const parentId = doc.info.smartParent?.docId;
  if (parentId === undefined) return;
  const childId = doc.info.id;
  settlePendingEdits();
  await withBusy(t("Mise à jour de l'objet dynamique…"), async () => {
    // Les modifications du contenu en cours passent d'abord, puis celles du parent attendent la mise à jour.
    const result = await enqueue(parentId, () => enqueue(childId, () => api.smartContentCommit(childId)));
    if (app.documents.some((d) => d.info.id === result.document.id)) adopt(result.document, result.dirty);
    doc.savedRevision = doc.info.revision;
    forgetRecovery(doc);
  });
}

/** Remplacer le contenu… : toutes les instances prennent le nouveau fichier, à la taille affichée actuelle. */
export async function replaceSmartContent(id?: number): Promise<void> {
  const layer = smartLayer(id);
  if (!layer) return;
  const path = await open({ multiple: false, filters: placeFilters() });
  if (path) await withBusy(t("Remplacement…"), async () => void (await edit({ op: "replaceSmartContent", id: layer.id, path })));
}

/** Relier… : un objet lié (fichier déplacé, renommé, introuvable) retrouve son fichier. */
export async function relinkSmart(id?: number): Promise<void> {
  const layer = smartLayer(id);
  if (!layer) return;
  const path = await open({ multiple: false, filters: placeFilters() });
  if (path) await withBusy(t("Import…"), async () => void (await edit({ op: "relinkSmart", id: layer.id, path })));
}

/** Exporter le contenu… : l'image d'origine (à l'octet près), le fichier lié, ou le contenu en projet `.graf`. */
export async function exportSmartContent(id?: number): Promise<void> {
  const doc = app.active;
  const layer = smartLayer(id);
  if (!doc || !layer?.smart) return;
  const name = layer.smart.exportName;
  const extension = name.includes(".") ? (name.split(".").pop() ?? PROJECT_EXTENSION).toLowerCase() : PROJECT_EXTENSION;
  const filters = extension === PROJECT_EXTENSION ? [{ name: t("Projet Lumitype"), extensions: [PROJECT_EXTENSION] }] : [{ name: extension.toUpperCase(), extensions: [extension] }, { name: t("Projet Lumitype"), extensions: [PROJECT_EXTENSION] }];
  const path = await save({ defaultPath: await inSaveFolder(name), filters });
  if (!path) return;
  const [docId, layerId] = [doc.info.id, layer.id];
  await withBusy(t("Exportation…"), async () => void (await enqueue(docId, () => api.smartContentExport(docId, layerId, path))));
}

/** Dernière vérification des liens de chaque document, et fichiers introuvables déjà signalés. */
const linksCheckedAt = new Map<number, number>();
const reportedMissing = new Map<number, string>();
/** Délai minimal entre deux vérifications des liens d'un même document. */
const LINKS_CHECK_INTERVAL_MS = 2000;

/**
 * Vérifie les fichiers liés du document (retour dans la fenêtre, document activé) : un fichier modifié ailleurs met
 * l'objet à jour ; un fichier introuvable est signalé une fois (puis de nouveau si la liste change).
 */
export async function checkSmartLinks(doc: OpenDocument | null = app.active): Promise<void> {
  if (!doc || !doc.info.layers.some((l) => l.smart?.linked)) return;
  const docId = doc.info.id;
  const now = Date.now();
  if (now - (linksCheckedAt.get(docId) ?? 0) < LINKS_CHECK_INTERVAL_MS) return;
  linksCheckedAt.set(docId, now);
  try {
    const result = await enqueue(docId, () => api.smartLinksCheck(docId));
    const entry = app.documents.find((d) => d.info.id === docId);
    if (!entry) return; // fermé entre-temps
    if (result.updated.length > 0 || result.document.revision !== entry.info.revision) adopt(result.document, result.dirty);
    const missing = [...result.missing].sort((a, b) => a - b).join(",");
    if (result.missing.length > 0 && reportedMissing.get(docId) !== missing) app.error = t("{n} fichier(s) lié(s) introuvable(s)", { n: result.missing.length });
    reportedMissing.set(docId, missing);
    if (result.failed.length > 0) app.error = result.failed.join(" · ");
  } catch (error) {
    app.fail(error);
  }
}
