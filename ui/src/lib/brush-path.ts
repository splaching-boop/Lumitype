// Tracé d'un trait de pinceau, calculé côté interface pour l'aperçu immédiat.
// MÊME algorithme que le moteur (crates/engine/src/paint.rs) : le trait passe par les milieux des
// points reçus, chaque point servant de point de contrôle d'une courbe quadratique, et chaque
// petit segment de courbe est tracé d'un seul tenant, en « gélule » (cercle balayé), ce qui donne
// un bord parfaitement lisse. Toute modification doit être faite des deux côtés.

/** Gélule : cercle de rayon `ar` -> `br` balayé de `(ax, ay)` à `(bx, by)`, en pixels du document. */
export interface Dab {
  ax: number;
  ay: number;
  ar: number;
  bx: number;
  by: number;
  br: number;
}

interface Sample {
  x: number;
  y: number;
  p: number;
}

export class DabPath {
  private last: Sample | null = null;
  private drawn: Sample | null = null;
  /** Mode empreintes : distance parcourue depuis la dernière empreinte (reportée d'un segment au suivant). */
  private travelled = 0;

  /**
   * `spacing` : pinceau flou, tracé comme dans le moteur en empreintes rondes espacées de `spacing` fois le
   * diamètre (0,1 pour le pinceau flou), chacune rendue comme un cercle (`a` = `b`). Absent : gélules continues (pinceau net).
   */
  constructor(
    private readonly size: number,
    private readonly pressureSize: boolean,
    private readonly spacing: number | null = null,
  ) {}

  /** Ajoute un point reçu `[x, y, pression]` et retourne les nouvelles empreintes. */
  add([x = 0, y = 0, p = 1]: number[]): Dab[] {
    const dabs: Dab[] = [];
    if (!this.last || !this.drawn) {
      const first = { x, y, p };
      [this.last, this.drawn] = [first, first];
      return [this.capsule(first, first)];
    }
    const control = this.last;
    if (Math.hypot(x - control.x, y - control.y) < 0.75) return dabs; // bruit de capteur
    const point = { x, y, p: (control.p + p) / 2 }; // pression adoucie
    const middle = { x: (control.x + point.x) / 2, y: (control.y + point.y) / 2, p: (control.p + point.p) / 2 };
    this.curve(this.drawn, control, middle, dabs);
    [this.last, this.drawn] = [point, middle];
    return dabs;
  }

  /** Fin du trait : le lissage garde un demi-segment d'avance, à tracer jusqu'au dernier point. */
  finish(): Dab[] {
    const dabs: Dab[] = [];
    if (this.drawn && this.last) this.segment(this.drawn, this.last, dabs);
    this.drawn = this.last;
    return dabs;
  }

  private radius(pressure: number): number {
    const scale = this.pressureSize ? Math.min(1, Math.max(0, pressure)) : 1;
    return Math.max(this.size * 0.5 * scale, 0.5);
  }

  private capsule(from: Sample, to: Sample): Dab {
    return { ax: from.x, ay: from.y, ar: this.radius(from.p), bx: to.x, by: to.y, br: this.radius(to.p) };
  }

  private curve(from: Sample, control: Sample, to: Sample, out: Dab[]): void {
    const length = Math.hypot(control.x - from.x, control.y - from.y) + Math.hypot(to.x - control.x, to.y - control.y);
    // Segments d'autant plus longs que le pinceau est gros : moins de gélules, sans écart visible à la courbe.
    const radius = this.radius(Math.max(from.p, to.p));
    const step = Math.min(Math.max(64, radius * 0.2), Math.max(3, 2 * Math.sqrt(radius)));
    const steps = Math.min(64, Math.max(1, Math.ceil(length / step)));
    let previous = from;
    for (let step = 1; step <= steps; step++) {
      const t = step / steps;
      const u = 1 - t;
      const blend = (a: number, b: number, c: number) => u * u * a + 2 * u * t * b + t * t * c;
      const next = { x: blend(from.x, control.x, to.x), y: blend(from.y, control.y, to.y), p: blend(from.p, control.p, to.p) };
      this.segment(previous, next, out);
      previous = next;
    }
  }

  private segment(from: Sample, to: Sample, out: Dab[]): void {
    if (this.spacing === null) {
      if (Math.hypot(to.x - from.x, to.y - from.y) > 1e-6) out.push(this.capsule(from, to));
      return;
    }
    // Empreintes régulièrement espacées : même boucle que `Stroke::stamps` dans le moteur.
    const [dx, dy] = [to.x - from.x, to.y - from.y];
    const length = Math.hypot(dx, dy);
    if (length <= 1e-6) return;
    let position = 0;
    for (;;) {
      const pressure = from.p + (to.p - from.p) * Math.min(1, position / length);
      const spacing = Math.max(1, this.radius(pressure) * 2 * this.spacing);
      const step = spacing - this.travelled;
      if (position + step > length) {
        this.travelled += length - position;
        return;
      }
      position += step;
      this.travelled = 0;
      const t = position / length;
      const at = { x: from.x + dx * t, y: from.y + dy * t, p: from.p + (to.p - from.p) * t };
      out.push(this.capsule(at, at));
    }
  }
}
