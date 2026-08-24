// src/renderer/awards/__tests__/speechGenerator.test.ts
//
// 수상 소감 생성기: 일기 발췌의 월 필터·균등 샘플링·절단·예산, 일기 0편 경로,
// 프로필 없음, 요약기 실패 사유 매핑(diaryGenerator와 같은 어휘). summarize·
// loadDiary·시계·provider·게이트를 전부 주입해 스토어/IPC 없이 검증한다.
import { beforeEach, describe, expect, it, vi } from "vitest";

// appStore를 거쳐 들어오는 tauriApi 모듈을 차단한다(테스트는 전부 주입으로 돈다).
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { summarizeText: vi.fn(), loadDiary: vi.fn(), setAppSettings: vi.fn() },
}));

import { fixedOffsetCalendar } from "../../analytics/aggregate";
import {
  AWARD_SPEECH_SYSTEM_PROMPT,
  buildDiaryExcerpt,
  generateSpeech,
  type SpeechDeps,
} from "../speechGenerator";
import type { AgentProfile, AwardRecord, DiaryEntry } from "@shared/types";

const KST = fixedOffsetCalendar(540);

/** 2026-07-`day` 12:00 KST. */
function julyAt(day: number): number {
  return Date.UTC(2026, 6, day, 3, 0, 0, 0);
}

function entry(at: number, body: string): DiaryEntry {
  return { at, sessionId: "s1", body };
}

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "컴파일러",
    role: "빌드 담당",
    seed: "x",
    createdAt: 0,
    deskIndex: 0,
    ...overrides,
  };
}

function record(overrides: Partial<AwardRecord> = {}): AwardRecord {
  return {
    month: "2026-07",
    decidedAt: 0,
    rulesVersion: 1,
    winner: {
      agentId: "a1",
      name: "컴파일러",
      role: "빌드 담당",
      hasPortrait: false,
      stats: {
        workedMs: 12 * 3_600_000,
        turns: 84,
        toolEvents: 300,
        activeDays: 15,
        tokensIn: 1000,
        tokensOut: 2000,
        costUsd: 1.5,
      },
    },
    leaderboard: [],
    speeches: [],
    ...overrides,
  };
}

let summarize: ReturnType<typeof vi.fn>;
let loadDiary: ReturnType<typeof vi.fn>;

function deps(extra: SpeechDeps = {}): SpeechDeps {
  return {
    summarizeFn: summarize as unknown as SpeechDeps["summarizeFn"],
    loadDiaryFn: loadDiary as unknown as SpeechDeps["loadDiaryFn"],
    provider: "claude",
    enabled: true,
    cal: KST,
    now: () => 1_700_000_000_000,
    ...extra,
  };
}

beforeEach(() => {
  summarize = vi.fn().mockResolvedValue("오늘도 무사히 빌드를 넘겼습니다. 다들 고맙습니다.");
  loadDiary = vi.fn().mockResolvedValue([]);
});

