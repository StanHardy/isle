/**
 * Minimal critically-damped-friendly spring engine for island morphs.
 * Semi-implicit Euler; snaps instantly when the user prefers reduced motion.
 */
export class Spring {
  value: number;
  velocity = 0;
  target: number;
  stiffness: number;
  damping: number;
  precision: number;

  constructor(
    target: number,
    opts: { stiffness?: number; damping?: number; precision?: number; initial?: number } = {},
  ) {
    this.target = target;
    this.value = opts.initial ?? target;
    this.stiffness = opts.stiffness ?? 170;
    this.damping = opts.damping ?? 22;
    this.precision = opts.precision ?? 0.1;
  }

  set(target: number) {
    this.target = target;
  }

  /** Advance one step; returns true when settled. */
  step(dt: number): boolean {
    const delta = this.target - this.value;
    const accel = this.stiffness * delta - this.damping * this.velocity;
    this.velocity += accel * dt;
    this.value += this.velocity * dt;
    if (
      Math.abs(this.target - this.value) < this.precision &&
      Math.abs(this.velocity) < this.precision
    ) {
      this.value = this.target;
      this.velocity = 0;
      return true;
    }
    return false;
  }

  snap(target: number) {
    this.value = target;
    this.target = target;
    this.velocity = 0;
  }
}

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** rAF loop driver shared by all springs; stops itself when idle. */
export class SpringRunner {
  private springs = new Set<Spring>();
  private frame: number | null = null;
  private last = 0;

  add(spring: Spring) {
    this.springs.add(spring);
    this.start();
  }

  remove(spring: Spring) {
    this.springs.delete(spring);
    if (this.springs.size === 0 && this.frame !== null) {
      cancelAnimationFrame(this.frame);
      this.frame = null;
    }
  }

  private start() {
    if (this.frame !== null) return;
    this.last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min((now - this.last) / 1000, 1 / 30);
      this.last = now;
      let anyAlive = false;
      for (const s of this.springs) {
        if (prefersReducedMotion()) {
          s.snap(s.target);
        } else if (!s.step(dt)) {
          anyAlive = true;
        }
      }
      this.frame = anyAlive || this.springs.size > 0 ? requestAnimationFrame(tick) : null;
      if (this.frame === null) this.last = 0;
    };
    this.frame = requestAnimationFrame(tick);
  }
}
