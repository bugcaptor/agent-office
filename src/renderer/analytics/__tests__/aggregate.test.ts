// src/renderer/analytics/__tests__/aggregate.test.ts
//
// aggregate.ts 순수 함수 검증: 턴 재구성 규칙(기본 페어링, 연속 prompt, stop
// 유실+exited 마감, 미마감 턴), 자정 분할, 로컬 날짜 귀속, 삭제 에이전트 이름/
// 색 폴백. 타임존은 fixedOffsetCalendar(540)=KST로 고정한다.
import { describe, expect, it } from "vitest";
import type { AgentProfile, SessionEventKind, SessionEventRecord } from "@shared/types";
import {
  aggregate,
  agentMeta,
  dailySummary,
  dayRange,
  fixedOffsetCalendar,
  reconstructTurns,
  splitTurnByDay,
} from "../aggregate";
import { DELETED_GRAYS } from "../colors";

const KST = fixedOffsetCalendar(540);
/** KST 벽시계 시각의 실제 epoch ms. */
function kst(y: number, mo: number, d: number, h: number, mi = 0): number {
  return Date.UTC(y, mo, d, h, mi) - 540 * 60_000;
}

let seq = 0;
function ev(partial: Partial<SessionEventRecord> & { kind: SessionEventKind; at: number }): SessionEventRecord {
  return {
    schemaVersion: 1,
    runId: "r",
    seq: seq++,
    agentId: "a1",
    sessionId: "s1",
    ...partial,
  };
}

function profile(id: string, name: string): AgentProfile {
  return {
    id,
    name,
    role: "",
    seed: `seed-${id}`,
    createdAt: 0,
    deskIndex: 0,
  };
}

describe("reconstructTurns", () => {
  it("prompt→stop 기본 페어링", () => {
    const start = kst(2026, 6, 11, 10, 0);
    const stop = kst(2026, 6, 11, 10, 30);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: start }),
      ev({ kind: "stop", at: stop }),
    ]);
    expect(turns).toEqual([{ agentId: "a1", sessionId: "s1", startAt: start, endAt: stop }]);
  });

  it("연속 prompt는 같은 턴(첫 prompt에서 시작)", () => {
    const p1 = kst(2026, 6, 11, 10, 0);
    const p2 = kst(2026, 6, 11, 10, 5);
    const stop = kst(2026, 6, 11, 10, 30);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: p1 }),
      ev({ kind: "prompt", at: p2 }),
      ev({ kind: "stop", at: stop }),
    ]);
    expect(turns).toHaveLength(1);
    expect(turns[0].startAt).toBe(p1);
    expect(turns[0].endAt).toBe(stop);
  });

  it("stop 유실 시 exited로 강제 마감", () => {
    const start = kst(2026, 6, 11, 10, 0);
    const exited = kst(2026, 6, 11, 10, 20);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: start }),
      ev({ kind: "session_state", state: "exited", at: exited }),
    ]);
    expect(turns).toEqual([{ agentId: "a1", sessionId: "s1", startAt: start, endAt: exited }]);
  });

  it("끝까지 안 닫힌 턴은 세션 마지막 이벤트로 마감", () => {
    const start = kst(2026, 6, 11, 10, 0);
    const tool = kst(2026, 6, 11, 10, 10);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: start }),
      ev({ kind: "tool", at: tool }),
    ]);
    expect(turns).toEqual([{ agentId: "a1", sessionId: "s1", startAt: start, endAt: tool }]);
  });

  it("열린 턴 없이 온 stop은 무시한다", () => {
    const turns = reconstructTurns([ev({ kind: "stop", at: kst(2026, 6, 11, 10, 0) })]);
    expect(turns).toEqual([]);
  });

  it("서로 다른 세션은 독립적으로 재구성한다", () => {
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), sessionId: "s1" }),
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 1), sessionId: "s2" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 5), sessionId: "s2" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 9), sessionId: "s1" }),
    ]);
    expect(turns).toHaveLength(2);
    const s1 = turns.find((t) => t.sessionId === "s1")!;
    const s2 = turns.find((t) => t.sessionId === "s2")!;
    expect(s1.endAt - s1.startAt).toBe(9 * 60_000);
    expect(s2.endAt - s2.startAt).toBe(4 * 60_000);
  });
});

