// src/renderer/awards/__tests__/awardsStore.test.ts
//
// 시상 스토어: write-once 소급 확정(이미 있는 달 skip, 누락 월만, 12개월 캡,
// 진행 중인 이번 달 제외, 빈 달도 winner:null로 확정), 수상자 스냅샷 조립,
// 소감 인플라이트 가드. IPC·appStore·소감 생성기를 전부 목으로 대체한다.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, AwardRecord, AwardsFile, SessionEventRecord } from "@shared/types";

const h = vi.hoisted(() => ({
  loadAwards: vi.fn(),
  finalizeAward: vi.fn(),
  appendAwardSpeech: vi.fn(),
  loadAwardPortrait: vi.fn(),
  loadSessionEvents: vi.fn(),
  generateSpeech: vi.fn(),
  agents: {} as Record<string, AgentProfile>,
}));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadAwards: h.loadAwards,
    finalizeAward: h.finalizeAward,
    appendAwardSpeech: h.appendAwardSpeech,
    loadAwardPortrait: h.loadAwardPortrait,
    loadSessionEvents: h.loadSessionEvents,
  },
}));
// 프로필은 appStore에서 읽지만 스토어 전체를 끌어올 이유는 없다 — getState만 흉내낸다.
vi.mock("../../store/appStore", () => ({
  useAppStore: { getState: () => ({ agents: h.agents }) },
}));
vi.mock("../speechGenerator", () => ({ generateSpeech: h.generateSpeech }));

import { localDayCalendar } from "../../analytics/aggregate";
import { monthKeyOf, monthRange, shiftMonth } from "../selection";
import { BACKFILL_MONTHS, useAwardsStore } from "../awardsStore";

/** 테스트 기준 시각(아무 달이나 되게 시스템 TZ와 무관하게 파생시킨다). */
const NOW = Date.UTC(2026, 7, 20, 6, 0, 0, 0);
const THIS_MONTH = monthKeyOf(NOW, localDayCalendar);
/** `back`달 전의 monthKey. */
const back = (n: number): string => shiftMonth(THIS_MONTH, -n);

function file(records: AwardRecord[]): AwardsFile {
  return { version: 1, awards: records.slice().sort((a, b) => (a.month < b.month ? -1 : 1)) };
}

function record(month: string, winnerId?: string): AwardRecord {
  return {
    month,
    decidedAt: 0,
    rulesVersion: 1,
    winner: winnerId
      ? {
          agentId: winnerId,
          name: winnerId,
          role: "개발",
          hasPortrait: false,
          stats: {
            workedMs: 0,
            turns: 0,
            toolEvents: 0,
            activeDays: 0,
            tokensIn: 0,
            tokensOut: 0,
            costUsd: 0,
          },
        }
      : null,
    leaderboard: [],
    speeches: [],
  };
}

function profile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return { id, name: id, role: "개발", seed: "s", createdAt: 0, deskIndex: 0, ...overrides };
}

/** `at`에 시작해 `ms` 동안 이어진 턴 한 개(prompt+stop). */
function turnEvents(agentId: string, at: number, ms: number, seq: number): SessionEventRecord[] {
  const base = { schemaVersion: 1, runId: "r1", agentId, sessionId: "s1" };
  return [
    { ...base, seq, at, kind: "prompt" as const },
    { ...base, seq: seq + 1, at: at + ms, kind: "stop" as const },
  ];
}

beforeEach(() => {
  h.loadAwards.mockReset().mockResolvedValue(file([]));
  // 확정 요청을 누적해 돌려주는 가짜 백엔드(파일 한 장 계약).
  const stored: AwardRecord[] = [];
  h.finalizeAward.mockReset().mockImplementation(async (rec: AwardRecord) => {
    stored.push(rec);
    return file(stored);
  });
  h.appendAwardSpeech.mockReset();
  h.loadSessionEvents.mockReset().mockResolvedValue([]);
  h.generateSpeech.mockReset();
  h.agents = {};
  useAwardsStore.setState({
    awards: [],
    loaded: false,
    finalizing: false,
    generating: {},
    error: undefined,
  });
});

