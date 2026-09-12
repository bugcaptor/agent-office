import { describe, expect, it } from "vitest";
import { createScrollbackGuard } from "../scrollbackGuard";

describe("web scrollback guard", () => {
  it("removes ESC[3J across output chunk boundaries", () => {
    const guard = createScrollbackGuard();
    expect(guard.filter("before\x1b[")).toBe("before");
    expect(guard.filter("3Jafter")).toBe("after");
  });

  it("drops a held partial escape when a snapshot starts a new stream", () => {
    const guard = createScrollbackGuard();
    guard.filter("\x1b[");
    guard.reset();
    expect(guard.filter("text")).toBe("text");
  });
});
