<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { untrack } from "svelte";
  import { api, type Preflight } from "../../lib/ipc";
  import { colorModeOf, exportSpace } from "../../lib/color-mode.svelte";
  import { profileShort } from "../../lib/color-settings.svelte";
  import { fitRect } from "../../lib/artboards.svelte";
  import { ink, inkLimitSet, printPrefs, recommendedInk, setPref } from "../../lib/print.svelte";
  import { app } from "../../state/app.svelte";

  // Vérifier avant impression… : la liste des points à revoir avant d'envoyer le fichier à l'imprimeur, chacun avec
  // « Voir » (la vue se cadre dessus) ou une correction directe. Rouge : à corriger, orange : à vérifier, vert : bon.
  let dialog = $state<HTMLDialogElement>();
  let result = $state<Preflight | null>(null);
  let failed = $state(false);
  const info = $derived(app.active?.info ?? null);

  type Level = "error" | "warn" | "ok";
  interface Check {
    level: Level;
    title: string;
    detail?: string;
    view?: [number, number, number, number];
    fix?: { label: string; run: () => void };
  }

  async function run(): Promise<void> {
    const doc = app.active?.info;
    if (!doc) return;
    result = null;
    failed = false;
    try {
      result = await api.preflight(doc.id, inkLimitSet(doc) ?? recommendedInk(exportSpace(doc).profile), exportSpace(doc));
      Object.assign(ink, { docId: doc.id, revision: doc.revision, report: result.ink });
    } catch {
      failed = true;
    }
  }

  $effect(() => {
    if (!app.preflightOpen) {
      dialog?.close();
      return;
    }
    dialog?.showModal();
    // Vérification lancée à l'ouverture seulement (document lu hors suivi) : chaque révision relançait
    // l'analyse et faisait clignoter la liste ; « Revérifier » la relance à la demande.
    untrack(() => void run());
  });

  const checks = $derived.by((): Check[] => {
    const doc = info;
    if (!doc || !result) return [];
    const out: Check[] = [];
    const open = (what: "printSetup" | "correctInk") => () => {
      app.preflightOpen = false;
      if (what === "printSetup") app.printSetupDialogOpen = true;
      else app.correctInkDialogOpen = true;
    };
    if (!doc.print) out.push({ level: "warn", title: t("Pas de format d'impression"), detail: t("Ni fonds perdus ni zone de sécurité : l'imprimeur demande en général 3 mm de fonds perdus."), fix: { label: t("Définir…"), run: open("printSetup") } });
    if (doc.dpi < 250) out.push({ level: "error", title: t("Résolution {dpi} ppp", { dpi: Math.round(doc.dpi) }), detail: t("300 ppp conseillés pour l'impression : l'image risque d'être floue.") });
    const mode = colorModeOf(doc);
    if (mode.kind !== "cmykWorkflow") out.push({ level: "warn", title: t("Document en RVB"), detail: t("Converti en CMJN ({profile}) à l'export : les couleurs vives peuvent changer. Image › Mode › Couleurs CMJN pour le voir.", { profile: profileShort(exportSpace(doc).profile) }) });
    const r = result.ink;
    const set = inkLimitSet(doc);
    const advised = recommendedInk(exportSpace(doc).profile);
    const profile = profileShort(exportSpace(doc).profile);
    if (r.maxPercent === 0) out.push({ level: "ok", title: t("Aucune encre"), detail: t("Document vide : rien à imprimer pour l'instant.") });
    else if (set === null)
      // Pas de limite réglée : on indique l'encrage trouvé et la limite d'usage du profil, avec de quoi la régler.
      out.push({
        level: r.overPixels > 0 ? "warn" : "ok",
        title: t("Encrage {max} % maximum", { max: r.maxPercent }),
        detail: t("Aucune limite réglée · conseillé : {advised} % pour {profile}", { advised, profile }),
        view: r.overPixels > 0 ? r.regions[0] : undefined,
        fix: { label: t("Définir la limite…"), run: open("printSetup") },
      });
    else if (r.overPixels > 0)
      out.push({ level: "warn", title: t("Encrage {max} %", { max: r.maxPercent }), detail: t("{count} zone(s) au-delà de {limit} %", { count: r.regions.length, limit: r.limit }), view: r.regions[0], fix: { label: t("Corriger…"), run: open("correctInk") } });
    for (const [, name, box] of result.textsOutside) out.push({ level: "warn", title: t("Texte hors zone de sécurité"), detail: `« ${name} »`, view: box });
    if (result.bleedFilled) {
      const sides = [t("haut"), t("droite"), t("bas"), t("gauche")].filter((_, i) => !result!.bleedFilled![i]);
      if (sides.length) out.push({ level: "error", title: t("Fonds perdus vides"), detail: t("Le fond s'arrête à la coupe : {sides}.", { sides: sides.join(", ") }), fix: { label: t("Étendre…"), run: open("printSetup") } });
      else out.push({ level: "ok", title: t("Fonds perdus remplis"), detail: t("Le fond touche les 4 bords") });
    }
    if (set !== null && r.maxPercent > 0 && r.overPixels === 0) out.push({ level: "ok", title: t("Encrage {max} % maximum", { max: r.maxPercent }), detail: t("Sous la limite de {limit} %", { limit: r.limit }) });
    if (mode.kind === "cmykWorkflow") out.push({ level: "ok", title: t("Couleurs CMJN"), detail: profileShort(exportSpace(doc).profile) });
    if (doc.dpi >= 250) out.push({ level: "ok", title: t("Résolution {dpi} ppp", { dpi: Math.round(doc.dpi) }) });
    out.push(printPrefs.textBlack ? { level: "ok", title: t("Texte noir en 100 % N") } : { level: "warn", title: t("Texte noir en quadri"), detail: t("Un léger décalage des plaques ferait un liseré coloré."), fix: { label: t("Activer"), run: () => setPref("textBlack", true) } });
    const order: Record<Level, number> = { error: 0, warn: 1, ok: 2 };
    return out.sort((a, b) => order[a.level] - order[b.level]);
  });
  const alerts = $derived(checks.filter((c) => c.level !== "ok").length);

  function view(box: [number, number, number, number]): void {
    app.preflightOpen = false;
    const pad = Math.max(box[2], box[3]) * 0.4 + 20;
    fitRect({ x: box[0] - pad, y: box[1] - pad, width: box[2] + 2 * pad, height: box[3] + 2 * pad });
    if (!printPrefs.inkAlert) setPref("inkAlert", true);
  }

  function exportPdf(): void {
    app.preflightOpen = false;
    app.exportOpen = true;
  }
