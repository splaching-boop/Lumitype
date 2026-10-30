// Empreintes d'un trait de pointe (pinceau importé : ABR, GBR, image), calculées côté interface.
//
// MÊME algorithme que le moteur (crates/engine/src/paint.rs : `Stroke::extend`, `curve`, `segment`, `stamps`,
// `stamp`, `dual_along`, `dual_stamp`, `random`) : lissage par les milieux des points reçus, courbes quadratiques
// découpées en segments, empreintes espacées le long de chaque segment, dynamiques tirées au sort dans le même ordre.
// Les calculs sont arrondis en flottants 32 bits comme dans le moteur (`Math.fround`).
//
// L'interface est la référence : elle montre aussitôt ces empreintes par la carte graphique (renderer.ts), puis les
// envoie au moteur, qui les pose telles quelles (`stroke_stamps`, `Stroke::extend_stamps`). Ce qu'on voit pendant le
// geste est donc le résultat. Le moteur garde son propre calcul (aperçus des pinceaux, voie sans carte graphique) :
// ui/tests/tip-stamps.test.mjs vérifie que les deux donnent les mêmes empreintes (fixture produite par le moteur).
import type { BrushDynamics } from "./ipc";

const f = Math.fround;
const TAU = f(Math.PI * 2);
const RADS_PER_DEG = f(f(Math.PI) / 180);
const EPSILON = 1.1920929e-7; // f32::EPSILON

/** Empreinte de la pointe, repère document : `[x, y, diamètre, rotation (rad), rondeur, retourné x, retourné y, force]`. */
export interface TipStamp {
  x: number;
  y: number;
  diameter: number;
  turn: number;
  roundness: number;
  flipX: boolean;
  flipY: boolean;
  strength: number;
}

/** Empreinte du pinceau double, repère document. */
export interface DualTipStamp {
  x: number;
  y: number;
  diameter: number;
  flipX: boolean;
  flipY: boolean;
}

/** Réglages du trait qui comptent pour les empreintes. */
export interface TipStroke {
  /** Diamètre en pixels. */
  size: number;
  pressureSize: boolean;
  pressureOpacity: boolean;
  /** Espacement propre à la pointe (fraction du diamètre), quand la dynamique n'en impose pas. */
  tipSpacing: number;
  dynamics: BrushDynamics;
  dual: { ratio: number; spacing: number; scatter: number; bothAxes: boolean; count: number; flip: boolean } | null;
}

interface Point {
  x: number;
  y: number;
  p: number;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const hypot = (x: number, y: number) => f(Math.hypot(x, y));

export class TipStamper {
  private last: Point | null = null;
  private drawn: Point | null = null;
  private heading = 0;
  private travelled = 0;
  private dualTravelled = 0;
  private seed = 0x9e3779b9;
  private stamps: TipStamp[] = [];
  private duals: DualTipStamp[] = [];

  constructor(private readonly brush: TipStroke) {}

  /** Ajoute des points reçus `[x, y, pression]` (repère document) ; retourne les nouvelles empreintes. */
  add(points: readonly (readonly number[])[]): { stamps: TipStamp[]; duals: DualTipStamp[] } {
    for (const raw of points) {
      const point = { x: f(raw[0] ?? 0), y: f(raw[1] ?? 0), p: f(raw[2] ?? 1) };
      const control = this.last;
      const from = this.drawn;
      if (!control || !from) {
        this.dab(point);
        [this.last, this.drawn] = [point, point];
        continue;
      }
      // Points quasi confondus : du bruit de capteur.
      if (hypot(f(point.x - control.x), f(point.y - control.y)) < 0.75) continue;
      // Pression adoucie (moyenne glissante), puis milieu du segment.
      const smoothed = { ...point, p: f(f(control.p + point.p) / 2) };
      const middle = { x: f(f(control.x + smoothed.x) / 2), y: f(f(control.y + smoothed.y) / 2), p: f(f(control.p + smoothed.p) / 2) };
      this.curve(from, control, middle);
      [this.last, this.drawn] = [smoothed, middle];
    }
    return this.take();
  }

  /** Fin du trait : le lissage garde un demi-segment d'avance, tracé jusqu'au dernier point. */
  finish(): { stamps: TipStamp[]; duals: DualTipStamp[] } {
    if (this.drawn && this.last) {
      this.segment(this.drawn, this.last);
      this.drawn = this.last;
    }
    return this.take();
  }

