// Faux processus Rust pour faire tourner l'interface de Lumitype dans un simple Chromium (captures de la notice).
// Injecté par `page.addInitScript` AVANT le chargement de la page : il définit `window.__TAURI_INTERNALS__`
// (ce que lit `@tauri-apps/api` : invoke, transformCallback, convertFileSrc, metadata) et
// `window.__TAURI_EVENT_PLUGIN_INTERNALS__`, puis répond aux commandes de `ui/src/lib/ipc.ts` et `lib/myref.ts`.
//
// Scène choisie par `window.__MOCK_SCENE__` (posé par capture.mjs avant ce script) :
//   « poster »    : affiche A4 300 ppp, quatre calques (Title, Shape, Photo avec masque, Background) ;
//   « print »     : la même, avec un format d'impression (fonds perdus 3 mm, zone de sécurité 5 mm) ;
//   « artboards » : deux plans de travail côte à côte.
// Toute commande inconnue répond `null` et laisse une ligne « [mock] inconnue : … » dans la console : capture.mjs
// les rassemble, c'est la liste de ce qu'il reste à simuler après un changement de l'interface.
(() => {
  "use strict";
  const scene = window.__MOCK_SCENE__ || "poster";
  const log = (...args) => console.log("[mock]", ...args);

  // ------------------------------------------------------------------ rappels et événements

  const callbacks = new Map();
  let nextCallback = 1;
  function transformCallback(callback, once = false) {
    const id = nextCallback++;
    callbacks.set(id, (data) => {
      if (once) callbacks.delete(id);
      return callback && callback(data);
    });
    return id;
  }
  const listeners = new Map(); // événement -> [identifiant de rappel]
  function emit(event, payload) {
    for (const handler of listeners.get(event) || []) callbacks.get(handler)?.({ event, id: handler, payload });
  }

  // ------------------------------------------------------------------ dessins générés

  const SCALE = 0.5; // les images des calques sont tenues à mi-résolution : largement assez pour une capture
  const canvas = (w, h) => new OffscreenCanvas(Math.max(1, Math.round(w)), Math.max(1, Math.round(h)));

  /** Générateur pseudo-aléatoire reproductible. */
  function rng(seed) {
    let s = seed >>> 0 || 1;
    return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  }
  const hashText = (text) => [...String(text)].reduce((h, c) => (Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0), 2166136261);

  const PALETTES = [
    ["#1d3557", "#457b9d", "#a8dadc", "#f1faee", "#e63946"],
    ["#264653", "#2a9d8f", "#e9c46a", "#f4a261", "#e76f51"],
    ["#22223b", "#4a4e69", "#9a8c98", "#c9ada7", "#f2e9e4"],
    ["#003049", "#d62828", "#f77f00", "#fcbf49", "#eae2b7"],
    ["#0b132b", "#1c2541", "#3a506b", "#5bc0be", "#ffffff"],
    ["#2b2d42", "#8d99ae", "#edf2f4", "#ef233c", "#d90429"],
    ["#606c38", "#283618", "#fefae0", "#dda15e", "#bc6c25"],
    ["#5f0f40", "#9a031e", "#fb8b24", "#e36414", "#0f4c5c"],
  ];

  /** Petite composition abstraite (vignettes, aperçus, images de My ref). */
  function paintAbstract(ctx, w, h, seed) {
    const r = rng(seed);
    const pal = PALETTES[seed % PALETTES.length];
    const g = ctx.createLinearGradient(0, 0, w * r(), h);
    g.addColorStop(0, pal[0]);
    g.addColorStop(1, pal[1]);
    ctx.fillStyle = g;
    ctx.fillRect(0, 0, w, h);
    const kind = seed % 3;
    if (kind === 0) {
      // Paysage : soleil et collines.
      ctx.fillStyle = pal[3];
      ctx.beginPath();
      ctx.arc(w * (0.3 + r() * 0.4), h * 0.38, Math.min(w, h) * 0.16, 0, Math.PI * 2);
      ctx.fill();
      for (let i = 0; i < 3; i++) {
        ctx.fillStyle = pal[(i + 2) % 5];
        ctx.globalAlpha = 0.85;
        ctx.beginPath();
        ctx.moveTo(0, h);
        const base = h * (0.55 + i * 0.13);
        for (let x = 0; x <= w; x += w / 8) ctx.lineTo(x, base + Math.sin(x / w * 6 + i * 2 + r()) * h * 0.06);
        ctx.lineTo(w, h);
        ctx.fill();
      }
    } else if (kind === 1) {
      // Cercles qui se recouvrent.
      for (let i = 0; i < 6; i++) {
        ctx.globalAlpha = 0.75;
        ctx.fillStyle = pal[2 + (i % 3)];
        ctx.beginPath();
        ctx.arc(w * r(), h * r(), Math.min(w, h) * (0.12 + r() * 0.25), 0, Math.PI * 2);
        ctx.fill();
      }
    } else {
      // Bandes géométriques.
      for (let i = 0; i < 7; i++) {
        ctx.globalAlpha = 0.8;
        ctx.fillStyle = pal[(i + 1) % 5];
        ctx.save();
        ctx.translate(w / 2, h / 2);
        ctx.rotate(-0.5 + r() * 0.2);
        ctx.fillRect(-w, -h + i * h * 0.32, w * 2, h * 0.14);
        ctx.restore();
      }
    }
    ctx.globalAlpha = 1;
  }

  // ------------------------------------------------------------------ documents

  const A4 = { width: 2480, height: 3508 };
  const docs = new Map();
  let nextDocId = 1;

  function baseLayer(fields) {
    return {
      parent: null,
      index: 0,
      depth: 0,
      offset: [0, 0],
      isGroup: false,
      expanded: true,
      childCount: 0,
      visible: true,
      opacity: 1,
      fill: 1,
      blend: "normal",
      locks: { transparency: false, pixels: false, position: false, all: false },
      clipped: false,
      blendIf: null,
      hasMask: false,
      maskEnabled: true,
      maskLinked: true,
      text: null,
      adjustment: null,
      shape: null,
      fillLayer: null,
      effects: [],
      label: null,
      smart: null,
      filters: null,
      artboard: null,
      ...fields,
    };
  }

  /** Calques de l'affiche : dessin de chaque calque dans son propre repère (à mi-résolution), et taille. */
  function posterLayers(originX = 0, originY = 0, parent = null, firstId = 1, depth = 0) {
    const ids = { background: firstId, photo: firstId + 1, shape: firstId + 2, title: firstId + 3 };
    const background = {
      info: baseLayer({ id: ids.background, name: "Background", parent, depth, index: 0, offset: [originX, originY], locks: { transparency: false, pixels: false, position: true, all: false } }),
      size: [A4.width, A4.height],
      paint(ctx, w, h) {
        const g = ctx.createLinearGradient(0, 0, 0, h);
        g.addColorStop(0, "#1b2a4a");
        g.addColorStop(0.55, "#b8476a");
        g.addColorStop(1, "#f6a65d");
        ctx.fillStyle = g;
        ctx.fillRect(0, 0, w, h);
      },
    };
    const photo = {
      info: baseLayer({ id: ids.photo, name: "Photo", parent, depth, index: 1, offset: [originX + 240, originY + 1000], hasMask: true }),
      size: [2000, 1900],
      paint(ctx, w, h) {
        const sky = ctx.createLinearGradient(0, 0, 0, h * 0.7);
        sky.addColorStop(0, "#3c6e91");
        sky.addColorStop(1, "#ffd3a5");
        ctx.fillStyle = sky;
        ctx.fillRect(0, 0, w, h);
        ctx.fillStyle = "#fff4d6";
        ctx.beginPath();
        ctx.arc(w * 0.62, h * 0.42, w * 0.11, 0, Math.PI * 2);
        ctx.fill();
        const hills = ["#5c8d89", "#3f6b6a", "#284b4f", "#1a3236"];
        hills.forEach((color, i) => {
          ctx.fillStyle = color;
          ctx.beginPath();
          ctx.moveTo(0, h);
          const base = h * (0.52 + i * 0.12);
          for (let x = 0; x <= w; x += w / 40) ctx.lineTo(x, base + Math.sin((x / w) * (4 + i) + i * 1.7) * h * 0.05 + Math.sin((x / w) * 13 + i) * h * 0.012);
          ctx.lineTo(w, h);
          ctx.fill();
        });
      },
      // Masque : rectangle aux coins arrondis, fondu vers le bas.
      paintMask(ctx, w, h) {
        ctx.fillStyle = "#000";
        ctx.fillRect(0, 0, w, h);
        const g = ctx.createLinearGradient(0, h * 0.75, 0, h);
        g.addColorStop(0, "#fff");
        g.addColorStop(1, "#000");
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.roundRect(0, 0, w, h, w * 0.04);
        ctx.fill();
      },
    };
    const shape = {
      info: baseLayer({
        id: ids.shape,
        name: "Shape",
        parent,
        depth,
        index: 2,
        offset: [originX + 1560, originY + 640],
        opacity: 0.9,
        shape: { kind: "ellipse", width: 640, height: 640, radius: 0, fill: [255, 209, 102, 255], stroke: null, strokeWidth: 0 },
      }),
      size: [640, 640],
      paint(ctx, w, h) {
        ctx.fillStyle = "#ffd166";
        ctx.beginPath();
        ctx.ellipse(w / 2, h / 2, w / 2, h / 2, 0, 0, Math.PI * 2);
        ctx.fill();
      },
    };
    const title = {
      info: baseLayer({
        id: ids.title,
        name: "Title",
        parent,
        depth,
        index: 3,
        offset: [originX + 240, originY + 360],
        text: { content: "Summer Light", fontFamily: "Geist", size: 300, lineHeight: 1.1, tracking: 0, align: "left", color: [255, 255, 255], weight: 700, italic: false, runs: [], frame: null, dynamic: null, caps: "normal", style: null },
      }),
      size: [1500, 640],
      paint(ctx, w, h) {
        ctx.fillStyle = "#ffffff";
        ctx.textBaseline = "top";
        ctx.font = `700 ${Math.round(h * 0.42)}px "Geist Variable", "Geist", sans-serif`;
        ctx.fillText("Summer", 0, h * 0.02);
        ctx.fillText("Light", 0, h * 0.5);
      },
    };
    return [title, shape, photo, background];
  }

  function rasterize(layer) {
    const [w, h] = layer.size;
    const c = canvas(w * SCALE, h * SCALE);
    layer.paint(c.getContext("2d"), c.width, c.height);
    layer.canvas = c;
    if (layer.paintMask) {
      const m = canvas(w * SCALE, h * SCALE);
      layer.paintMask(m.getContext("2d"), m.width, m.height);
      layer.maskCanvas = m;
      // Pixels du calque masqués : composés une fois pour toutes.
      const masked = canvas(c.width, c.height);
      const mctx = masked.getContext("2d");
      mctx.drawImage(c, 0, 0);
      const pixels = mctx.getImageData(0, 0, c.width, c.height);
      const mask = m.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      for (let i = 0; i < pixels.data.length; i += 4) pixels.data[i + 3] = (pixels.data[i + 3] * mask[i]) / 255;
      mctx.putImageData(pixels, 0, 0);
      layer.composited = masked;
    }
  }

  function makeDocument(kind, name) {
    const id = nextDocId++;
    let width = A4.width;
    let height = A4.height;
    let layers;
    let print = null;
    let trim;
    let backdrop = null;
    if (kind === "artboards") {
      // Deux plans de travail (format 1080 × 1350, réseaux sociaux) côte à côte, 200 px d'écart.
      width = 2 * 1080 + 3 * 200;
      height = 1350 + 2 * 200;
      layers = [];
      const boards = [
        { name: "Spring post", x: 200, seed: 3 },
        { name: "Summer post", x: 1480, seed: 7 },
      ];
      let nextId = 1;
      const all = [];
      boards.forEach((board, i) => {
        const boardId = nextId++;
        const childId = nextId++;
        const textId = nextId++;
        const child = {
          info: baseLayer({ id: childId, name: i === 0 ? "Artwork" : "Gradient", parent: boardId, depth: 1, index: 0, offset: [board.x, 200] }),
          size: [1080, 1350],
          paint: (ctx, w, h) => paintAbstract(ctx, w, h, board.seed),
        };
        const label = {
          info: baseLayer({
            id: textId,
            name: i === 0 ? "Headline" : "Caption",
            parent: boardId,
            depth: 1,
            index: 1,
            offset: [board.x + 90, 200 + 1050],
            text: { content: i === 0 ? "New collection" : "Coming soon", fontFamily: "Geist", size: 110, lineHeight: 1.1, tracking: 0, align: "left", color: [255, 255, 255], weight: 700, italic: false, runs: [], frame: null, dynamic: null, caps: "normal", style: null },
          }),
          size: [900, 180],
          paint(ctx, w, h) {
            ctx.fillStyle = "#fff";
            ctx.textBaseline = "top";
            ctx.font = `700 ${Math.round(h * 0.6)}px "Geist Variable", sans-serif`;
            ctx.fillText(i === 0 ? "New collection" : "Coming soon", 0, 0);
          },
        };
        const group = {
          info: baseLayer({
            id: boardId,
            name: board.name,
            isGroup: true,
            childCount: 2,
            index: i,
            offset: [board.x, 200],
            artboard: { x: board.x, y: 200, width: 1080, height: 1350, background: [255, 255, 255], clip: true, preset: null, backgroundLayer: null },
          }),
          size: [0, 0],
          paint() {},
        };
        all.push({ group, children: [label, child] });
      });
      // Du plus haut au plus bas, enfants juste sous leur groupe.
      layers = [];
      for (const { group, children } of all.reverse()) layers.push(group, ...children);
    } else {
      layers = posterLayers();
      if (kind === "print") {
        // Format d'impression : 3 mm de fonds perdus ajoutés autour du format fini (A4).
        const bleed = Math.round((3 / 25.4) * 300);
        print = { bleedMm: 3, safeMm: 5, inkLimit: 300 };
        width = A4.width + 2 * bleed;
        height = A4.height + 2 * bleed;
        trim = [bleed, bleed, A4.width, A4.height];
        layers = posterLayers(0, 0);
        for (const layer of layers) {
          if (layer.info.name === "Background") layer.size = [width, height];
          else layer.info.offset = [layer.info.offset[0] + bleed, layer.info.offset[1] + bleed];
        }
      }
    }
    for (const layer of layers) if (layer.size[0] > 0) rasterize(layer);
    const tileSize = 256;
    let tiles = Math.ceil(Math.max(width, height) / tileSize);
    let maxLevel = 0;
    while (tiles > 1) {
      tiles = Math.ceil(tiles / 2);
      maxLevel++;
    }
    const doc = {
      kind,
      layers,
      backdrop,
      composite: null,
      info: {
        id,
        name,
        width,
        height,
        dpi: 300,
        tileSize,
        maxLevel,
        projectPath: null,
        revision: 1,
        hasSelection: false,
        guides: [],
        guidesLocked: false,
        paths: [],
        layers: layers.map((l) => l.info),
        history: ["New document", "Place image", "Add layer mask", "Ellipse", "Text"],
        historyCursor: 4,
        canUndo: true,
        canRedo: false,
        smartParent: null,
        colorMode: { kind: "rgb" },
        textStyles: [],
        print,
        trim: trim ?? [0, 0, width, height],
        workSeconds: 1260,
      },
    };
    compose(doc);
    docs.set(id, doc);
    return doc;
  }

  /** Image composée du document entier, à mi-résolution (les tuiles y sont découpées). */
  function compose(doc) {
    const { width, height } = doc.info;
    const c = canvas(width * SCALE, height * SCALE);
    const ctx = c.getContext("2d");
    const byId = new Map(doc.layers.map((l) => [l.info.id, l]));
    const visible = (layer) => {
      for (let l = layer; l; l = l.info.parent != null ? byId.get(l.info.parent) : null) if (!l.info.visible) return false;
      return true;
    };
    // Fonds des plans de travail d'abord.
    for (const layer of doc.layers) {
      const board = layer.info.artboard;
      if (board && visible(layer) && board.background) {
        ctx.fillStyle = `rgb(${board.background.join(",")})`;
        ctx.fillRect(board.x * SCALE, board.y * SCALE, board.width * SCALE, board.height * SCALE);
      }
    }
    for (const layer of [...doc.layers].reverse()) {
      if (!layer.canvas || !visible(layer)) continue;
      ctx.globalAlpha = layer.info.opacity;
      const source = layer.composited && layer.info.maskEnabled ? layer.composited : layer.canvas;
      ctx.save();
      if (layer.transform) {
        const [a, b, cc, d, e, f] = layer.transform;
        ctx.setTransform(a, d, b, e, cc * SCALE, f * SCALE);
      }
      ctx.drawImage(source, layer.info.offset[0] * SCALE, layer.info.offset[1] * SCALE);
      ctx.restore();
    }
    ctx.globalAlpha = 1;
    doc.composite = c;
  }

  const tileCanvas = canvas(256, 256);
  const tileCtx = tileCanvas.getContext("2d", { willReadFrequently: true });

  /** Tuile : 8 octets de révision, puis RGBA8 prémultiplié de tileSize² pixels (rien si la tuile est vide). */
  function tile(doc, level, x, y) {
    const size = doc.info.tileSize;
    const span = size * 2 ** level;
    const [x0, y0] = [x * span, y * span];
    const header = new ArrayBuffer(8);
    new DataView(header).setBigUint64(0, BigInt(doc.info.revision), true);
    if (x0 >= doc.info.width || y0 >= doc.info.height) return header;
    tileCtx.clearRect(0, 0, size, size);
    tileCtx.imageSmoothingQuality = "high";
    // Partie de l'image composée (mi-résolution) couverte par la tuile.
    tileCtx.drawImage(doc.composite, x0 * SCALE, y0 * SCALE, span * SCALE, span * SCALE, 0, 0, size, size);
    const data = tileCtx.getImageData(0, 0, size, size).data;
    let empty = true;
    for (let i = 3; i < data.length; i += 4) {
      const a = data[i];
      if (a !== 0) empty = false;
      if (a !== 255) {
        data[i - 3] = (data[i - 3] * a) / 255;
        data[i - 2] = (data[i - 2] * a) / 255;
        data[i - 1] = (data[i - 1] * a) / 255;
      }
    }
    if (empty) return header;
    const out = new Uint8Array(8 + data.length);
    out.set(new Uint8Array(header), 0);
    out.set(data, 8);
    return out.buffer;
  }

  /** Miniature d'un calque ou de son masque : largeur, hauteur (u32) puis RGBA à alpha droit. */
  function thumbnail(doc, id, mask) {
    const layer = doc.layers.find((l) => l.info.id === id);
    const source = layer && (mask ? layer.maskCanvas : layer.canvas);
    if (!source) return new ArrayBuffer(0);
    const max = 96;
    const k = Math.min(max / source.width, max / source.height);
    const [w, h] = [Math.max(1, Math.round(source.width * k)), Math.max(1, Math.round(source.height * k))];
    const c = canvas(w, h);
    const ctx = c.getContext("2d");
    ctx.drawImage(source, 0, 0, w, h);
    const data = ctx.getImageData(0, 0, w, h).data;
    const out = new Uint8Array(8 + data.length);
    new DataView(out.buffer).setUint32(0, w, true);
    new DataView(out.buffer).setUint32(4, h, true);
    out.set(data, 8);
    return out.buffer;
  }

  async function png(w, h, painter) {
    const c = canvas(w, h);
    painter(c.getContext("2d"), w, h);
    return (await c.convertToBlob({ type: "image/png" })).arrayBuffer();
  }

  async function documentPng(doc, maxSide) {
    const k = Math.min(1, maxSide / Math.max(doc.composite.width, doc.composite.height));
    return png(Math.round(doc.composite.width * k), Math.round(doc.composite.height * k), (ctx, w, h) => ctx.drawImage(doc.composite, 0, 0, w, h));
  }

  /** Bilan d'une modification : le document (révision suivante) et ce qui est à redessiner. */
  function result(doc, all = true) {
    doc.info = { ...doc.info, revision: doc.info.revision + 1, layers: doc.layers.map((l) => l.info) };
    if (all) compose(doc);
    return { document: doc.info, dirty: { all, tiles: [], layers: doc.layers.map((l) => l.info.id), selection: false } };
  }

  function edit(doc, op) {
    const find = (id) => doc.layers.find((l) => l.info.id === id);
    switch (op.op) {
      case "setProps":
        for (const id of op.ids) {
          const layer = find(id);
          if (layer) layer.info = { ...layer.info, ...op.patch };
        }
        return result(doc);
      case "setExpanded": {
        const layer = find(op.id);
        if (layer) layer.info = { ...layer.info, expanded: op.expanded };
        return result(doc, false);
      }
      case "setMaskEnabled": {
        const layer = find(op.id);
        if (layer) layer.info = { ...layer.info, maskEnabled: op.enabled };
        return result(doc);
      }
      case "setPrintSetup": {
        doc.info = { ...doc.info, print: op.setup };
        return result(doc, false);
      }
      case "transform": {
        // Aperçu de Ctrl+T : la matrice totale est appliquée au dessin du calque.
        for (const id of op.ids) {
          const layer = find(id);
          if (layer) layer.transform = op.matrix;
        }
        return result(doc);
      }
      case "cancelGesture":
        for (const layer of doc.layers) delete layer.transform;
        return result(doc);
      default:
        log("edit sans effet :", op.op);
        return result(doc, false);
    }
  }

  // ------------------------------------------------------------------ My ref

  const REF_ROOT = "/home/alex/Pictures/My ref";
  const REF_COLLECTIONS = [
    { name: "Color palettes", count: 9, seed: 11 },
    { name: "Landscapes", count: 14, seed: 20 },
    { name: "Typography", count: 7, seed: 31 },
    { name: "Packaging", count: 5, seed: 45 },
    { name: "Posters", count: 11, seed: 52 },
  ];
  const DAY = 86400000;
  const NOW = Date.UTC(2026, 9, 20, 10, 0, 0);
  function refItem(collection, i, seed) {
    const file = `${collection.toLowerCase().replace(/\s+/g, "-")}-${String(i + 1).padStart(2, "0")}.jpg`;
    const landscape = (seed + i) % 3 !== 1;
    return {
      id: `${collection}/${file}`,
      collection,
      file,
      path: `${REF_ROOT}/${collection}/${file}`,
      name: file.replace(/\.jpg$/, ""),
      source: "",
      tags: [],
      note: "",
      favorite: i % 4 === 0,
      added: NOW - (seed + i * 3) * DAY,
      width: landscape ? 1600 : 1200,
      height: landscape ? 1200 : 1600,
      size: 380000 + ((seed * 7919 + i * 104729) % 900000),
      colors: PALETTES[(seed + i) % PALETTES.length].slice(0, 4),
    };
  }
  const refItems = (c) => Array.from({ length: c.count }, (_, i) => refItem(c.name, i, c.seed));
  function refOverview() {
    const collections = REF_COLLECTIONS.map((c) => {
      const items = refItems(c);
      return { name: c.name, count: c.count, cover: null, covers: items.slice(0, 4), favorites: items.filter((i) => i.favorite).length, modified: items[0].added };
    });
    const all = REF_COLLECTIONS.flatMap(refItems);
    return {
      root: REF_ROOT,
      collections,
      favorites: all.filter((i) => i.favorite).slice(0, 8),
      recent: [...all].sort((a, b) => b.added - a.added).slice(0, 12),
      total: all.length,
    };
  }
  function refImage(id, side) {
    const seed = hashText(id);
    const item = REF_COLLECTIONS.flatMap(refItems).find((i) => i.id === id);
    const landscape = !item || item.width >= item.height;
    const [w, h] = landscape ? [side, Math.round(side * 0.75)] : [Math.round(side * 0.75), side];
    return png(w, h, (ctx) => paintAbstract(ctx, w, h, seed));
  }

  // ------------------------------------------------------------------ commandes

  const RECENTS = [
    { path: "/home/alex/Projects/Summer poster.graf", name: "Summer poster.graf", width: 2480, height: 3508, at: NOW - 2 * 3600000, pinned: true },
    { path: "/home/alex/Projects/Album cover.graf", name: "Album cover.graf", width: 3000, height: 3000, at: NOW - 26 * 3600000, pinned: false },
    { path: "/home/alex/Projects/Web banner.graf", name: "Web banner.graf", width: 1920, height: 600, at: NOW - 3 * DAY, pinned: false },
    { path: "/home/alex/Pictures/Lake at dusk.jpg", name: "Lake at dusk.jpg", width: 4000, height: 2667, at: NOW - 6 * DAY, pinned: false },
  ];
  const SETTINGS = {
    language: { locale: "en" },
    appearance: { theme: "legacy", accent: "#ec2a3c" },
    recents: RECENTS,
    myref: { root: REF_ROOT },
    ...(window.__MOCK_SETTINGS__ || {}),
  };

  const docOf = (args) => {
    const doc = docs.get(args?.docId);
    if (!doc) throw `document inconnu : ${args?.docId}`;
    return doc;
  };

  const handlers = {
    // Événements et fenêtre.
    "plugin:event|listen": ({ event, handler }) => {
      if (!listeners.has(event)) listeners.set(event, []);
      listeners.get(event).push(handler);
      return handler;
    },
    "plugin:event|unlisten": ({ event, eventId }) => {
      const list = listeners.get(event) || [];
      const i = list.indexOf(eventId);
      if (i >= 0) list.splice(i, 1);
      return null;
    },
    "plugin:event|emit": ({ event, payload }) => (emit(event, payload), null),
    "plugin:event|emit_to": ({ event, payload }) => (emit(event, payload), null),
    "plugin:app|version": () => "1.2.88",
    "plugin:app|name": () => "Lumitype",
    "plugin:app|tauri_version": () => "2.12.0",
    "plugin:window|is_maximized": () => false,
    "plugin:window|is_fullscreen": () => false,
    "plugin:window|is_focused": () => true,
    "plugin:window|is_decorated": () => false,
    "plugin:window|is_visible": () => true,
    "plugin:window|scale_factor": () => window.devicePixelRatio || 1,
    "plugin:window|inner_size": () => ({ width: window.innerWidth, height: window.innerHeight }),
    "plugin:window|outer_size": () => ({ width: window.innerWidth, height: window.innerHeight }),
    "plugin:window|inner_position": () => ({ x: 0, y: 0 }),
    "plugin:window|outer_position": () => ({ x: 0, y: 0 }),
    "plugin:window|theme": () => "dark",
    "plugin:window|title": () => "Lumitype",

    // Réglages.
    load_settings: ({ name }) => (name in SETTINGS && SETTINGS[name] != null ? JSON.stringify(SETTINGS[name]) : null),
    save_settings: ({ name, content }) => {
      try {
        SETTINGS[name] = JSON.parse(content);
      } catch {
        // contenu non JSON : ignoré
      }
      return null;
    },

    // Système.
    system_info: () => ({ cores: 12, ramMb: 32768, vramMb: 8192, vramShared: false }),
    system_gpus: () => ["NVIDIA GeForce RTX 3060"],
    memory_status: () => ({ capMb: 19660, ramMb: 32768, documentsMb: 412, historyMb: 96, cacheMb: 238, scratchBytes: 0, scratchDir: "/home/alex/.cache/lumitype/scratch" }),
    set_performance: () => null,
    startup_files: () => [],
    recovery_list: () => [],
    raw_close_all: () => null,
    close_splash: () => null,
    bug_report_context: () => ({ os: "Linux (Fedora Linux 44)", arch: "x86_64", logsDir: "/home/alex/.cache/lumitype" }),
    file_exists: () => true,
    directory_exists: () => true,
    clipboard_fingerprint: () => 0,

    // Bibliothèques.
    list_fonts: () =>
      ["Geist", "Inter", "Noto Sans", "Noto Serif", "DejaVu Sans", "Liberation Serif", "Source Code Pro"].map((family, i) => ({
        family,
        styles: [400, 700].flatMap((weight) => [{ weight, italic: false }, { weight, italic: true }]),
        bundled: i === 0,
      })),
    bundled_font_file: () => new ArrayBuffer(0),
    list_brushes: () => [],
    brush_library: () => ({ tips: [], presets: [] }),
    pattern_catalog: () => [
      { id: "dots", name: "Dots", width: 16, height: 16 },
      { id: "stripes", name: "Stripes", width: 16, height: 16 },
      { id: "grid", name: "Grid", width: 16, height: 16 },
    ],
    list_templates: () => [
      { id: "poster", file: "poster.graf", name: "Event poster", description: "A4 poster with title, photo and shapes." },
      { id: "social", file: "social.graf", name: "Social post", description: "Square post for social networks." },
      { id: "card", file: "card.graf", name: "Business card", description: "85 × 55 mm with bleed." },
      { id: "cover", file: "cover.graf", name: "Album cover", description: "3000 × 3000 px artwork." },
    ],
    template_preview: ({ id }) => png(320, 240, (ctx, w, h) => paintAbstract(ctx, w, h, hashText(id))),
    load_preview: ({ key }) => png(320, 240, (ctx, w, h) => paintAbstract(ctx, w, h, hashText(key))),
    save_preview: () => null,
    cmyk_profiles: () => [
      { id: "fogra39", name: "Coated FOGRA39 (ISO 12647-2:2004)", short: "FOGRA39", builtin: true, origin: "builtin", path: null },
      { id: "fogra51", name: "PSO Coated v3 (FOGRA51)", short: "FOGRA51", builtin: true, origin: "builtin", path: null },
      { id: "fogra52", name: "PSO Uncoated v3 (FOGRA52)", short: "FOGRA52", builtin: true, origin: "builtin", path: null },
      { id: "gracol", name: "GRACoL 2013 (CRPC6)", short: "GRACoL", builtin: true, origin: "builtin", path: null },
      { id: "swop", name: "U.S. Web Coated (SWOP) v2", short: "SWOP", builtin: true, origin: "builtin", path: null },
    ],
    cmyk_profile_info: ({ profile }) => ({ id: profile, name: profile, short: profile.toUpperCase(), builtin: true }),
    cmyk_of: ({ color }) => {
      const [r, g, b] = color.map((v) => v / 255);
      const k = 1 - Math.max(r, g, b);
      const f = (v) => (k >= 1 ? 0 : Math.round(((1 - v - k) / (1 - k)) * 100));
      return [f(r), f(g), f(b), Math.round(k * 100)];
    },
    cmyk_gamut: ({ color }) => ({ printable: color, deltaE: 0, outOfGamut: false }),

    // Documents.
    new_document: () => makeDocument(scene, scene === "artboards" ? "Social campaign" : "Summer poster").info,
    open_template: () => makeDocument(scene, "Event poster").info,
    close_document: ({ docId }) => (docs.delete(docId), null),
    get_tile: (args) => tile(docOf(args), args.level, args.x, args.y),
    get_tile_part: (args) => tile(docOf(args), args.level, args.x, args.y),
    get_selection_tile: () => {
      const buffer = new ArrayBuffer(9);
      new DataView(buffer).setBigUint64(0, 1n, true);
      return buffer;
    },
    thumbnail: (args) => thumbnail(docOf(args), args.layer, args.mask),
    // Composition par la carte graphique : en-tête seul (taille annoncée, pixels absents), l'interface retombe sur
    // les tuiles composées ci-dessus, plus simples à simuler.
    layer_pixels: (args) => {
      const layer = docOf(args).layers.find((l) => l.info.id === args.layer);
      const out = new ArrayBuffer(16);
      const view = new DataView(out);
      view.setUint32(8, layer ? layer.size[0] : 0, true);
      view.setUint32(12, layer ? layer.size[1] : 0, true);
      return out;
    },
    layer_mask: () => new ArrayBuffer(20),
    layer_effects: () => ({ under: [], over: [] }),
    adjust_plan: () => null,
    move_plan: () => null,
    move_layout: () => null,
    content_bounds: (args) => {
      const doc = docOf(args);
      const layers = doc.layers.filter((l) => args.ids.includes(l.info.id) && l.size[0] > 0);
      if (layers.length === 0) return null;
      const xs = layers.flatMap((l) => [l.info.offset[0], l.info.offset[0] + l.size[0]]);
      const ys = layers.flatMap((l) => [l.info.offset[1], l.info.offset[1] + l.size[1]]);
      return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
    },
    other_bounds: () => [],
    pick_layer: () => null,
    pick_text_layer: () => null,
    sample_color: () => null,
    edit: (args) => edit(docOf(args), args.op),
    autosave: () => null,
    smart_links_check: (args) => ({ ...result(docOf(args), false), updated: [], missing: [], failed: [] }),
    document_preview: (args) => documentPng(docOf(args), args.maxSide),
    preflight: (args) => ({
      ink: { limit: 300, maxPercent: 262, overPixels: 0, cell: 16, cols: 0, rows: 0, mask: [], regions: [] },
      bleedFilled: [true, true, true, true],
      textsOutside: [],
    }),
    ink_report: () => ({ limit: 300, maxPercent: 262, overPixels: 0, cell: 16, cols: 0, rows: 0, mask: [], regions: [] }),

    // Export : aperçu = 16 octets (poids u64, largeur, hauteur) puis une image affichable.
    export_preview_release: () => null,
    export_exact_size: () => 1843200,
    layer_file_stem: ({ name }) => name.toLowerCase().replace(/\s+/g, "-"),

    // My ref.
    myref_overview: () => refOverview(),
    myref_collection: ({ name }) => {
      const c = REF_COLLECTIONS.find((x) => x.name === name);
      return { name, items: c ? refItems(c) : [], cover: null, sort: "recent", modified: NOW };
    },
    myref_search: ({ query }) => REF_COLLECTIONS.flatMap(refItems).filter((i) => i.name.includes(String(query).toLowerCase())),
    myref_memberships: ({ id }) => [id.split("/")[0]],
    myref_thumbnail: ({ id }) => refImage(id, 360),
    myref_image: ({ id }) => refImage(id, 1200),
  };

  /**
   * Aperçu d'export (ExportDialog.svelte) : largeur et hauteur du fichier, de la vue d'ensemble (u32 × 4), poids (u64),
   * exact (u8), type d'image (u8 : 0 PNG), puis l'image.
   */
  async function exportOverview(args) {
    const doc = docOf(args);
    const k = Math.min(1, (args.maxSide || 1200) / Math.max(doc.info.width, doc.info.height));
    const [w, h] = [Math.round(doc.info.width * k), Math.round(doc.info.height * k)];
    const image = await png(w, h, (ctx) => ctx.drawImage(doc.composite, 0, 0, w, h));
    const out = new Uint8Array(26 + image.byteLength);
    const view = new DataView(out.buffer);
    [doc.info.width, doc.info.height, w, h].forEach((v, i) => view.setUint32(i * 4, v, true));
    view.setBigUint64(16, 1843200n, true);
    view.setUint8(24, 1);
    view.setUint8(25, 0);
    out.set(new Uint8Array(image), 26);
    return out.buffer;
  }
  /** Zone visible à pleine résolution : largeur, hauteur (u32), type (u8), puis l'image. */
  async function exportRegion(args) {
    const doc = docOf(args);
    const [x, y, w, h] = args.rect;
    const image = await png(w, h, (ctx) => ctx.drawImage(doc.composite, x * SCALE, y * SCALE, w * SCALE, h * SCALE, 0, 0, w, h));
    const out = new Uint8Array(9 + image.byteLength);
    const view = new DataView(out.buffer);
    view.setUint32(0, w, true);
    view.setUint32(4, h, true);
    out.set(new Uint8Array(image), 9);
    return out.buffer;
  }
  handlers.export_overview = exportOverview;
  handlers.export_region = exportRegion;

  const unknown = new Set();
  async function invoke(cmd, args = {}, _options) {
    const handler = handlers[cmd];
    if (!handler) {
      if (cmd.startsWith("plugin:window|") || cmd.startsWith("plugin:webview|")) return null;
      if (!unknown.has(cmd)) {
        unknown.add(cmd);
        log("inconnue :", cmd, JSON.stringify(args)?.slice(0, 200));
      }
      return null;
    }
    try {
      return await handler(args);
    } catch (error) {
      log("erreur :", cmd, String(error));
      throw error;
    }
  }

  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, data) => callbacks.get(id)?.(data),
    callbacks,
    convertFileSrc: (path, protocol = "asset") => `${protocol}://localhost/${encodeURIComponent(path)}`,
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    plugins: { path: { sep: "/", delimiter: ":" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => callbacks.delete(id) };
  // Accès pour capture.mjs : état du faux moteur et événements.
  window.__MOCK__ = { docs, emit, unknown, makeDocument };
})();
