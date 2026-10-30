<script lang="ts">
  import { i18n, LOCALES, type Locale, t } from "../lib/i18n.svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { PHOTOSHOP_KEYMAP } from "../lib/keymap-photoshop";
  import { type MemoryStatus, api } from "../lib/ipc";
  import {
    DEFAULT_PERFORMANCE,
    GPU_PERCENT_RANGE,
    MEMORY_PERCENT_RANGE,
    SCRATCH_QUOTA_RANGE,
    applyPerformance,
    gpuPercentOf,
    isIntegratedGpu,
    laptopModeActive,
    machineInfo,
    memoryPercentOf,
    scratchQuotaOf,
  } from "../lib/performance.svelte";
  import { AUTOSAVE_MAX, AUTOSAVE_MIN } from "../lib/autosave.svelte";
  import { UNITS, printPrefs, setPref } from "../lib/print.svelte";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";
  import type { ActionId } from "../lib/actions";
  import { ACCENTS, DEFAULT_APPEARANCE, THEMES, appearance } from "../lib/appearance.svelte";

  // Préférences : une liste de sections à gauche, leurs réglages à droite, une ligne par réglage (nom et courte
  // description à gauche, commande à droite). Les explications longues sont repliées (« Comment ça marche »,
  // « Réglages avancés »). Tout s'applique tout de suite, sans redémarrage, et s'enregistre dans le dossier de
  // configuration de l'application. La fenêtre garde la même hauteur d'une section à l'autre : chaque section défile.
  const SECTIONS = [
    { id: "general", label: "Général" },
    { id: "appearance", label: "Apparence" },
    { id: "shortcuts", label: "Raccourcis" },
    { id: "files", label: "Fichiers" },
    { id: "performance", label: "Performances" },
  ] as const;
  type SectionId = (typeof SECTIONS)[number]["id"];

  /** Raccourcis montrés à titre d'exemple (lecture seule) ; les touches viennent de la disposition en vigueur. */
  const COMMON_SHORTCUTS: { id: string; name: string; description?: string; actions: ActionId[] }[] = [
    { id: "tool.brush", name: "Pinceau", actions: ["tool.brush"] },
    { id: "view.fit", name: "Ajuster le plan de travail à la fenêtre", actions: ["view.fit"] },
    { id: "view.mirror", name: "Vue miroir", description: "Retourne seulement l'affichage, pour vérifier les proportions d'un dessin.", actions: ["view.mirror"] },
    { id: "edit.undo", name: "Annuler · Rétablir", actions: ["edit.undo", "edit.redo"] },
    { id: "edit.preferences", name: "Préférences", actions: ["edit.preferences"] },
  ];

  let dialog = $state<HTMLDialogElement>();
  let section = $state<SectionId>("general");
  // Ce que la machine offre : lu au démarrage (performance.svelte.ts), relu ici si cela avait échoué.
  const system = $derived(app.systemInfo);

  const perf = $derived(app.performance);
  const customized = $derived(Object.keys(keymap.overrides).length);
  // Mémoire vive : plafond = part réglée de la mémoire de la machine (voir engine::memory).
  const memoryPercent = $derived(memoryPercentOf(perf));
  const capMb = $derived(system ? Math.floor((system.ramMb * memoryPercent) / 100) : null);
  // Mémoire vidéo : part réglée de la mémoire détectée (ou 2 Gio supposés) ; sur une carte intégrée, au plus un quart de la mémoire vive.
  const gpuPercent = $derived(gpuPercentOf(perf));
  const video = $derived(machineInfo());
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
  // Mode portable : un seul budget. La carte graphique (même mémoire) compte dans la jauge, sur le même plafond.
  const laptop = $derived(laptopModeActive(perf, video));
  const autoLaptop = $derived(perf.laptopAuto !== false);
  const gpuUsedMb = $derived(laptop ? (app.gpuStatus?.memoryMb ?? 0) : 0);
  const usedMb = $derived(memory ? memory.documentsMb + memory.historyMb + memory.cacheMb + gpuUsedMb : gpuUsedMb);
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
  const gpuName = $derived((masked ? systemGpus[0] : app.gpuStatus?.renderer) || "");
  const softwareRendering = $derived(!!app.gpuStatus && /llvmpipe|softpipe|swiftshader|software/i.test(app.gpuStatus.renderer));

  onMount(() => {
    if (!app.systemInfo) api.systemInfo().then((info) => (app.systemInfo = info), () => undefined);
    api.systemGpus().then((gpus) => (systemGpus = gpus), () => undefined);
  });

  function update(patch: Partial<typeof DEFAULT_PERFORMANCE>): void {
    app.performance = { ...app.performance, ...patch };
    applyPerformance(true);
  }

  // ---- Cœurs : le réglage garde « 0 = tous » ; le curseur va de 1 au nombre de cœurs, sa butée droite vaut « tous ».
  const totalCores = $derived(system?.cores ?? 8);
  const coresShown = $derived(perf.cores <= 0 || perf.cores >= totalCores ? totalCores : perf.cores);

  // ---- Fichiers : dossier d'enregistrement par défaut.
  let folderMissing = $state(false);
  // Vérifié après un temps de repos de la frappe ; une réponse dépassée par une frappe plus récente est ignorée.
  $effect(() => {
    const folder = app.saveFolder.trim();
    folderMissing = false;
    if (!folder) return;
    let current = true;
    const timer = setTimeout(() => {
      api.directoryExists(folder).then((exists) => {
        if (current) folderMissing = !exists;
      }, () => undefined);
    }, 150);
    return () => {
      current = false;
      clearTimeout(timer);
    };
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
  const scratchOn = $derived(perf.scratchEnabled !== false);

  /** « 4,2 Go », « 512 Mo » : la décimale suit la langue (virgule en français). */
  function gb(mb: number): string {
    if (mb < 1024) return t("{n} Mo", { n: Math.round(mb) });
    const value = mb / 1024;
    const n = value.toLocaleString(i18n.info.tag, { minimumFractionDigits: 0, maximumFractionDigits: 1 });
    return t("{n} Go", { n });
  }

  /** Rétablit toutes les sections : performances, apparence, sauvegarde automatique, unité. La langue et le dossier d'enregistrement restent. */
  function resetAll(): void {
    update({ ...DEFAULT_PERFORMANCE });
    appearance.set(DEFAULT_APPEARANCE.theme, DEFAULT_APPEARANCE.accent);
    app.autosaveMinutes = 3;
    saveFileSettings();
    setPref("unit", "px");
  }

  function close(): void {
    app.preferencesOpen = false;
  }
</script>

{#snippet icon(id: SectionId)}
  {#if id === "general"}
    <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.2 2.2M16.9 16.9l2.2 2.2M4.9 19.1l2.2-2.2M16.9 7.1l2.2-2.2" /></svg>
  {:else if id === "appearance"}
    <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a9 9 0 1 0 0 18c1.5 0 2-1 2-2s-1-2 0-3 3 0 3-2a9 9 0 0 0-5-11z" /><circle cx="7.5" cy="11" r="1" /><circle cx="10.5" cy="7" r="1" /><circle cx="15.5" cy="7.5" r="1" /></svg>
  {:else if id === "shortcuts"}
    <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="6" width="18" height="12" /><path d="M7 10h.01M11 10h.01M15 10h.01M7 14h10" /></svg>
  {:else if id === "files"}
    <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 6a2 2 0 0 1 2-2h4l2 2h6a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z" /></svg>
  {:else}
    <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M4 14a8 8 0 0 1 16 0" /><path d="M12 14l4-5" /><path d="M3 18h18" /></svg>
  {/if}
{/snippet}

{#snippet chevron()}
  <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
{/snippet}

<dialog bind:this={dialog} onclose={close}>
  <header>
    <h2>{t("Préférences")}</h2>
    <button class="icon" aria-label={t("Fermer")} onclick={close}>
      <svg class="i" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
    </button>
  </header>
  <div class="body">
    <nav aria-label={t("Sections")}>
      {#each SECTIONS as item (item.id)}
        <button class:on={section === item.id} onclick={() => (section = item.id)}>{@render icon(item.id)}{t(item.label)}</button>
      {/each}
    </nav>

    {#if section === "general"}
      <!-- ============ GÉNÉRAL ============ -->
      <section>
        <div class="grp">
          <div class="grp-head"><h3>{t("Interface")}</h3></div>
          <div class="card">
            <div class="row">
              <div class="l">
                <div class="n">{t("Langue")}</div>
                <div class="d">{t("Prise en compte tout de suite, sans redémarrer.")}</div>
              </div>
              <div class="c">
                <select class="select" aria-label={t("Langue")} value={i18n.locale} onchange={(event) => (i18n.locale = event.currentTarget.value as Locale)}>
                  {#each LOCALES as locale (locale.code)}
                    <option value={locale.code} lang={locale.code}>{locale.name}</option>
                  {/each}
                </select>
              </div>
            </div>
            <div class="row">
              <div class="l">
                <div class="n">{t("Unité des règles et des tailles")}</div>
                <div class="d">{t("Utilisée par Nouveau document, Taille de l'image et Format d'impression.")}</div>
              </div>
              <div class="c">
                <div class="seg" role="radiogroup" aria-label={t("Unité")}>
                  {#each UNITS as unit (unit.id)}
                    <button type="button" role="radio" aria-checked={printPrefs.unit === unit.id} class:on={printPrefs.unit === unit.id} title={t(unit.label)} onclick={() => setPref("unit", unit.id)}>{unit.id}</button>
                  {/each}
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>
    {:else if section === "appearance"}
      <!-- ============ APPARENCE ============ -->
      <section>
        <div class="grp">
          <div class="grp-head"><h3>{t("Thème")}</h3></div>
          <div class="card">
            <div class="themes">
              {#each THEMES as theme (theme.id)}
                {@const c = theme.colors}
                <button type="button" class="theme" class:on={appearance.theme === theme.id} onclick={() => appearance.set(theme.id, appearance.accent)}>
                  <span class="mini" style:background={c.app} style:border-color={c.border}>
                    <span class="mt" style:background={c.bar}><i style:background={appearance.accent}></i><s style:background={c.muted}></s><s style:background={c.muted}></s></span>
                    <span class="mb">
                      <span class="ml" style:background={c.panel}><s style:background={c.muted}></s><s style:background={c.muted}></s><b style:background={appearance.accent}></b></span>
                      <span class="mc" style:background={c.canvas}><span class="doc"></span></span>
                      <span class="mr" style:background={c.panel}><s style:background={c.muted}></s><s class="sel" style:background={appearance.accent}></s><s style:background={c.muted}></s></span>
                    </span>
                  </span>
                  <span class="theme-name">{t(theme.name)}</span>
                </button>
              {/each}
            </div>
          </div>
        </div>
        <div class="grp">
          <div class="grp-head"><h3>{t("Couleur d'accent")}</h3></div>
          <div class="card">
            <div class="row wide">
              <div class="l">
                <div class="n">{t("Accent")}</div>
                <div class="d">{t("Cadres de sélection, onglet actif, boutons principaux, jauge.")}</div>
              </div>
              <div class="c accents">
                {#each ACCENTS as accent (accent.color)}
                  <button type="button" class="swatch" class:on={appearance.accent === accent.color} style:--c={accent.color} title={t(accent.name)} aria-label={t(accent.name)} onclick={() => appearance.set(appearance.theme, accent.color)}></button>
                {/each}
                <label class="swatch custom" class:on={!ACCENTS.some((a) => a.color === appearance.accent)} title={t("Couleur personnalisée")} style:--c={appearance.accent}>
                  <input type="color" value={appearance.accent} oninput={(event) => appearance.set(appearance.theme, event.currentTarget.value)} />
                  +
                </label>
                <input class="hex" type="text" value={appearance.accent} spellcheck="false" aria-label={t("Code hexadécimal")} onkeydown={(event) => event.stopPropagation()} onchange={(event) => appearance.set(appearance.theme, event.currentTarget.value)} />
              </div>
            </div>
          </div>
        </div>
      </section>
    {:else if section === "shortcuts"}
      <!-- ============ RACCOURCIS ============ -->
      <section>
        <div class="grp">
          <div class="grp-head"><h3>{t("Disposition")}</h3></div>
          <div class="card">
            <div class="row">
              <div class="l">
                <div class="n">
                  {t("Raccourcis clavier")}
                  {#if customized === 0}
                    <span class="badge">{t("Active")}</span>
                  {:else}
                    <span class="badge ok">{customized > 1 ? t("{n} modifiés", { n: customized }) : t("1 modifié")}</span>
                  {/if}
                </div>
                <div class="d">{t("{n} raccourcis, claviers AZERTY et QWERTY pris en charge.", { n: PHOTOSHOP_KEYMAP.length })}</div>
              </div>
              <div class="c">
                <button
                  class="pill"
                  onclick={() => {
                    app.preferencesOpen = false;
                    app.shortcutsDialogOpen = true;
                  }}
                >
                  {t("Personnaliser…")}
                </button>
                <button class="ghost" disabled={customized === 0} onclick={() => keymap.resetAll()}>{t("Rétablir par défaut")}</button>
              </div>
            </div>
            <details>
              <summary>{@render chevron()}{t("Comment ça marche")}</summary>
              <div class="how">
                <p>{t("Chaque raccourci peut être remplacé ; un conflit est signalé au moment de la saisie.")}</p>
                <p>{t("Vos changements sont enregistrés dans")} <span class="kbd">{t("keymap.json")}</span> {t("et survivent aux mises à jour.")}</p>
              </div>
            </details>
          </div>
        </div>
        <div class="grp">
          <div class="grp-head"><h3>{t("Les plus utilisés")}</h3><small>{t("Lecture seule ici")}</small></div>
          <div class="card">
            {#each COMMON_SHORTCUTS as item (item.id)}
              <div class="row">
                <div class="l">
                  <div class="n">{t(item.name)}</div>
                  {#if item.description}<div class="d">{t(item.description)}</div>{/if}
                </div>
                <div class="c">
                  {#each item.actions as action (action)}
                    <span class="kbd" class:none={!keymap.label(action)}>{keymap.label(action) || t("Aucun raccourci")}</span>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        </div>
      </section>
    {:else if section === "files"}
      <!-- ============ FICHIERS ============ -->
      <section>
        <div class="grp">
          <div class="grp-head"><h3>{t("Sauvegarde automatique")}</h3></div>
          <div class="card">
            <div class="row">
              <div class="l">
                <div class="n">{t("Copie de secours")}</div>
                <div class="d">{t("Chaque document modifié est copié à part, historique compris. Votre fichier n'est jamais écrit à votre place.")}</div>
              </div>
              <div class="c">
                <label class="range">
                  <input type="range" min={AUTOSAVE_MIN} max={AUTOSAVE_MAX} step="1" aria-label={t("Intervalle de la sauvegarde automatique")} bind:value={app.autosaveMinutes} onchange={saveFileSettings} />
                  <output>{t("Toutes les {n} min", { n: app.autosaveMinutes })}</output>
                </label>
              </div>
            </div>
            <details>
              <summary>{@render chevron()}{t("Comment ça marche")}</summary>
              <div class="how">
                <p>{t("Chaque document modifié est copié, historique compris, dans un dossier de secours à part : votre fichier n'est jamais écrit à votre place. Après une fermeture imprévue, Lumitype propose de tout récupérer au démarrage. Enregistrer ou fermer un document supprime sa copie. Pendant la copie, un filet en haut de la fenêtre avance ; vous pouvez continuer à travailler : ce que vous faites alors sera dans la copie suivante.")}</p>
              </div>
            </details>
          </div>
        </div>
        <div class="grp">
          <div class="grp-head"><h3>{t("Enregistrement")}</h3></div>
          <div class="card">
            <div class="row wide">
              <div class="l">
                <div class="n">{t("Dossier par défaut")}</div>
                <div class="d">{t("Les boîtes « Enregistrer » et « Exporter » s'ouvrent dans ce dossier, avec le nom du document déjà rempli. Vous restez libre d'aller ailleurs à chaque enregistrement.")}</div>
              </div>
              <div class="c folder">
                <input class="field" type="text" readonly value={app.saveFolder || t("Celui que propose le système")} class:empty={!app.saveFolder} aria-label={t("Dossier par défaut")} />
                <button class="ghost" onclick={chooseFolder}>{t("Choisir…")}</button>
                <button class="ghost" disabled={!app.saveFolder} onclick={() => setFolder("")}>{t("Effacer")}</button>
              </div>
              {#if folderMissing}
                <p class="warning">{t("Ce dossier est introuvable pour l'instant (renommé, supprimé, ou disque débranché). Rien n'est bloqué : tant qu'il manque, les boîtes d'enregistrement s'ouvrent à l'emplacement habituel du système.")}</p>
              {/if}
            </div>
          </div>
        </div>
      </section>
    {:else}
      <!-- ============ PERFORMANCES ============ -->
      <section>
        <div class="grp">
          <div class="grp-head"><h3>{t("Cette machine")}</h3></div>
          <div class="card">
            <div class="row" class:off={autoLaptop}>
              <div class="l">
                <div class="n">
                  {t("Mode portable")}
                  {#if autoLaptop}<span class="badge acc">{t("réglage automatique")}</span>{/if}
                </div>
                <div class="d">{t("Économise la mémoire et la batterie sur les petites machines.")}</div>
              </div>
              <div class="c">
                <button class="switch" role="switch" aria-checked={laptop} aria-label={t("Mode portable")} disabled={autoLaptop} onclick={() => update({ laptopMode: !laptop })}></button>
              </div>
            </div>
            <div class="row">
              <div class="l">
                <div class="n">{t("Laisser Lumitype décider")}</div>
                <div class="d">
                  {t("S'active seul sur une carte graphique intégrée.")}
                  {#if autoLaptop}
                    {laptop ? t("Ici : mode portable activé.") : t("Ici : mode portable inutile.")}
                  {:else}
                    {integrated ? t("Ici, il l'activerait.") : t("Ici, il le laisserait désactivé.")}
                  {/if}
                </div>
              </div>
              <div class="c">
                <button class="switch" role="switch" aria-checked={autoLaptop} aria-label={t("Laisser Lumitype décider")} onclick={() => update({ laptopAuto: !autoLaptop, laptopMode: laptop })}></button>
              </div>
            </div>
            <div class="machine">
              <span>{t("Détecté :")} <b>{system ? t("{n} cœurs", { n: system.cores }) : "…"}</b></span>
              <span>{t("{ram} de mémoire", { ram: system ? gb(system.ramMb) : "…" })}</span>
              <span>
                {integrated ? t("Carte graphique intégrée") : t("Carte graphique dédiée")}
                <b>{gpuName || t("nom inconnu")}</b>
                {#if system?.vramMb != null}· {gb(system.vramMb)}{system.vramShared ? t(", mémoire partagée") : ""}{/if}
              </span>
              {#if masked && systemGpus.length > 1}
                <span>{t("Autres cartes")} <b>{systemGpus.slice(1).join(" · ")}</b></span>
              {/if}
              {#if softwareRendering}
                <span class="warn">{t("Rendu logiciel : la carte graphique n'est pas utilisée par l'affichage. Sous Linux, vérifiez le pilote ; Lumitype démarre déjà en X11, le mode le plus sûr.")}</span>
              {/if}
            </div>
            <div class="row">
              <div class="l">
                <div class="n">{t("Accélération matérielle")}</div>
                <div class="d">{t("Aperçus et composition des calques calculés par la carte graphique ; l'export reste calculé par le moteur, à l'identique.")}</div>
              </div>
              <div class="c">
                <button class="switch" role="switch" aria-checked={perf.gpuPreviews} aria-label={t("Accélération matérielle")} onclick={() => update({ gpuPreviews: !perf.gpuPreviews })}></button>
              </div>
            </div>
            <div class="row">
              <div class="l">
                <div class="n">{t("Résolution dynamique pendant les gestes")}</div>
                <div class="d">{t("Affichage allégé le temps d'un déplacement ou d'un zoom, net dès qu'on lâche.")}</div>
              </div>
              <div class="c">
                <button class="switch" role="switch" aria-checked={perf.dynamicResolution === true} aria-label={t("Résolution dynamique pendant les gestes")} onclick={() => update({ dynamicResolution: perf.dynamicResolution !== true })}></button>
              </div>
            </div>
          </div>
        </div>

        <div class="grp">
          <div class="grp-head">
            <h3>{t("Mémoire")}</h3>
            <small>{t("{used} utilisés sur {cap}", { used: gb(usedMb), cap: gb(capMb ?? 0) })}</small>
          </div>
          <div class="card">
            <div class="row">
              <div class="l">
                <div class="n">{t("Mémoire réservée à Lumitype")}</div>
                <div class="d">{t("Part de la mémoire de la machine. Le reste va au système et aux autres logiciels ; au-delà, le cache est libéré puis l'historique ancien part sur le disque.")}</div>
              </div>
              <div class="c">
                <label class="range">
                  <input type="range" min={MEMORY_PERCENT_RANGE.min} max={MEMORY_PERCENT_RANGE.max} step="1" value={memoryPercent} aria-label={t("Mémoire réservée à Lumitype")} oninput={(event) => update({ memoryPercent: event.currentTarget.valueAsNumber })} />
                  <output>{capMb !== null ? `${memoryPercent} % · ${gb(capMb)}` : `${memoryPercent} %`}</output>
                </label>
              </div>
            </div>
            <div class="memory" role="img" aria-label={t("{used} utilisés sur {cap}", { used: gb(usedMb), cap: gb(capMb ?? 0) })}>
              <div class="bar">
                <i class="c1" style:width="{segment(memory?.documentsMb ?? 0)}%"></i>
                <i class="c2" style:width="{segment(memory?.historyMb ?? 0)}%"></i>
                <i class="c3" style:width="{segment(memory?.cacheMb ?? 0)}%"></i>
                {#if laptop}<i class="c4" style:width="{segment(gpuUsedMb)}%"></i>{/if}
              </div>
              <div class="leg">
                <span><i class="c1"></i>{t("Documents {n}", { n: gb(memory?.documentsMb ?? 0) })}</span>
                <span><i class="c2"></i>{t("Historique {n}", { n: gb(memory?.historyMb ?? 0) })}</span>
                <span><i class="c3"></i>{t("Cache {n}", { n: gb(memory?.cacheMb ?? 0) })}</span>
                {#if laptop}<span><i class="c4"></i>{t("Carte graphique {n}", { n: gb(gpuUsedMb) })}</span>{/if}
                <span>{t("Libre {n}", { n: gb(freeMb) })}</span>
                {#if memory && memory.scratchBytes > 0}<span>{t("Sur le disque {n}", { n: gb(memory.scratchBytes / (1024 * 1024)) })}</span>{/if}
              </div>
            </div>
            <div class="row">
              <div class="l">
                <div class="n">{t("Disque de travail pour l'historique")}</div>
                <div class="d">{t("Les anciennes étapes passent sur le disque au lieu d'occuper la mémoire, et sont relues quand vous y revenez.")}</div>
              </div>
              <div class="c">
                <button class="switch" role="switch" aria-checked={scratchOn} aria-label={t("Disque de travail pour l'historique")} onclick={() => update({ scratchEnabled: !scratchOn })}></button>
              </div>
            </div>
            <details>
              <summary>{@render chevron()}{t("Réglages avancés")}</summary>
              <div class="adv">
                <div class="row wide" class:off={!scratchOn}>
                  <div class="l">
                    <div class="n">{t("Dossier de cache")}</div>
                    <div class="d">{t("Un disque rapide (SSD) de préférence. Le fichier est supprimé à la fermeture du document.")}</div>
                  </div>
                  <div class="c folder">
                    <input class="field" type="text" readonly value={perf.scratchDir || memory?.scratchDir || t("Dossier de cache de l'application")} class:empty={!perf.scratchDir} aria-label={t("Dossier de cache")} />
                    <button class="ghost" onclick={chooseScratchDir}>{t("Choisir…")}</button>
                    <button class="ghost" disabled={!perf.scratchDir} onclick={() => update({ scratchDir: "" })}>{t("Par défaut")}</button>
                  </div>
                </div>
                <div class="row" class:off={!scratchOn}>
                  <div class="l">
                    <div class="n">{t("Place sur le disque")}</div>
                    <div class="d">{t("Au-delà, les étapes les plus anciennes sont oubliées.")}</div>
                  </div>
                  <div class="c">
                    <label class="range">
                      <input type="range" min={SCRATCH_QUOTA_RANGE.min} max={SCRATCH_QUOTA_RANGE.max} step="1" value={scratchQuota} aria-label={t("Place sur le disque")} oninput={(event) => update({ scratchQuotaGb: event.currentTarget.valueAsNumber })} />
                      <output>{t("{n} Go", { n: scratchQuota })}</output>
                    </label>
                  </div>
                </div>
                <div class="row">
                  <div class="l">
                    <div class="n">{t("Cœurs utilisés")}</div>
                    <div class="d">{t("Pour les filtres et les exports. Un cœur libre garde le système réactif.")}</div>
                  </div>
                  <div class="c">
                    <label class="range">
                      <input type="range" min="1" max={totalCores} step="1" value={coresShown} aria-label={t("Cœurs utilisés")} oninput={(event) => update({ cores: event.currentTarget.valueAsNumber >= totalCores ? 0 : event.currentTarget.valueAsNumber })} />
                      <output>{t("{n} sur {total}", { n: coresShown, total: system?.cores ?? "…" })}</output>
                    </label>
                  </div>
                </div>
                <div class="row" class:off={laptop}>
                  <div class="l">
                    <div class="n">{t("Mémoire vidéo")}</div>
                    <div class="d">
                      {#if laptop}
                        {t("En mode portable, la carte graphique prend sa part sur le budget unique : ce curseur ne s'applique pas.")}
                      {:else}
                        {t("Part de la carte graphique réservée aux calques et aux aperçus.")}
                      {/if}
                    </div>
                  </div>
                  <div class="c">
                    <label class="range">
                      <input type="range" min={GPU_PERCENT_RANGE.min} max={GPU_PERCENT_RANGE.max} step="1" value={gpuPercent} disabled={laptop} aria-label={t("Mémoire vidéo")} oninput={(event) => update({ gpuMemoryPercent: event.currentTarget.valueAsNumber })} />
                      <output>{gpuPercent} %</output>
                    </label>
                  </div>
                </div>
                <div class="row">
                  <div class="l">
                    <div class="n">{t("Afficher les images par seconde")}</div>
                    <div class="d">{t("Dans la barre d'état, pour diagnostiquer les saccades. La mesure empêche l'affichage de se reposer : à laisser éteint en usage normal.")}</div>
                  </div>
                  <div class="c">
                    <button class="switch" role="switch" aria-checked={!!perf.showFps} aria-label={t("Afficher les images par seconde")} onclick={() => update({ showFps: !perf.showFps })}></button>
                  </div>
                </div>
              </div>
            </details>
          </div>
        </div>

        <div class="grp">
          <div class="grp-head"><h3>{t("Historique")}</h3></div>
          <div class="card">
            <div class="row">
              <div class="l">
                <div class="n">{t("Étapes d'annulation")}</div>
                <div class="d">{t("Les étapes partagent leurs pixels inchangés : en garder davantage coûte peu tant que chacune ne modifie qu'une partie de l'image.")}</div>
              </div>
              <div class="c">
                <label class="range">
                  <input type="range" min="10" max="300" step="5" value={perf.historyStates} aria-label={t("Étapes d'annulation")} oninput={(event) => update({ historyStates: event.currentTarget.valueAsNumber })} />
                  <output>{t("{n} étapes", { n: perf.historyStates })}</output>
                </label>
              </div>
            </div>
          </div>
        </div>
      </section>
    {/if}
  </div>
  <footer>
    <button class="ghost" onclick={resetAll}>{t("Rétablir les réglages par défaut")}</button>
    <span class="hint">{t("Tout s'applique tout de suite.")}</span>
    <div class="acts"><button class="pill" onclick={close}>{t("Fermer")}</button></div>
  </footer>
</dialog>

<style>
  dialog {
    width: min(880px, calc(100vw - 48px));
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
    height: 50px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px 0 20px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 500;
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
  svg.i {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* Hauteur fixe : la fenêtre ne change jamais de taille d'une section à l'autre, chaque section défile dedans. */
  .body {
    display: flex;
    height: min(560px, calc(100vh - 200px));
    min-height: 320px;
  }
  nav {
    width: 176px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 8px;
    border-right: 1px solid var(--border);
    background: var(--bg-bar);
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    text-align: left;
    color: var(--text-3);
  }
  nav button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  nav button.on {
    background: var(--bg-active);
    color: var(--text-strong);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  nav button svg.i {
    width: 15px;
    height: 15px;
    opacity: 0.85;
  }

  section {
    flex: 1;
    min-width: 0;
    padding: 18px 22px 24px;
    overflow-y: auto;
  }
  .grp {
    margin-bottom: 20px;
  }
  .grp:last-child {
    margin-bottom: 0;
  }
  .grp-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    padding: 0 2px 6px;
  }
  h3 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .grp-head small {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .card {
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-card);
  }

  /* La ligne : nom + description à gauche, commande à droite, toujours alignée sur la même colonne. */
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 6px 18px;
    padding: 11px 14px;
    border-top: 1px solid var(--border);
    min-height: 54px;
  }
  .row:first-child {
    border-top: 0;
  }
  .row .l {
    min-width: 0;
  }
  .row .n {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    font-weight: 500;
    color: var(--text);
    font-size: 12.5px;
  }
  .row .d {
    color: var(--text-muted);
    margin-top: 2px;
    max-width: 60ch;
    line-height: 1.45;
  }
  .row .c {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    min-width: 0;
  }
  .row.off .l {
    opacity: 0.45;
  }
  .row.off .c {
    pointer-events: none;
    opacity: 0.45;
  }
  .row.wide {
    grid-template-columns: 1fr;
  }
  .row.wide .c {
    justify-content: flex-start;
    flex-wrap: wrap;
  }

  .badge {
    font-size: 10px;
    font-weight: 500;
    padding: 2px 6px;
    border-radius: var(--radius-pill);
    background: var(--bg-active);
    color: var(--text-2);
  }
  .badge.acc {
    background: var(--accent);
    color: var(--on-accent);
  }
  .badge.ok {
    color: #2fa84f;
    background: color-mix(in srgb, #2fa84f 16%, transparent);
  }
  .kbd {
    font: 11px var(--font-mono, ui-monospace, monospace);
    padding: 2px 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xs);
    background: var(--bg-field);
    color: var(--text-2);
  }
  .kbd.none {
    font-family: var(--font);
    font-style: italic;
    color: var(--text-muted);
  }

  /* Commandes */
  .switch {
    position: relative;
    width: 36px;
    height: 20px;
    flex-shrink: 0;
    border-radius: var(--radius-pill);
    background: var(--bg-active);
    border: 1px solid var(--border-strong);
    transition: background 0.16s;
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: var(--radius-xs);
    background: var(--text-3);
    transition:
      transform 0.16s var(--ease),
      background 0.16s;
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch[aria-checked="true"]::after {
    transform: translateX(16px);
    background: var(--on-accent);
  }
  .switch:disabled {
    cursor: default;
  }

  .range {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 240px;
    max-width: 100%;
  }
  .range input {
    flex: 1;
    accent-color: var(--accent);
    min-width: 0;
  }
  .range input:disabled {
    opacity: 0.4;
  }
  .range output {
    min-width: 64px;
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }

  .seg {
    display: inline-flex;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    overflow: hidden;
  }
  .seg button {
    padding: 5px 10px;
    color: var(--text-3);
    border-left: 1px solid var(--border);
  }
  .seg button:first-child {
    border-left: 0;
  }
  .seg button:hover {
    color: var(--text);
    background: var(--bg-hover);
  }
  .seg button.on {
    background: var(--accent);
    color: var(--on-accent);
  }

  /* Liste déroulante native, habillée comme les autres commandes. */
  .select {
    appearance: none;
    -webkit-appearance: none;
    padding: 5px 28px 5px 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field) url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23888' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'><path d='M6 9l6 6 6-6'/></svg>") no-repeat right 8px center / 12px;
    color: var(--text-2);
    cursor: pointer;
  }
  .select:hover {
    background-color: var(--bg-hover);
  }
  .select option {
    background: var(--bg-menu);
    color: var(--text);
  }

  .pill {
    padding: 6px 12px;
    border-radius: var(--radius-pill);
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
  .pill:hover {
    filter: brightness(1.08);
  }
  .ghost {
    padding: 6px 12px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-pill);
    color: var(--text-2);
    background: var(--bg-field);
  }
  .ghost:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  .ghost:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .ghost:disabled:hover {
    background: var(--bg-field);
    color: var(--text-2);
  }
  /* Champ + boutons sur toute la largeur d'une ligne `.wide` (plus précis que `.row .c`, qui pose un flex). */
  .row .c.folder {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: 8px;
    width: 100%;
  }
  .field {
    padding: 6px 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text-2);
    width: 100%;
    min-width: 0;
    user-select: text;
  }
  .field.empty {
    color: var(--text-muted);
    font-style: italic;
  }
  .warning {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: rgba(255, 170, 60, 0.12);
    border: 1px solid rgba(255, 170, 60, 0.4);
    color: var(--text-2);
    line-height: 1.45;
  }

  /* Replis : « Comment ça marche », « Réglages avancés ». */
  details {
    border-top: 1px solid var(--border);
  }
  summary {
    list-style: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 14px;
    color: var(--text-3);
    cursor: pointer;
    user-select: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
  summary svg.i {
    width: 12px;
    height: 12px;
    transition: transform 0.16s var(--ease);
  }
  details[open] summary svg.i {
    transform: rotate(90deg);
  }
  .how {
    padding: 2px 14px 12px;
    color: var(--text-3);
    max-width: 68ch;
    line-height: 1.5;
  }
  .how p {
    margin: 0 0 6px;
  }
  .adv .row {
    border-top: 1px solid var(--border);
  }

  /* Jauge mémoire */
  .memory {
    padding: 10px 14px 12px;
    border-top: 1px solid var(--border);
  }
  .memory .bar {
    height: 8px;
    display: flex;
    background: var(--bg-well);
    border: 1px solid var(--border);
    overflow: hidden;
  }
  .memory .bar i {
    display: block;
    height: 100%;
    flex-shrink: 0;
    transition: width 0.3s var(--ease);
  }
  .memory .leg {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 14px;
    margin-top: 8px;
    color: var(--text-muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .memory .leg i {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 5px;
    vertical-align: -1px;
  }
  .c1 {
    background: var(--accent);
  }
  .c2 {
    background: color-mix(in srgb, var(--accent) 55%, var(--text-muted));
  }
  .c3 {
    background: var(--text-muted);
  }
  .c4 {
    background: var(--text-disabled);
  }

  /* Machine détectée */
  .machine {
    display: flex;
    gap: 6px 14px;
    flex-wrap: wrap;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    color: var(--text-3);
    font-size: 11px;
  }
  .machine b {
    color: var(--text-2);
    font-weight: 500;
  }
  .machine .warn {
    flex-basis: 100%;
    color: #ffb020;
  }

  /* Thèmes & accents */
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(118px, 1fr));
    gap: 10px;
    padding: 12px 14px;
  }
  .theme {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: stretch;
    color: var(--text-3);
  }
  .theme.on {
    color: var(--text-strong);
    font-weight: 500;
  }
  .mini {
    aspect-ratio: 16 / 10;
    display: grid;
    grid-template-rows: 11px 1fr;
    border: 1px solid;
    outline: 2px solid transparent;
    outline-offset: 1px;
    overflow: hidden;
  }
  .theme.on .mini {
    outline-color: var(--accent);
  }
  .mt {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 5px;
  }
  .mt i {
    width: 6px;
    height: 6px;
  }
  .mt s,
  .ml s,
  .mr s {
    height: 3px;
    opacity: 0.55;
  }
  .mt s {
    width: 12px;
  }
  .mb {
    display: grid;
    grid-template-columns: 22% 1fr 26%;
  }
  .ml,
  .mr {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px 5px;
  }
  .ml b {
    height: 6px;
    margin-top: auto;
  }
  .mr s.sel {
    opacity: 1;
    height: 4px;
  }
  .mc {
    display: grid;
    place-items: center;
  }
  .doc {
    width: 55%;
    height: 55%;
    background: #fff;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.4);
  }
  .theme-name {
    text-align: center;
  }
  .accents {
    gap: 8px;
  }
  .swatch {
    width: 22px;
    height: 22px;
    background: var(--c);
    border: 1px solid rgba(0, 0, 0, 0.25);
    outline: 2px solid transparent;
    outline-offset: 2px;
  }
  .swatch.on {
    outline-color: var(--text-strong);
  }
  .swatch.custom {
    position: relative;
    display: grid;
    place-items: center;
    overflow: hidden;
    color: #fff;
    text-shadow: 0 0 2px #000;
    font-size: 15px;
    cursor: pointer;
  }
  .swatch.custom input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .hex {
    width: 84px;
    padding: 5px 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--bg-field);
    color: var(--text-2);
    font: 11px var(--font-mono, ui-monospace, monospace);
    user-select: text;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 16px 10px 20px;
    border-top: 1px solid var(--border);
    background: var(--bg-bar);
    flex-wrap: wrap;
  }
  footer .hint {
    color: var(--text-muted);
    font-size: 11px;
  }
  footer .acts {
    display: flex;
    gap: 8px;
    margin-left: auto;
  }

  @media (prefers-reduced-motion: reduce) {
    .switch,
    .switch::after,
    summary svg.i,
    .memory .bar i {
      transition: none;
    }
  }
</style>
