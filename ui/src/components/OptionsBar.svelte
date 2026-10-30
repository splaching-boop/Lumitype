<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { isActionEnabled, runAction } from "../lib/actions";
  import { editLatest } from "../lib/engine";
  import { resolveStops, toCss } from "../lib/gradient";
  import { backgroundInMode, foregroundInMode } from "../lib/color-mode.svelte";
  import BrushPicker from "./BrushPicker.svelte";
  import GradientEditor from "./GradientEditor.svelte";
  import type { IconName } from "../lib/icons";
  import { app, type ToolId } from "../state/app.svelte";
  import Icon from "./Icon.svelte";
  import { currentVariant } from "../lib/tool-groups.svelte";
  import { normalizeAngle, rotationOf } from "../viewport/camera";

  const TOOL_ICON: Record<ToolId, IconName> = { move: "move", marquee: "marquee", lasso: "lasso", wand: "wand", brush: "brush", eraser: "eraser", text: "type", crop: "crop", heal: "heal", quickSelect: "quickSelect", shape: "shape", fill: "bucket", eyedropper: "pipette", pen: "pen", pathSelect: "pathSelect", hand: "hand", rotateView: "rotateView", zoom: "zoom" };
  /** Icône de l'outil en main : celle de sa variante (Ellipse de sélection, Dégradé…) quand c'est un outil empilé. */
  const toolIcon = $derived(currentVariant()?.icon ?? TOOL_ICON[app.tool]);

  /** Rotation de l'affichage du document actif, en degrés (sens horaire), arrondie au dixième. */
  const viewAngle = $derived(Math.round((rotationOf(app.active?.camera) * 1800) / Math.PI) / 10);
  function setViewAngle(degrees: number): void {
    if (!Number.isFinite(degrees)) return;
    app.setViewRotation(normalizeAngle((degrees * Math.PI) / 180));
  }
  const HINT: Partial<Record<ToolId, string>> = {
    text: "Clic : écrire sur l'image · clic dans un texte : le modifier · Ctrl+Entrée : valider · Échap : annuler",
    eyedropper: "Clic ou glisser : prélever la couleur de premier plan · Alt avec le pinceau ou le pot de peinture : pipette temporaire",
    hand: "Glisser pour déplacer la vue · Espace : main temporaire",
    zoom: "Clic : zoom avant · Alt+clic : zoom arrière · Alt+molette : zoom continu",
  };

  const settings = $derived(app.paintTool ? app.brushes[app.paintTool] : null);

  /** Lecture du cadre de transformation : taille relative et angle du côté haut. */
  const transformReadout = $derived.by(() => {
    // Le cadre de l'outil Déplacement ne prend la barre d'options qu'une fois une poignée tirée.
    const t = app.transform && (!app.transform.auto || app.transform.previewed) ? app.transform : null;
    const [s0, s1, , s3] = t?.source ?? [];
    const [q0, q1, , q3] = t?.quad ?? [];
    if (!s0 || !s1 || !s3 || !q0 || !q1 || !q3) return null;
    const ratio = (a: typeof q0, b: typeof q0, c: typeof s0, d: typeof s0) => Math.hypot(b.x - a.x, b.y - a.y) / Math.max(1e-6, Math.hypot(d.x - c.x, d.y - c.y));
    return {
      width: Math.round(ratio(q0, q1, s0, s1) * 1000) / 10,
      height: Math.round(ratio(q0, q3, s0, s3) * 1000) / 10,
      angle: Math.round((Math.atan2(q1.y - q0.y, q1.x - q0.x) * 1800) / Math.PI) / 10,
    };
  });

  const press = (key: string) => window.dispatchEvent(new KeyboardEvent("keydown", { key }));

  const CROP_RATIOS = [
    { label: "Libre", value: null },
    { label: "1:1", value: 1 },
    { label: "4:3", value: 4 / 3 },
    { label: "3:2", value: 3 / 2 },
    { label: "16:9", value: 16 / 9 },
    { label: "9:16", value: 9 / 16 },
  ];

  let gradientEditor = $state<{ x: number; y: number } | null>(null);

  /**
   * Option « Texte dynamique » de l'outil Texte : vaut pour les prochains cadres tracés, et pour le texte de paragraphe
   * actif s'il y en a un (un seul état d'historique).
   */
  function setTextDynamic(on: boolean): void {
    app.textDynamic = on;
    const layer = app.activeLayer;
    if (!layer?.text?.frame || !!layer.text.dynamic === on) return;
    const text = { ...$state.snapshot(layer.text), ...(app.textEdit?.layerId === layer.id ? { content: app.textEdit.draft, runs: $state.snapshot(app.textEdit.runs) } : {}) };
    const session = app.textEdit?.layerId === layer.id ? app.textEdit.id : `texte-dynamique-${layer.id}-${Date.now()}`;
    editLatest(session, { op: "setText", id: layer.id, text: { ...text, dynamic: on ? (text.dynamic ?? { lines: 0, manual: false, gap: 0.12 }) : null }, session });
  }