describe("buildDiaryExcerpt", () => {
  it("그 달에 속한 일기만 담는다", () => {
    const entries = [
      entry(Date.UTC(2026, 5, 30, 3), "6월 마지막날"),
      entry(julyAt(5), "7월 초"),
      entry(julyAt(28), "7월 말"),
      entry(Date.UTC(2026, 7, 1, 3), "8월 첫날"),
    ];
    const out = buildDiaryExcerpt(entries, "2026-07", KST);
    expect(out).toContain("7월 초");
    expect(out).toContain("7월 말");
    expect(out).not.toContain("6월 마지막날");
    expect(out).not.toContain("8월 첫날");
  });

  it("경계(로컬 자정) 판정은 캘린더를 따른다", () => {
    // 2026-07-01 00:00 KST = 06-30T15:00Z (7월), 그 1ms 전은 6월.
    const boundary = Date.UTC(2026, 5, 30, 15, 0, 0, 0);
    const out = buildDiaryExcerpt(
      [entry(boundary - 1, "6월분"), entry(boundary, "7월분")],
      "2026-07",
      KST,
    );
    expect(out).toContain("7월분");
    expect(out).not.toContain("6월분");
  });

  it("한도 이하면 전부 담고 시간순으로 정렬한다", () => {
    const entries = [entry(julyAt(9), "나중"), entry(julyAt(2), "먼저")];
    const out = buildDiaryExcerpt(entries, "2026-07", KST);
    expect(out.split("\n")).toHaveLength(2);
    expect(out.indexOf("먼저")).toBeLessThan(out.indexOf("나중"));
  });

  it("한도를 넘으면 양끝을 포함해 균등 간격으로 뽑는다(최신 우선 아님)", () => {
    // 31편 → 10편으로. 인덱스 0,3,7,10,13,17,20,23,27,30.
    const entries = Array.from({ length: 31 }, (_, i) => entry(julyAt(i + 1), `일기${i}`));
    const out = buildDiaryExcerpt(entries, "2026-07", KST, {
      maxEntries: 10,
      perEntryChars: 300,
      totalChars: 8000,
    });
    const lines = out.split("\n");
    expect(lines).toHaveLength(10);
    // 첫 편과 마지막 편이 반드시 들어간다 — 월 전체 흐름을 담는 것이 목적.
    expect(lines[0]).toContain("일기0");
    expect(lines[9]).toContain("일기30");
    // 최신 10편만 뽑은 것이 아님을 확인(월초 일기가 살아 있다).
    expect(out).not.toContain("일기29");
  });

  it("기본 한도는 20편·편당 300자다", () => {
    const long = "가".repeat(500);
    const entries = Array.from({ length: 25 }, (_, i) => entry(julyAt(i + 1), `${i}:${long}`));
    const out = buildDiaryExcerpt(entries, "2026-07", KST);
    const lines = out.split("\n");
    expect(lines).toHaveLength(20);
    for (const line of lines) {
      expect(line.endsWith("…")).toBe(true);
      // "- YYYY-MM-DD: " 접두 14자 + 본문 300자 + "…" 1자.
      expect(Array.from(line)).toHaveLength(14 + 300 + 1);
    }
  });

  it("짧은 일기는 자르지 않고 …도 붙이지 않는다", () => {
    const out = buildDiaryExcerpt([entry(julyAt(3), "짧다")], "2026-07", KST);
    expect(out).toBe("- 2026-07-03: 짧다");
  });

  it("총 예산을 넘기 직전에 멈춘다", () => {
    const entries = Array.from({ length: 10 }, (_, i) => entry(julyAt(i + 1), "가".repeat(50)));
    // 한 줄 = 14 + 50 = 64자, 줄바꿈 1자. 200자 예산이면 3줄(64+65+65=194)에서 멈춘다.
    const out = buildDiaryExcerpt(entries, "2026-07", KST, {
      maxEntries: 100,
      perEntryChars: 300,
      totalChars: 200,
    });
    expect(out.split("\n")).toHaveLength(3);
    expect(Array.from(out).length).toBeLessThanOrEqual(200);
  });

  it("그 달 일기가 없으면 (일기 없음)", () => {
    expect(buildDiaryExcerpt([], "2026-07", KST)).toBe("(일기 없음)");
    expect(buildDiaryExcerpt([entry(julyAt(3), "7월")], "2026-08", KST)).toBe("(일기 없음)");
  });
});

