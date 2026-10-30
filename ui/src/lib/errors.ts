// Erreurs reçues par l'interface. Le moteur envoie `{ code, message }` (voir `CommandError` côté Rust) : un code
// stable (E-FAMILLE-NNN), qui identifie le problème même quand le message est reformulé ou traduit, et un message
// lisible en français. Les autres erreurs (chaînes, exceptions JavaScript) n'ont pas de code.
import { t } from "./i18n.svelte";

export interface AppError {
  /** Code stable du moteur, ou `null` pour une erreur de l'interface elle-même. */
  code: string | null;
  /** Message en français : l'affichage le passe par `t()`. */
  message: string;
}

/** Ramène n'importe quelle erreur rattrapée à `{ code, message }`. */
export function toAppError(error: unknown): AppError {
  if (typeof error === "string") return { code: null, message: error };
  if (error && typeof error === "object") {
    const { code, message } = error as { code?: unknown; message?: unknown };
    if (typeof message === "string") return { code: typeof code === "string" ? code : null, message };
  }
  return { code: null, message: String(error) };
}

/** Texte d'une erreur pour un champ d'une fenêtre : le message traduit, précédé du code s'il y en a un. */
export function describeError(error: unknown): string {
  const { code, message } = toAppError(error);
  return code ? `${code} · ${t(message)}` : t(message);
}