describe("splitTurnByDay", () => {
  it("자정을 걸치는 턴을 로컬 날짜 경계에서 분할한다", () => {
    const start = kst(2026, 6, 11, 23, 30);
    const end = kst(2026, 6, 12, 0, 30);
    const slices = splitTurnByDay(start, end, KST);
    expect(slices).toEqual([
      { date: "2026-07-11", ms: 30 * 60_000 },
      { date: "2026-07-12", ms: 30 * 60_000 },
    ]);
  });

  it("하루 안의 턴은 한 조각", () => {
    const start = kst(2026, 6, 11, 9, 0);
    const end = kst(2026, 6, 11, 9, 45);
    expect(splitTurnByDay(start, end, KST)).toEqual([
      { date: "2026-07-11", ms: 45 * 60_000 },
    ]);
  });
});

describe("dailySummary", () => {
  it("작업시간은 자정 분할, 턴은 시작일, 도구는 발생일에 귀속한다", () => {
    const start = kst(2026, 6, 11, 23, 30);
    const end = kst(2026, 6, 12, 0, 30);
    const events = [
      ev({ kind: "prompt", at: start }),
      ev({ kind: "tool", at: kst(2026, 6, 11, 23, 40) }), // 07-11
      ev({ kind: "tool", at: kst(2026, 6, 12, 0, 10) }), // 07-12
      ev({ kind: "stop", at: end }),
    ];
    const turns = reconstructTurns(events);
    const daily = dailySummary(events, turns, KST);

    expect(daily["2026-07-11"].a1.workedMs).toBe(30 * 60_000);
    expect(daily["2026-07-12"].a1.workedMs).toBe(30 * 60_000);
    // 턴 수는 시작일(07-11)에만 1.
    expect(daily["2026-07-11"].a1.turns).toBe(1);
    expect(daily["2026-07-12"].a1.turns).toBe(0);
    // 도구 이벤트는 각자의 로컬 날짜.
    expect(daily["2026-07-11"].a1.toolEvents).toBe(1);
    expect(daily["2026-07-12"].a1.toolEvents).toBe(1);
  });
});

describe("dailySummary 토큰", () => {
  it("stop의 tokens를 stop 시각의 로컬 날짜에 합산하고 비용을 환산한다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 11, 10, 20),
        tokens: { input: 1_000, output: 2_000, cacheRead: 10_000, model: "claude-opus-5" },
      }),
    ];
    const daily = dailySummary(events, reconstructTurns(events), KST);
    const cell = daily["2026-07-11"].a1;
    expect(cell.tokensIn).toBe(1_000);
    expect(cell.tokensOut).toBe(2_000);
    expect(cell.tokensCacheRead).toBe(10_000);
    expect(cell.tokensCacheWrite).toBe(0);
    // opus: 1000*5 + 2000*25 + 10000*0.5 = 5000 + 50000 + 5000 = 60000 / 1e6
    expect(cell.costUsd).toBeCloseTo(0.06, 10);
    expect(cell.costUnknownTurns).toBe(0);
  });

  it("tokens 없는 과거 stop이 섞여도 0으로 견딘다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 20) }), // 과거 기록 — tokens 없음
    ];
    const daily = dailySummary(events, reconstructTurns(events), KST);
    const cell = daily["2026-07-11"].a1;
    expect(cell.tokensIn).toBe(0);
    expect(cell.tokensOut).toBe(0);
    expect(cell.costUsd).toBe(0);
    expect(cell.costUnknownTurns).toBe(0);
  });

  it("자정을 걸친 턴의 토큰은 마감(stop)일에 통째로 귀속한다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 23, 30) }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 12, 0, 30),
        tokens: { input: 500, model: "claude-sonnet-4-5" },
      }),
    ];
    const daily = dailySummary(events, reconstructTurns(events), KST);
    expect(daily["2026-07-11"].a1.tokensIn).toBe(0);
    expect(daily["2026-07-12"].a1.tokensIn).toBe(500);
  });
});