  private take(): { stamps: TipStamp[]; duals: DualTipStamp[] } {
    const out = { stamps: this.stamps, duals: this.duals };
    [this.stamps, this.duals] = [[], []];
    return out;
  }

  private radius(pressure: number): number {
    const floor = clamp(this.brush.dynamics.minSize, 0, 1);
    const scale = this.brush.pressureSize ? f(floor + f(f(1 - floor) * clamp(pressure, 0, 1))) : 1;
    return Math.max(f(f(f(this.brush.size) * 0.5) * scale), 0.5);
  }

  private curve(from: Point, control: Point, to: Point): void {
    const length = f(hypot(f(control.x - from.x), f(control.y - from.y)) + hypot(f(to.x - control.x), f(to.y - control.y)));
    const radius = this.radius(Math.max(from.p, to.p));
    const step = clamp(f(2 * f(Math.sqrt(radius))), 3, Math.max(64, f(radius * 0.2)));
    const steps = clamp(Math.ceil(f(length / step)), 1, 64);
    let previous = from;
    for (let i = 1; i <= steps; i++) {
      const t = f(i / steps);
      const u = f(1 - t);
      const blend = (a: number, b: number, c: number) => f(f(f(f(u * u) * a) + f(f(f(2 * u) * t) * b)) + f(f(t * t) * c));
      const next = { x: blend(from.x, control.x, to.x), y: blend(from.y, control.y, to.y), p: blend(from.p, control.p, to.p) };
      this.segment(previous, next);
      previous = next;
    }
  }

  private segment(from: Point, to: Point): void {
    this.dualAlong(from, to);
    this.stampsAlong(from, to);
  }

  /** Premier point du trait : une première empreinte. */
  private dab(at: Point): void {
    this.dualStamp(at);
    this.stamp(at);
  }

  private stampsAlong(from: Point, to: Point): void {
    const [dx, dy] = [f(to.x - from.x), f(to.y - from.y)];
    const length = hypot(dx, dy);
    if (length <= EPSILON) return;
    const dyn = this.brush.dynamics;
    let position = 0;
    for (;;) {
      const pressure = f(from.p + f(f(to.p - from.p) * Math.min(f(position / length), 1)));
      const diameter = f(this.radius(pressure) * 2);
      const factor = dyn.spacing > 0 ? clamp(dyn.spacing, 0.01, 5) : f(this.brush.tipSpacing);
      const spacing = Math.max(f(diameter * factor), 1);
      this.heading = f(Math.atan2(dy, dx));
      const step = f(spacing - this.travelled);
      if (f(position + step) > length) {
        this.travelled = f(this.travelled + f(length - position));
        return;
      }
      position = f(position + step);
      this.travelled = 0;
      const t = f(position / length);
      this.stamp({ x: f(from.x + f(dx * t)), y: f(from.y + f(dy * t)), p: f(from.p + f(f(to.p - from.p) * t)) });
    }
  }

  private stamp(at: Point): void {
    const dyn = this.brush.dynamics;
    const count = clamp(dyn.count, 1, 16);
    for (let n = 0; n < count; n++) {
      let [x, y] = [at.x, at.y];
      // Dispersion : l'empreinte s'écarte du trait, perpendiculairement ou dans toutes les directions.
      if (dyn.scatter > 0) {
        const reach = f(f(f(clamp(dyn.scatter, 0, 10)) * this.radius(at.p)) * 2);
        let ox: number;
        let oy: number;
        if (dyn.scatterPerpendicular) {
          const offset = f(f(f(this.random() * 2) - 1) * reach);
          [ox, oy] = [f(-f(Math.sin(this.heading)) * offset), f(f(Math.cos(this.heading)) * offset)];
        } else {
          const angle = f(this.random() * TAU);
          const distance = f(this.random() * reach);
          [ox, oy] = [f(f(Math.cos(angle)) * distance), f(f(Math.sin(angle)) * distance)];
        }
        [x, y] = [f(x + ox), f(y + oy)];
      }
      const scale = f(1 - f(this.random() * clamp(dyn.sizeJitter, 0, 1)));
      const follow = dyn.followDirection ? this.heading : 0;
      const turn = f(f(f(f(dyn.angle) * RADS_PER_DEG) + follow) + f(f(f(this.random() - 0.5) * clamp(dyn.angleJitter, 0, 1)) * TAU));
      const base = dyn.roundness > 0 ? clamp(dyn.roundness, 0.01, 1) : 1;
      const floor = clamp(dyn.minRoundness, 0, 1);
      const roundness = Math.max(Math.max(f(base * f(1 - f(this.random() * clamp(dyn.roundnessJitter, 0, 1)))), Math.min(floor, base)), 0.01);
      let strength = this.brush.pressureOpacity ? clamp(at.p, 0, 1) : 1;
      if (dyn.pressureFlow) strength = f(strength * clamp(at.p, 0, 1));
      strength = f(strength * f(1 - f(this.random() * clamp(dyn.opacityJitter, 0, 1))));
      strength = f(strength * f(1 - f(this.random() * clamp(dyn.flowJitter, 0, 1))));
      const diameter = f(f(f(this.radius(at.p) * 2) * Math.max(scale, 0.05)));
      this.stamps.push({ x, y, diameter, turn, roundness: f(roundness), flipX: dyn.flipX, flipY: dyn.flipY, strength });
    }
  }

