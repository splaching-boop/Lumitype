// Cache de tuiles côté GPU, commun à l'image composée et au masque de sélection :
// chargement à la demande, repli sur un ancêtre, rechargement par révision, éviction LRU.

export type TileFetcher = (level: number, x: number, y: number) => Promise<ArrayBuffer>;

/** Contenu décodé d'une réponse du moteur (sans l'en-tête de révision). */
export interface Decoded {
  /** `null` : tuile uniforme, aucune texture nécessaire. */
  texture: WebGLTexture | null;
  /** Valeur d'une tuile uniforme, `0..255`. */
  uniform: number;
}

interface Entry extends Decoded {
  /**
   * Nouvelle version déjà reçue mais pas encore affichée. Les tuiles périmées sont remplacées
   * TOUTES ENSEMBLE (voir `promote`) : sinon l'écran montrerait un instant un mélange de deux
   * états du document, ce qui se voit comme une image « en escalier » pendant un déplacement.
   */
  staged: (Decoded & { revision: number }) | null;
  /**
   * Exception à l'échange groupé : pendant un trait de pinceau, chaque tuile est remplacée dès
   * son arrivée. Les tuiles d'un trait sont justes indépendamment les unes des autres, et
   * attendre « que tout soit prêt » figerait l'affichage tant que le pinceau bouge.
   */
  immediate: boolean;
  lastUsed: number;
  /**
   * Révision du document que la tuile doit au moins refléter. Tant que ce n'est
   * pas le cas, elle reste affichée (pas de clignotement) et est redemandée.
   */
  needs: number;
  revision: number;
}

export interface Resolved extends Decoded {
  level: number;
  x: number;
  y: number;
}

const MAX_IN_FLIGHT = 8;

export class TileCache {
  private readonly entries = new Map<string, Entry>();
  private readonly inFlight = new Set<string>();
  private generation = 0;
  /** Révision minimale exigée de toute tuile, relevée par un changement global. */
  private floor = 0;

  constructor(
    private readonly gl: WebGL2RenderingContext,
    private readonly fetchTile: TileFetcher,
    private readonly decode: (payload: Uint8Array) => Decoded,
    private readonly onLoaded: () => void,
    private readonly maxTextures: number,
  ) {}

  /** Changement de document : tout est oublié, les réponses en vol seront ignorées. */
  reset(): void {
    this.generation += 1;
    this.inFlight.clear();
    for (const entry of this.entries.values()) {
      this.gl.deleteTexture(entry.texture);
      this.gl.deleteTexture(entry.staged?.texture ?? null);
    }
    this.entries.clear();
    this.floor = 0;
  }

  invalidateAll(revision: number): void {
    this.floor = revision;
    for (const entry of this.entries.values()) entry.immediate = false;
  }

  /** `tiles` : coordonnées de niveau 0 ; tous leurs ancêtres sont périmés avec elles. */
  invalidate(tiles: readonly [number, number][], maxLevel: number, revision: number, immediate: boolean): void {
    for (const [x, y] of tiles) {
      for (let level = 0; level <= maxLevel; level++) {
        const entry = this.entries.get(`${level}/${x >> level}/${y >> level}`);
        if (entry) Object.assign(entry, { needs: revision, immediate });
      }
    }
  }

  private required(entry: Entry): number {
    return Math.max(entry.needs, this.floor);
  }

  /** Nombre de tuiles vues lors du dernier rendu qui attendent encore leur nouvelle version. */
  private waiting = 0;
  /** Nombre de tuiles visibles jamais reçues (premier affichage). */
  private missing = 0;
  /** Nombre de tuiles visibles d'un trait de pinceau encore en cours de rechargement. */
  private refreshing = 0;

  /**
   * Échange suspendu : pendant un trait dont l'aperçu est dessiné par la carte graphique, l'image
   * de départ reste affichée telle quelle. Les nouvelles tuiles s'accumulent en coulisse et ne
   * prennent la place des anciennes qu'à la fin du trait, toutes ensemble.
   */
  hold = false;

  beginFrame(): void {
    this.waiting = 0;
    this.missing = 0;
    this.refreshing = 0;
  }

  /** Vrai quand tout ce qui est visible est à jour : l'image affichée reflète le document. */
  get settled(): boolean {
    return this.waiting === 0 && this.missing === 0 && this.refreshing === 0;
  }