describe("kind=usage (알림과 분리된 사용량 채널)", () => {
  it("① usage 레코드의 tokens도 발생 시각의 로컬 날짜에 합산·비용 환산된다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({
        kind: "usage",
        at: kst(2026, 6, 11, 10, 20),
        tokens: { input: 1_000, output: 2_000, cacheRead: 10_000, model: "claude-opus-5" },
      }),
    ];
    const daily = dailySummary(events, reconstructTurns(events), KST);
    const cell = daily["2026-07-11"].a1;
    expect(cell.tokensIn).toBe(1_000);
    expect(cell.tokensOut).toBe(2_000);
    expect(cell.tokensCacheRead).toBe(10_000);
    // opus: 1000*5 + 2000*25 + 10000*0.5 = 60000 / 1e6
    expect(cell.costUsd).toBeCloseTo(0.06, 10);
  });

  it("② usage는 턴을 닫지 않는다 — prompt→usage→stop에서 endAt은 stop 시각이다", () => {
    const start = kst(2026, 6, 11, 10, 0);
    const usageAt = kst(2026, 6, 11, 10, 5);
    const stop = kst(2026, 6, 11, 10, 30);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: start }),
      ev({ kind: "usage", at: usageAt, tokens: { input: 10 } }),
      ev({ kind: "stop", at: stop }),
    ]);
    expect(turns).toEqual([{ agentId: "a1", sessionId: "s1", startAt: start, endAt: stop }]);
  });

  it("③ prompt+usage만 있는 세션은 조기 마감되지 않고 마지막 이벤트(usage)로 마감된다", () => {
    const start = kst(2026, 6, 11, 10, 0);
    const usageAt = kst(2026, 6, 11, 10, 10);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: start }),
      ev({ kind: "usage", at: usageAt, tokens: { input: 10 } }),
    ]);
    // stop/session_state 부재 → usage가 턴을 닫지 못하므로, tool과 같은 처리
    // (세션 마지막 이벤트 시각으로 강제 마감)로 떨어진다.
    expect(turns).toEqual([{ agentId: "a1", sessionId: "s1", startAt: start, endAt: usageAt }]);
  });

  it("④ usage 레코드는 toolEvents에 섞이지 않는다(kind==='tool'만 센다)", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({ kind: "tool", at: kst(2026, 6, 11, 10, 5) }),
      ev({ kind: "usage", at: kst(2026, 6, 11, 10, 10), tokens: { input: 10 } }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 20) }),
    ];
    const daily = dailySummary(events, reconstructTurns(events), KST);
    expect(daily["2026-07-11"].a1.toolEvents).toBe(1);
  });
});

