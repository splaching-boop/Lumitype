// Profils d'impression de l'épreuve CMJN (Affichage › Épreuve : …).
//
// Chaque profil a sa table 3D précalculée dans ui/src/assets/proof/<id>.bin, générée par
// `python3 export/lut-cmjn.py --tous` depuis les profils ICC d'export/profils (provenance et licences dans
// export/profils/LISEZ-MOI.md). Le fichier de chaque table est relié à son identifiant dans viewport/renderer.ts.
//
// Le dernier profil choisi est gardé dans les réglages (`proof`) : Ctrl+Y (Épreuve CMJN) le rallume.
import { app } from "../state/app.svelte";
import { api } from "./ipc";

export type ProofProfileId = "fogra39" | "fogra51" | "fogra52" | "gracol2006" | "swop2006c3";

export interface ProofProfile {
  id: ProofProfileId;
  /** Nom court, affiché dans l'onglet du document et la barre d'état (« CMJN FOGRA39 ») : non traduit. */
  short: string;
}

/** Dans l'ordre du menu Affichage. */
export const PROOF_PROFILES: readonly ProofProfile[] = [
  { id: "fogra39", short: "FOGRA39" },
  { id: "fogra51", short: "FOGRA51" },
  { id: "fogra52", short: "FOGRA52" },
  { id: "gracol2006", short: "GRACoL 2006" },
  { id: "swop2006c3", short: "SWOP 2006" },
];

const SETTINGS_NAME = "proof";

export function isProofProfileId(value: unknown): value is ProofProfileId {
  return PROOF_PROFILES.some((profile) => profile.id === value);
}

/** Nom court du profil (« FOGRA39 »). */
export function proofShortName(id: ProofProfileId): string {
  return PROOF_PROFILES.find((profile) => profile.id === id)?.short ?? id;
}

/**
 * Choix d'un profil dans le menu : une seule épreuve à la fois, et re-choisir l'épreuve active l'éteint.
 * Le profil choisi devient le dernier profil (Ctrl+Y), enregistré dans les réglages.
 */
export function chooseProofProfile(id: ProofProfileId): void {
  if (app.proofProfile === id) {
    app.proofProfile = null;
    return;
  }
  app.proofProfile = id;
  if (app.lastProofProfile !== id) {
    app.lastProofProfile = id;
    void api.saveSettings(SETTINGS_NAME, JSON.stringify({ profile: id })).catch(() => undefined);
  }
}

/** Au démarrage : relit le dernier profil choisi (l'épreuve elle-même reste éteinte). */
export async function loadProofProfile(): Promise<void> {
  try {
    const text = await api.loadSettings(SETTINGS_NAME);
    const stored: unknown = text ? JSON.parse(text) : {};
    const profile = (stored as { profile?: unknown } | null)?.profile;
    if (isProofProfileId(profile)) app.lastProofProfile = profile;
  } catch {
    // Réglage illisible : on garde FOGRA39.
  }
}
