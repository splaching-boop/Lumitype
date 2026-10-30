<script lang="ts">
  // Image › Mode › Bitmap… : comme Photoshop, l'image doit d'abord être aplatie (on le demande ici, dans
  // l'application), puis la fenêtre Bitmap règle la méthode (seuil à 50 %, diffusion de Floyd–Steinberg, trame de
  // demi-teintes : linéature, angle, point rond) et la résolution de sortie. L'aperçu montre un extrait central à
  // 100 % de la sortie, avant et après, calculé par le moteur (`bitmap_preview`), avec la trame exacte du fichier.
  // OK : une seule conversion, un seul état d'historique (Ctrl+Z rend l'image et le mode d'avant).
  import { t } from "../lib/i18n.svelte";
  import { api } from "../lib/ipc";
  import { bitmapDefaults, bitmapOptions, colorModeOf, convertColorMode, modeDialogs, needsFlatten } from "../lib/color-mode.svelte";
  import { app } from "../state/app.svelte";

  const PREVIEW = { width: 200, height: 112 };
  const METHODS = [
    { id: "threshold", label: "Seuil 50 %" },
    { id: "diffusion", label: "Diffusion" },
    { id: "halftone", label: "Trame" },
  ] as const;

  let dialog = $state<HTMLDialogElement>();
  /** Étape : confirmer l'aplatissement, puis régler. */
  let step = $state<"flatten" | "settings">("settings");
  let before = $state<HTMLCanvasElement>();
  let after = $state<HTMLCanvasElement>();
  let busy = $state(false);
  let failure = $state<string | null>(null);

  const info = $derived(app.active?.info ?? null);
  const dpi = $derived(info?.dpi && info.dpi > 0 ? info.dpi : 72);
  let outputDpi = $state(72);
  const valid = $derived(Number.isFinite(outputDpi) && outputDpi >= 1 && outputDpi <= 4800 && (bitmapDefaults.method !== "halftone" || (bitmapDefaults.frequency >= 1 && bitmapDefaults.frequency <= 999 && Number.isFinite(bitmapDefaults.angle))));
  /** Taille de l'image en sortie (rééchantillonnée si la résolution change). */
  const outputSize = $derived(info ? { width: Math.max(1, Math.round((info.width * outputDpi) / dpi)), height: Math.max(1, Math.round((info.height * outputDpi) / dpi)) } : null);

  $effect(() => {
    if (modeDialogs.bitmap) {
      step = needsFlatten(app.active?.info) && colorModeOf(app.active?.info).kind !== "bitmap" ? "flatten" : "settings";
      outputDpi = bitmapDefaults.outputDpi ?? (app.active?.info.dpi || 72);
      failure = null;
      dialog?.showModal();
    } else dialog?.close();
  });

  // Aperçu : redemandé à chaque réglage (le dernier seul est affiché).
  let request = 0;
  $effect(() => {
    if (!modeDialogs.bitmap || step !== "settings" || !info || !valid) return;
    const options = { ...bitmapOptions(dpi), outputDpi };
    const mine = ++request;
    const timer = setTimeout(() => {
      // Un pixel de sortie par pixel physique de l'écran : la trame se voit telle qu'elle sera (pas de moiré de réduction).
      const ratio = window.devicePixelRatio || 1;
      api.bitmapPreview(info.id, options, Math.round(PREVIEW.width * ratio), Math.round(PREVIEW.height * ratio)).then(
        (buffer) => {
          if (mine !== request) return;
          const head = new DataView(buffer);
          const [w, h] = [head.getUint32(0, true), head.getUint32(4, true)];
          const pixels = new Uint8Array(buffer, 8);
          for (const [canvas, offset] of [[before, 0], [after, w * h]] as const) {
            if (!canvas) continue;
            [canvas.width, canvas.height] = [w, h];
            const image = new ImageData(w, h);
            for (let i = 0; i < w * h; i++) {
              const v = pixels[offset + i] ?? 0;
              image.data.set([v, v, v, 255], i * 4);
            }
            canvas.getContext("2d")?.putImageData(image, 0, 0);
          }
          failure = null;
        },
        (error: unknown) => {
          if (mine === request) failure = String((error as { message?: string })?.message ?? error);
        },
      );
    }, 120);
    return () => clearTimeout(timer);
  });

  function close(): void {
    modeDialogs.bitmap = false;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!valid || !info || busy) return;
    bitmapDefaults.outputDpi = outputDpi === dpi ? null : outputDpi;
    busy = true;
    const done = await convertColorMode({ kind: "bitmap" }, { ...bitmapOptions(dpi), outputDpi });
    busy = false;
    if (done) close();
  }
</script>

