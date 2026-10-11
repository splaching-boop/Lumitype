// Bannière de l'auteur (dossier assets/splash), si elle existe : photo des articles et du bandeau « avant / après ».
export const banner: string | undefined = Object.values(
  import.meta.glob<string>("../../assets/splash/banner.{jpg,jpeg,png,webp}", { eager: true, import: "default", query: "?url" }),
)[0];
