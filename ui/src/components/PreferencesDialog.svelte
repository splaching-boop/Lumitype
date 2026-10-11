<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { PHOTOSHOP_KEYMAP } from "../lib/keymap-photoshop";
  import { type MemoryStatus, api } from "../lib/ipc";
  import {
    ASSUMED_VRAM_MB,
    DEFAULT_PERFORMANCE,
    GPU_PERCENT_RANGE,
    MEMORY_PERCENT_RANGE,
    SCRATCH_QUOTA_RANGE,
    applyPerformance,
    gpuMemoryBudgetMb,
    gpuPercentOf,
    isIntegratedGpu,
    memoryPercentOf,
    scratchQuotaOf,
  } from "../lib/performance.svelte";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";

  // Préférences : une liste de sections à gauche, leurs réglages à droite. Tout s'applique tout de
  // suite, sans redémarrage, et s'enregistre dans le dossier de configuration de l'application.
  const SECTIONS = [
    { id: "shortcuts", label: "Raccourcis" },
    { id: "performance", label: "Performances" },
    { id: "files", label: "Fichiers" },
  ] as const;

  let dialog = $state<HTMLDialogElement>();
  let section = $state<(typeof SECTIONS)[number]["id"]>("shortcuts");
  // Ce que la machine offre : lu au démarrage (performance.svelte.ts), relu ici si cela avait échoué.
  const system = $derived(app.systemInfo);

  const perf = $derived(app.performance);
  const customized = $derived(Object.keys(keymap.overrides).length);
  // Mémoire vive : plafond = part réglée de la mémoire de la machine (voir engine::memory).
  const memoryPercent = $derived(memoryPercentOf(perf));
  const capMb = $derived(system ? Math.floor((system.ramMb * memoryPercent) / 100) : null);
  // Mémoire vidéo : part réglée de la mémoire détectée (ou 2 Gio supposés) ; sur une carte intégrée, au plus un quart de la mémoire vive.
  const gpuPercent = $derived(gpuPercentOf(perf));
  const video = $derived({ vramMb: system?.vramMb, vramShared: system?.vramShared, ramMb: system?.ramMb });
  const gpuBudgetMb = $derived(gpuMemoryBudgetMb(perf, video));
  const integrated = $derived(isIntegratedGpu(video));

  // Jauge : ce que Lumitype occupe, relevé toutes les secondes tant que la section Performances est affichée.
  let memory = $state<MemoryStatus | null>(null);
  $effect(() => {
    if (!app.preferencesOpen || section !== "performance") return;
    const poll = () => api.memoryStatus().then((status) => (memory = status), () => undefined);
    poll();
    const timer = setInterval(poll, 1000);
    return () => clearInterval(timer);
  });
  const usedMb = $derived(memory ? memory.documentsMb + memory.historyMb + memory.cacheMb : 0);
  /** Ce qui reste libre sous le plafond (le plafond peut être dépassé un instant : jamais négatif). */
  const freeMb = $derived(Math.max(0, (capMb ?? 0) - usedMb));
  /** Largeur d'un segment de la jauge, en pour cent du plafond (la somme ne dépasse jamais 100). */
  const segment = (mb: number) => (capMb ? Math.min(100, (mb / capMb) * 100) : 0);

  $effect(() => {
    if (app.preferencesOpen) dialog?.showModal();
    else dialog?.close();
  });

  // La webview de Linux (WebKitGTK) masque le vrai nom de la carte et répond « Apple GPU » : le nom est alors lu dans le système.
  let systemGpus = $state<string[]>([]);
  const onMac = /Mac/i.test(navigator.userAgent) && !/Linux/i.test(navigator.userAgent);
  const masked = $derived(!!app.gpuStatus && !onMac && /^apple gpu$/i.test(app.gpuStatus.renderer.trim()));

  onMount(() => {
    if (!app.systemInfo) api.systemInfo().then((info) => (app.systemInfo = info), () => undefined);
    api.systemGpus().then((gpus) => (systemGpus = gpus), () => undefined);
  });

  function update(patch: Partial<typeof DEFAULT_PERFORMANCE>): void {
    app.performance = { ...app.performance, ...patch };
    applyPerformance(true);
  }

  // ---- Fichiers : dossier d'enregistrement par défaut.
  let folderMissing = $state(false);
  $effect(() => {
    const folder = app.saveFolder.trim();
    folderMissing = false;
    if (folder) api.directoryExists(folder).then((exists) => (folderMissing = !exists), () => undefined);
  });

  function saveFileSettings(): void {
    const settings = { saveFolder: app.saveFolder, autosaveMinutes: app.autosaveMinutes };
    void api.saveSettings("files", JSON.stringify(settings, null, 2)).catch((error: unknown) => app.fail(error));
  }

  function setFolder(folder: string): void {
    app.saveFolder = folder;
    saveFileSettings();
  }

  async function chooseFolder(): Promise<void> {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: app.saveFolder || undefined });
      if (typeof picked === "string") setFolder(picked);
    } catch (error) {
      app.fail(error);
    }
  }

  // ---- Disque de travail de l'historique : dossier du fichier d'échange (vide = cache de l'application).
  async function chooseScratchDir(): Promise<void> {
    try {
      const picked = await open({ directory: true, multiple: false, defaultPath: perf.scratchDir || memory?.scratchDir || undefined });
      if (typeof picked === "string") update({ scratchDir: picked });
    } catch (error) {
      app.fail(error);
    }
  }
  const scratchQuota = $derived(scratchQuotaOf(perf));
  /** Octets du disque de travail, en Mio arrondis (la jauge parle en Mio et Gio). */
  const scratchMb = $derived(Math.round((memory?.scratchBytes ?? 0) / (1024 * 1024)));

  const gib = (mb: number) => (mb >= 1024 ? `${(mb / 1024).toFixed(mb % 1024 === 0 ? 0 : 1).replace(".", ",")} Gio` : `${mb} Mio`);
