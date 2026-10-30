<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { measureFps } from "../lib/fps.svelte";
  import { app } from "../state/app.svelte";
  import { fade } from "svelte/transition";
  import { profileShort } from "../lib/color-settings.svelte";
  import { activeColorMode, effectiveProof, workflowSpace } from "../lib/color-mode.svelte";
  import { formatLength, ink, printPrefs } from "../lib/print.svelte";
  import { formatWork, workTime } from "../lib/work-time.svelte";
  import type { LayerInfo } from "../lib/ipc";

  /** Détail du nombre de calques par type, pour le survol. */
  function layerKinds(layers: LayerInfo[]): { detail: string } {
    const count = { pixels: 0, text: 0, shape: 0, adjust: 0, fill: 0, smart: 0, group: 0 };
    for (const l of layers) {
      if (l.isGroup) count.group++;
      else if (l.text) count.text++;
      else if (l.shape) count.shape++;
      else if (l.adjustment) count.adjust++;
      else if (l.fillLayer) count.fill++;
      else if (l.smart) count.smart++;
      else count.pixels++;
    }
    const parts = [
      [count.pixels, t("pixels")],
      [count.text, t("textes")],
      [count.shape, t("formes")],
      [count.adjust, t("réglages")],
      [count.fill, t("remplissages")],
      [count.smart, t("objets dynamiques")],
      [count.group, t("groupes")],
    ]
      .filter(([n]) => (n as number) > 0)
      .map(([n, label]) => `${n} ${label}`);
    return { detail: parts.join(" · ") };
  }

  // La mesure empêche la webview de se reposer : elle ne tourne que si la case des Préférences est cochée.
  $effect(() => (app.performance.showFps ? measureFps() : undefined));

  const TOOL_NAME = { move: "Déplacement — V", marquee: "Sélection — M", lasso: "Lasso — L", wand: "Baguette magique — W", brush: "Pinceau — B", eraser: "Gomme — E", text: "Texte — T", crop: "Recadrage — C", heal: "Correcteur localisé — J", quickSelect: "Sélection rapide — Maj+W", shape: "Forme — U", fill: "Dégradé / pot de peinture — G", pen: "Plume — P", pathSelect: "Sélection directe — A", eyedropper: "Pipette — I", hand: "Main — H", rotateView: "Rotation de l'affichage — R", zoom: "Zoom — Z" } as const;
</script>

