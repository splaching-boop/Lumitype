import { mount } from "svelte";
import "@fontsource-variable/geist";
import "./theme/tokens.css";
import "./theme/base.css";
import App from "./App.svelte";
import { api } from "./lib/ipc";
import { appearance } from "./lib/appearance.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Élément #app introuvable dans index.html");
}

// La webview n'est pas un navigateur : pas de menu contextuel « Recharger / Inspecter ».
window.addEventListener("contextmenu", (event) => event.preventDefault());

// Webview rechargée : les fenêtres « Développement RAW » de la page précédente ont disparu, leurs sessions aussi.
void api.rawCloseAll().catch(() => undefined);

// Thème et couleur d'accent (Préférences › Apparence), avant le premier affichage autant que possible.
void appearance.init();

export default mount(App, { target });