</script>

<dialog bind:this={dialog} onclose={() => (app.preflightOpen = false)}>
  <h2>{result ? (alerts ? t("Vérification · {count} alerte(s)", { count: alerts }) : t("Vérification · prêt à imprimer")) : t("Vérification…")}</h2>
  {#if failed}
    <p class="muted">{t("La vérification n'a pas pu se faire.")}</p>
  {:else if !result}
    <p class="muted">{t("Analyse de l'image aplatie…")}</p>
  {:else}
    <ul>
      {#each checks as c, i (i)}
        <li>
          <span class="ic {c.level}">{c.level === "ok" ? "✓" : "!"}</span>
          <div><b>{c.title}</b>{#if c.detail}<small>{c.detail}</small>{/if}</div>
          <span class="links">
            {#if c.view}<button type="button" onclick={() => view(c.view!)}>{t("Voir")}</button>{/if}
            {#if c.fix}<button type="button" onclick={c.fix.run}>{c.fix.label}</button>{/if}
          </span>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="actions">
    <button type="button" class="ghost" onclick={() => void run()} disabled={!result && !failed}>{t("Revérifier")}</button>
    <span class="spacer"></span>
    <button type="button" class="ghost" onclick={() => (app.preflightOpen = false)}>{t("Fermer")}</button>
    <button type="button" class="pill" onclick={exportPdf}>{t("Exporter en PDF…")}</button>
  </div>
</dialog>

<style>
  dialog {
    width: 460px;
    padding: 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    color: var(--text);
    box-shadow: var(--shadow-menu);
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  h2 {
    margin: 0 0 10px;
    font-size: 14px;
    font-weight: 500;
  }
  ul {
    list-style: none;
    margin: 0 0 14px;
    padding: 0;
    max-height: 60vh;
    overflow-y: auto;
  }
  li {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    gap: 10px;
    align-items: start;
    padding: 9px 0;
    border-top: 1px solid var(--border);
  }
  li b {
    display: block;
    font-weight: 600;
    color: var(--text-strong, var(--text));
  }
  li small {
    display: block;
    margin-top: 2px;
    color: var(--text-muted);
  }
  .ic {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 700;
    color: #111;
  }
  .ic.error {
    background: #ff4d5e;
  }
  .ic.warn {
    background: #ffb020;
  }
  .ic.ok {
    background: #3ecf6e;
  }
  .links {
    display: flex;
    gap: 10px;
  }
  .links button {
    color: var(--accent);
  }
  .links button:hover {
    text-decoration: underline;
  }
  .muted {
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    gap: 8px;
  }
  .spacer {
    flex: 1;
  }
  .actions button {
    height: 28px;
    padding: 0 16px;
    border-radius: var(--radius-pill);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--pill-bg);
    color: var(--pill-text);
    font-weight: 500;
  }
</style>
