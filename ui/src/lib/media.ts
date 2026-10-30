// Démonstrations filmées des outils, en un seul endroit : la bulle de la barre d'outils (ToolTip.svelte) et les
// cartes d'outils de la notice (components/home/ToolDemoCard.svelte) lisent le même film.
//
// - `assets/media/demos/<nom>.webm` et `.mp4`  le film, en deux encodages (voir le README du dossier) ;
// - `assets/media/demos/<nom>.jpg`             son image fixe, montrée au repos.
//
// Déposer un fichier au bon nom suffit : rien à déclarer ici ni ailleurs, sauf pour rattacher un outil à son film
// (table `TOOL_DEMO` ci-dessous).

// Deux encodages du même film. **WebM/VP9 d'abord** : c'est le seul décodé partout sans greffon
// supplémentaire — les compilations libres de Chromium et WebKitGTK sur Fedora n'ont pas H.264 par
// défaut, et le film y resterait noir. Le MP4 reste en second pour les systèmes qui préfèrent H.264.
const demoWebm = import.meta.glob<string>("../assets/media/demos/*.webm", { eager: true, import: "default" });
const demoMp4 = import.meta.glob<string>("../assets/media/demos/*.mp4", { eager: true, import: "default" });
/** Image fixe de chaque film (`<id>.jpg`, une image prise à 1,5 s) : la vignette au repos, avant que le film ne tourne. */
const demoPosters = import.meta.glob<string>("../assets/media/demos/*.jpg", { eager: true, import: "default" });

/**
 * Quel film démontre quel outil. Les films gardent le nom des anciens guides (`outil-…`) : un même fichier sert à
 * la bulle et à la notice, et rien n'est stocké deux fois.
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

/**
 * Un film, chargé en mémoire puis servi par une adresse `blob:` — avec son type d'encodage.
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
/** Image fixe d'un film ; vide s'il n'y en a pas. */
export const demoPoster = (article: string): string => demoPosters[`../assets/media/demos/${article}.jpg`] ?? "";

/** Le film qui démontre cet outil, ou `null`. */
export const toolDemoArticle = (tool: string): string | null => TOOL_DEMO[tool] ?? null;
/** Les encodages du film de l'outil. */
export const toolDemoSources = (tool: string): { src: string; type: string }[] => demoSources(TOOL_DEMO[tool] ?? "");

let preloaded = false;
/**
 * Met les films des outils en mémoire quand l'application n'a rien à faire : sans cela, la toute première bulle
 * attendrait le chargement de son film et s'afficherait vide pendant un instant.
 */
export function preloadAnimations(): void {
  if (preloaded) return;
  preloaded = true;
  for (const article of new Set(Object.values(TOOL_DEMO))) void loadFilm(article);
}