describe("agentMeta", () => {
  it("현재 프로필이 있으면 이름과 대표색(비회색)을 쓴다", () => {
    const events = [ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" })];
    const meta = agentMeta(events, { a1: profile("a1", "Ada") });
    expect(meta.a1.name).toBe("Ada");
    expect(meta.a1.deleted).toBe(false);
    expect(meta.a1.color).toMatch(/^#[0-9a-f]{6}$/);
    expect(DELETED_GRAYS).not.toContain(meta.a1.color);
  });

  it("삭제된 에이전트는 마지막 session_started 이름 + 회색 폴백", () => {
    const events = [
      ev({
        kind: "session_started",
        at: kst(2026, 6, 11, 9, 0),
        agentId: "gone",
        agentName: "옛이름",
      }),
      ev({
        kind: "session_started",
        at: kst(2026, 6, 11, 12, 0),
        agentId: "gone",
        agentName: "새이름",
      }),
      ev({ kind: "prompt", at: kst(2026, 6, 11, 12, 1), agentId: "gone" }),
    ];
    const meta = agentMeta(events, {}); // 프로필 없음
    expect(meta.gone.name).toBe("새이름"); // 가장 최근 스냅샷
    expect(meta.gone.deleted).toBe(true);
    expect(DELETED_GRAYS).toContain(meta.gone.color);
  });

  it("프로필도 스냅샷도 없으면 ID를 축약한다", () => {
    const events = [ev({ kind: "tool", at: kst(2026, 6, 11, 10, 0), agentId: "abcdefghijklmnop" })];
    const meta = agentMeta(events, {});
    expect(meta.abcdefghijklmnop.name).toBe("abcdefgh");
  });
});

describe("aggregate", () => {
  it("요약을 작업시간 내림차순으로 정렬하고 활동일을 센다", () => {
    const events = [
      // a1: 07-11 20분 턴 + 07-12 40분 턴 = 60분, 활동 2일
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 20), agentId: "a1" }),
      ev({ kind: "prompt", at: kst(2026, 6, 12, 10, 0), agentId: "a1" }),
      ev({ kind: "stop", at: kst(2026, 6, 12, 10, 40), agentId: "a1" }),
      // a2: 07-11 10분 턴, 활동 1일
      ev({ kind: "prompt", at: kst(2026, 6, 11, 11, 0), agentId: "a2", sessionId: "s2" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 11, 10), agentId: "a2", sessionId: "s2" }),
    ];
    const profiles = { a1: profile("a1", "Ada"), a2: profile("a2", "Bob") };
    const data = aggregate(events, profiles, KST);

    expect(data.summary.map((r) => r.agentId)).toEqual(["a1", "a2"]);
    expect(data.summary[0].workedMs).toBe(60 * 60_000);
    expect(data.summary[0].activeDays).toBe(2);
    expect(data.summary[0].turns).toBe(2);
    expect(data.summary[1].workedMs).toBe(10 * 60_000);
    expect(data.summary[1].activeDays).toBe(1);
  });

  it("이벤트가 없으면 빈 집계", () => {
    const data = aggregate([], {}, KST);
    expect(data.summary).toEqual([]);
    expect(data.daily).toEqual({});
    expect(data.meta).toEqual({});
  });

  it("range: 경계를 걸친 턴은 fromAt부터 클립 귀속, lookback 전용 에이전트는 제외", () => {
    const fromAt = kst(2026, 6, 11, 0, 0);
    const toAt = kst(2026, 6, 12, 23, 59);
    const events = [
      // a1: lookback prompt(07-10 23:30) + 범위 내 stop(07-11 00:30) → 30분만 07-11 귀속.
      ev({ kind: "prompt", at: kst(2026, 6, 10, 23, 30), agentId: "a1" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 0, 30), agentId: "a1" }),
      // ghost: lookback에서만 활동(전 구간이 fromAt 이전) → 클립 시 사라진다.
      ev({ kind: "prompt", at: kst(2026, 6, 10, 20, 0), agentId: "ghost", sessionId: "sg" }),
      ev({ kind: "stop", at: kst(2026, 6, 10, 20, 10), agentId: "ghost", sessionId: "sg" }),
    ];
    const data = aggregate(events, { a1: profile("a1", "Ada") }, KST, { fromAt, toAt });

    // ghost는 요약/메타에 없다(유령 방지).
    expect(data.summary.map((r) => r.agentId)).toEqual(["a1"]);
    expect(data.meta.ghost).toBeUndefined();
    // a1은 창 안 몫(30분)만 07-11에 귀속, 턴 수는 dayKey(max(start, fromAt))=07-11.
    expect(data.daily["2026-07-11"].a1.workedMs).toBe(30 * 60_000);
    expect(data.daily["2026-07-11"].a1.turns).toBe(1);
    expect(data.summary[0].workedMs).toBe(30 * 60_000);
    // lookback 날짜(07-10)에는 아무것도 새지 않는다.
    expect(data.daily["2026-07-10"]).toBeUndefined();
  });

  it("range 미지정 시 기존 동작(전체 집계)과 동일하다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 20), agentId: "a1" }),
    ];
    const withRange = aggregate(events, { a1: profile("a1", "Ada") }, KST);
    expect(withRange.summary[0].workedMs).toBe(20 * 60_000);
    expect(withRange.daily["2026-07-11"].a1.turns).toBe(1);
  });
});

