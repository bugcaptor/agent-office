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

  it("reset은 0으로 만들고, 0이 아니었을 때만 통지한다 (구독 시 현재값 replay 포함)", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.bump("a", +1);
    t.subscribe(cb);
    t.reset("a");
    t.reset("a");
    expect(t.get("a")).toBe(0);
    expect(cb.mock.calls).toEqual([
      ["a", 1],
      ["a", 0],
    ]);
  });

  it("setAbsolute는 음수를 0으로 클램프하고 실수를 내림한다", () => {
    const t = new SubagentCountTracker();
    t.setAbsolute("a", 3.9);
    expect(t.get("a")).toBe(3);
    t.setAbsolute("a", -2);
    expect(t.get("a")).toBe(0);
  });

  it("setAbsolute는 동일한 절대값이면 통지하지 않는다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    t.subscribe(cb);
    t.setAbsolute("a", 2);
    t.setAbsolute("a", 2.8);
    expect(cb.mock.calls).toEqual([["a", 2]]);
  });

  it("unsubscribe 후에는 통지하지 않는다", () => {
    const t = new SubagentCountTracker();
    const cb = vi.fn();
    const off = t.subscribe(cb);
    off();
    t.bump("a", +1);
    expect(cb).not.toHaveBeenCalled();
  });

  describe("at 워터마크(스테일 스냅샷 방어)", () => {
    it("더 최신 델타 뒤에 도착한 오래된 절대 스냅샷은 무시한다(클로버링 방지)", () => {
      const t = new SubagentCountTracker();
      // A, B 실행 중(count=2)
      t.bump("p", +1, 10); // A start @10
      t.bump("p", +1, 20); // B start @20
      // D 시작(@30)이 먼저 반영되어 count=3, 워터마크=30
      t.bump("p", +1, 30);
      expect(t.get("p")).toBe(3);
      // 뒤늦게 도착한 A 정지 스냅샷(@25, running=1)은 워터마크(30)보다 오래됨 → 무시
      t.setAbsolute("p", 1, 25);
      expect(t.get("p")).toBe(3);
      // 이후 최신 스냅샷(@40)은 정상 반영
      t.setAbsolute("p", 2, 40);
      expect(t.get("p")).toBe(2);
    });

    it("워터마크와 at이 같으면 반영한다(< 만 스테일)", () => {
      const t = new SubagentCountTracker();
      t.bump("a", +1, 100);
      t.setAbsolute("a", 5, 100);
      expect(t.get("a")).toBe(5);
    });

    it("at 없이 호출하면 스테일 검사를 하지 않는다(하위호환)", () => {
      const t = new SubagentCountTracker();
      t.bump("a", +1, 100);
      t.setAbsolute("a", 7); // at 미지정 → 무조건 반영
      expect(t.get("a")).toBe(7);
    });

    it("reset은 워터마크를 지워 다음 세션의 오래된 at 스냅샷도 반영되게 한다", () => {
      const t = new SubagentCountTracker();
      t.setAbsolute("a", 3, 500);
      t.reset("a"); // 세션 경계
      t.setAbsolute("a", 2, 10); // 새 세션(작은 at)도 반영
      expect(t.get("a")).toBe(2);
    });
  });
});
