// @vitest-environment jsdom
import { describe, expect, it, vi, beforeEach } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";

const invokeMock = vi.fn(
  async (cmd: string, _payload?: unknown): Promise<unknown> => null,
);
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: unknown) => invokeMock(cmd, payload),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

// IslandRoot imports modules that read settings via invoke — all returns null.
import IslandRoot from "../src/island/IslandRoot";

describe("IslandRoot", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    invokeMock.mockImplementation(async () => null);
    cleanup();
  });

  it("renders the compact pill with a clock by default", async () => {
    render(<IslandRoot />);
    // Settings arrive as null (mocked invoke), pill should still render.
    const pill = await screen.findByRole("button", { name: /isle/i });
    expect(pill).toBeTruthy();
    expect(pill.getAttribute("aria-expanded")).toBe("false");
  });

  it("does not render an expanded dialog while compact", () => {
    render(<IslandRoot />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("drives the window frame through island_set_frame", async () => {
    invokeMock.mockImplementation(async (cmd: string): Promise<unknown> => {
      if (cmd === "island_anchor") {
        return { x: 0, y: 0, width: 1920, height: 1040, scale: 1 };
      }
      return null;
    });
    render(<IslandRoot />);
    await new Promise((r) => setTimeout(r, 80));
    const frameCalls = invokeMock.mock.calls.filter(
      (c) => c[0] === "island_set_frame",
    );
    expect(frameCalls.length).toBeGreaterThan(0);
    const payload = frameCalls[0][1] as Record<string, number>;
    expect(payload).toHaveProperty("x");
    expect(payload).toHaveProperty("y");
    expect(payload).toHaveProperty("w");
    expect(payload).toHaveProperty("h");
  });
});