  private dualAlong(from: Point, to: Point): void {
    const dual = this.brush.dual;
    if (!dual) return;
    const [dx, dy] = [f(to.x - from.x), f(to.y - from.y)];
    const length = hypot(dx, dy);
    if (length <= EPSILON) return;
    const diameter = Math.max(f(f(this.brush.size) * f(dual.ratio)), 1);
    const spacing = Math.max(f(diameter * clamp(dual.spacing, 0.01, 10)), 1);
    let position = 0;
    let travelled = this.dualTravelled;
    for (;;) {
      const step = f(spacing - travelled);
      if (f(position + step) > length) {
        this.dualTravelled = f(f(travelled + length) - position);
        return;
      }
      position = f(position + step);
      travelled = 0;
      const t = f(position / length);
      this.heading = f(Math.atan2(dy, dx));
      this.dualStamp({ x: f(from.x + f(dx * t)), y: f(from.y + f(dy * t)), p: f(from.p + f(f(to.p - from.p) * t)) });
    }
  }

  private dualStamp(at: Point): void {
    const dual = this.brush.dual;
    if (!dual) return;
    const diameter = Math.max(f(f(this.brush.size) * f(dual.ratio)), 1);
    const count = clamp(dual.count, 1, 16);
    for (let n = 0; n < count; n++) {
      let [x, y] = [at.x, at.y];
      if (dual.scatter > 0) {
        const reach = f(f(clamp(dual.scatter, 0, 10)) * diameter);
        if (dual.bothAxes) {
          const angle = f(this.random() * TAU);
          const distance = f(this.random() * reach);
          [x, y] = [f(x + f(f(Math.cos(angle)) * distance)), f(y + f(f(Math.sin(angle)) * distance))];
        } else {
          const offset = f(f(f(this.random() * 2) - 1) * reach);
          [x, y] = [f(x - f(f(Math.sin(this.heading)) * offset)), f(y + f(f(Math.cos(this.heading)) * offset))];
        }
      }
      let [flipX, flipY] = [false, false];
      if (dual.flip) {
        flipX = this.random() < 0.5;
        flipY = this.random() < 0.5;
      }
      this.duals.push({ x, y, diameter, flipX, flipY });
    }
  }

  /** xorshift32, comme le moteur : un même geste redonne les mêmes empreintes. */
  private random(): number {
    let s = this.seed;
    s = (s ^ (s << 13)) >>> 0;
    s = (s ^ (s >>> 17)) >>> 0;
    s = (s ^ (s << 5)) >>> 0;
    this.seed = s;
    return (s >>> 8) / 16777216;
  }
}

/** Empreintes à plat pour le moteur (`stroke_stamps`). */
export function packStamps(stamps: readonly TipStamp[]): number[][] {
  return stamps.map((s) => [s.x, s.y, s.diameter, s.turn, s.roundness, s.flipX ? 1 : 0, s.flipY ? 1 : 0, s.strength]);
}

export function packDuals(duals: readonly DualTipStamp[]): number[][] {
  return duals.map((d) => [d.x, d.y, d.diameter, d.flipX ? 1 : 0, d.flipY ? 1 : 0]);
}
