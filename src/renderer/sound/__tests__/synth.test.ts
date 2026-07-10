// agentFreqMul(폴백 합성 클릭 음색)·agentRateMul(샘플 재생 속도): agentId
// 기반으로 결정적이고 각자 범위를 지켜야 하며, 다른 id는 (대체로) 다른
// 배율을 가진다.
import { describe, expect, it } from "vitest";
import { agentFreqMul, agentRateMul } from "../synth";

describe("agentFreqMul", () => {
  it("같은 id는 항상 같은 배율(결정적)", () => {
    expect(agentFreqMul("agent-1")).toBe(agentFreqMul("agent-1"));
  });

  it("배율은 0.85~1.2 범위", () => {
    for (const id of ["a", "agent-1", "b2c3", "아무개", ""]) {
      const m = agentFreqMul(id);
      expect(m).toBeGreaterThanOrEqual(0.85);
      expect(m).toBeLessThanOrEqual(1.2);
    }
  });

  it("서로 다른 id는 다른 배율(해시 분산 스모크)", () => {
    expect(agentFreqMul("agent-1")).not.toBe(agentFreqMul("agent-2"));
  });
});

describe("agentRateMul", () => {
  it("같은 id는 항상 같은 배율(결정적)", () => {
    expect(agentRateMul("agent-1")).toBe(agentRateMul("agent-1"));
  });

  it("배율은 0.9~1.15 범위", () => {
    for (const id of ["a", "agent-1", "b2c3", "아무개", ""]) {
      const m = agentRateMul(id);
      expect(m).toBeGreaterThanOrEqual(0.9);
      expect(m).toBeLessThanOrEqual(1.15);
    }
  });

  it("서로 다른 id는 다른 배율(해시 분산 스모크)", () => {
    expect(agentRateMul("agent-1")).not.toBe(agentRateMul("agent-2"));
  });
});
