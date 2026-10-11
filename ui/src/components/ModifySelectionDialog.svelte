<script lang="ts">
  import { t } from "../lib/i18n.svelte";
  import { edit } from "../lib/engine";
  import { app } from "../state/app.svelte";

  // Retouche du contour d'une sélection : la dilater ou la contracter de quelques pixels (adieu le
  // liseré autour d'un détourage), et arrondir ses irrégularités.
  let dialog = $state<HTMLDialogElement>();
  let amount = $state(0);
  let smooth = $state(0);

  $effect(() => {
    dialog?.showModal();
  });

  function close(apply: boolean): void {
    app.modifySelectionOpen = false;
    if (apply && (amount !== 0 || smooth !== 0)) void edit({ op: "modifySelection", amount, smooth });
  }
</script>

<dialog bind:this={dialog} oncancel={(event) => (event.preventDefault(), close(false))}>
  <form onsubmit={(event) => (event.preventDefault(), close(true))}>
    <h2>{t("Modifier la sélection")}</h2>
    <label>
      <span>{amount < 0 ? "Contracter" : "Dilater"}</span>
      <output>{amount > 0 ? "+" : ""}{amount} px</output>
      <input type="range" min="-100" max="100" step="1" bind:value={amount} />
    </label>
    <label>
      <span>{t("Lisser le contour")}</span>
      <output>{t("{smooth} px", { smooth })}</output>
      <input type="range" min="0" max="60" step="1" bind:value={smooth} />
    </label>
    <p>{t("Contracter d'un ou deux pixels supprime le liseré de fond qui reste autour d'un détourage à la baguette. Le lissage arrondit les dents et les petits trous. Pour un bord fondu, utilisez ensuite Contour progressif.")}</p>
    <footer>
      <button type="button" class="ghost" onclick={() => close(false)}>{t("Annuler")}</button>
      <button type="submit" class="pill">OK</button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 400px;
    box-sizing: border-box;
    padding: 22px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--bg-panel);
    color: var(--text);
    box-shadow: 0 30px 90px rgba(0, 0, 0, 0.65);
  }
  dialog::backdrop {
    background: rgba(5, 5, 6, 0.5);
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 500;
  }
  label {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 4px;
    color: var(--text-3);
  }
  input {
    grid-column: 1 / -1;
    width: 100%;
  }
  output {
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  p {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.5;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .ghost,
  .pill {
    height: 32px;
    padding: 0 18px;
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