<footer>
  {#if app.active}
    {@const { info, camera } = app.active}
    <span class="zoom">{camera ? `${Math.round(camera.zoom * 1000) / 10} %` : ""}</span>
    {@const work = app.active.workSeconds}
    <span
      class="stat"
      class:paused={workTime.idle}
      title={[t("Temps passé sur ce projet : {time}", { time: formatWork(work) }), t("Cette session : {time}", { time: formatWork(Math.max(0, work - app.active.workAtOpen)) }), t("Enregistré avec le projet (.graf), il reprend à l'ouverture."), t("Seul le temps actif compte : pause après 5 min d'inactivité.")].join("\n")}
    >
      <svg viewBox="0 0 12 12" aria-hidden="true">{#if workTime.idle}<circle cx="6" cy="6" r="4.6" /><path d="M5 4v4M7 4v4" />{:else}<circle cx="6" cy="6" r="4.6" /><path d="M6 3.4V6l1.8 1.2" />{/if}</svg>
      {formatWork(work)}{workTime.idle ? ` · ${t("en pause")}` : ""}
    </span>
    {@const kinds = layerKinds(info.layers)}
    <span class="stat" title={kinds.detail}>
      <svg viewBox="0 0 12 12" aria-hidden="true"><path d="M6 1.5 10.5 4 6 6.5 1.5 4zM1.5 6.2 6 8.7l4.5-2.5M1.5 8.4 6 10.9l4.5-2.5" /></svg>
      {info.layers.length === 1 ? t("1 calque") : t("{n} calques", { n: info.layers.length })}
    </span>
    <!-- Vue tournée (outil Rotation de l'affichage) : l'angle est rappelé, l'image elle-même n'a pas tourné. -->
    {#if camera?.rotation}<span title={t("Rotation de l'affichage")}>↻ {Math.round((camera.rotation * 1800) / Math.PI) / 10}°</span>{/if}
    <span>{t("{width} × {height} px ({dpi} ppp)", { width: info.width, height: info.height, dpi: info.dpi })}</span>
    {#if info.print && info.trim}
      <span title={t("Format fini (sans les fonds perdus)")}>{formatLength(info.trim[2], "mm", info.dpi)} × {formatLength(info.trim[3], "mm", info.dpi)}</span>
    {/if}
    {#if ink.docId === info.id && ink.report && ink.report.overPixels > 0}
      <button class="ink" title={t("Encrage au-delà de la limite : Image › Corriger l'encrage… ou Vérifier avant impression…")} onclick={() => (app.preflightOpen = true)}>
        ⚠ {ink.report.regions.length > 1 ? t("{count} zones > {limit} % d'encre", { count: ink.report.regions.length, limit: ink.report.limit }) : t("1 zone > {limit} % d'encre", { limit: ink.report.limit })}
      </button>
    {/if}
    {#if app.pointer}<span>X {app.pointer.x} · Y {app.pointer.y}</span>{/if}
    {#if info.smartParent}
      {@const parent = app.documents.find((d) => d.info.id === info.smartParent?.docId)}
      <!-- Contenu d'un objet dynamique : on rappelle d'où il vient et ce que fait Ctrl+S. -->
      <span class="smart" title={t("Fermer sans enregistrer ne change rien à l'objet dynamique.")}>
        {parent ? t("Contenu de « {name} » dans {document} · Ctrl+S met à jour l'objet", { name: info.smartParent.name, document: parent.info.name }) : t("Contenu de « {name} » · son document a été fermé", { name: info.smartParent.name })}
      </span>
    {/if}
  {/if}
  <div class="spacer"></div>
  {#if app.busy}<span class="busy" role="status">{app.busy}</span>{/if}
  {#if app.loading}
    {@const ratio = app.loading.total > 0 ? app.loading.done / app.loading.total : 0}
    <!-- Jauge discrète : les calques du projet arrivent encore sur la carte graphique. -->
    <span class="loading" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(ratio * 100)} out:fade={{ duration: 250 }} title={t("{done} calques sur {total} chargés", { done: app.loading.done, total: app.loading.total })}>
      {t("Chargement")}
      <i><b style:width="{ratio * 100}%"></b></i>
      {Math.round(ratio * 100)} %
    </span>
  {/if}
  {#if app.fps}
    {@const ideal = 1000 / Math.max(app.fps.rate, 1)}
    <span class="fps" title={t("Images par seconde de l'affichage, et durée de la plus longue image de la dernière demi-seconde. C'est cette seconde valeur qu'on ressent : une image longue est un à-coup, même si la moyenne est bonne.")}>
      {t("{rate} i/s · pire", { rate: app.fps.rate })} <strong class:hitch={app.fps.worstMs > ideal * 1.5}>{app.fps.worstMs.toFixed(1).replace(".", ",")} ms</strong>
      {#if app.fps.drawMs > 0 || app.fps.gpuMs > 0}
        <span class="cost" title={t("Pire image de la zone de travail : temps passé dans le code (JS) puis temps attendu de la carte graphique.")}>· JS {app.fps.drawMs.toFixed(1).replace(".", ",")} · {t("carte")} {app.fps.gpuMs.toFixed(1).replace(".", ",")} ms</span>
      {/if}
      <!-- Diagnostic (avec la mesure des images par seconde) : qui compose l'image, et comment se fait le dernier déplacement. -->
      {#if app.gpuStatus}
        {@const move = app.gpuStatus.move}
        <span class="cost">· {app.gpuStatus.active ? t("composition : carte graphique") : t("composition : moteur ({reason})", { reason: t(app.gpuStatus.reason ?? "") })}</span>
        {#if move}
          <span class="cost" title={move.detail}>· {move.mode === "cache" ? t("déplacement : images en cache ({ms} ms)", { ms: move.ms ?? 0 }) : move.mode === "collapse" ? t("déplacement : groupe aplati ({ms} ms)", { ms: move.ms ?? 0 }) : move.mode === "pending" ? t("déplacement : préparation…") : move.mode === "gpu" ? t("déplacement : carte graphique, calque par calque") : t("déplacement : moteur")}</span>
        {/if}
      {/if}
    </span>
  {/if}
  {#if app.active && activeColorMode().kind === "grayscale"}
    <span class="proof" title={t("Mode Niveaux de gris (Image › Mode) : l'image et ses exports sont en gris ; les couleurs choisies sont ramenées au gris.")}><svg viewBox="0 0 12 12" aria-hidden="true"><rect x="1" y="1" width="10" height="10" rx="2" fill="url(#mode-gray)" /><defs><linearGradient id="mode-gray"><stop offset="0" stop-color="#111" /><stop offset="1" stop-color="#eee" /></linearGradient></defs></svg>{t("Niveaux de gris")}</span>
  {:else if app.active && activeColorMode().kind === "bitmap"}
    <span class="proof" title={t("Mode Bitmap (Image › Mode) : noir et blanc seulement, 1 bit à l'export ; ce qui est peint en gris ou à demi transparent est seuillé à 50 %.")}><svg viewBox="0 0 12 12" aria-hidden="true"><rect x="1" y="1" width="5" height="5" fill="#111" /><rect x="6" y="1" width="5" height="5" fill="#eee" /><rect x="1" y="6" width="5" height="5" fill="#eee" /><rect x="6" y="6" width="5" height="5" fill="#111" /></svg>{t("Bitmap")}</span>
  {/if}
  {#if app.active?.info.print && (printPrefs.showBleed || printPrefs.showSafe)}
    <!-- Légende des cadres d'impression (Affichage › Impression), juste avant le profil CMJN. -->
    <span class="legend" aria-label={t("Légende des cadres d'impression")}>
      {#if printPrefs.showBleed && app.active.info.print.bleedMm > 0}
        <span><i class="bleed"></i>{t("Fonds perdus")}</span>
        <span><i class="trim"></i>{t("Format fini")}</span>
      {/if}
      {#if printPrefs.showSafe && app.active.info.print.safeMm > 0}<span><i class="safe"></i>{t("Zone de sécurité")}</span>{/if}
    </span>
  {/if}
  {#if effectiveProof()}
    {@const proof = effectiveProof()?.profile ?? "fogra39"}
    <span class="proof" title={workflowSpace() ? t("Mode Couleurs CMJN (Image › Mode) : l'épreuve écran de ce profil reste allumée, les couleurs hors gamme sont signalées dans le sélecteur et l'export propose le CMJN. Les pixels restent en RVB : repasser en RVB ne perd rien.") : t("Épreuve CMJN (Ctrl+Y) : l'affichage simule une impression quadri selon le profil des Paramètres de couleur (Édition). Le document reste en RVB ; l'export PDF, TIFF ou JPEG peut être converti en CMJN avec ce profil.")}><svg viewBox="0 0 12 12" aria-hidden="true"><circle cx="4" cy="4" r="2.6" fill="#3fb6c9" /><circle cx="8" cy="4" r="2.6" fill="#c94f97" /><circle cx="4" cy="8" r="2.6" fill="#d8c54a" /><circle cx="8" cy="8" r="2.6" fill="#55555a" /></svg>{t("CMJN")} {profileShort(proof)}</span>
  {/if}
  {#if app.viewMirrored}
    <button class="mirror" title={t("Vue miroir : l'affichage est retourné, l'image ne l'est pas. Cliquer pour revenir à la vue normale.")} onclick={() => (app.viewMirrored = false)}>⇋ {t("Miroir")}</button>
  {/if}
  {#if app.renderScale !== 1}
    <span class="scale" title={t("Résolution dynamique : la zone de travail est dessinée plus petite que l'écran pendant ce geste.")}>×{app.renderScale.toFixed(2).replace(/\.?0+$/, "").replace(".", ",")}</span>
  {/if}
  <span>{t(TOOL_NAME[app.tool])}</span>
</footer>

<style>
  .smart {
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stat {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .stat svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .stat.paused {
    color: var(--text-muted);
  }
  .legend {
    display: inline-flex;
    gap: 12px;
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .legend i {
    width: 14px;
    height: 0;
    border-top: 1.5px dashed;
  }
  .legend .bleed {
    border-color: #ff4d5e;
  }
  .legend .trim {
    border-top-style: solid;
    border-color: var(--text-2);
  }
  .legend .safe {
    border-color: #4aa3ff;
  }
  .ink {
    height: 18px;
    padding: 0 8px;
    border-radius: 9px;
    border: 1px solid rgba(255, 176, 32, 0.45);
    background: rgba(255, 176, 32, 0.14);
    color: #ffc04d;
    font-size: 11px;
  }
  .scale {
    color: var(--accent, #ec2a3c);
    font-variant-numeric: tabular-nums;
  }
  /* Épreuve CMJN : discrète, une mini pastille quadri (cyan, magenta, jaune, noir) devant le texte de la barre. */
  .proof {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text-2);
  }
  .proof svg {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
  }
  footer {
    height: 26px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 0 14px;
    background: var(--bg-bar);
    border-top: 1px solid var(--border);
    color: var(--text-muted);
  }
  .zoom,
  .busy {
    color: var(--text-2);
  }
  .loading {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 16px;
    padding: 0 8px;
    font-variant-numeric: tabular-nums;
    background: var(--bg-field);
    border: 1px solid var(--border);
  }
  .loading i {
    display: block;
    width: 90px;
    height: 3px;
    background: rgb(var(--ink) / 0.1);
  }
  .loading b {
    display: block;
    height: 100%;
    background: var(--accent, #ec2a3c);
    transition: width 120ms linear;
  }
  .spacer {
    flex: 1;
  }
  .fps {
    font-variant-numeric: tabular-nums;
  }
  .fps strong {
    font-weight: 400;
    color: var(--text-2);
  }
  /* Image nettement plus longue que la cadence moyenne : c'est l'à-coup qu'on ressent. */
  .fps strong.hitch {
    color: var(--danger);
    font-weight: 600;
  }
  .mirror {
    padding: 0 8px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--text-strong);
  }
</style>