describe("aggregate 토큰·비용", () => {
  it("모델별 구성의 알려진 비용을 합산하고 미지 단가는 표시한다", () => {
    const data = aggregate([
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({ kind: "usage", at: kst(2026, 6, 11, 10, 1), agentId: "a1", partial: true,
        tokens: { input: 3000, model: "gpt-5.5", byModel: [
          { input: 1000, model: "gpt-5.5" },
          { input: 1000, model: "gpt-6-astra" },
          { input: 1000, model: "unknown-model" },
        ] },
      }),
    ], { a1: profile("a1", "Ada") }, KST);
    expect(data.summary[0].tokensIn).toBe(3000);
    expect(data.summary[0].costUsd).toBeCloseTo(0.015);
    expect(data.summary[0].costUnknownTurns).toBe(1);
  });

  it("요약에 토큰과 비용을 합산한다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 11, 10, 20),
        agentId: "a1",
        tokens: { input: 1_000, output: 1_000, model: "claude-opus-5" },
      }),
      ev({ kind: "prompt", at: kst(2026, 6, 12, 10, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 12, 10, 40),
        agentId: "a1",
        tokens: { input: 1_000, cacheWrite: 1_000, model: "claude-opus-5" },
      }),
    ];
    const data = aggregate(events, { a1: profile("a1", "Ada") }, KST);
    const row = data.summary[0];
    expect(row.tokensIn).toBe(2_000);
    expect(row.tokensOut).toBe(1_000);
    expect(row.tokensCacheWrite).toBe(1_000);
    // (1000*5 + 1000*25) + (1000*5 + 1000*6.25) = 30000 + 11250 = 41250 / 1e6
    expect(row.costUsd).toBeCloseTo(0.04125, 10);
    expect(row.costUnknownTurns).toBe(0);
    // 활동일 판정은 그대로(작업/턴/도구 기준) 2일.
    expect(row.activeDays).toBe(2);
  });

  it("미지 모델은 costUnknownTurns로 세고 costUsd에서 뺀다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 11, 10, 20),
        agentId: "a1",
        tokens: { input: 1_000_000, model: "llama-3" },
      }),
      ev({ kind: "prompt", at: kst(2026, 6, 11, 11, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 11, 11, 20),
        agentId: "a1",
        tokens: { input: 1_000_000, model: "claude-haiku-4-5" },
      }),
    ];
    const data = aggregate(events, { a1: profile("a1", "Ada") }, KST);
    const row = data.summary[0];
    // 토큰 합계에는 미지 모델도 들어간다(쓴 건 쓴 것).
    expect(row.tokensIn).toBe(2_000_000);
    // 비용은 haiku 몫(1$)만.
    expect(row.costUsd).toBeCloseTo(1, 10);
    expect(row.costUnknownTurns).toBe(1);
  });

  it("range 밖 stop의 토큰은 제외한다", () => {
    const fromAt = kst(2026, 6, 11, 0, 0);
    const toAt = kst(2026, 6, 12, 23, 59);
    const events = [
      // lookback 구간(07-10)에서 시작·마감된 a1 턴 — 토큰이 새면 안 된다.
      ev({ kind: "prompt", at: kst(2026, 6, 10, 20, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 10, 20, 30),
        agentId: "a1",
        tokens: { input: 999_999, model: "claude-opus-5" },
      }),
      // 창 안 턴.
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0), agentId: "a1" }),
      ev({
        kind: "stop",
        at: kst(2026, 6, 11, 10, 20),
        agentId: "a1",
        tokens: { input: 1_000, model: "claude-opus-5" },
      }),
    ];
    const data = aggregate(events, { a1: profile("a1", "Ada") }, KST, { fromAt, toAt });
    expect(data.summary[0].tokensIn).toBe(1_000);
    expect(data.summary[0].costUsd).toBeCloseTo(0.005, 10);
    expect(data.daily["2026-07-10"]).toBeUndefined();
  });
});

describe("dayRange", () => {
  it("로컬 기간을 양끝 포함해 날짜 키로 나열한다", () => {
    const from = kst(2026, 6, 10, 5, 0);
    const to = kst(2026, 6, 12, 23, 0);
    expect(dayRange(from, to, KST)).toEqual(["2026-07-10", "2026-07-11", "2026-07-12"]);
  });

  it("역전된 범위는 빈 목록", () => {
    expect(dayRange(kst(2026, 6, 12, 0, 0), kst(2026, 6, 10, 0, 0), KST)).toEqual([]);
  });
});

