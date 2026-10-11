import { t } from "./i18n.svelte";
// Images AVIF. Le moteur ne décode pas l'AV1 ; la webview, elle, sait le faire (WebView2 sous Windows, WebKitGTK
// sous Linux) : l'image est décodée ici, puis confiée au moteur en PNG, sans perte (8 bits par canal).
import { api } from "./ipc";

export const isAvif = (name: string, type = ""): boolean => type === "image/avif" || /\.avif$/i.test(name);

/** Nom du fichier converti : même nom, extension .png (le calque ou le document garde le nom d'origine). */
const pngName = (name: string): string => `${name.replace(/\.avif$/i, "") || "Image"}.png`;

/** Décode une image AVIF et la rend en fichier PNG. */
export async function avifToPng(data: Blob, name: string): Promise<File> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(data.type === "image/avif" ? data : new Blob([data], { type: "image/avif" }));
  } catch {
    throw new Error(`Impossible de lire l'image AVIF « ${name} » : ce système ne sait pas décoder ce format.`);
  }
  const canvas = document.createElement("canvas");
  [canvas.width, canvas.height] = [bitmap.width, bitmap.height];
  const context = canvas.getContext("2d");
  if (!context) throw new Error(t("Conversion de l'image AVIF impossible."));
  context.drawImage(bitmap, 0, 0);
  bitmap.close();
  const png = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!png) throw new Error(`Conversion de l'image AVIF « ${name} » impossible.`);
  return new File([png], pngName(name), { type: "image/png" });
}

/** Fichier AVIF désigné par son chemin (boîte d'ouverture, double-clic) : lu, décodé, rendu en PNG. */
export async function avifPathToPng(path: string): Promise<File> {
  const bytes = await api.readFileBytes(path);
  return avifToPng(new Blob([bytes], { type: "image/avif" }), path.split(/[\\/]/).pop() ?? "Image.avif");
}
