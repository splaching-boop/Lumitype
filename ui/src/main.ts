import { mount } from "svelte";
import "@fontsource-variable/geist";
import "./theme/tokens.css";
import "./theme/base.css";
import App from "./App.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Élément #app introuvable dans index.html");
}

// La webview n'est pas un navigateur : pas de menu contextuel « Recharger / Inspecter ».
window.addEventListener("contextmenu", (event) => event.preventDefault());

export default mount(App, { target });
