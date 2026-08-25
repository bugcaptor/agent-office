// src/renderer/awards/__tests__/selection.test.ts
//
// 선정 로직의 결정성(같은 입력 → 같은 결과, 입력 불변), 4단계 동점 처리,
// 임계 미달/삭제, 봇 몫이 사람 몫에서 빠지는 것, 퇴근은 제외 사유가 아님,
// 그리고 월 경계 계산을 fixedOffsetCalendar(540)로 못박아 검증한다(시스템
// TZ와 무관하게 결정적).
import { describe, expect, it } from "vitest";
import { fixedOffsetCalendar, type AgentSummary } from "../../analytics/aggregate";
import {
  DEFAULT_MIN_ACTIVE_DAYS,
  DEFAULT_MIN_WORKED_MS,
  humanTurns,
  humanWorkedMs,
  monthKeyOf,
  monthRange,
  pickWinner,
  shiftMonth,
} from "../selection";

const KST = fixedOffsetCalendar(540);

/**
 * 기본은 "봇 몫 0"이다 — `humanActiveDays`는 따로 주지 않으면 `activeDays`를
 * 따라간다(봇이 없으면 둘이 같다는 집계 계약을 테스트에서도 유지).
 */
function summary(overrides: Partial<AgentSummary> & { agentId: string }): AgentSummary {
  const activeDays = overrides.activeDays ?? DEFAULT_MIN_ACTIVE_DAYS;
  return {
    name: overrides.agentId,
    color: "#fff",
    deleted: false,
    workedMs: DEFAULT_MIN_WORKED_MS,
    turns: 10,
    toolEvents: 5,
    activeDays,
    botWorkedMs: 0,
    botTurns: 0,
    humanActiveDays: activeDays,
    tokensIn: 0,
    tokensOut: 0,
    tokensCacheRead: 0,
    tokensCacheWrite: 0,
    costUsd: 0,
    costUnknownTurns: 0,
    ...overrides,
  };
}

describe("pickWinner — 정렬과 결정성", () => {
  it("workedMs 내림차순 1위가 수상자다", () => {
    const rows = [
      summary({ agentId: "a", workedMs: 3_600_000 }),
      summary({ agentId: "b", workedMs: 7_200_000 }),
    ];
    const r = pickWinner(rows);
    expect(r.winner?.agentId).toBe("b");
    expect(r.leaderboard.map((s) => s.agentId)).toEqual(["b", "a"]);
  });

  it("workedMs 동점이면 turns가 많은 쪽", () => {
    const rows = [
      summary({ agentId: "a", turns: 5 }),
      summary({ agentId: "b", turns: 9 }),
    ];
    expect(pickWinner(rows).winner?.agentId).toBe("b");
  });

  it("workedMs·turns 동점이면 activeDays가 많은 쪽", () => {
    const rows = [
      summary({ agentId: "a", activeDays: 3 }),
      summary({ agentId: "b", activeDays: 7 }),
    ];
    expect(pickWinner(rows).winner?.agentId).toBe("b");
  });

  it("셋 다 동점이면 agentId 사전식으로 앞선 쪽(완전한 타이브레이크)", () => {
    const rows = [summary({ agentId: "zeta" }), summary({ agentId: "alpha" })];
    expect(pickWinner(rows).winner?.agentId).toBe("alpha");
    // 입력 순서를 뒤집어도 같은 결과 — 정렬이 안정성에 기대지 않는다.
    expect(pickWinner(rows.slice().reverse()).winner?.agentId).toBe("alpha");
  });

  it("같은 입력이면 같은 결과이고 입력 배열/원소를 변형하지 않는다", () => {
    const rows = [
      summary({ agentId: "b", workedMs: 7_200_000 }),
      summary({ agentId: "a", workedMs: 3_600_000 }),
      summary({ agentId: "c", workedMs: 5_400_000 }),
    ];
    const before = JSON.parse(JSON.stringify(rows));
    const first = pickWinner(rows);
    const second = pickWinner(rows);
    expect(first.leaderboard.map((s) => s.agentId)).toEqual(
      second.leaderboard.map((s) => s.agentId),
    );
    expect(rows).toEqual(before);
    expect(rows.map((s) => s.agentId)).toEqual(["b", "a", "c"]);
  });

  it("순위표는 자격자 상위 5인까지만 담는다", () => {
    const rows = Array.from({ length: 8 }, (_, i) =>
      summary({ agentId: `a${i}`, workedMs: 10_000_000 - i * 1000 }),
    );
    const r = pickWinner(rows);
    expect(r.leaderboard).toHaveLength(5);
    expect(r.leaderboard.map((s) => s.agentId)).toEqual(["a0", "a1", "a2", "a3", "a4"]);
  });
});

