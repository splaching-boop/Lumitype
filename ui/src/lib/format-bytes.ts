// Taille d'un fichier en toutes lettres (« 216 Ko », « 4,2 Mo »), dans la langue de l'interface : l'unité passe
// par le dictionnaire (« Mo » → « MB »), la décimale suit les usages de la langue (virgule en français).
import { i18n, t } from "./i18n.svelte";

const KIB = 1024;
const MIB = 1024 * 1024;

/** Une décimale, séparateur de la langue courante ; sans `Intl` (tests hors navigateur), point ou virgule à la main. */
function oneDecimal(value: number): string {
  try {
    return new Intl.NumberFormat(i18n.info.tag, { minimumFractionDigits: 1, maximumFractionDigits: 1 }).format(value);
  } catch {
    const text = value.toFixed(1);
    return i18n.locale === "fr" ? text.replace(".", ",") : text;
  }
}

/** « 512 o », « 216 Ko » (au moins 1 Ko dès 1024 octets), « 4,2 Mo » à partir d'un mébioctet. */
export function formatBytes(size: number): string {
  const bytes = Math.max(0, Math.round(size));
  if (bytes < KIB) return t("{n} o", { n: bytes });
  if (bytes < MIB) return t("{n} Ko", { n: Math.max(1, Math.round(bytes / KIB)) });
  return t("{n} Mo", { n: oneDecimal(bytes / MIB) });
}
