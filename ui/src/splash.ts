// Bannière de lancement (fenêtre « splashscreen »). Elle ne charge rien elle-même : la fenêtre
// principale, encore cachée, prépare l'application et lui envoie l'étape en cours et l'avancement.
import { listen } from "@tauri-apps/api/event";
import logo from "./assets/logo/logo-256.png";

// Deux façons de personnaliser la bannière, dans ui/src/assets/splash/ :
//   - banner.(jpg|png|webp)     : bannière COMPLÈTE, affichée seule sur toute la fenêtre (1040 x 430, soit un rapport
//                                 de 2,42 ; idéalement 2080 x 860 px ou plus). Elle porte elle-même le nom du logiciel.
//   - background.(jpg|png|webp) : simple image de fond ; le nom « Lumitype » est alors écrit par-dessus, à gauche.
const pick = (files: Record<string, string>) => Object.values(files)[0];
const banner = pick(import.meta.glob<string>("./assets/splash/banner.{jpg,jpeg,png,webp}", { eager: true, import: "default", query: "?url" }));
const background = pick(import.meta.glob<string>("./assets/splash/background.{jpg,jpeg,png,webp}", { eager: true, import: "default", query: "?url" }));

const image = document.getElementById("image");
if (image && (banner ?? background)) image.style.backgroundImage = `url("${banner ?? background}")`;
if (banner) document.getElementById("splash")?.classList.add("full");

const logoElement = document.getElementById("logo");
if (logoElement instanceof HTMLImageElement) logoElement.src = logo;
const version = document.getElementById("version");
if (version) version.textContent = `v${__APP_VERSION__}`;

void listen<{ label: string; progress: number }>("splash-status", ({ payload }) => {
  const status = document.getElementById("status");
  const progress = document.getElementById("progress");
  if (status) status.textContent = payload.label.toUpperCase();
  if (progress) progress.style.width = `${Math.round(Math.min(1, Math.max(0.04, payload.progress)) * 100)}%`;
});
