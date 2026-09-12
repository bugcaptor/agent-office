import { describe, expect, it } from "vitest";
import { remoteSessionStatus } from "../sessionState";

describe("remoteSessionStatus", () => {
  it("keeps a starting remote session mounted until its PTY becomes running", () => {
    expect(remoteSessionStatus("starting")).toBe("starting");
  });

  it("treats absent and disposed sessions as idle", () => {
    expect(remoteSessionStatus(undefined)).toBe("idle");
    expect(remoteSessionStatus("disposed")).toBe("idle");
  });
});
