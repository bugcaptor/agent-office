// src/renderer/awards/__tests__/speechGenerator.test.ts
//
// 수상 소감 생성기: 일기 발췌의 월 필터·균등 샘플링·절단·예산, 일기 0편 경로,
// 프로필 없음, 요약기 실패 사유 매핑(diaryGenerator와 같은 어휘). summarize·
// loadDiary·시계·provider·게이트를 전부 주입해 스토어/IPC 없이 검증한다.
import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";

// appStore를 거쳐 들어오는 tauriApi 모듈을 차단한다(테스트는 전부 주입으로 돈다).
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { summarizeText: vi.fn(), loadDiary: vi.fn(), setAppSettings: vi.fn() },
}));

import { SOURCE_LANGUAGE, initI18nForTest } from "@renderer/i18n";
import { fixedOffsetCalendar } from "../../analytics/aggregate";
import { speechPromptProfile } from "../../i18n/promptProfiles";
import {
  buildDiaryExcerpt,
  clampSpeech,
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

  it("기본 한도는 예산(1,500자)에서 역산한 10편·편당 135자다", () => {
    // 편수를 위에서부터 시도해 예산에 실제로 들어맞는 첫 편수를 쓴다.
    // 10편 × (접두 14 + 절단본 135) + 개행 9 = 1,499자 ≤ 1,500.
    const long = "가".repeat(500);
    const entries = Array.from({ length: 25 }, (_, i) => entry(julyAt(i + 1), `${i}:${long}`));
    const out = buildDiaryExcerpt(entries, "2026-07", KST);
    const lines = out.split("\n");
    expect(lines).toHaveLength(10);
    for (const line of lines) {
      expect(line.endsWith("…")).toBe(true);
      expect(Array.from(line)).toHaveLength(14 + 135);
    }
    expect(Array.from(out).length).toBeLessThanOrEqual(speechPromptProfile().excerptLimits.totalChars);
  });

  it("문장부호가 있으면 문장 경계에서 끊는다(중간이 뚝 끊기지 않게)", () => {
    const body = `${"가".repeat(80)}. ${"나".repeat(80)}. ${"다".repeat(80)}.`;
    const out = buildDiaryExcerpt([entry(julyAt(3), body)], "2026-07", KST, {
      maxEntries: 1,
      perEntryChars: 100,
      totalChars: 1_000,
    });
    expect(out.endsWith("…")).toBe(false);
    expect(out.endsWith(".")).toBe(true);
    expect(out).not.toContain("나");
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
    expect(instruction).toBe(speechPromptProfile().systemPrompt);
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

describe("clampSpeech", () => {
  it("문장 수 상한을 넘으면 앞에서부터 남기고 뒤를 버린다", () => {
    const out = clampSpeech("하나. 둘! 셋? 넷. 다섯.");
    expect(out).toBe("하나. 둘! 셋? 넷.");
  });

  it("글자 상한을 넘으면 문장 단위로 버려 끝을 깔끔하게 남긴다", () => {
    const a = `${"가".repeat(150)}.`;
    const b = `${"나".repeat(150)}.`;
    const out = clampSpeech(`${a} ${b}`);
    expect(out).toBe(a);
    expect(Array.from(out).length).toBeLessThanOrEqual(speechPromptProfile().speechMaxChars);
  });

  it("한 문장이 통째로 상한을 넘으면 마지막 수단으로 글자 절단한다", () => {
    const out = clampSpeech("가".repeat(400));
    expect(Array.from(out).length).toBe(speechPromptProfile().speechMaxChars); // `…` 포함 상한 이하
    expect(out.endsWith("…")).toBe(true);
  });

  it("연속 부호는 한 문장으로 묶고 개행도 문장 경계로 본다", () => {
    expect(clampSpeech("정말요?! 네.\n감사합니다.")).toBe("정말요?! 네. 감사합니다.");
  });

  it("상한 안이면 그대로 둔다", () => {
    expect(clampSpeech("고맙습니다. 다음 달도 잘 하겠습니다.")).toBe(
      "고맙습니다. 다음 달도 잘 하겠습니다.",
    );
  });
});

describe("프롬프트 예산", () => {
  it("일기가 아무리 많아도 프롬프트 총량이 백엔드 상한 아래로 유지된다", async () => {
    // 백엔드 cap_text(2,000자)의 head/tail 중략에 걸리면 월 가운데가 통째로
    // 날아간다 — 프런트에서 미리 잘라 그 절단이 발동하지 않게 한다.
    const entries = Array.from({ length: 60 }, (_, i) =>
      entry(julyAt((i % 28) + 1), "가".repeat(1_000)),
    );
    loadDiary.mockResolvedValue(entries);
    await generateSpeech(record(), profile({ personalityPrompt: "무뚝뚝함" }), 0, deps());
    const text = summarize.mock.calls[0][2] as string;
    expect(Array.from(text).length).toBeLessThanOrEqual(speechPromptProfile().promptBudgetChars);
  });

  it("성격이 길면 잘라 일기 발췌 예산을 지킨다", async () => {
    loadDiary.mockResolvedValue([entry(julyAt(4), "리팩터링을 했다")]);
    await generateSpeech(record(), profile({ personalityPrompt: "성".repeat(900) }), 0, deps());
    const text = summarize.mock.calls[0][2] as string;
    const personality = text.split("\n\n")[0].replace("[성격]\n", "");
    expect(Array.from(personality).length).toBeLessThanOrEqual(speechPromptProfile().personalityMaxChars);
    // 성격이 예산을 다 먹지 않아 일기는 그대로 들어간다.
    expect(text).toContain("리팩터링을 했다");
  });

  it("출력도 상한을 넘으면 잘라서 저장한다", async () => {
    summarize.mockResolvedValue(`${"가".repeat(300)}. ${"나".repeat(300)}.`);
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r.ok).toBe(true);
    if (r.ok) expect(Array.from(r.speech.text).length).toBeLessThanOrEqual(speechPromptProfile().speechMaxChars);
  });
});

// en 프로필 — 프롬프트·머리말·sentinel·출력 상한이 UI 언어를 따르는지.
describe("수상 소감(en 프로필)", () => {
  beforeEach(async () => {
    await initI18nForTest("en");
  });

  afterAll(async () => {
    await initI18nForTest(SOURCE_LANGUAGE); // 정본 복구
  });

  it("en 프롬프트·머리말로 조립하고 수상 정보도 영어로 낸다", async () => {
    loadDiary.mockResolvedValue([entry(julyAt(4), "Refactored the build script.")]);
    summarize.mockResolvedValue("Thanks, everyone. The build held up all month.");
    const r = await generateSpeech(record(), profile({ personalityPrompt: "terse" }), 2, deps());

    expect(r.ok).toBe(true);
    const p = speechPromptProfile("en");
    const [, instruction, text] = summarize.mock.calls[0] as [string, string, string];
    expect(instruction).toBe(p.systemPrompt);
    expect(text).toContain(`${p.headers.personality}\nterse`);
    expect(text).toContain("Month: 2026-07");
    expect(text).toContain("Time worked: about 12 hours");
    expect(text).toContain("Turns: 84");
    expect(text).toContain("Active days: 15");
    expect(text).toContain("Awards to date: 3 (including this one)");
    expect(text).toContain("Refactored the build script.");
  });

  it("일기가 없으면 en sentinel을 넣는다", async () => {
    loadDiary.mockResolvedValue([]);
    const p = speechPromptProfile("en");
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r.ok).toBe(true);
    expect(summarize.mock.calls[0][2] as string).toContain(`${p.headers.diary}\n${p.noDiaryText}`);
    expect(p.noDiaryText).toBe("(no diary)");
  });

  it("출력 상한이 en 값(ko보다 큼)이라 ko라면 잘렸을 소감도 남는다", async () => {
    const ko = speechPromptProfile("ko").speechMaxChars;
    const en = speechPromptProfile("en").speechMaxChars;
    expect(en).toBeGreaterThan(ko);
    // ko 상한은 넘지만 en 상한 안에 드는 두 문장.
    const long = `${"a".repeat(ko - 20)}. ${"b".repeat(30)}.`;
    summarize.mockResolvedValue(long);
    const r = await generateSpeech(record(), profile(), 0, deps());
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(Array.from(r.speech.text).length).toBeGreaterThan(ko);
      expect(Array.from(r.speech.text).length).toBeLessThanOrEqual(en);
    }
  });
});
