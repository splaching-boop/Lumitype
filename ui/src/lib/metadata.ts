// Métadonnées par défaut des images exportées (menu Copyright) : valeurs, lecture au démarrage, enregistrement,
// et construction de ce que le moteur inscrit dans chaque fichier (voir crates/engine/src/metadata.rs).
import { app } from "../state/app.svelte";
import { type ImageMetadata, api } from "./ipc";

export interface MetadataSettings {
  firstName: string;
  lastName: string;
  /** Mention de copyright complète, telle qu'elle sera inscrite (ex. « © 2026 Antoine Degardin — Tous droits réservés »). */
  copyright: string;
  /** Site ou adresse de contact : XMP xmpRights:WebStatement, PNG Source. */
  contact: string;
  /** Mots-clés séparés par des virgules. */
  keywords: string;
  description: string;
  /** Inscrire ces métadonnées dans les images exportées. Décoché : rien n'est écrit, quels que soient les champs. */
  embed: boolean;
  /** Ajouter « Lumitype x.y.z » dans le champ Logiciel. */
  software: boolean;
  /** Inscrire la date et l'heure de l'export comme date de création. */
  date: boolean;
}

export const DEFAULT_METADATA: MetadataSettings = { firstName: "", lastName: "", copyright: "", contact: "", keywords: "", description: "", embed: true, software: true, date: false };

const SETTINGS_NAME = "metadata";

/** Au démarrage : relit les réglages enregistrés (les valeurs absentes gardent leur défaut). */
export async function loadMetadata(): Promise<void> {
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : {};
    if (stored && typeof stored === "object") app.metadata = { ...DEFAULT_METADATA, ...(stored as Partial<MetadataSettings>) };
  } catch {
    app.metadata = { ...DEFAULT_METADATA };
  }
}

export function saveMetadata(settings: MetadataSettings): void {
  app.metadata = { ...settings };
  void api.saveSettings(SETTINGS_NAME, JSON.stringify(app.metadata, null, 2)).catch((error: unknown) => app.fail(error));
}

/** Mots-clés : séparés par des virgules (ou des points-virgules), sans doublon ni blanc. */
export function splitKeywords(text: string): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of text.split(/[,;]/)) {
    const word = raw.trim();
    const key = word.toLocaleLowerCase();
    if (word !== "" && !seen.has(key)) {
      seen.add(key);
      out.push(word);
    }
  }
  return out;
}

/** Nom complet de l'auteur, tel qu'il sera inscrit. */
export const authorName = (settings: MetadataSettings): string => `${settings.firstName.trim()} ${settings.lastName.trim()}`.trim();

/** Mention de copyright proposée quand le champ est vide : « © année Prénom Nom ». */
export const suggestedCopyright = (settings: MetadataSettings): string => {
  const author = authorName(settings);
  return author ? `© ${new Date().getFullYear()} ${author}` : "";
};

const pad = (n: number) => String(n).padStart(2, "0");

/** Date et heure courantes aux deux formats attendus : EXIF (`AAAA:MM:JJ HH:MM:SS`) et ISO 8601 avec fuseau. */
export function stampNow(now = new Date()): { created: string; createdIso: string } {
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
  const time = `${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;
  const offset = -now.getTimezoneOffset();
  const zone = `${offset >= 0 ? "+" : "-"}${pad(Math.floor(Math.abs(offset) / 60))}:${pad(Math.abs(offset) % 60)}`;
  return { created: `${date.replaceAll("-", ":")} ${time}`, createdIso: `${date}T${time}${zone}` };
}

/**
 * Ce que le moteur inscrit dans un export : les réglages par défaut, surchargés pour cette image (description et
 * mots-clés propres à l'image). `null` si l'inscription est décochée ou s'il n'y a rien à écrire.
 */
export function buildMetadata(settings: MetadataSettings, image?: { description?: string; keywords?: string }, now = new Date()): ImageMetadata | null {
  if (!settings.embed) return null;
  const stamp = settings.date ? stampNow(now) : { created: "", createdIso: "" };
  const meta: ImageMetadata = {
    author: authorName(settings),
    copyright: settings.copyright.trim(),
    description: (image?.description ?? settings.description).trim(),
    keywords: splitKeywords(image?.keywords ?? settings.keywords),
    contact: settings.contact.trim(),
    software: settings.software ? `Lumitype ${__APP_VERSION__}` : "",
    ...stamp,
  };
  const empty = !meta.author && !meta.copyright && !meta.description && meta.keywords.length === 0 && !meta.contact && !meta.software && !meta.created;
  return empty ? null : meta;
}