<dialog bind:this={dialog} onclose={close} aria-label={t("Bitmap")}>
  {#if step === "flatten"}
    <div class="confirm">
      <h2>{t("Aplatir l'image ?")}</h2>
      <p>{t("Le mode Bitmap n'a qu'un calque : calques, textes, formes, réglages et effets seront fusionnés en une seule image. Annuler (Ctrl+Z) rendra tout.")}</p>
      <div class="actions">
        <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
        <button type="button" class="pill" onclick={() => (step = "settings")}>{t("Aplatir")}</button>
      </div>
    </div>
  {:else}
    <form onsubmit={submit}>
      <h2>{t("Bitmap")}</h2>
      <div class="previews">
        <figure><canvas bind:this={before} aria-label={t("Avant")}></canvas><figcaption>{t("Avant")}</figcaption></figure>
        <figure><canvas bind:this={after} aria-label={t("Après")}></canvas><figcaption>{t("Après")} · 100 %</figcaption></figure>
      </div>
      <span class="caption">{t("Méthode")}</span>
      <div class="segmented" role="radiogroup" aria-label={t("Méthode")}>
        {#each METHODS as method (method.id)}
          <button type="button" role="radio" aria-checked={bitmapDefaults.method === method.id} class:on={bitmapDefaults.method === method.id} onclick={() => (bitmapDefaults.method = method.id)}>{t(method.label)}</button>
        {/each}
      </div>
      <div class="row">
        <label class="unit" title={t("Résolution de sortie : l'image est rééchantillonnée si elle diffère de celle du document")}>
          <input type="number" min="1" max="4800" step="1" bind:value={outputDpi} aria-label={t("Résolution de sortie")} />
          <span>{t("ppp")}</span>
        </label>
        {#if bitmapDefaults.method === "halftone"}
          <label class="unit" title={t("Linéature de la trame, en lignes par pouce")}>
            <input type="number" min="1" max="999" step="1" bind:value={bitmapDefaults.frequency} aria-label={t("Fréquence")} />
            <span>{t("l/po")}</span>
          </label>
        {/if}
      </div>
      {#if bitmapDefaults.method === "halftone"}
        <div class="row">
          <label class="unit" title={t("Angle de la trame")}>
            <input type="number" min="-180" max="180" step="1" bind:value={bitmapDefaults.angle} aria-label={t("Angle")} />
            <span>° {t("trame")}</span>
          </label>
          <label class="unit">
            <select aria-label={t("Forme")} disabled><option>{t("Ronde")}</option></select>
            <span>{t("forme")}</span>
          </label>
        </div>
      {/if}
      <p class="hint">
        {#if outputSize}{t("Sortie : {width} × {height} px, noir et blanc 1 bit", { width: outputSize.width, height: outputSize.height })}{/if}
        {#if failure}<br /><span class="error">{failure}</span>{/if}
      </p>
      <div class="actions">
        <button type="button" class="ghost" onclick={close}>{t("Annuler")}</button>
        <button type="submit" class="pill" disabled={!valid || busy}>OK</button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  dialog {
    width: 452px;
    padding: 18px 20px 20px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-menu);
    color: var(--text);
    box-shadow: var(--shadow-menu);
  }
  dialog::backdrop {
    background: rgba(0, 0, 0, 0.5);
  }
  form,
  .confirm {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .confirm p {
    margin: 0;
    color: var(--text-3);
    line-height: 1.45;
  }
  .previews {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  canvas {
    width: 100%;
    aspect-ratio: 200 / 112;
    object-fit: cover;
    image-rendering: pixelated;
    background: #fff;
    border-radius: var(--radius-s);
  }
  figcaption,
  .caption {
    color: var(--text-muted);
    font-size: 11px;
  }
  .caption {
    margin-bottom: -6px;
  }
  .segmented {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .segmented button {
    height: 30px;
    color: var(--text-2);
  }
  .segmented button + button {
    border-left: 1px solid var(--border-strong);
  }
  .segmented button.on {
    background: var(--bg-active);
    color: var(--text);
    font-weight: 500;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .unit {
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-m);
    background: var(--bg-field);
    color: var(--text-muted);
  }
  .unit input,
  .unit select {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    user-select: text;
  }
  .unit select:disabled {
    opacity: 1;
  }
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
    min-height: 14px;
  }
  .error {
    color: var(--danger, #ff8080);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    height: 32px;
    padding: 0 18px;
    border-radius: var(--radius-m);
  }
  .ghost {
    border: 1px solid var(--border-strong);
  }
  .pill {
    background: var(--accent, #ec2a3c);
    color: var(--on-accent);
    font-weight: 500;
  }
  .pill:disabled {
    opacity: 0.35;
  }
</style>
