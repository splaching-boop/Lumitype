// Raccourcis effectifs : preset Photoshop + personnalisations, enregistrées sur disque.
import { invoke } from "@tauri-apps/api/core";
import type { ActionId } from "../lib/actions";
import { type Binding, type Chord, chordLabel, chordsConflict } from "../lib/keymap";
import { PHOTOSHOP_KEYMAP } from "../lib/keymap-photoshop";
import { app } from "./app.svelte";

const SETTINGS_NAME = "keymap";
const FORMAT_VERSION = 1;

interface StoredKeymap {
  version: number;
  /** Seules les actions modifiées sont stockées : le preset peut évoluer sans écraser les réglages. */
  overrides: Partial<Record<ActionId, Chord[]>>;
}

const presetFor = (action: ActionId): Chord[] => PHOTOSHOP_KEYMAP.filter((b) => b.action === action).map((b) => b.chord);

class Keymap {
  overrides = $state<Partial<Record<ActionId, Chord[]>>>({});

  readonly bindings: Binding<ActionId>[] = $derived.by(() => {
    const untouched = PHOTOSHOP_KEYMAP.filter((b) => !(b.action in this.overrides));
    const custom = Object.entries(this.overrides).flatMap(([action, chords]) =>
      (chords ?? []).map((chord) => ({ action: action as ActionId, chord })),
    );
    return [...untouched, ...custom];
  });

  chordsOf(action: ActionId): Chord[] {
    return this.overrides[action] ?? presetFor(action);
  }

  /** Premier raccourci de l'action, pour les menus. */
  label(action: ActionId): string {
    const chord = this.chordsOf(action)[0];
    return chord ? chordLabel(chord) : "";
  }

  isCustomized(action: ActionId): boolean {
    return action in this.overrides;
  }

  /** Actions déjà déclenchées par ce raccourci (hors `except`). */
  conflicts(chord: Chord, except: ActionId): ActionId[] {
    const found = this.bindings.filter((b) => b.action !== except && chordsConflict(b.chord, chord)).map((b) => b.action);
    return [...new Set(found)];
  }

  /** Ajoute un raccourci à une action et le retire de celles qui l'utilisaient. */
  assign(action: ActionId, chord: Chord): void {
    for (const other of this.conflicts(chord, action)) {
      this.overrides[other] = this.chordsOf(other).filter((c) => !chordsConflict(c, chord));
    }
    this.overrides[action] = [...this.chordsOf(action).filter((c) => !chordsConflict(c, chord)), chord];
    this.save();
  }

  remove(action: ActionId, chord: Chord): void {
    this.overrides[action] = this.chordsOf(action).filter((c) => c !== chord);
    this.save();
  }

  reset(action: ActionId): void {
    delete this.overrides[action];
    this.save();
  }

  resetAll(): void {
    this.overrides = {};
    this.save();
  }

  async load(): Promise<void> {
    try {
      const text = await invoke<string | null>("load_settings", { name: SETTINGS_NAME });
      if (!text) return;
      const stored = JSON.parse(text) as StoredKeymap;
      // Une version inconnue est ignorée plutôt que mal interprétée : on retombe sur le preset.
      if (stored.version === FORMAT_VERSION) this.overrides = stored.overrides;
    } catch (error) {
      app.fail(`Raccourcis personnalisés illisibles, preset par défaut utilisé — ${String(error)}`);
    }
  }

  private save(): void {
    const stored: StoredKeymap = { version: FORMAT_VERSION, overrides: this.overrides };
    invoke("save_settings", { name: SETTINGS_NAME, content: JSON.stringify(stored, null, 2) }).catch((error: unknown) =>
      app.fail(error),
    );
  }
}

export const keymap = new Keymap();