</script>

<dialog bind:this={dialog} onclose={() => (app.preferencesOpen = false)}>
  <header>
    <h2>{t("Préférences")}</h2>
    <button class="icon" aria-label={t("Fermer")} onclick={() => (app.preferencesOpen = false)}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
  </header>
  <div class="body">
    <nav aria-label={t("Sections")}>
      {#each SECTIONS as item (item.id)}
        <button class:on={section === item.id} onclick={() => (section = item.id)}>{t(item.label)}</button>
      {/each}
    </nav>

    {#if section === "shortcuts"}
      <section>
        <h3>{t("Disposition des raccourcis")}</h3>
        <div class="card">
          <div>
            <strong>{t("Photoshop")}</strong>
            <span>{t("Disposition par défaut : {length} raccourcis, claviers AZERTY et QWERTY pris en charge.", { length: PHOTOSHOP_KEYMAP.length })}</span>
          </div>
          <span class="tag">{customized === 0 ? t("Active") : customized > 1 ? t("Active · {n} modifiés", { n: customized }) : t("Active · 1 modifié")}</span>
        </div>
        <p>{t("Chaque raccourci peut être remplacé ; un conflit est signalé au moment de la saisie. Vos changements sont enregistrés dans")} <code>{t("keymap.json")}</code>.</p>
        <div class="actions">
          <button
            class="pill"
            onclick={() => {
              app.preferencesOpen = false;
              app.shortcutsDialogOpen = true;
            }}
          >
            {t("Personnaliser les raccourcis…")}
          </button>
          <button class="ghost" disabled={customized === 0} onclick={() => keymap.resetAll()}>{t("Rétablir la disposition Photoshop")}</button>
        </div>
      </section>
    {:else if section === "files"}
      <section>
        <h3>{t("Sauvegarde automatique")}</h3>
        <label class="slider">
          <span>{t("Toutes les")}</span>
          <output>{t("{autosaveMinutes} min", { autosaveMinutes: app.autosaveMinutes })}</output>
          <input type="range" min="2" max="60" step="1" bind:value={app.autosaveMinutes} onchange={saveFileSettings} />
        </label>
        <p>{t("Chaque document modifié est copié, historique compris, dans un dossier de secours à part : votre fichier n'est jamais écrit à votre place. Après une fermeture imprévue, Lumitype propose de tout récupérer au démarrage. Enregistrer ou fermer un document supprime sa copie. Pendant la copie, un filet en haut de la fenêtre avance ; vous pouvez continuer à travailler : ce que vous faites alors sera dans la copie suivante.")}</p>
        <h3>{t("Dossier d'enregistrement par défaut")}</h3>
        <div class="folder">
          <input type="text" readonly value={app.saveFolder || t("Celui que propose le système")} class:empty={!app.saveFolder} />
          <button class="pill" onclick={chooseFolder}>{t("Choisir…")}</button>
          <button class="ghost" disabled={!app.saveFolder} onclick={() => setFolder("")}>{t("Effacer")}</button>
        </div>
        {#if folderMissing}
          <p class="warning">{t("Ce dossier est introuvable pour l'instant (renommé, supprimé, ou disque débranché). Rien n'est bloqué : tant qu'il manque, les boîtes d'enregistrement s'ouvrent à l'emplacement habituel du système.")}</p>
        {/if}
        <p>{t("Les boîtes « Enregistrer » et « Exporter » s'ouvrent dans ce dossier, avec le nom du document déjà rempli. Vous restez libre d'aller ailleurs à chaque enregistrement.")}</p>
      </section>
    {:else}
      <section>
        <h3>{t("Mémoire")}</h3>
        <label class="slider">
          <span>{t("Mémoire vive utilisable par Lumitype")}</span>
          <output>{system && capMb !== null ? t("{percent} % · {cap} sur {ram}", { percent: memoryPercent, cap: gib(capMb), ram: gib(system.ramMb) }) : `${memoryPercent} %`}</output>
          <input type="range" min={MEMORY_PERCENT_RANGE.min} max={MEMORY_PERCENT_RANGE.max} step="1" value={memoryPercent} oninput={(event) => update({ memoryPercent: event.currentTarget.valueAsNumber })} />
        </label>
        <p>{t("Lumitype n'en prend que ce dont il a besoin et ne dépasse jamais ce plafond : au-delà, il libère d'abord le cache de rendu, puis envoie les états d'annulation anciens sur le disque de travail (sans lui, il les abandonne).")}</p>
        <div class="memory" role="img" aria-label={t("Utilisation actuelle : {used} sur {cap}", { used: gib(usedMb), cap: gib(capMb ?? 0) })}>
          <div class="legend">
            <span><i class="documents"></i>{t("Documents ouverts")} <output>{gib(memory?.documentsMb ?? 0)}</output></span>
            <span><i class="history"></i>{t("Historique")} <output>{gib(memory?.historyMb ?? 0)}</output></span>
            <span><i class="cache"></i>{t("Cache de rendu")} <output>{gib(memory?.cacheMb ?? 0)}</output></span>
            <span><i class="free"></i>{t("Libre sous le plafond")} <output>{gib(freeMb)}</output></span>
          </div>
          <div class="gauge">
            <div class="documents" style:width="{segment(memory?.documentsMb ?? 0)}%"></div>
            <div class="history" style:width="{segment(memory?.historyMb ?? 0)}%"></div>
            <div class="cache" style:width="{segment(memory?.cacheMb ?? 0)}%"></div>
          </div>
          <span class="usage">{t("Utilisation actuelle : {used} sur {cap}", { used: gib(usedMb), cap: gib(capMb ?? 0) })}</span>
          <span class="usage">{t("Sur le disque : {size}", { size: gib(scratchMb) })}</span>
        </div>
        <label class="check"><input type="checkbox" checked={perf.scratchEnabled !== false} onchange={(event) => update({ scratchEnabled: event.currentTarget.checked })} /> {t("Disque de travail pour l'historique")}</label>
        <p>{t("Comme le disque de travail de Photoshop : au-delà du plafond, les états d'annulation anciens sont écrits dans un fichier d'échange au lieu d'être abandonnés, puis relus quand vous y revenez (Ctrl+Z, panneau Historique). Le fichier est supprimé à la fermeture du document.")}</p>
        <div class="folder">
          <input type="text" readonly value={perf.scratchDir || memory?.scratchDir || t("Dossier de cache de l'application")} class:empty={!perf.scratchDir} disabled={perf.scratchEnabled === false} />
          <button class="pill" disabled={perf.scratchEnabled === false} onclick={chooseScratchDir}>{t("Choisir…")}</button>
          <button class="ghost" disabled={!perf.scratchDir || perf.scratchEnabled === false} onclick={() => update({ scratchDir: "" })}>{t("Par défaut")}</button>
        </div>
        <label class="slider">
          <span>{t("Place maximale sur le disque")}</span>
          <output>{gib(scratchQuota * 1024)}</output>
          <input type="range" min={SCRATCH_QUOTA_RANGE.min} max={SCRATCH_QUOTA_RANGE.max} step="1" value={scratchQuota} disabled={perf.scratchEnabled === false} oninput={(event) => update({ scratchQuotaGb: event.currentTarget.valueAsNumber })} />
        </label>
        <p>{t("Disque plein, quota atteint ou erreur d'écriture : les plus anciens états sont abandonnés, comme sans disque de travail. Un disque rapide (SSD) rend le retour vers un état ancien presque immédiat.")}</p>

        <h3>{t("Processeur")}</h3>
        <label class="slider">
          <span>{t("Cœurs utilisés par le moteur")}</span>
          <output>{perf.cores === 0 ? t("Tous ({n})", { n: system?.cores ?? "…" }) : perf.cores}</output>
          <input type="range" min="0" max={system?.cores ?? 8} step="1" value={perf.cores >= (system?.cores ?? 8) ? 0 : perf.cores} oninput={(event) => update({ cores: event.currentTarget.valueAsNumber })} />
        </label>
        <p>{t("À gauche : tous les cœurs. En laisser un ou deux libres garde le reste du système réactif pendant un filtre lourd ou un export.")}</p>

        <h3>{t("Carte graphique")}</h3>
        {#if app.gpuStatus}
          <div class="gpu">
            <div><span>{t("Carte utilisée")}</span><strong>{masked && systemGpus[0] ? systemGpus[0] : app.gpuStatus.renderer}</strong></div>
            {#if masked && systemGpus.length > 1}
              <div><span>{t("Autres cartes")}</span><strong>{systemGpus.slice(1).join(" · ")}</strong></div>
            {/if}
            <div><span>{t("Taille maximale de texture")}</span><strong>{t("{maxTexture} px", { maxTexture: app.gpuStatus.maxTexture })}</strong></div>
            <div>
              <span>{t("Composition des calques")}</span>
              <strong class:ok={app.gpuStatus.active}>{app.gpuStatus.active ? t("par la carte graphique · {n} Mio", { n: app.gpuStatus.memoryMb }) : t("par le processeur — {reason}", { reason: t(app.gpuStatus.reason ?? "") })}</strong>
            </div>
          </div>
          {#if masked}
            <p>{t("Nom lu dans le système : sous Linux, la webview masque le nom exact de la carte (elle répond « Apple GPU », par protection de la vie privée). Pour vérifier que l'affichage l'utilise bien :")} <code>{t("glxinfo -B")}</code> {t("doit indiquer ta carte, pas « llvmpipe ».")}</p>
          {/if}
          {#if /llvmpipe|softpipe|swiftshader|software/i.test(app.gpuStatus.renderer)}
            <p class="warning">{t("Rendu logiciel : la carte graphique n'est pas utilisée par l'affichage. Sous Linux, vérifiez le pilote ; Lumitype démarre déjà en X11, le mode le plus sûr.")}</p>
          {/if}
        {/if}
        <label class="check"><input type="checkbox" checked={perf.gpuPreviews} onchange={(event) => update({ gpuPreviews: event.currentTarget.checked })} /> {t("Accélération matérielle")}</label>
        <p>{t("La carte graphique assemble tout le document (modes de fusion, masques, écrêtage, groupes, réglages et effets de calque) et dessine les aperçus pendant les gestes : zoom, déplacement et curseurs suivent la souris sans attendre le moteur. L'enregistrement et l'export restent calculés par le moteur : le résultat est identique.")}</p>
        <label class="slider">
          <span>{t("Mémoire vidéo utilisable")}</span>
          <output>
            {#if system?.vramMb != null}
              {t("{percent} % · {budget} sur {vram} détectés", { percent: gpuPercent, budget: gib(gpuBudgetMb), vram: gib(system.vramMb) })}
            {:else}
              {t("{percent} % · {budget} · VRAM non détectée, {assumed} supposés", { percent: gpuPercent, budget: gib(gpuBudgetMb), assumed: gib(ASSUMED_VRAM_MB) })}
            {/if}
          </output>
          <input type="range" min={GPU_PERCENT_RANGE.min} max={GPU_PERCENT_RANGE.max} step="1" value={gpuPercent} oninput={(event) => update({ gpuMemoryPercent: event.currentTarget.valueAsNumber })} />
        </label>
        {#if integrated}
          <p>{t("Mémoire vidéo partagée ou inconnue : sur une carte graphique intégrée, elle est prise sur la mémoire vive, et Lumitype n'en réserve jamais plus d'un quart.")}</p>
        {/if}
        <p>{t("Lumitype n'en réserve rien d'avance : la carte reçoit les calques à mesure, jusqu'à ce plafond. Au-delà, le document est composé par le processeur, sans rien planter.")}</p>
        <label class="check"><input type="checkbox" checked={perf.adaptiveResolution !== false} onchange={(event) => update({ adaptiveResolution: event.currentTarget.checked })} /> {t("Résolution dynamique pendant les gestes")}</label>
        <p>{t("Si l'affichage ne suit pas pendant un déplacement, un zoom ou un trait, la zone de travail est dessinée plus petite que l'écran (jusqu'à quatre fois moins de pixels), puis redevient nette au relâchement. Sans effet sur une machine qui suit déjà. La barre d'état indique le facteur en cours (×0,75, ×0,5).")}</p>
        <label class="check"><input type="checkbox" checked={!!perf.showFps} onchange={(event) => update({ showFps: event.currentTarget.checked })} /> {t("Afficher les images par seconde")}</label>
        <p>
          {t("La barre d'état indique la cadence de l'affichage et")} <strong>{t("la durée de la plus longue image")}</strong> {t("de la dernière demi-seconde. C'est cette seconde valeur qui compte : une moyenne de 60 i/s peut cacher une image à 40 ms, et c'est elle qu'on ressent comme un à-coup en déplaçant un calque. Elle passe en rouge quand elle dépasse nettement la cadence moyenne. La mesure empêche l'affichage de se reposer entre deux images : à laisser décochée en usage normal.")}
        </p>
        <p>{t("Un calque 4K occupe environ 45 Mio.")}</p>

        <h3>{t("Historique")}</h3>
        <label class="slider">
          <span>{t("États d'annulation")}</span>
          <output>{perf.historyStates}</output>
          <input type="range" min="10" max="300" step="5" value={perf.historyStates} oninput={(event) => update({ historyStates: event.currentTarget.valueAsNumber })} />
        </label>
        <p>{t("Les états partagent leurs pixels inchangés : en garder davantage coûte peu tant que chaque étape ne modifie qu'une partie de l'image.")}</p>
        <div class="actions">
          <button class="ghost" onclick={() => update({ ...DEFAULT_PERFORMANCE })}>{t("Rétablir les réglages par défaut")}</button>
        </div>
      </section>
    {/if}
  </div>
</dialog>

<style>
  dialog {
    width: min(820px, calc(100vw - 48px));
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
    overflow: hidden;
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.6);
  }
  header {
    height: 54px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 0 22px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  h3 {
    margin: 14px 0 0;
    font-size: 13px;
    font-weight: 500;
  }
  h3:first-child {
    margin-top: 0;
  }
  .icon {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    color: var(--text-muted);
  }
  .icon:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
  }
  .body {
    display: flex;
    height: min(560px, calc(100vh - 180px));
  }
  nav {
    width: 190px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 10px;
    border-right: 1px solid var(--border);
  }
  nav button {
    height: 36px;
    padding: 0 12px;
    border-radius: var(--radius-m);
    text-align: left;
    color: var(--text-3);
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.on {
    background: var(--bg-active);
    color: #fff;
  }
  section {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 20px 24px;
    overflow-y: auto;
  }
  .card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px;
    border-radius: var(--radius-l);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border-strong);
  }
  .card div {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .card strong {
    font-weight: 500;
    font-size: 14px;
  }
  .card span,
  p {
    color: var(--text-muted);
    line-height: 1.5;
  }
  .tag {
    flex-shrink: 0;
    padding: 3px 10px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--border-strong);
    color: var(--text) !important;
  }
  p {
    margin: 0;
  }
  code {
    font-family: var(--font-mono, monospace);
    color: var(--text-2);
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 6px;
  }
  .slider {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px;
    color: var(--text-3);
  }
  .slider input {
    grid-column: 1 / -1;
    width: 100%;
  }
  .slider input:disabled {
    opacity: 0.4;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ghost,
  .pill {
    height: 32px;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .ghost:disabled {
    opacity: 0.35;
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .folder {
    display: flex;
    gap: 8px;
  }
  .folder input {
    flex: 1;
    min-width: 0;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--bg-field);
    color: var(--text);
    user-select: text;
  }
  .folder input.empty {
    color: var(--text-muted);
  }
  .folder input:disabled,
  .pill:disabled {
    opacity: 0.4;
  }
  .warning {
    padding: 10px 12px;
    border-radius: var(--radius-m);
    background: rgba(255, 170, 60, 0.12);
    border: 1px solid rgba(255, 170, 60, 0.4);
    color: #ffd9a8;
  }
  .gpu {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px 14px;
    border-radius: var(--radius-l);
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border);
  }
  .gpu div {
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }
  .gpu span {
    color: var(--text-muted);
  }
  .gpu strong {
    font-weight: 500;
    text-align: right;
  }
  .gpu strong.ok {
    color: #7ee0a8;
  }
  /* Jauge de mémoire : trois segments à bords droits sur toute la largeur du plafond. */
  .memory {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--border);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 18px;
    color: var(--text-muted);
  }
  .legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .legend i {
    width: 10px;
    height: 10px;
    flex-shrink: 0;
  }
  .gauge {
    display: flex;
    height: 12px;
    background: var(--bg-field);
    border: 1px solid var(--border-strong);
    overflow: hidden;
  }
  .gauge div {
    height: 100%;
    flex-shrink: 0;
    transition: width 0.4s ease;
  }
  .documents {
    background: #ec2a3c;
  }
  .history {
    background: #8d1f2a;
  }
  .cache {
    background: #5a5557;
  }
  .legend .free {
    background: var(--bg-field);
    border: 1px solid var(--border-strong);
  }
  .usage {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
