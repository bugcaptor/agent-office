// src/renderer/office/__tests__/awardee.test.ts
//
// Pure-logic tests for "이 달의 우수사원" 오피스 씬 연출(docs/employee-of-the-month-design.md
// §7) — no Pixi/DOM dependency, per gen/ 관례.

import { describe, expect, it } from "vitest";
import type { AwardRecord } from "@shared/types";
import { latestAwardee, awardeeEquals, resolveAwardeeSeat, shouldShowAwardFrame } from "../awardee";
import { Tile, type OfficeMap, type DeskSlot } from "../map/mapData";
import type { AgentProfile } from "../types";

function mkRecord(month: string, winnerAgentId: string | null): AwardRecord {
  return {
    month,
    decidedAt: 0,
    rulesVersion: 1,
    winner: winnerAgentId
      ? {
          agentId: winnerAgentId,
          name: `Name-${winnerAgentId}`,
          role: "eng",
          hasPortrait: true,
          stats: { workedMs: 0, turns: 0, toolEvents: 0, activeDays: 0, tokensIn: 0, tokensOut: 0, costUsd: 0 },
        }
      : null,
    leaderboard: [],
    speeches: [],
  };
}

describe("latestAwardee", () => {
  it("winner가 있는 가장 최근(month 최댓값) 레코드를 고른다", () => {
    const awards = [mkRecord("2026-05", "a1"), mkRecord("2026-07", "a2"), mkRecord("2026-06", "a3")];
    expect(latestAwardee(awards)?.agentId).toBe("a2");
    expect(latestAwardee(awards)?.month).toBe("2026-07");
  });

  it("입력 순서와 무관하다(오름차순이 아니어도 month로 직접 비교)", () => {
    const ascending = [mkRecord("2026-01", "a1"), mkRecord("2026-02", "a2")];
    const shuffled = [mkRecord("2026-02", "a2"), mkRecord("2026-01", "a1")];
    expect(latestAwardee(ascending)).toEqual(latestAwardee(shuffled));
  });

  it("가장 최근 달의 winner가 null이면 그 이전 달 중 winner가 있는 최신 달을 고른다", () => {
    const awards = [mkRecord("2026-05", "a1"), mkRecord("2026-06", null)];
    expect(latestAwardee(awards)?.agentId).toBe("a1");
    expect(latestAwardee(awards)?.month).toBe("2026-05");
  });

  it("winner가 있는 레코드가 하나도 없으면 null", () => {
    expect(latestAwardee([mkRecord("2026-05", null), mkRecord("2026-06", null)])).toBeNull();
  });

  it("빈 배열이면 null", () => {
    expect(latestAwardee([])).toBeNull();
  });

  it("hasPortrait/name을 winner 스냅샷 그대로 옮긴다", () => {
    const rec = mkRecord("2026-07", "a1");
    (rec.winner as { hasPortrait: boolean }).hasPortrait = false;
    expect(latestAwardee([rec])).toEqual({ agentId: "a1", name: "Name-a1", month: "2026-07", hasPortrait: false });
  });
});

describe("awardeeEquals", () => {
  it("agentId/name/month/hasPortrait이 모두 같으면 true(참조가 달라도)", () => {
    const a = { agentId: "a1", name: "N", month: "2026-07", hasPortrait: true };
    const b = { agentId: "a1", name: "N", month: "2026-07", hasPortrait: true };
    expect(awardeeEquals(a, b)).toBe(true);
  });

  it("한쪽만 null이면 false", () => {
    expect(awardeeEquals(null, { agentId: "a1", name: "N", month: "2026-07", hasPortrait: true })).toBe(false);
    expect(awardeeEquals({ agentId: "a1", name: "N", month: "2026-07", hasPortrait: true }, null)).toBe(false);
  });

  it("둘 다 null이면 true", () => {
    expect(awardeeEquals(null, null)).toBe(true);
  });

  it("필드 하나라도 다르면 false", () => {
    const a = { agentId: "a1", name: "N", month: "2026-07", hasPortrait: true };
    expect(awardeeEquals(a, { ...a, hasPortrait: false })).toBe(false);
    expect(awardeeEquals(a, { ...a, month: "2026-08" })).toBe(false);
  });
});

// 5x5 floor map, 2 desks — OfficeWorld.test.ts의 makeMap과 같은 모양.
function makeMap(deskCount = 2): OfficeMap {
  const row = (chars: string) => [...chars].map((c) => (c === "W" ? Tile.Wall : Tile.Floor));
  const tiles = [row("WWWWW"), row("WFFFW"), row("WFFFW"), row("WFFFW"), row("WWWWW")];
  const desks: DeskSlot[] = Array.from({ length: deskCount }, (_, i) => ({
    index: i,
    seat: { tx: 1 + i, ty: 2 },
    facing: "up",
  }));
  return { width: 5, height: 5, tiles, desks };
}

const profile = (id: string, overrides: Partial<AgentProfile> = {}): AgentProfile => ({
  id,
  name: id,
  role: "eng",
  seed: id,
  ...overrides,
});

describe("resolveAwardeeSeat", () => {
  it("agentId가 null이면 null", () => {
    expect(resolveAwardeeSeat(makeMap(), null, [profile("a1")])).toBeNull();
  });

  it("수상자가 로스터에 없으면(프로필 삭제) null", () => {
    expect(resolveAwardeeSeat(makeMap(), "gone", [profile("a1")])).toBeNull();
  });

  it("수상자가 좌석을 배정받으면 그 좌석을 돌려준다(수동 지정 없이도 assignDesks의 배정을 그대로 따른다)", () => {
    const map = makeMap();
    const seat = resolveAwardeeSeat(map, "a1", [profile("a1")]);
    expect(map.desks.map((d) => d.seat)).toContainEqual(seat);
  });

  it("책상이 부족해 수상자가 배정을 못 받으면 null", () => {
    const map = makeMap(1); // 책상 1개
    // a1이 정렬상 밀려 책상을 못 받도록 다른 에이전트에 수동 배정을 준다.
    const profiles = [profile("a1"), profile("a2", { assignedDeskIndex: 0 })];
    expect(resolveAwardeeSeat(map, "a1", profiles)).toBeNull();
  });

  it("assignedDeskIndex(수동 지정)를 반영한다", () => {
    const map = makeMap(2);
    const profiles = [profile("a1", { assignedDeskIndex: 1 })];
    expect(resolveAwardeeSeat(map, "a1", profiles)).toEqual(map.desks[1].seat);
  });
});

describe("shouldShowAwardFrame", () => {
  it("awardee가 있으면 true", () => {
    expect(shouldShowAwardFrame({ agentId: "a1", name: "N", month: "2026-07", hasPortrait: true })).toBe(true);
  });

  it("awardee가 null이면 false", () => {
    expect(shouldShowAwardFrame(null)).toBe(false);
  });
});
