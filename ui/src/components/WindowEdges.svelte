<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  // Sans barre de titre du système, la fenêtre n'a plus de bordure à saisir : huit zones invisibles
  // de 6 px, le long des bords et aux coins, rendent le redimensionnement à la souris.
  // Elles disparaissent quand la fenêtre est agrandie (rien à redimensionner, et elles gêneraient les bords de l'écran).
  type Direction = "North" | "South" | "East" | "West" | "NorthEast" | "NorthWest" | "SouthEast" | "SouthWest";
  const EDGES: { dir: Direction; cursor: string; style: string }[] = [
    { dir: "North", cursor: "n-resize", style: "top:0;left:6px;right:6px;height:6px" },
    { dir: "South", cursor: "s-resize", style: "bottom:0;left:6px;right:6px;height:6px" },
    { dir: "West", cursor: "w-resize", style: "left:0;top:6px;bottom:6px;width:6px" },
    { dir: "East", cursor: "e-resize", style: "right:0;top:6px;bottom:6px;width:6px" },
    { dir: "NorthWest", cursor: "nw-resize", style: "top:0;left:0;width:10px;height:10px" },
    { dir: "NorthEast", cursor: "ne-resize", style: "top:0;right:0;width:10px;height:10px" },
    { dir: "SouthWest", cursor: "sw-resize", style: "bottom:0;left:0;width:10px;height:10px" },
    { dir: "SouthEast", cursor: "se-resize", style: "bottom:0;right:0;width:10px;height:10px" },
  ];

  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    const refresh = async () => (maximized = await win.isMaximized().catch(() => false));
    void refresh();
    const unlisten = win.onResized(() => void refresh());
    return () => void unlisten.then((stop) => stop());
  });
</script>

{#if !maximized}
  {#each EDGES as edge (edge.dir)}
    <div
      class="edge"
      style={edge.style}
      style:cursor={edge.cursor}
      role="presentation"
      onpointerdown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        void win.startResizeDragging(edge.dir);
      }}
    ></div>
  {/each}
{/if}

<style>
  .edge {
    position: fixed;
    z-index: 1000;
  }
</style>
