// Visionneuse Lumitype (fenêtre « viewer », lancée par `lumitype --visionneuse`) : voir viewer/Viewer.svelte.
import { mount } from "svelte";
import "@fontsource-variable/geist";
import "./theme/tokens.css";
import "./theme/base.css";
import Viewer from "./viewer/Viewer.svelte";
import { loadLocale } from "./lib/i18n.svelte";
import { appearance } from "./lib/appearance.svelte";

window.addEventListener("contextmenu", (event) => event.preventDefault());

const target = document.getElementById("viewer");
if (!target) throw new Error("Élément #viewer introuvable dans viewer.html");

// Langue de l'éditeur (mêmes réglages), avant le premier affichage.
await loadLocale();
void appearance.init();

export default mount(Viewer, { target });