  /**
   * À appeler après avoir parcouru les tuiles visibles. Si toutes les tuiles périmées ont
   * leur remplaçante, l'échange se fait d'un coup. Retourne vrai s'il a eu lieu (à redessiner).
   */
  promote(): boolean {
    if (this.hold || this.waiting > 0) return false;
    let swapped = false;
    for (const entry of this.entries.values()) {
      if (!entry.staged) continue;
      this.gl.deleteTexture(entry.texture);
      Object.assign(entry, entry.staged, { staged: null });
      swapped = true;
    }
    return swapped;
  }

  /** Tuile demandée si elle est en cache, sinon son plus proche ancêtre disponible. */
  resolve(level: number, tx: number, ty: number, maxLevel: number, frame: number): Resolved | null {
    const exact = this.touch(level, tx, ty, frame);
    if (exact) {
      const required = this.required(exact);
      if (exact.revision < required && (exact.staged?.revision ?? -1) < required) {
        if (exact.immediate) this.refreshing += 1;
        else this.waiting += 1;
        // Image figée (trait en cours, dessiné par la carte graphique) : on ne redemande rien au moteur. Chaque
        // réponse serait périmée avant d'arriver — le trait continue — et ces allers-retours disputent au trait
        // le processeur et le verrou du document : c'était la première cause des saccades. Tout est demandé d'un
        // coup à la fin du trait.
        if (!this.hold) this.request(level, tx, ty, frame);
      }
      return { ...exact, level, x: tx, y: ty };
    }
    this.missing += 1;
    this.request(level, tx, ty, frame);
    for (let up = 1; level + up <= maxLevel; up++) {
      const x = tx >> up;
      const y = ty >> up;
      const ancestor = this.touch(level + up, x, y, frame);
      if (ancestor) return { ...ancestor, level: level + up, x, y };
    }
    return null;
  }

  request(level: number, x: number, y: number, frame: number): void {
    const key = `${level}/${x}/${y}`;
    const cached = this.entries.get(key);
    const upToDate = cached !== undefined && Math.max(cached.revision, cached.staged?.revision ?? -1) >= this.required(cached);
    if (upToDate || this.inFlight.has(key) || this.inFlight.size >= MAX_IN_FLIGHT) return;
    const generation = this.generation;
    this.inFlight.add(key);

    this.fetchTile(level, x, y)
      .then((buffer) => {
        if (generation !== this.generation) return;
        const revision = Number(new DataView(buffer).getBigUint64(0, true));
        const previous = this.entries.get(key);
        // Réponse calculée avant une modification déjà signalée : on garde l'ancienne, elle sera redemandée.
        if (previous && revision < this.required(previous)) return;
        const decoded = this.decode(new Uint8Array(buffer, 8));
        if (previous?.immediate) {
          this.gl.deleteTexture(previous.texture);
          Object.assign(previous, decoded, { revision });
        } else if (previous) {
          // Une version est déjà à l'écran : la nouvelle attend que ses voisines soient prêtes aussi.
          this.gl.deleteTexture(previous.staged?.texture ?? null);
          previous.staged = { ...decoded, revision };
        } else {
          this.entries.set(key, { ...decoded, staged: null, immediate: false, lastUsed: frame, needs: 0, revision });
        }
      })
      .catch((error: unknown) => {
        // La tuile sera redemandée au prochain rendu qui en aura besoin.
        console.error(`Tuile ${key} indisponible`, error);
      })
      .finally(() => {
        if (generation !== this.generation) return;
        this.inFlight.delete(key);
        this.onLoaded();
      });
  }

  /** Libère les textures les moins récemment affichées au-delà du budget. */
  evict(frame: number): void {
    const textured = [...this.entries.entries()].filter(([, entry]) => entry.texture !== null);
    if (textured.length <= this.maxTextures) return;
    textured
      .filter(([, entry]) => entry.lastUsed !== frame)
      .sort(([, a], [, b]) => a.lastUsed - b.lastUsed)
      .slice(0, textured.length - this.maxTextures)
      .forEach(([key, entry]) => {
        this.gl.deleteTexture(entry.texture);
        this.gl.deleteTexture(entry.staged?.texture ?? null);
        this.entries.delete(key);
      });
  }

  private touch(level: number, x: number, y: number, frame: number): Entry | undefined {
    const entry = this.entries.get(`${level}/${x}/${y}`);
    if (entry) entry.lastUsed = frame;
    return entry;
  }
}