describe("봇 주입 턴 분리(kbm #2j8)", () => {
  it("턴은 자기를 연 prompt의 origin을 물려받고, 뒤이은 prompt는 덮어쓰지 않는다", () => {
    const p1 = kst(2026, 6, 11, 10, 0);
    const p2 = kst(2026, 6, 11, 10, 5);
    const stop = kst(2026, 6, 11, 10, 30);
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: p1, origin: "bot" }),
      ev({ kind: "prompt", at: p2 }), // 같은 턴으로 흡수 — 출처를 바꾸지 않는다
      ev({ kind: "stop", at: stop }),
    ]);
    expect(turns).toEqual([
      { agentId: "a1", sessionId: "s1", startAt: p1, endAt: stop, origin: "bot" },
    ]);
  });

  it("origin이 없는 프롬프트로 열린 턴에는 origin 키가 붙지 않는다(하위호환)", () => {
    const turns = reconstructTurns([
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 10) }),
    ]);
    expect(turns[0].origin).toBeUndefined();
  });

  it("봇 몫은 총계에서 빼지 않고 bot* 로 따로 쌓인다", () => {
    const events = [
      // 봇 턴 20분.
      ev({ kind: "prompt", at: kst(2026, 6, 11, 9, 0), origin: "bot" }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 9, 20) }),
      // 사람 턴 40분.
      ev({ kind: "prompt", at: kst(2026, 6, 11, 10, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 10, 40) }),
    ];
    const data = aggregate(events, { a1: profile("a1", "Ada") }, KST);
    const row = data.summary[0];
    // 총계는 그대로 — 분석 패널에 보이는 수치는 바뀌지 않는다.
    expect(row.workedMs).toBe(60 * 60_000);
    expect(row.turns).toBe(2);
    expect(row.botWorkedMs).toBe(20 * 60_000);
    expect(row.botTurns).toBe(1);
    const cell = data.daily["2026-07-11"].a1;
    expect(cell.workedMs).toBe(60 * 60_000);
    expect(cell.botWorkedMs).toBe(20 * 60_000);
    expect(cell.botTurns).toBe(1);
  });

  it("자정을 걸친 봇 턴은 봇 몫도 날짜별로 쪼개진다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 23, 0), origin: "bot" }),
      ev({ kind: "stop", at: kst(2026, 6, 12, 1, 0) }),
    ];
    const data = aggregate(events, {}, KST);
    expect(data.daily["2026-07-11"].a1.botWorkedMs).toBe(60 * 60_000);
    expect(data.daily["2026-07-12"].a1.botWorkedMs).toBe(60 * 60_000);
    // 턴 수는 시작 날짜에만.
    expect(data.daily["2026-07-11"].a1.botTurns).toBe(1);
    expect(data.daily["2026-07-12"].a1.botTurns).toBe(0);
  });

  it("봇만 돌린 날은 사람 활동일로 세지 않는다(도구 이벤트가 있어도)", () => {
    const events = [
      // 07-10: 봇만 + 도구.
      ev({ kind: "prompt", at: kst(2026, 6, 10, 9, 0), origin: "bot" }),
      ev({ kind: "tool", at: kst(2026, 6, 10, 9, 5) }),
      ev({ kind: "stop", at: kst(2026, 6, 10, 9, 30) }),
      // 07-11: 사람.
      ev({ kind: "prompt", at: kst(2026, 6, 11, 9, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 9, 30) }),
    ];
    const row = aggregate(events, {}, KST).summary[0];
    expect(row.activeDays).toBe(2);
    expect(row.humanActiveDays).toBe(1);
  });

  it("origin이 전혀 없는 과거 이벤트는 전부 사람 몫이다", () => {
    const events = [
      ev({ kind: "prompt", at: kst(2026, 6, 11, 9, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 11, 9, 30) }),
      ev({ kind: "prompt", at: kst(2026, 6, 12, 9, 0) }),
      ev({ kind: "stop", at: kst(2026, 6, 12, 9, 30) }),
    ];
    const row = aggregate(events, {}, KST).summary[0];
    expect(row.botWorkedMs).toBe(0);
    expect(row.botTurns).toBe(0);
    expect(row.humanActiveDays).toBe(row.activeDays);
  });
});
