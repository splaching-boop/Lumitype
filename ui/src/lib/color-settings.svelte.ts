// Paramètres de couleur (Édition › Paramètres de couleur…, Maj+Ctrl+K), comme Photoshop : un seul CMJN pour tout
// Lumitype. L'espace CMJN choisi (profil de presse, intention de rendu, compensation du point noir) sert à l'épreuve
// écran (Affichage › Épreuve CMJN, Ctrl+Y), à la conversion Image › Mode › Couleurs CMJN et à l'export CMJN d'un
// document RVB. Un document déjà en mode Couleurs CMJN garde l'espace de sa conversion (comme Photoshop garde le
// profil d'un document CMJN) ; repasser Image › Mode › Couleurs CMJN lui applique les paramètres du moment.
//
// Les tables sont calculées par le moteur (LittleCMS, crates/engine/src/cmyk.rs) pour n'importe quel profil : les
// cinq livrés, ceux installés sur l'ordinateur (par Photoshop notamment) et ceux chargés ici (recopiés dans le
// dossier de réglages de Lumitype). Enregistré dans les réglages `couleurs`.
import { api, type CmykProfileEntry, type CmykSpace } from "./ipc";

export type { CmykSpace, CmykProfileEntry } from "./ipc";

/** Réglage par défaut de Photoshop en Europe : Coated FOGRA39, colorimétrie relative, compensation du point noir. */
export const DEFAULT_SPACE: CmykSpace = { profile: "fogra39", intent: "relative", bpc: true };

const SETTINGS_NAME = "couleurs";
/** Ancien réglage (dernier profil de l'épreuve, jusqu'à la 1.2.0) : repris une fois. */
const LEGACY_SETTINGS = "proof";

export const colorSettings = $state<{ cmyk: CmykSpace; profiles: CmykProfileEntry[]; dialogOpen: boolean }>({
  cmyk: { ...DEFAULT_SPACE },
  profiles: [],
  dialogOpen: false,
});

/** Noms des profils qui ne sont pas dans la liste (profil d'un document ouvert ailleurs) : id → nom court. */
const extraNames = $state<Record<string, { name: string; short: string }>>({});

/** Clé d'un espace : une table d'épreuve par clé. */
export function spaceKey(space: CmykSpace): string {
  return `${space.profile}/${space.intent}/${space.bpc ? 1 : 0}`;
}

export function sameSpace(a: CmykSpace | null | undefined, b: CmykSpace | null | undefined): boolean {
  return Boolean(a && b) && spaceKey(a as CmykSpace) === spaceKey(b as CmykSpace);
}

function entry(id: string): { name: string; short: string } | undefined {
  return colorSettings.profiles.find((p) => p.id === id) ?? extraNames[id];
}

/** Nom court d'un profil (« FOGRA39 », « U.S. Web Coated ») : onglet du document, barre d'état. */
export function profileShort(id: string): string {
  const known = entry(id);
  if (known) return known.short;
  if (id.startsWith("icc-")) void learnProfile(id);
  return id.startsWith("icc-") ? "ICC" : id.toUpperCase();
}

/** Nom complet d'un profil (« Coated FOGRA39 (ISO 12647-2:2004) »). */
export function profileName(id: string): string {
  return entry(id)?.name ?? profileShort(id);
}

const learning = new Set<string>();
/** Nom d'un profil absent de la liste (gardé dans un projet), demandé une fois au moteur. */
async function learnProfile(id: string): Promise<void> {
  if (learning.has(id)) return;
  learning.add(id);
  try {
    const info = await api.cmykProfileInfo(id);
    extraNames[id] = { name: info.name, short: info.short };
  } catch {
    // Profil inconnu ici : l'épreuve le signalera.
  }
}

function isSpace(value: unknown): value is CmykSpace {
  const v = value as Partial<CmykSpace> | null;
  return typeof v?.profile === "string" && (v.intent === "relative" || v.intent === "perceptual") && typeof v.bpc === "boolean";
}

/** Relit la liste des profils (livrés, chargés, installés sur l'ordinateur). */
export async function refreshProfiles(): Promise<void> {
  try {
    colorSettings.profiles = await api.cmykProfiles();
  } catch {
    // Liste indisponible : les profils livrés restent utilisables par leur identifiant.
  }
}

/** Au démarrage : paramètres enregistrés (ou ancien profil d'épreuve), puis liste des profils. */
export async function loadColorSettings(): Promise<void> {
  await refreshProfiles();
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : null;
    const cmyk = (stored as { cmyk?: unknown } | null)?.cmyk;
    if (isSpace(cmyk)) {
      colorSettings.cmyk = cmyk;
    } else {
      const legacy = await api.loadSettings(LEGACY_SETTINGS).catch(() => null);
      const profile = (legacy ? (JSON.parse(legacy) as { profile?: unknown } | null) : null)?.profile;
      if (typeof profile === "string") colorSettings.cmyk = { ...DEFAULT_SPACE, profile };
    }
  } catch {
    // Réglage illisible : FOGRA39.
  }
  // Profil enregistré qui n'existe plus sur cet ordinateur (fichier supprimé) : retour au défaut.
  if (colorSettings.profiles.length > 0 && !colorSettings.profiles.some((p) => p.id === colorSettings.cmyk.profile)) {
    colorSettings.cmyk = { ...DEFAULT_SPACE };
  }
}

/** Applique et enregistre de nouveaux paramètres. */
export function applyColorSettings(cmyk: CmykSpace): void {
  colorSettings.cmyk = { ...cmyk };
  void api.saveSettings(SETTINGS_NAME, JSON.stringify({ cmyk })).catch(() => undefined);
}

/** Charge un profil ICC choisi par l'utilisateur (vérifié et recopié par le moteur) ; rend son identifiant. */
export async function importProfile(path: string): Promise<string> {
  const info = await api.cmykImportProfile(path);
  await refreshProfiles();
  return info.id;
}
