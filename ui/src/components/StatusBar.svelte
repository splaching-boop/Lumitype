<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { measureFps } from "../lib/fps.svelte";
  import { app } from "../state/app.svelte";
  import { fade } from "svelte/transition";
  import { proofShortName } from "../lib/proof-profiles";

  // La mesure empêche la webview de se reposer : elle ne tourne que si la case des Préférences est cochée.
  $effect(() => (app.performance.showFps ? measureFps() : undefined));

  const TOOL_NAME = { move: "Déplacement — V", marquee: "Sélection — M", lasso: "Lasso — L", wand: "Baguette magique — W", brush: "Pinceau — B", eraser: "Gomme — E", text: "Texte — T", crop: "Recadrage — C", heal: "Correcteur localisé — J", quickSelect: "Sélection rapide — Maj+W", shape: "Forme — U", fill: "Dégradé / pot de peinture — G", pen: "Plume — P", pathSelect: "Sélection directe — A", eyedropper: "Pipette — I", hand: "Main — H", zoom: "Zoom — Z" } as const;
</script>

<footer>
  {#if app.active}
    {@const { info, camera } = app.active}
    <span class="zoom">{camera ? `${Math.round(camera.zoom * 1000) / 10} %` : ""}</span>
    <span>{t("{width} × {height} px ({dpi} ppp)", { width: info.width, height: info.height, dpi: info.dpi })}</span>
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
    </span>
  {/if}
  {#if app.proofProfile}
    <span class="proof" title={t("Épreuve CMJN : l'affichage simule une impression quadri selon le profil choisi (Affichage › Épreuve, Ctrl+Y pour le dernier profil). Le document et l'export restent en RVB.")}>{t("CMJN")} {proofShortName(app.proofProfile)}</span>
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
  .scale {
    color: var(--accent, #ec2a3c);
    font-variant-numeric: tabular-nums;
  }
  .proof {
    color: var(--pill-text);
    background: #ffb84d;
    border-radius: var(--radius-pill);
    padding: 0 7px;
    font-weight: 600;
    font-size: 10px;
    line-height: 16px;
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
    background: rgba(255, 255, 255, 0.1);
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
</style>
