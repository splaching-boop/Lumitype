import { defineConfig } from "vite";
import manifest from "./package.json" with { type: "json" };
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Les sources de l'interface vivent dans `ui/`, la sortie dans `dist/` (lue par Tauri).
// Version affichée dans « À propos » : celle de package.json, injectée à la compilation.
export default defineConfig({
  root: "ui",
  define: { __APP_VERSION__: JSON.stringify(manifest.version) },
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // Deux pages : l'application, et la bannière de lancement (sa propre petite fenêtre).
  build: { outDir: "../dist", emptyOutDir: true, target: "es2022", rollupOptions: { input: { main: "ui/index.html", splash: "ui/splash.html" } } },
});
