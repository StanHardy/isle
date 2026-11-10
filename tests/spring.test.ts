import { describe, expect, it } from "vitest";
import { Spring } from "../src/shared/spring";

describe("Spring", () => {
  it("settles at the target and reports settled", () => {
    const s = new Spring(100, { initial: 0, stiffness: 400, damping: 40 });
    let settled = false;
    for (let i = 0; i < 600 && !settled; i++) {
      settled = s.step(1 / 60);
    }
    expect(settled).toBe(true);
    expect(Math.abs(s.value - 100)).toBeLessThan(0.5);
  });

  it("overshoots with low damping (springiness)", () => {
    const s = new Spring(10, { initial: 0, stiffness: 300, damping: 8 });
    let max = 0;
    for (let i = 0; i < 120; i++) {
      s.step(1 / 60);
      max = Math.max(max, s.value);
    }
    expect(max).toBeGreaterThan(10);
    expect(max).toBeLessThan(20);
  });

  it("does not move when already at target", () => {
    const s = new Spring(42, { initial: 42 });
    expect(s.step(1 / 60)).toBe(true);
    expect(s.value).toBe(42);
    expect(s.velocity).toBe(0);
  });

  it("snaps instantly", () => {
    const s = new Spring(1, { initial: 0 });
    s.snap(7);
    expect(s.value).toBe(7);
    expect(s.target).toBe(7);
    expect(s.velocity).toBe(0);
  });

  it("steps toward a moving target (frame retarget)", () => {
    const s = new Spring(0, { initial: 0, stiffness: 300, damping: 26 });
    for (let i = 0; i < 30; i++) s.step(1 / 60);
    s.set(50);
    for (let i = 0; i < 30; i++) s.step(1 / 60);
    expect(s.value).toBeGreaterThan(10);
    s.set(0);
    for (let i = 0; i < 200; i++) {
      if (s.step(1 / 60)) break;
    }
    expect(Math.abs(s.value)).toBeLessThan(0.5);
  });
});