describe("pickWinner — 후보 제외", () => {
  it("활동일/작업시간 임계 미달만 있으면 수상자가 없다", () => {
    const rows = [
      summary({ agentId: "a", activeDays: DEFAULT_MIN_ACTIVE_DAYS - 1 }),
      summary({ agentId: "b", workedMs: DEFAULT_MIN_WORKED_MS - 1 }),
    ];
    expect(pickWinner(rows)).toEqual({ winner: null, leaderboard: [] });
  });

  it("삭제된 캐릭터는 후보에서 뺀다", () => {
    const rows = [
      summary({ agentId: "gone", workedMs: 99_000_000, deleted: true }),
      summary({ agentId: "here", workedMs: 3_600_000 }),
    ];
    const r = pickWinner(rows);
    expect(r.winner?.agentId).toBe("here");
    expect(r.leaderboard.map((s) => s.agentId)).toEqual(["here"]);
  });

  it("봇 시간만 있는 캐릭터는 사람 몫이 임계 미달이라 후보에서 빠진다", () => {
    // 규칙 v1은 프로필에 봇 설정이 있다는 이유로 캐릭터를 통째로 뺐다. v2는
    // 프로필을 아예 보지 않고, 봇이 돌린 몫만 빼서 사람 몫으로 판정한다.
    const rows = [
      summary({
        agentId: "botonly",
        workedMs: 99_000_000,
        botWorkedMs: 99_000_000,
        turns: 400,
        botTurns: 400,
        activeDays: 20,
        humanActiveDays: 0,
      }),
      summary({ agentId: "human", workedMs: 3_600_000 }),
    ];
    const r = pickWinner(rows);
    expect(r.winner?.agentId).toBe("human");
    expect(r.leaderboard.map((s) => s.agentId)).toEqual(["human"]);
  });

  it("봇 시간이 섞인 캐릭터는 사람 몫만으로 겨룬다(캐릭터째 빠지지 않는다)", () => {
    // 실제 관측 사례: 총 59.4h 1위가 봇 설정 하나 때문에 통째로 빠지고
    // 18.5h가 수상했다. 이제 봇 몫만 빠지므로 사람 몫이 남으면 겨룬다.
    const rows = [
      summary({
        agentId: "mixed",
        workedMs: 59_400_000,
        botWorkedMs: 20_000_000, // 사람 몫 39.4M
        turns: 200,
        botTurns: 60,
        activeDays: 20,
        humanActiveDays: 15,
      }),
      summary({ agentId: "human", workedMs: 18_500_000, activeDays: 12 }),
    ];
    const r = pickWinner(rows);
    expect(r.winner?.agentId).toBe("mixed");
    expect(r.leaderboard.map((s) => s.agentId)).toEqual(["mixed", "human"]);
  });

  it("사람 몫이 뒤집히면 순위도 뒤집힌다(총계가 아니라 사람 몫으로 정렬)", () => {
    const rows = [
      summary({ agentId: "botheavy", workedMs: 90_000_000, botWorkedMs: 80_000_000 }),
      summary({ agentId: "steady", workedMs: 30_000_000, botWorkedMs: 0 }),
    ];
    // 총계로는 botheavy가 3배지만 사람 몫은 10M 대 30M이다.
    expect(pickWinner(rows).winner?.agentId).toBe("steady");
  });

  it("사람 workedMs 동점이면 사람 turns로, 그 다음 사람 activeDays로 가른다", () => {
    const byTurns = [
      summary({ agentId: "a", turns: 20, botTurns: 15 }), // 사람 5
      summary({ agentId: "b", turns: 10, botTurns: 1 }), // 사람 9
    ];
    expect(pickWinner(byTurns).winner?.agentId).toBe("b");

    const byDays = [
      summary({ agentId: "a", activeDays: 20, humanActiveDays: 3 }),
      summary({ agentId: "b", activeDays: 5, humanActiveDays: 5 }),
    ];
    expect(pickWinner(byDays).winner?.agentId).toBe("b");
  });

  it("사람 활동일이 임계 미달이면 총 활동일이 넉넉해도 후보가 아니다", () => {
    const rows = [
      summary({
        agentId: "a",
        workedMs: 99_000_000,
        botWorkedMs: 0,
        activeDays: 30,
        humanActiveDays: DEFAULT_MIN_ACTIVE_DAYS - 1,
      }),
    ];
    expect(pickWinner(rows).winner).toBeNull();
  });

  it("origin이 없던 옛 이벤트 집계(봇 몫 0)는 전부 사람 몫으로 잡힌다", () => {
    // 하위호환: aggregate가 과거 파일에서 만들어 내는 요약은 bot* 가 0이다.
    const rows = [summary({ agentId: "legacy", workedMs: 7_200_000, turns: 40 })];
    expect(humanWorkedMs(rows[0])).toBe(7_200_000);
    expect(humanTurns(rows[0])).toBe(40);
    expect(pickWinner(rows).winner?.agentId).toBe("legacy");
  });

  it("퇴근(clockedOut)은 제외 사유가 아니다", () => {
    // 프로필을 아예 보지 않으므로 clockedOut도 판정에 끼어들 여지가 없다.
    const rows = [summary({ agentId: "off", workedMs: 7_200_000 })];
    expect(pickWinner(rows).winner?.agentId).toBe("off");
  });

  it("임계는 옵션으로 낮출 수 있다", () => {
    const rows = [summary({ agentId: "a", activeDays: 1, workedMs: 60_000 })];
    expect(pickWinner(rows).winner).toBeNull();
    expect(
      pickWinner(rows, { minActiveDays: 1, minWorkedMs: 1000 }).winner?.agentId,
    ).toBe("a");
  });
});