describe("load", () => {
  it("시상 파일을 읽어 상태에 담는다", async () => {
    h.loadAwards.mockResolvedValue(file([record(back(1), "a1")]));
    await useAwardsStore.getState().load();
    expect(useAwardsStore.getState().awards).toHaveLength(1);
    expect(useAwardsStore.getState().loaded).toBe(true);
    expect(useAwardsStore.getState().error).toBeUndefined();
  });

  it("실패해도 던지지 않고 안내만 남긴다", async () => {
    h.loadAwards.mockRejectedValue(new Error("boom"));
    await useAwardsStore.getState().load();
    expect(useAwardsStore.getState().error).toBeTruthy();
    expect(useAwardsStore.getState().awards).toEqual([]);
  });
});

describe("ensureFinalized", () => {
  it("완료된 12개월만 오래된 순으로 확정한다(이번 달은 제외)", async () => {
    await useAwardsStore.getState().ensureFinalized(NOW);
    const months = h.finalizeAward.mock.calls.map((c) => (c[0] as AwardRecord).month);
    expect(months).toHaveLength(BACKFILL_MONTHS);
    expect(months[0]).toBe(back(BACKFILL_MONTHS));
    expect(months[months.length - 1]).toBe(back(1));
    // 진행 중인 이번 달은 절대 확정하지 않는다.
    expect(months).not.toContain(THIS_MONTH);
    // 오름차순(오래된 것부터)인지.
    expect(months).toEqual([...months].sort());
  });

  it("이미 레코드가 있는 달은 건너뛴다(write-once)", async () => {
    h.loadAwards.mockResolvedValue(file([record(back(1), "a1"), record(back(5))]));
    await useAwardsStore.getState().ensureFinalized(NOW);
    const months = h.finalizeAward.mock.calls.map((c) => (c[0] as AwardRecord).month);
    expect(months).toHaveLength(BACKFILL_MONTHS - 2);
    expect(months).not.toContain(back(1));
    expect(months).not.toContain(back(5));
  });

  it("빠진 달이 없으면 아무것도 하지 않는다", async () => {
    const all = Array.from({ length: BACKFILL_MONTHS }, (_, i) => record(back(i + 1)));
    h.loadAwards.mockResolvedValue(file(all));
    await useAwardsStore.getState().ensureFinalized(NOW);
    expect(h.finalizeAward).not.toHaveBeenCalled();
    expect(h.loadSessionEvents).not.toHaveBeenCalled();
  });

  it("12개월보다 오래된 달은 소급하지 않는다", async () => {
    await useAwardsStore.getState().ensureFinalized(NOW);
    const months = h.finalizeAward.mock.calls.map((c) => (c[0] as AwardRecord).month);
    expect(months).not.toContain(back(BACKFILL_MONTHS + 1));
    expect(months).not.toContain(back(24));
  });

  it("이벤트가 없는 달도 winner:null로 확정해 재계산을 막는다", async () => {
    await useAwardsStore.getState().ensureFinalized(NOW);
    for (const call of h.finalizeAward.mock.calls) {
      const rec = call[0] as AwardRecord;
      expect(rec.winner).toBeNull();
      expect(rec.leaderboard).toEqual([]);
      expect(rec.speeches).toEqual([]);
      expect(rec.decidedAt).toBe(NOW);
      expect(rec.rulesVersion).toBe(1);
      expect(call[1]).toBeUndefined(); // 초상 스냅샷 요청 없음
    }
    expect(useAwardsStore.getState().awards).toHaveLength(BACKFILL_MONTHS);
  });

  it("월 창을 lookback만큼 앞서 조회하고 aggregate에는 정확한 창을 쓴다", async () => {
    await useAwardsStore.getState().ensureFinalized(NOW);
    const target = back(BACKFILL_MONTHS);
    const range = monthRange(target, localDayCalendar);
    const [fromAt, toAt] = h.loadSessionEvents.mock.calls[0] as [number, number];
    expect(fromAt).toBe(range.fromAt - 24 * 60 * 60 * 1000);
    expect(toAt).toBe(range.toAt);
  });

  it("자격자가 있으면 수상자·순위표를 스냅샷하고 초상 대상 id를 넘긴다", async () => {
    const target = back(1);
    const { fromAt } = monthRange(target, localDayCalendar);
    const DAY = 86_400_000;
    const HOUR = 3_600_000;
    h.agents = {
      a1: profile("a1", { name: "컴파일러", role: "빌드 담당", archetype: "cat", portraitUpdatedAt: 7 }),
      a2: profile("a2", { name: "린터" }),
    };
    // a1: 3일 × 1시간, a2: 3일 × 30분 → 둘 다 임계를 넘고 a1이 앞선다.
    const events: SessionEventRecord[] = [];
    for (let d = 0; d < 3; d++) {
      events.push(...turnEvents("a1", fromAt + d * DAY + HOUR, HOUR, d * 10 + 1));
      events.push(
        ...turnEvents("a2", fromAt + d * DAY + HOUR, HOUR / 2, d * 10 + 5).map((e) => ({
          ...e,
          sessionId: "s2",
        })),
      );
    }
    h.loadSessionEvents.mockImplementation(async (from: number) =>
      // 대상 달의 조회에만 이벤트를 준다(나머지 달은 빈 달로 확정된다).
      from === fromAt - DAY ? events : [],
    );

    await useAwardsStore.getState().ensureFinalized(NOW);

    const call = h.finalizeAward.mock.calls.find((c) => (c[0] as AwardRecord).month === target);
    expect(call).toBeDefined();
    const rec = call![0] as AwardRecord;
    expect(rec.winner).toMatchObject({
      agentId: "a1",
      name: "컴파일러",
      role: "빌드 담당",
      archetype: "cat",
      hasPortrait: true,
    });
    expect(rec.winner?.stats.activeDays).toBe(3);
    expect(rec.winner?.stats.workedMs).toBe(3 * HOUR);
    expect(rec.leaderboard.map((s) => s.agentId)).toEqual(["a1", "a2"]);
    expect(call![1]).toBe("a1");
  });

  it("확정 도중 실패하면 안내를 남기고 finalizing을 내린다", async () => {
    h.finalizeAward.mockRejectedValue(new Error("boom"));
    await useAwardsStore.getState().ensureFinalized(NOW);
    expect(useAwardsStore.getState().error).toBeTruthy();
    expect(useAwardsStore.getState().finalizing).toBe(false);
  });

  it("이미 도는 중이면 두 번째 호출은 즉시 돌아온다", async () => {
    useAwardsStore.setState({ finalizing: true });
    await useAwardsStore.getState().ensureFinalized(NOW);
    expect(h.finalizeAward).not.toHaveBeenCalled();
  });
});

