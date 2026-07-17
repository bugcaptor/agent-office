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
    note: "",
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
