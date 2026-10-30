import { defineConfig } from "vite";
import manifest from "./package.json" with { type: "json" };
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Les sources de l'interface vivent dans `ui/`, la sortie dans `dist/` (lue par Tauri).
// Version affichée dans « À propos » : celle de package.json, injectée à la compilation.
/** Pages compilées : toutes, sauf la visionneuse pour le test de fumée. */
function pages(mode: string): Record<string, string> {
  const all: Record<string, string> = { main: "ui/index.html", splash: "ui/splash.html", viewer: "ui/viewer.html" };
  if (mode === "smoke") delete all.viewer;
  return all;
}

export default defineConfig(({ mode }) => ({
  root: "ui",
  define: { __APP_VERSION__: JSON.stringify(manifest.version) },
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Trois pages : l'application, la bannière de lancement (sa propre petite fenêtre) et la visionneuse. Le test de fumée
  // (npm run smoke) ne compile que l'application et la bannière (`--mode smoke`) : sans la visionneuse, le code reste
  // dans le paquet principal, que le test sait évaluer.
  build: { outDir: "../dist", emptyOutDir: true, target: "es2022", rollupOptions: { input: pages(mode) } },
}));