</script>

<div class="options">
  {#if app.quickMask}
    <button class="quick" title={t("Quitter le masque rapide (Q)")} onclick={() => (app.quickMask = false)}>{t("Masque rapide · peindre : sélectionner · Alt : désélectionner · Q : quitter")}</button>
  {/if}
  <span class="tool"><Icon name={toolIcon} /></span>
  <div class="rule"></div>
  {#if transformReadout}
    <span class="hint">{t("Transformation")}</span>
    <span>L {transformReadout.width} %</span>
    <span>H {transformReadout.height} %</span>
    <span>{transformReadout.angle}°</span>
    <span class="hint">{t("Coin : proportions conservées (Maj : libres) · côté : une dimension (Maj : proportions) · Alt : depuis le centre · Ctrl : déformer un coin · Ctrl+Maj + côté : incliner · près d'un angle, à l'extérieur : rotation (Maj : par pas de 15°) · Ctrl+Z : revenir d'un pas")}</span>
    <div class="spacer"></div>
    <button onclick={() => press("Escape")}>{t("Annuler · Échap")}</button>
    <button class="primary" onclick={() => press("Enter")}>{t("Valider · Entrée")}</button>
  {:else if settings}
    <BrushPicker />
    <label title={t("Touches [ et ]")}>
      {t("Taille")}
      <input type="range" min="1" max="500" bind:value={settings.size} />
      <input type="number" min="1" max="2500" bind:value={settings.size} /> px
    </label>
    <label>
      {t("Dureté")}
      <input type="range" min="0" max="1" step="0.01" bind:value={settings.hardness} />
      <output>{Math.round(settings.hardness * 100)} %</output>
    </label>
    <label title={t("Touches 1 à 9, 0 = 100 %")}>
      {t("Opacité")}
      <input type="range" min="0.01" max="1" step="0.01" bind:value={settings.opacity} />
      <output>{Math.round(settings.opacity * 100)} %</output>
    </label>
    <label>
      {t("Lissage")}
      <input type="range" min="0" max="100" value={Math.round(app.strokeSmoothing * 100)} oninput={(event) => (app.strokeSmoothing = event.currentTarget.valueAsNumber / 100)} />
      <output>{Math.round(app.strokeSmoothing * 100)} %</output>
    </label>
    <label class="check"><input type="checkbox" bind:checked={settings.pressureSize} /> {t("Pression → taille")}</label>
    <label class="check"><input type="checkbox" bind:checked={settings.pressureOpacity} /> {t("Pression → opacité")}</label>
  {:else if app.tool === "marquee" || app.tool === "lasso"}
    <div class="segmented">
      {#if app.tool === "marquee"}
        <button class:on={app.marqueeShape === "rect"} onclick={() => (app.marqueeShape = "rect")}>{t("Rectangle")}</button>
        <button class:on={app.marqueeShape === "ellipse"} onclick={() => (app.marqueeShape = "ellipse")}>{t("Ellipse")}</button>
      {:else}
        <button class:on={app.lassoShape === "free"} onclick={() => (app.lassoShape = "free")}>{t("Libre")}</button>
        <button class:on={app.lassoShape === "polygon"} onclick={() => (app.lassoShape = "polygon")}>{t("Polygonal")}</button>
      {/if}
    </div>
    <span class="hint">
      {t("Maj : ajouter · Alt : soustraire · Maj+Alt : intersection")}
      {app.tool === "lasso" && app.lassoShape === "polygon" ? t(" · double-clic ou Entrée : fermer · Échap : annuler") : ""}
    </span>
  {:else if app.tool === "quickSelect"}
    <label title={t("Touches [ et ]")}>
      {t("Taille")}
      <input type="range" min="4" max="300" bind:value={app.quickSelect.size} />
      <output>{Math.round(app.quickSelect.size)} px</output>
    </label>
    <label>
      {t("Sensibilité")}
      <input type="range" min="4" max="100" bind:value={app.quickSelect.tolerance} />
      <output>{app.quickSelect.tolerance}</output>
    </label>
    <label class="check"><input type="checkbox" bind:checked={app.wand.allLayers} /> {t("Tous les calques")}</label>
    <span class="hint">{t("Peignez sur le sujet : la sélection s'étend jusqu'à ses contours · chaque trait ajoute · Alt : retirer · Maj+W : baguette magique")}</span>
  {:else if app.tool === "heal"}
    <label title={t("Touches [ et ]")}>
      {t("Taille")}
      <input type="range" min="4" max="400" bind:value={app.heal.size} />
      <input type="number" min="1" max="1000" bind:value={app.heal.size} /> px
    </label>
    <label>
      {t("Dureté")}
      <input type="range" min="0" max="1" step="0.01" bind:value={app.heal.hardness} />
      <output>{Math.round(app.heal.hardness * 100)} %</output>
    </label>
    <span class="hint">{t("Peignez sur le défaut en le débordant un peu : au relâchement, il est remplacé par la matière voisine, raccordée en lumière et en couleur")}</span>
  {:else if app.tool === "shape"}
    <div class="segmented">
      <button class:on={app.shapeKind === "rect"} onclick={() => (app.shapeKind = "rect")}>{t("Rectangle")}</button>
      <button class:on={app.shapeKind === "ellipse"} onclick={() => (app.shapeKind = "ellipse")}>{t("Ellipse")}</button>
      <button class:on={app.shapeKind === "polygon" && app.shapeSides === 3} onclick={() => ((app.shapeKind = "polygon"), (app.shapeSides = 3))}>{t("Triangle")}</button>
      <button class:on={app.shapeKind === "polygon" && app.shapeSides !== 3} onclick={() => ((app.shapeKind = "polygon"), (app.shapeSides = Math.max(5, app.shapeSides)))}>{t("Polygone")}</button>
    </div>
    {#if app.shapeKind === "polygon"}
      <label>
        {t("Côtés")}
        <input type="number" min="3" max="64" bind:value={app.shapeSides} />
      </label>
    {/if}
    <span class="hint">{t("Glisser pour tracer · double-clic : choisir le nombre de côtés et la taille · Maj : proportions égales · Alt : depuis le centre · arrondi, fond et contour se règlent ensuite dans Propriétés")}</span>
  {:else if app.tool === "crop"}
    <div class="segmented">
      {#each CROP_RATIOS as ratio (ratio.label)}
        <button class:on={app.cropRatio === ratio.value} onclick={() => (app.cropRatio = ratio.value)}>{t(ratio.label)}</button>
      {/each}
    </div>
    <span class="hint">{t("Tirer les poignées ou déplacer le cadre · il peut dépasser pour agrandir la zone de travail · Maj : garder les proportions · Entrée ou double-clic : valider · Échap : réinitialiser")}</span>
  {:else if app.tool === "fill"}
    <div class="segmented">
      <button class:on={app.fill.kind === "gradient"} onclick={() => (app.fill.kind = "gradient")}>{t("Dégradé")}</button>
      <button class:on={app.fill.kind === "bucket"} onclick={() => (app.fill.kind = "bucket")}>{t("Pot de peinture")}</button>
    </div>
    {#if app.fill.kind === "gradient"}
      <div class="segmented">
        <button class:on={!app.fill.radial} onclick={() => (app.fill.radial = false)}>{t("Linéaire")}</button>
        <button class:on={app.fill.radial} onclick={() => (app.fill.radial = true)}>{t("Radial")}</button>
      </div>
      <!-- Rectangle du dégradé courant : un clic ouvre l'éditeur. -->
      <button
        class="gradient"
        aria-label={t("Modifier le dégradé")}
        title={t("Modifier le dégradé")}
        onclick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect();
          gradientEditor = gradientEditor ? null : { x: rect.left, y: rect.bottom + 8 };
        }}
      >
        <span style:background={toCss(resolveStops(app.gradient, foregroundInMode(), backgroundInMode()))}></span>
      </button>
      <span class="hint">{t("Glisser sur l'image pour tracer le dégradé · Maj : pas de 45°")}</span>
    {:else}
      <label>
        {t("Tolérance")}
        <input type="range" min="0" max="255" bind:value={app.fill.tolerance} />
        <output>{app.fill.tolerance}</output>
      </label>
      <label class="check"><input type="checkbox" bind:checked={app.fill.contiguous} /> {t("Pixels contigus")}</label>
      <span class="hint">{t("Clic : remplir la zone avec la couleur de premier plan")}</span>
    {/if}
  {:else if app.tool === "pen" || app.tool === "pathSelect"}
    <button onclick={() => runAction("path.toSelection")} disabled={!isActionEnabled("path.toSelection")}>{t("Sélection")}</button>
    <button onclick={() => runAction("path.fill")} disabled={!isActionEnabled("path.fill")}>{t("Fond")}</button>
    <button onclick={() => runAction("path.stroke")} disabled={!isActionEnabled("path.stroke")}>{t("Contour")}</button>
    <button onclick={() => runAction("path.clear")} disabled={!isActionEnabled("path.clear")}>{t("Effacer le tracé")}</button>
    <span class="hint">
      {app.tool === "pen"
        ? t("Clic : angle · glisser : courbe · clic sur le 1er point : fermer · sur le tracé : ajouter · sur un point : retirer · Alt+clic : convertir · Entrée : terminer")
        : t("Glisser un point ou une poignée · Alt : poignée indépendante · Suppr : retirer le point")}
    </span>
  {:else if app.tool === "move"}
    <label class="check"><input type="checkbox" bind:checked={app.autoSelect} /> {t("Sélection automatique")}</label>
    <select class="auto-mode" bind:value={app.autoSelectMode} disabled={!app.autoSelect} aria-label={t("Sélection automatique")}>
      <option value="layer">{t("Calque")}</option>
      <option value="group">{t("Groupe")}</option>
    </select>
    <span class="hint">{t("Glisser : déplacer · Maj + clic : ajouter un calque à la sélection (cadre commun) · Maj en glissant : contraindre à un axe · flèches : 1 px, Maj+flèches : 10 px")}</span>
  {:else if app.tool === "rotateView"}
    <label title={t("Angle de rotation de l'affichage, en degrés (sens horaire)")}>
      {t("Angle de rotation")}
      <input
        type="number"
        class="angle"
        min="-180"
        max="180"
        step="1"
        value={viewAngle}
        disabled={!app.active?.camera}
        onchange={(event) => setViewAngle(event.currentTarget.valueAsNumber)}
        onkeydown={(event) => {
          // Entrée valide et rend le clavier à l'image (Échap, lettres des outils…).
          if (event.key !== "Enter") return;
          setViewAngle(event.currentTarget.valueAsNumber);
          event.currentTarget.blur();
        }}
      />°
    </label>
    <button onclick={() => runAction("view.resetRotation")} disabled={!app.active?.camera || viewAngle === 0}>{t("Réinitialiser la vue")}</button>
    <span class="hint">{t("Glisser sur l'image pour faire tourner la vue (les pixels ne changent pas) · Maj : pas de 15° · Échap ou double-clic sur l'outil : 0°")}</span>
  {:else if app.tool === "wand"}
    <label>
      {t("Tolérance")}
      <input type="range" min="0" max="255" bind:value={app.wand.tolerance} />
      <output>{app.wand.tolerance}</output>
    </label>
    <label class="check"><input type="checkbox" bind:checked={app.wand.contiguous} /> {t("Pixels contigus")}</label>
    <label class="check"><input type="checkbox" bind:checked={app.wand.antiAlias} /> {t("Lissage")}</label>
    <label class="check"><input type="checkbox" bind:checked={app.wand.allLayers} /> {t("Tous les calques")}</label>
    <label>
      {t("Échantillon")}
      <select bind:value={app.wand.sample}>
        <option value={1}>{t("Point")}</option>
        <option value={3}>{t("Moyenne 3 × 3")}</option>
        <option value={5}>{t("Moyenne 5 × 5")}</option>
      </select>
    </label>
    <span class="hint">{t("Cliquer puis glisser à droite ou à gauche : régler la tolérance en direct · Maj+W : sélection rapide")}</span>
    <span class="hint">{t("Maj : ajouter · Alt : soustraire")}</span>
  {:else if app.tool === "text"}
    <label class="check" title={t("Glisser un cadre avec l'outil Texte : chaque ligne prend la taille qui remplit la largeur du cadre, les mots se répartissent pour en remplir la hauteur")}>
      <input type="checkbox" checked={app.textDynamic} onchange={(event) => setTextDynamic(event.currentTarget.checked)} />
      {t("Texte dynamique")}
    </label>
    <span class="hint">{t(app.textDynamic ? "Glisser : tracer le cadre du texte dynamique · ses poignées redistribuent les lignes · clic dans un texte : le modifier" : (HINT.text ?? ""))}</span>
  {:else}
    <span class="hint">{t(HINT[app.tool] ?? "")}</span>
  {/if}
  {#if !transformReadout}
    <div class="spacer"></div>
    <button onclick={() => runAction("view.fit")} disabled={!app.active}>{t("Taille écran")}</button>
    <button onclick={() => runAction("view.actualSize")} disabled={!app.active}>100 %</button>
  {/if}
</div>

{#if gradientEditor}
  <GradientEditor x={gradientEditor.x} y={gradientEditor.y} onclose={() => (gradientEditor = null)} />
{/if}

<style>
  .quick {
    height: 26px;
    flex-shrink: 0;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: rgba(255, 60, 70, 0.22);
    border: 1px solid rgba(255, 90, 100, 0.6);
    color: #ffd7da;
  }
  .gradient {
    width: 120px;
    height: 26px;
    padding: 0;
    border-radius: var(--radius-m);
    overflow: hidden;
    border: 1px solid var(--border-strong);
    background: repeating-conic-gradient(#3a3a3e 0% 25%, #232326 0% 50%) 0 0 / 10px 10px;
  }
  .gradient:hover {
    border-color: var(--text-strong);
  }
  .gradient span {
    display: block;
    width: 100%;
    height: 100%;
  }
  .options {
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 14px;
    background: var(--bg-bar-2);
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    white-space: nowrap;
  }
  .tool {
    display: grid;
    color: var(--text);
  }
  .rule {
    width: 1px;
    height: 18px;
    background: var(--border-strong);
  }
  .hint {
    color: var(--text-3);
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-3);
  }
  input[type="range"] {
    width: 96px;
  }
  input[type="number"] {
    width: 58px;
    height: 26px;
    padding: 0 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    text-align: right;
    user-select: text;
  }
  input[type="checkbox"] {
    accent-color: var(--pill-bg);
  }
  output {
    width: 40px;
    color: var(--text);
  }
  .check {
    color: var(--text-2);
  }
  .spacer {
    flex: 1;
  }
  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-field);
    border: 1px solid var(--border);
  }
  .segmented button {
    height: 22px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text-strong);
  }
  button {
    height: 26px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    background: var(--bg-field);
    border: 1px solid var(--border);
    transition: border-color 120ms var(--ease);
  }
  button:not(:disabled):hover {
    border-color: var(--border-strong);
    color: var(--text);
  }
  button:disabled {
    color: var(--text-disabled);
  }
  button.primary {
    border: 0;
    border-radius: var(--radius-pill);
    padding: 0 14px;
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  button.primary:hover {
    color: var(--pill-text);
  }
  .auto-mode {
    height: 24px;
    margin-left: -4px;
    padding: 0 6px;
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
  }
</style>
