<script lang="ts">
  import { t } from "../../lib/i18n.svelte";
  import { api, type DocumentInfo } from "../../lib/ipc";
  import { exportSpace } from "../../lib/color-mode.svelte";
  import { printPrefs, setPref } from "../../lib/print.svelte";

  // Affichage › Impression › Séparations : l'image séparée par plaque (cyan, magenta, jaune, noir), chaque plaque
  // dans la couleur de son encre sur papier blanc. Un clic sur une plaque l'agrandit ; un second clic les remontre
  // toutes. Recalculée (en réduit) quand le document change.
  let { info }: { info: DocumentInfo } = $props();
  // Les lettres des plaques suivent le sigle traduit (« CMJN » → « CMYK ») : même ordre que les plaques.
  const PLATES = [
    { id: "c", at: 0, name: "Cyan" },
    { id: "m", at: 1, name: "Magenta" },
    { id: "y", at: 2, name: "Jaune" },
    { id: "k", at: 3, name: "Noir" },
  ] as const;
  const letters = $derived(t("CMJN"));
  let urls = $state<string[]>([]);
  let busy = $state(false);

  // Chaque relance porte un numéro : une réponse arrivée après une demande plus récente (ou après le démontage)
  // est jetée sans toucher aux URLs ni à `busy`, qui ne retombe qu'avec la dernière demande en vol.
  let serial = 0;
  $effect(() => {
    const doc = info;
    void doc.revision;
    const space = exportSpace(doc);
    const mine = ++serial;
    const timer = setTimeout(async () => {
      busy = true;
      try {
        const body = await api.separations(doc.id, space, 520);
        if (mine !== serial) return;
        const view = new DataView(body);
        let at = 8;
        const next: string[] = [];
        for (let i = 0; i < 4; i++) {
          const len = view.getUint32(at, true);
          at += 4;
          next.push(URL.createObjectURL(new Blob([body.slice(at, at + len)], { type: "image/png" })));
          at += len;
        }
        urls.forEach((u) => URL.revokeObjectURL(u));
        urls = next;
      } catch {
        // Document fermé entre-temps : rien à montrer.
      } finally {
        if (mine === serial) busy = false;
      }
    }, 500);
    return () => {
      clearTimeout(timer);
      if (mine === serial) serial++;
    };
  });
  $effect(() => () => urls.forEach((u) => URL.revokeObjectURL(u)));

  const selected = $derived(PLATES.findIndex((p) => p.id === printPrefs.plate));
</script>

<section class="separations" aria-label={t("Séparations")}>
  <header>
    <b>{t("Séparations")}</b>
    {#if busy}<span class="muted">…</span>{/if}
    <button type="button" class="close" aria-label={t("Fermer")} onclick={() => setPref("separations", false)}>✕</button>
  </header>
  {#if selected >= 0 && urls[selected]}
    <button type="button" class="big" title={t("Revoir toutes les plaques")} onclick={() => setPref("plate", "all")}>
      <img src={urls[selected]} alt={t(PLATES[selected]?.name ?? "")} />
    </button>
  {/if}
  <div class="plates">
    {#each PLATES as plate, i (plate.id)}
      <button type="button" class="plate {plate.id}" class:on={printPrefs.plate === plate.id} title={t(plate.name)} onclick={() => setPref("plate", printPrefs.plate === plate.id ? "all" : plate.id)}>
        {#if urls[i]}<img src={urls[i]} alt="" />{/if}
        <span>{letters[plate.at] ?? ""}</span>
      </button>
    {/each}
  </div>
  <p class="muted">{t("Clic sur une plaque : la voir seule.")}</p>
</section>

<style>
  .separations {
    position: absolute;
    top: 12px;
    right: 12px;
    z-index: 6;
    width: 300px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border-strong);
    background: var(--bg-panel);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  header b {
    flex: 1;
    font-weight: 500;
  }
  .close {
    color: var(--text-muted);
  }
  .big {
    display: block;
    border-radius: var(--radius-m);
    overflow: hidden;
    background: #fff;
  }
  .big img {
    display: block;
    width: 100%;
    max-height: 260px;
    object-fit: contain;
  }
  .plates {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
  }
  .plate {
    position: relative;
    height: 56px;
    border-radius: var(--radius-m);
    overflow: hidden;
    background: #fff;
    border: 2px solid transparent;
  }
  .plate.on {
    border-color: var(--accent);
  }
  .plate img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .plate span {
    position: absolute;
    left: 4px;
    bottom: 3px;
    padding: 0 4px;
    border-radius: 3px;
    font-weight: 700;
    font-size: 11px;
    color: #111;
    background: rgba(255, 255, 255, 0.85);
  }
  .plate.c span {
    box-shadow: inset 0 -2px #00aeef;
  }
  .plate.m span {
    box-shadow: inset 0 -2px #ec008c;
  }
  .plate.y span {
    box-shadow: inset 0 -2px #e6d600;
  }
  .plate.k span {
    box-shadow: inset 0 -2px #231f20;
  }
  .muted {
    margin: 0;
    font-size: 11px;
    color: var(--text-muted);
  }
</style>