describe("monthKeyOf / shiftMonth / monthRange (KST 고정)", () => {
  it("monthKeyOf는 로컬 자정 경계에서 달이 넘어간다", () => {
    // 2026-07-31 23:59:59.999 KST = 2026-07-31T14:59:59.999Z
    expect(monthKeyOf(Date.UTC(2026, 6, 31, 14, 59, 59, 999), KST)).toBe("2026-07");
    // 2026-08-01 00:00:00 KST = 2026-07-31T15:00:00Z
    expect(monthKeyOf(Date.UTC(2026, 6, 31, 15, 0, 0, 0), KST)).toBe("2026-08");
  });

  it("shiftMonth는 연 경계를 넘어간다", () => {
    expect(shiftMonth("2026-12", 1)).toBe("2027-01");
    expect(shiftMonth("2026-01", -1)).toBe("2025-12");
    expect(shiftMonth("2026-05", -17)).toBe("2024-12");
    expect(shiftMonth("2026-05", 0)).toBe("2026-05");
    expect(shiftMonth("2026-11", 14)).toBe("2028-01");
  });

  it("monthRange는 [월초 00:00, 익월초 00:00 - 1ms]다", () => {
    const r = monthRange("2026-07", KST);
    expect(r.fromAt).toBe(Date.UTC(2026, 5, 30, 15, 0, 0, 0)); // 07-01 00:00 KST
    expect(r.toAt).toBe(Date.UTC(2026, 6, 31, 15, 0, 0, 0) - 1); // 07-31 23:59:59.999 KST
    expect(monthKeyOf(r.fromAt, KST)).toBe("2026-07");
    expect(monthKeyOf(r.toAt, KST)).toBe("2026-07");
    expect(monthKeyOf(r.toAt + 1, KST)).toBe("2026-08");
  });

  it("12월 → 1월 경계도 어긋나지 않는다", () => {
    const dec = monthRange("2026-12", KST);
    expect(dec.fromAt).toBe(Date.UTC(2026, 10, 30, 15, 0, 0, 0)); // 12-01 00:00 KST
    expect(dec.toAt).toBe(Date.UTC(2026, 11, 31, 15, 0, 0, 0) - 1); // 12-31 23:59:59.999 KST

    const jan = monthRange("2027-01", KST);
    // 12월 창의 끝과 1월 창의 시작이 정확히 맞물린다(1ms 틈).
    expect(jan.fromAt).toBe(dec.toAt + 1);
    expect(monthKeyOf(jan.fromAt, KST)).toBe("2027-01");
  });

  it("2월(윤년 포함) 길이가 맞다", () => {
    const feb2028 = monthRange("2028-02", KST); // 윤년 29일
    expect(feb2028.toAt + 1 - feb2028.fromAt).toBe(29 * 86_400_000);
    const feb2026 = monthRange("2026-02", KST);
    expect(feb2026.toAt + 1 - feb2026.fromAt).toBe(28 * 86_400_000);
  });

  it("형식이 잘못된 monthKey는 던진다", () => {
    expect(() => monthRange("2026-13", KST)).toThrow();
    expect(() => shiftMonth("2026-7", 1)).toThrow();
  });
});