describe("generateSpeech", () => {
  it("성공하면 provider·시각·정제된 본문을 담은 소감을 돌려준다", async () => {
    summarize.mockResolvedValue("```\n  잘 부탁드립니다. 다음 달도 열심히 하겠습니다.  \n```");
    const r = await generateSpeech(record(), profile(), 2, deps());
    expect(r).toEqual({
      ok: true,
      speech: {
        at: 1_700_000_000_000,
        provider: "claude",
        text: "잘 부탁드립니다. 다음 달도 열심히 하겠습니다.",
      },
    });
  });

  it("시스템 프롬프트와 성격·수상 정보·일기 발췌를 조립해 넘긴다", async () => {
    loadDiary.mockResolvedValue([entry(julyAt(4), "리팩터링을 했다")]);
    await generateSpeech(record(), profile({ personalityPrompt: "무뚝뚝함" }), 2, deps());
    expect(summarize).toHaveBeenCalledTimes(1);
    const [provider, instruction, text] = summarize.mock.calls[0] as [string, string, string];
    expect(provider).toBe("claude");
    expect(instruction).toBe(AWARD_SPEECH_SYSTEM_PROMPT);
    expect(text).toContain("[성격]\n무뚝뚝함");
    expect(text).toContain("월: 2026-07");
    expect(text).toContain("작업 시간: 약 12시간"); // 12h → 시간 단위 반올림
    expect(text).toContain("턴 수: 84");
    expect(text).toContain("활동일: 15일");
    expect(text).toContain("통산 수상: 3회(이번 포함)"); // prior 2 + 이번
    expect(text).toContain("리팩터링을 했다");
    expect(loadDiary).toHaveBeenCalledWith("a1");
  });

  it("성격이 비면 (없음)으로 채운다", async () => {
    await generateSpeech(record(), profile({ personalityPrompt: "   " }), 0, deps());
    const text = summarize.mock.calls[0][1 + 1] as string;
    expect(text).toContain("[성격]\n(없음)");
  });

  it("일기가 한 편도 없으면 (일기 없음)으로 진행한다", async () => {
    loadDiary.mockResolvedValue([]);
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r.ok).toBe(true);
    expect(summarize.mock.calls[0][2] as string).toContain("[지난달 일기]\n(일기 없음)");
  });

  it("일기 로드가 실패해도 통계만으로 진행한다", async () => {
    loadDiary.mockRejectedValue(new Error("boom"));
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r.ok).toBe(true);
    expect(summarize.mock.calls[0][2] as string).toContain("(일기 없음)");
  });

  it("프로필이 없으면 profile-missing", async () => {
    const r = await generateSpeech(record(), undefined, 0, deps());
    expect(r).toEqual({ ok: false, reason: "profile-missing" });
    expect(summarize).not.toHaveBeenCalled();
  });

  it("수상자가 없는 달이면 no-winner", async () => {
    const r = await generateSpeech(record({ winner: null }), profile(), 0, deps());
    expect(r).toEqual({ ok: false, reason: "no-winner" });
    expect(summarize).not.toHaveBeenCalled();
  });

  it("요약기가 꺼져 있으면 호출조차 하지 않는다", async () => {
    const r = await generateSpeech(record(), profile(), 0, deps({ enabled: false }));
    expect(r).toEqual({ ok: false, reason: "disabled" });
    expect(summarize).not.toHaveBeenCalled();
  });

  it("CLI 미설치는 cli-missing", async () => {
    summarize.mockRejectedValue(new Error("claude-not-found"));
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r).toEqual({ ok: false, reason: "cli-missing" });
  });

  it("정확히 timeout인 실패만 timeout으로 본다", async () => {
    summarize.mockRejectedValue(new Error("timeout"));
    expect(await generateSpeech(record(), profile(), 0, deps())).toEqual({
      ok: false,
      reason: "timeout",
    });
    // stderr에 timeout이 섞인 exit 에러는 오분류하지 않는다.
    summarize.mockRejectedValue(new Error("claude exited 1: ... timeout ..."));
    expect(await generateSpeech(record(), profile(), 0, deps())).toEqual({
      ok: false,
      reason: "failed",
    });
  });

  it("백엔드 summarizer-disabled는 disabled로 매핑한다", async () => {
    summarize.mockRejectedValue(new Error("summarizer-disabled"));
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r).toEqual({ ok: false, reason: "disabled" });
  });

  it("빈 응답은 failed", async () => {
    summarize.mockResolvedValue("```\n\n```");
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r).toEqual({ ok: false, reason: "failed" });
  });
});
