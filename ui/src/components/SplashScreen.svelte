<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { emitTo, listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { openPaths } from "../lib/actions";
  import { startAutosave } from "../lib/autosave.svelte";
  import { loadRecents } from "../lib/recents.svelte";
  import { api } from "../lib/ipc";
  import { loadLocale } from "../lib/i18n.svelte";
  import { loadMetadata } from "../lib/metadata";
  import { loadPerformance } from "../lib/performance.svelte";
  import { loadProofProfile } from "../lib/proof-profiles";
  import { app } from "../state/app.svelte";
  import { keymap } from "../state/keymap.svelte";

  // Lancement. La fenêtre principale démarre cachée, derrière une bannière (fenêtre « splashscreen »,
  // voir ui/splash.html). Ce composant n'affiche rien : il charge ce qui est long à obtenir, tient la
  // bannière informée, puis demande à la coque de montrer l'application — au bout de cinq secondes au
  // moins, et jamais avant la fin du chargement.
  const MINIMUM_MS = import.meta.env.DEV ? 1500 : 5000; // en développement, on ne patiente pas à chaque rechargement

  onMount(() => {
    const started = performance.now();
    let [label, loaded, done] = ["Démarrage", false, false];
    // Chaque tâche est facultative : un échec ne doit jamais bloquer le lancement.
    const tasks: [string, () => Promise<unknown>][] = [
      ["Langue", () => loadLocale()],
      ["Raccourcis clavier", () => keymap.load()],
      ["Réglages de performance", () => loadPerformance()],
      ["Préférences de fichiers", async () => {
        const stored: unknown = JSON.parse((await api.loadSettings("files")) ?? "{}");
        const folder = (stored as { saveFolder?: unknown }).saveFolder;
        if (typeof folder === "string") app.saveFolder = folder;
        const minutes = (stored as { autosaveMinutes?: unknown }).autosaveMinutes;
        if (typeof minutes === "number") app.autosaveMinutes = Math.min(60, Math.max(2, Math.round(minutes)));
      }],
      ["Métadonnées par défaut", () => loadMetadata()],
      ["Profil d'épreuve", () => loadProofProfile()],
      ["Fichiers récents", () => loadRecents()],
      ["Polices", async () => (app.fonts = await api.listFonts())],
      ["Bibliothèque de pinceaux", async () => {
        const tips = await api.listBrushes();
        const known = new Set(app.brushTips.map((t) => t.id));
        app.brushTips = [...app.brushTips, ...tips.filter((t) => !known.has(t.id))];
      }],
    ];
    void (async () => {
      for (const [name, run] of tasks) {
        label = name;
        await run().catch(() => undefined);
      }
      [label, loaded] = ["Prêt", true];
    })();

    // La barre avance avec le temps écoulé, mais n'atteint 100 % qu'une fois le chargement terminé.
    let progress = 0.04;
    const timer = setInterval(() => {
      const elapsed = (performance.now() - started) / MINIMUM_MS;
      progress = Math.min(loaded ? 1 : 0.92, Math.max(progress, 1 - (1 - Math.min(1, elapsed)) ** 2));
      void emitTo("splashscreen", "splash-status", { label, progress }).catch(() => undefined);
      if (elapsed >= 1 && loaded && !done) {
        done = true;
        clearInterval(timer);
        // Un court instant à 100 %, puis la fenêtre principale remplace la bannière et l'accueil s'ouvre.
        setTimeout(() => {
          void invoke("close_splash").catch(() => undefined);
          // Copies de secours d'une session mal fermée : proposées tout de suite ; puis la sauvegarde automatique démarre.
          api.recoveryList().then((found) => (app.recoveries = found), () => undefined);
          startAutosave();
          // Lancé par un double-clic sur un projet : il s'ouvre directement ; sinon, l'accueil.
          api.startupFiles().then(
            // Sans fichier à ouvrir, l'écran d'accueil prend le relais (plus de fenêtre « Nouveau document » d'office).
            (files) => void (files.length > 0 && openPaths(files)),
            () => undefined,
          );
        }, 350);
      }
    }, 120);
    // Double-clic sur un autre projet alors que Lumitype tourne déjà : la coque nous le transmet.
    const stopListening = listen<string[]>("open-files", ({ payload }) => void openPaths(payload));
    return () => {
      clearInterval(timer);
      void stopListening.then((stop) => stop());
    };
  });
</script>
