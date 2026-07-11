import { describe, expect, it, vi } from "vitest";
import { SubagentCountTracker } from "../subagentCounts";

describe("SubagentCountTracker", () => {
  it("bump으로 증감하고 0에서 클램프한다", () => {
    const t = new SubagentCountTracker();
    t.bump("a", +1);
    t.bump("a", +1);
    expect(t.get("a")).toBe(2);
    t.bump("a", -1);
    t.bump("a", -1);
    t.bump("a", -1); // 이미 0 → 클램프
    expect(t.get("a")).toBe(0);
  });

  it("값이 실제로 바뀔 때만 구독자에게 통지한다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.subscribe(cb);
    t.bump("a", +1); // 0→1 통지
    t.bump("a", -1); // 1→0 통지
    t.bump("a", -1); // 0→0 통지 안 함
    expect(cb.mock.calls).toEqual([["a", 1], ["a", 0]]);
  });

  it("reset은 0으로 만들고, 0이 아니었을 때만 통지한다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.bump("a", +1);
    t.subscribe(cb);
    t.reset("a"); // 1→0 통지
    t.reset("a"); // 0→0 통지 안 함
    expect(t.get("a")).toBe(0);
    expect(cb.mock.calls).toEqual([["a", 0]]);
  });

  it("unsubscribe 후에는 통지하지 않는다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    const off = t.subscribe(cb);
    off();
    t.bump("a", +1);
    expect(cb).not.toHaveBeenCalled();
  });
});
