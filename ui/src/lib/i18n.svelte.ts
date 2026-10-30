// Langue de l'interface. Le français est la langue de référence : chaque chaîne est écrite en
// français dans le code, et `t("…")` en rend la traduction dans la langue choisie — ou le français
// lui-même si la traduction manque. Rien à inventer comme identifiant, rien à casser dans les
// catalogues existants (actions, filtres, aides d'outils) : ils restent en français, et c'est au
// moment de l'affichage que la traduction se fait.
//
// Les dictionnaires vivent dans `locales/<code>.json` : une table « français → traduction ».
// Une chaîne à variables garde ses `{nom}` dans les deux langues ; `t` les remplit après coup.
import { api } from "./ipc";
import de from "./locales/de.json";
import en from "./locales/en.json";
import es from "./locales/es.json";

export type Locale = "fr" | "en" | "es" | "de";

export interface LocaleInfo {
  code: Locale;
  /** Nom de la langue, dans cette langue : c'est ainsi qu'on la reconnaît dans une liste. */
  name: string;
  /** Étiquette BCP 47 pour les dates et les nombres. */
  tag: string;
}

export const LOCALES: LocaleInfo[] = [
  { code: "fr", name: "Français", tag: "fr-FR" },
  { code: "en", name: "English", tag: "en-GB" },
  { code: "es", name: "Español", tag: "es-ES" },
  { code: "de", name: "Deutsch", tag: "de-DE" },
];

const DICTIONARIES: Record<Exclude<Locale, "fr">, Record<string, string>> = { en, es, de };

const SETTINGS_NAME = "language";

let current = $state<Locale>("fr");

export const i18n = {
  get locale(): Locale {
    return current;
  },
  /** Change la langue et l'enregistre : elle sera la même au prochain lancement. */
  set locale(next: Locale) {
    if (next === current) return;
    current = next;
    document.documentElement.lang = next;
    void api.saveSettings(SETTINGS_NAME, JSON.stringify({ locale: next })).catch(() => undefined);
  },
  get info(): LocaleInfo {
    return LOCALES.find((l) => l.code === current) ?? LOCALES[0]!;
  },
};

/** Au démarrage : relit la langue enregistrée. Absente ou inconnue : français. */
export async function loadLocale(): Promise<void> {
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : null;
    const locale = stored && typeof stored === "object" ? (stored as { locale?: unknown }).locale : null;
    if (LOCALES.some((l) => l.code === locale)) {
      current = locale as Locale;
      document.documentElement.lang = current;
    }
  } catch {
    // réglage illisible : on reste en français
  }
}

/**
 * Traduit une chaîne française. `vars` remplit les `{nom}` ; une chaîne sans traduction revient
 * telle quelle, en français, plutôt que vide : l'interface reste toujours lisible.
 */
export function t(text: string, vars?: Record<string, string | number>): string {
  const found = current === "fr" ? text : (DICTIONARIES[current][text] ?? byPattern(current, text) ?? text);
  // Clé à contexte, « Édition|Annuler » : un même mot français qui se traduit différemment selon l'endroit
  // (Undo / Cancel). Le contexte sert seulement à la recherche dans le dictionnaire, il n'est jamais affiché.
  // Seul un « | » collé (sans espace autour) sépare un contexte : « Avant | Après » reste un texte ordinaire.
  const context = /^[^|]*\S\|(?=\S)/.exec(found);
  const translated = context ? found.slice(context[0].length) : found;
  if (!vars) return translated;
  return translated.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole));
}

/**
 * Message déjà rempli (erreur du moteur : « le calque « Fond » est verrouillé ») : il ne se trouve pas tel quel
 * dans le dictionnaire, mais sa phrase à variables, elle, y est. On la retrouve par motif, et la traduction
 * reprend les morceaux variables à leur place. Seules les chaînes à `{…}` sont candidates, et seulement
 * quand la recherche exacte a échoué : le coût ne se paie que sur les messages à variables.
 */
type Pattern = { regex: RegExp; names: string[]; translation: string; fixed: number };
const PATTERNS = new Map<Locale, Pattern[]>();

function byPattern(locale: Exclude<Locale, "fr">, text: string): string | null {
  let patterns = PATTERNS.get(locale);
  if (!patterns) {
    patterns = [];
    for (const [key, translation] of Object.entries(DICTIONARIES[locale])) {
      if (!/\{\w*\}/.test(key)) continue;
      const names: string[] = [];
      const source = key
        .split(/(\{\w*\})/)
        .map((part, index) => {
          if (index % 2 === 0) return part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
          names.push(part.slice(1, -1));
          return "([\\s\\S]+?)";
        })
        .join("");
      patterns.push({ regex: new RegExp(`^${source}$`), names, translation, fixed: key.replace(/\{\w*\}/g, "").length });
    }
    // Les motifs les plus précis d'abord (le plus de texte fixe) : « {a} : {b} » ne doit pas attraper une
    // phrase entière qui a sa propre traduction.
    patterns.sort((a, b) => b.fixed - a.fixed);
    PATTERNS.set(locale, patterns);
  }
  for (const { regex, names, translation } of patterns) {
    const match = regex.exec(text);
    if (!match) continue;
    // `{nom}` reprend le morceau de même nom ; `{}` (sans nom) les reprend dans l'ordre.
    let seen = 0;
    return translation.replace(/\{(\w*)\}/g, (whole, name: string) => {
      let index = names.indexOf(name);
      if (name === "") {
        let rank = 0;
        index = names.findIndex((n) => n === "" && rank++ === seen);
        seen += 1;
      }
      return index >= 0 ? (match[index + 1] ?? whole) : whole;
    });
  }
  return null;
}

/** Format d'une date dans la langue courante (`articleDate` et les récents). */
export function formatDate(date: Date, options: Intl.DateTimeFormatOptions): string {
  return date.toLocaleDateString(i18n.info.tag, options);
}
