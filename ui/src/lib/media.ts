// Images, animations et démonstrations filmées de l'aide, en un seul endroit.
//
// - `assets/media/demos/<article>.mp4`     démonstration filmée d'un guide : elle sert à la fois à la
//   bulle de la barre d'outils, au bandeau du guide et au lecteur dans son corps ;
// - `assets/media/tools/<outil>.gif`       ancienne animation d'un outil, encore acceptée ;
// - `assets/media/tutorials/<article>.gif` ancienne animation d'un tutoriel, encore acceptée ;
// - `assets/media/placeholder.gif`         ce qui s'affiche tant que rien n'a été tourné ;
// - `assets/help/<nom>.jpg`                illustrations fixes fabriquées par le moteur
//   (`cargo run -p engine --release --example adjust_previews`).
//
// Déposer un fichier au bon nom suffit : rien à déclarer ici ni ailleurs, sauf pour rattacher un
// outil à son guide (table `TOOL_DEMO` ci-dessous).
import placeholder from "../assets/media/placeholder.gif";

// Deux encodages du même film. **WebM/VP9 d'abord** : c'est le seul décodé partout sans greffon
// supplémentaire — les compilations libres de Chromium et WebKitGTK sur Fedora n'ont pas H.264 par
// défaut, et le film y resterait noir. Le MP4 reste en second pour les systèmes qui préfèrent H.264.
const demoWebm = import.meta.glob<string>("../assets/media/demos/*.webm", { eager: true, import: "default" });
const demoMp4 = import.meta.glob<string>("../assets/media/demos/*.mp4", { eager: true, import: "default" });
/** Image fixe de chaque film (`<id>.jpg`, une image prise à 1,5 s) : la vignette au repos, avant que le film ne tourne. */
const demoPosters = import.meta.glob<string>("../assets/media/demos/*.jpg", { eager: true, import: "default" });
const toolGifs = import.meta.glob<string>("../assets/media/tools/*.gif", { eager: true, import: "default" });
const tutorialGifs = import.meta.glob<string>("../assets/media/tutorials/*.gif", { eager: true, import: "default" });
const pictures = import.meta.glob<string>("../assets/help/*.jpg", { eager: true, import: "default" });

/**
 * Quel guide démontre quel outil. Les démonstrations sont nommées d'après l'article et non d'après
 * l'outil : un même film sert au guide et à la bulle, et rien n'est stocké deux fois.
 */
const TOOL_DEMO: Record<string, string> = {
  move: "outil-deplacement",
  marquee: "outil-selection",
  lasso: "outil-lasso",
  wand: "outil-baguette",
  quickSelect: "outil-selection-rapide",
  crop: "outil-recadrage",
  eyedropper: "outil-pipette",
  heal: "outil-correcteur",
  brush: "outil-pinceau",
  eraser: "outil-gomme",
  fill: "outil-degrade-pot",
  pen: "outil-plume",
  text: "outil-texte",
  pathSelect: "outil-selection-directe",
  shape: "outil-forme",
  hand: "outil-main",
  zoom: "outil-zoom",
};

/** Les encodages disponibles d'une démonstration, du plus sûr au moins sûr. Vide si rien n'a été tourné. */
export function demoSources(article: string): { src: string; type: string }[] {
  const sources: { src: string; type: string }[] = [];
  const webm = demoWebm[`../assets/media/demos/${article}.webm`];
  const mp4 = demoMp4[`../assets/media/demos/${article}.mp4`];
  if (webm) sources.push({ src: webm, type: "video/webm" });
  if (mp4) sources.push({ src: mp4, type: "video/mp4" });
  return sources;
}

/** Vraie tant que le guide a une démonstration filmée : l'appelant choisit alors la vidéo plutôt qu'un repli. */
export const hasDemo = (article: string): boolean => demoSources(article).length > 0;

/**
 * Le film d'un guide, chargé en mémoire puis servi par une adresse `blob:` — avec son type d'encodage.
 *
 * Les fichiers de l'application sont servis par le protocole interne de Tauri, d'un seul bloc et sans lecture par
 * morceaux (requêtes partielles) ; les lecteurs vidéo de WebKitGTK comme de Chromium refusent souvent de lire un
 * film servi ainsi : bulle vide, lecteur à 0:00. Lu depuis la mémoire, le même film passe partout. Chaque film n'est
 * chargé qu'une fois (quelques centaines de Ko), et son adresse est gardée pour toute la session.
 */
const films = new Map<string, Promise<{ src: string; type: string } | null>>();
export function loadFilm(article: string): Promise<{ src: string; type: string } | null> {
  let loading = films.get(article);
  if (!loading) {
    const [first] = demoSources(article);
    loading = first
      ? fetch(first.src)
          .then((response) => (response.ok ? response.blob() : Promise.reject(new Error(response.statusText))))
          .then((blob) => ({ src: URL.createObjectURL(blob.type ? blob : new Blob([blob], { type: first.type })), type: first.type }))
          .catch(() => {
            films.delete(article); // réessayé à la prochaine demande
            return null;
          })
      : Promise.resolve(null);
    films.set(article, loading);
  }
  return loading;
}
/** Vignette fixe du film d'un guide ; vide s'il n'y en a pas. */
export const demoPoster = (article: string): string => demoPosters[`../assets/media/demos/${article}.jpg`] ?? "";

/** Le guide qui démontre cet outil, ou `null`. */
export const toolDemoArticle = (tool: string): string | null => TOOL_DEMO[tool] ?? null;
/** Les encodages de la démonstration de l'outil, par son guide. */
export const toolDemoSources = (tool: string): { src: string; type: string }[] => demoSources(TOOL_DEMO[tool] ?? "");

/** Animation d'un outil, ou l'animation de remplacement. */
export const toolAnimation = (tool: string): string => toolGifs[`../assets/media/tools/${tool}.gif`] ?? placeholder;

/** Animation d'un tutoriel, ou l'animation de remplacement. */
export const tutorialAnimation = (name: string): string => tutorialGifs[`../assets/media/tutorials/${name}.gif`] ?? placeholder;

/** Faux tant que rien n'a été tourné pour ce tutoriel, film ou animation : l'article le signale au lecteur. */
export const hasTutorialAnimation = (name: string): boolean => hasDemo(name) || `../assets/media/tutorials/${name}.gif` in tutorialGifs;

/** Illustration fixe (moitié gauche corrigée, moitié droite d'origine), vide si elle n'a pas été produite. */
export const illustration = (name: string): string => pictures[`../assets/help/${name}.jpg`] ?? "";

let preloaded = false;
/**
 * Met les médias en cache quand l'application n'a rien à faire : sans cela, la toute première bulle
 * attendrait le chargement de son média et s'afficherait vide pendant un instant. Les vidéos sont
 * demandées en `metadata` seulement — de quoi connaître leurs dimensions et amorcer le cache du
 * navigateur, sans télécharger deux mégaoctets au démarrage.
 */
export function preloadAnimations(): void {
  if (preloaded) return;
  preloaded = true;
  for (const url of new Set([placeholder, ...Object.values(toolGifs)])) new Image().src = url;
  // Les films des outils, en mémoire : la première bulle s'affiche sans attendre.
  for (const article of new Set(Object.values(TOOL_DEMO))) void loadFilm(article);
}