describe("provisionalWinner", () => {
  it("이번 달을 지금까지의 창으로 계산하고 저장하지 않는다", async () => {
    const r = await useAwardsStore.getState().provisionalWinner(NOW);
    expect(r?.month).toBe(THIS_MONTH);
    expect(r?.winner).toBeNull();
    const range = monthRange(THIS_MONTH, localDayCalendar);
    const [fromAt, toAt] = h.loadSessionEvents.mock.calls[0] as [number, number];
    expect(fromAt).toBe(range.fromAt - 24 * 60 * 60 * 1000);
    expect(toAt).toBe(NOW); // 미래까지 창을 열지 않는다
    expect(h.finalizeAward).not.toHaveBeenCalled();
    expect(useAwardsStore.getState().awards).toEqual([]);
  });
});

describe("generateSpeechFor", () => {
  const MONTH = back(1);

  beforeEach(() => {
    useAwardsStore.setState({ awards: [record(MONTH, "a1")], loaded: true });
    h.agents = { a1: profile("a1") };
  });

  it("성공하면 append 결과로 상태를 갱신한다", async () => {
    const speech = { at: 5, provider: "claude", text: "고맙습니다." };
    h.generateSpeech.mockResolvedValue({ ok: true, speech });
    const appended = { ...record(MONTH, "a1"), speeches: [speech] };
    h.appendAwardSpeech.mockResolvedValue(file([appended]));

    await useAwardsStore.getState().generateSpeechFor(MONTH);

    expect(h.appendAwardSpeech).toHaveBeenCalledWith(MONTH, speech);
    expect(useAwardsStore.getState().awards[0].speeches).toEqual([speech]);
    expect(useAwardsStore.getState().error).toBeUndefined();
    expect(useAwardsStore.getState().generating[MONTH]).toBeUndefined();
  });

  it("같은 달을 동시에 두 번 부르면 한 번만 생성한다", async () => {
    let release!: () => void;
    h.generateSpeech.mockReturnValue(
      new Promise((resolve) => {
        release = () => resolve({ ok: false, reason: "failed" });
      }),
    );

    const first = useAwardsStore.getState().generateSpeechFor(MONTH);
    expect(useAwardsStore.getState().generating[MONTH]).toBe(true);
    await useAwardsStore.getState().generateSpeechFor(MONTH); // 가드에 막혀 즉시 반환
    expect(h.generateSpeech).toHaveBeenCalledTimes(1);

    release();
    await first;
    expect(useAwardsStore.getState().generating[MONTH]).toBeUndefined();
    // 가드가 풀린 뒤에는 다시 부를 수 있다.
    h.generateSpeech.mockResolvedValue({ ok: false, reason: "failed" });
    await useAwardsStore.getState().generateSpeechFor(MONTH);
    expect(h.generateSpeech).toHaveBeenCalledTimes(2);
  });

  it("다른 달은 서로 막지 않는다", async () => {
    const other = back(2);
    useAwardsStore.setState({ awards: [record(MONTH, "a1"), record(other, "a1")] });
    h.generateSpeech.mockResolvedValue({ ok: false, reason: "failed" });
    await Promise.all([
      useAwardsStore.getState().generateSpeechFor(MONTH),
      useAwardsStore.getState().generateSpeechFor(other),
    ]);
    expect(h.generateSpeech).toHaveBeenCalledTimes(2);
  });

  it("통산 수상 횟수는 그 달 이전 것만 센다", async () => {
    useAwardsStore.setState({
      awards: [record(back(9), "a1"), record(back(5), "a1"), record(MONTH, "a1")],
    });
    h.generateSpeech.mockResolvedValue({ ok: false, reason: "failed" });
    await useAwardsStore.getState().generateSpeechFor(MONTH);
    expect(h.generateSpeech.mock.calls[0][2]).toBe(2);
  });

  it("실패 사유를 안내 문구로 바꿔 담고 append는 하지 않는다", async () => {
    h.generateSpeech.mockResolvedValue({ ok: false, reason: "cli-missing" });
    await useAwardsStore.getState().generateSpeechFor(MONTH);
    expect(useAwardsStore.getState().error).toContain("CLI");
    expect(h.appendAwardSpeech).not.toHaveBeenCalled();
  });

  it("그 달 레코드가 없으면 생성하지 않는다", async () => {
    await useAwardsStore.getState().generateSpeechFor("1999-01");
    expect(h.generateSpeech).not.toHaveBeenCalled();
    expect(useAwardsStore.getState().error).toBeTruthy();
  });
});

describe("awardCountFor / recordFor", () => {
  it("그 캐릭터가 받은 횟수를 센다(수상자 없는 달은 빼고)", () => {
    useAwardsStore.setState({
      awards: [record(back(3), "a1"), record(back(2)), record(back(1), "a2")],
    });
    expect(useAwardsStore.getState().awardCountFor("a1")).toBe(1);
    expect(useAwardsStore.getState().awardCountFor("a2")).toBe(1);
    expect(useAwardsStore.getState().awardCountFor("nope")).toBe(0);
  });

  it("recordFor는 그 달 레코드를 돌려준다", () => {
    useAwardsStore.setState({ awards: [record(back(1), "a1")] });
    expect(useAwardsStore.getState().recordFor(back(1))?.winner?.agentId).toBe("a1");
    expect(useAwardsStore.getState().recordFor("1999-01")).toBeUndefined();
  });
});
