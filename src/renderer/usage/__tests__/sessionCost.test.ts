// src/renderer/usage/__tests__/sessionCost.test.ts
//
// sessionCost.ts 순수 함수 검증: addTurn 누적/불변성, 단가를 모르는 모델이
// costUnknownTurns로 빠지는지, mergeTotals, aggregateSeed의 stop·tokens·
// at<=maxAt 3중 필터.
import { describe, expect, it } from "vitest";
import type { SessionEventRecord } from "@shared/types";
import { addTurn, aggregateSeed, emptyTotals, mergeTotals } from "../sessionCost";

function record(overrides: Partial<SessionEventRecord>): SessionEventRecord {
  return {
    schemaVersion: 1,
    runId: "run1",
    seq: 1,
    at: 1000,
    agentId: "a1",
    sessionId: "s1",
    kind: "stop",
    ...overrides,
  };
}

describe("emptyTotals", () => {
  it("전부 0인 빈 누계를 반환한다", () => {
    expect(emptyTotals()).toEqual({
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      costUsd: 0,
      costUnknownTurns: 0,
      turns: 0,
    });
  });
});

describe("addTurn", () => {
  it("토큰을 누적하고 원본 totals는 건드리지 않는다(불변)", () => {
    const t0 = emptyTotals();
    const t1 = addTurn(t0, { input: 100, output: 50, cacheRead: 20, cacheWrite: 10, model: "claude-opus-5" });
    expect(t0).toEqual(emptyTotals()); // 원본 불변
    expect(t1.input).toBe(100);
    expect(t1.output).toBe(50);
    expect(t1.cacheRead).toBe(20);
    expect(t1.cacheWrite).toBe(10);
    expect(t1.turns).toBe(1);
    expect(t1.model).toBe("claude-opus-5");
    expect(t1.costUsd).toBeGreaterThan(0);
    expect(t1.costUnknownTurns).toBe(0);

    const t2 = addTurn(t1, { input: 100, model: "claude-opus-5" });
    expect(t2.input).toBe(200);
    expect(t2.turns).toBe(2);
  });

  it("단가를 모르는 모델은 costUsd에 더하지 않고 costUnknownTurns만 올린다", () => {
    const t1 = addTurn(emptyTotals(), { input: 1000, output: 1000, model: "llama-3" });
    expect(t1.costUsd).toBe(0);
    expect(t1.costUnknownTurns).toBe(1);
    expect(t1.turns).toBe(1);
  });

  it("model이 없는 턴은 이전 model을 유지하고, 있으면 최신 값으로 갱신한다", () => {
    const t1 = addTurn(emptyTotals(), { input: 10, model: "claude-sonnet-4-5" });
    const t2 = addTurn(t1, { input: 10 }); // 이 턴엔 model 없음
    expect(t2.model).toBe("claude-sonnet-4-5");
    const t3 = addTurn(t2, { input: 10, model: "claude-opus-5" });
    expect(t3.model).toBe("claude-opus-5");
  });
});

describe("mergeTotals", () => {
  it("두 누계를 항목별로 더한 새 객체를 반환한다", () => {
    const a = addTurn(emptyTotals(), { input: 100, output: 10, model: "claude-opus-5" });
    const b = addTurn(emptyTotals(), { input: 50, output: 5, model: "llama-3" });
    const merged = mergeTotals(a, b);
    expect(merged.input).toBe(150);
    expect(merged.output).toBe(15);
    expect(merged.turns).toBe(2);
    expect(merged.costUnknownTurns).toBe(1);
    // b의 model이 있으면 우선한다.
    expect(merged.model).toBe("llama-3");
  });

  it("b에 model이 없으면 a의 model을 유지한다", () => {
    const a = addTurn(emptyTotals(), { input: 100, model: "claude-opus-5" });
    const b = emptyTotals();
    expect(mergeTotals(a, b).model).toBe("claude-opus-5");
  });
});

describe("aggregateSeed", () => {
  it("kind 불문 tokens 존재·at<=maxAt인 레코드만 sessionId별로 합산한다", () => {
    const records: SessionEventRecord[] = [
      // 과거 파일: 토큰이 kind=stop에 실려 있다.
      record({ sessionId: "s1", at: 100, kind: "stop", tokens: { input: 10, model: "claude-opus-5" } }),
      // 신규 파일: 토큰이 kind=usage에 실려 있다 — 둘 다 kind 불문 합산된다.
      record({ sessionId: "s1", at: 200, kind: "usage", tokens: { input: 20, model: "claude-opus-5" } }),
      // tool은 tokens가 없으므로(실제로 안 실림) 자연히 제외된다.
      record({ sessionId: "s1", at: 150, kind: "tool" }),
      // tokens가 없으면 kind가 stop이어도 제외(신규 stop은 tokens가 없다).
      record({ sessionId: "s1", at: 160, kind: "stop" }),
      // at > maxAt이면 제외
      record({ sessionId: "s1", at: 9999, kind: "usage", tokens: { input: 999 } }),
      // 다른 세션은 따로 집계
      record({ sessionId: "s2", at: 50, kind: "stop", tokens: { input: 5, model: "claude-haiku-4-5" } }),
    ];
    const out = aggregateSeed(records, 1000);
    expect(out["s1"].input).toBe(30);
    expect(out["s1"].turns).toBe(2);
    expect(out["s2"].input).toBe(5);
    expect(out["s2"].turns).toBe(1);
  });

  it("usage/stop 혼재에서도 이중 계산이 없다(신규 stop엔 tokens가 없으므로 같은 턴이 두 번 안 잡힌다)", () => {
    const records: SessionEventRecord[] = [
      // 한 턴이 실제로 남기는 신규 기록 모양: usage(토큰 실림) + stop(토큰 없음).
      record({ sessionId: "s1", at: 100, kind: "usage", tokens: { input: 10, model: "claude-opus-5" } }),
      record({ sessionId: "s1", at: 101, kind: "stop" }),
    ];
    const out = aggregateSeed(records, 1000);
    expect(out["s1"].input).toBe(10);
    expect(out["s1"].turns).toBe(1);
  });

  it("해당하는 레코드가 없으면 빈 객체", () => {
    expect(aggregateSeed([], 1000)).toEqual({});
  });
});
