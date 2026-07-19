// src/renderer/diary/__tests__/diaryGenerator.test.ts
//
// 일기 생성기: opt-in 게이트, 작업 로그 없음 스킵, 성격 문체 주입, 성공 시
// append+세션 로그 소진, CLI 미설치/실패 조용한 폴백, 인플라이트 중복 방지.
// summarizeFn/appendFn/시계/버퍼 전부 주입.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    appendSessionTurn: vi.fn(),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import { DIARY_SYSTEM_PROMPT, generateDiary, sanitizeDiaryBody } from "../diaryGenerator";
import { WorkLog } from "../workLog";
import type { AgentProfile, AppSettings } from "@shared/types";

const SETTINGS_ON: AppSettings = {
  version: 1,
  summarizerEnabled: false,
  summaryProvider: "claude",
  diaryEnabled: true,
  observerEnabled: false,
  soundEnabled: true,
  soundVolume: 0.5,
  externalTerminal: "terminal",
  externalEditor: "system",
  attentionHoldMs: 5000,
  gitStatusEnabled: true,
  cliEnabled: false,
};

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "컴파일러",
    role: "빌드 담당",
    note: "",
    seed: "x",
    createdAt: 0,
    deskIndex: 0,
    ...overrides,
  };
}

function seededLog(): WorkLog {
  const log = new WorkLog();
  log.append("a1", { at: 1, sessionId: "s1", kind: "prompt", text: "이슈 56 해줘", goal: "일기 기능" });
  log.append("a1", { at: 2, sessionId: "s1", kind: "tool", text: "Bash: cargo test" });
  return log;
}

beforeEach(() => {
  useAppStore.setState({
    taskLabels: { a1: { sessionId: "s1" } },
    agents: { a1: profile() },
  });
  useAppStore.getState().hydrateSettings(SETTINGS_ON, false);
});

describe("sanitizeDiaryBody", () => {
  it("코드펜스·머리말을 제거하고 본문만 남긴다", () => {
    expect(sanitizeDiaryBody("```\n오늘은 일했다.\n```")).toBe("오늘은 일했다.");
  });
  it("너무 짧으면 null", () => {
    expect(sanitizeDiaryBody("  ")).toBeNull();
    expect(sanitizeDiaryBody("응")).toBeNull();
  });
});

describe("generateDiary", () => {
  it("opt-in OFF면 CLI를 호출하지 않고 disabled를 반환한다", async () => {
    useAppStore.getState().hydrateSettings({ ...SETTINGS_ON, diaryEnabled: false }, false);
    const summarizeFn = vi.fn();
    const result = await generateDiary("a1", { summarizeFn, log: seededLog() });
    expect(result).toEqual({ ok: false, reason: "disabled" });
    expect(summarizeFn).not.toHaveBeenCalled();
  });

  it("작업 로그가 없으면 no-work를 반환한다", async () => {
    const summarizeFn = vi.fn();
    const result = await generateDiary("a1", { summarizeFn, log: new WorkLog() });
    expect(result).toEqual({ ok: false, reason: "no-work" });
    expect(summarizeFn).not.toHaveBeenCalled();
  });

  it("성격을 문체로 주입하고 작업 로그를 담아 호출한다", async () => {
    useAppStore.setState({ agents: { a1: profile({ personalityPrompt: "명랑한 초등학생" }) } });
    const summarizeFn = vi.fn().mockResolvedValue("오늘은 이슈를 고쳤다! 신난다!");
    const appendFn = vi.fn().mockResolvedValue(undefined);
    const result = await generateDiary("a1", {
      summarizeFn,
      appendFn,
      now: () => 12345,
      log: seededLog(),
    });

    expect(result.ok).toBe(true);
    const [provider, instruction, text] = summarizeFn.mock.calls[0];
    expect(provider).toBe("claude"); // summaryProvider 공유
    expect(instruction).toBe(DIARY_SYSTEM_PROMPT);
    expect(text).toContain("명랑한 초등학생");
    expect(text).toContain("이슈 56 해줘");
    expect(text).toContain("Bash: cargo test");
    expect(appendFn).toHaveBeenCalledWith("a1", {
      at: 12345,
      sessionId: "s1",
      body: "오늘은 이슈를 고쳤다! 신난다!",
    });
  });

  it("성격 미설정이면 (없음)으로 담백하게 간다", async () => {
    const summarizeFn = vi.fn().mockResolvedValue("작업을 완료했다.");
    await generateDiary("a1", { summarizeFn, appendFn: vi.fn().mockResolvedValue(undefined), log: seededLog() });
    expect(summarizeFn.mock.calls[0][2]).toContain("[성격]\n(없음)");
  });

  it("성공하면 그 세션의 작업 로그를 소진한다", async () => {
    const log = seededLog();
    await generateDiary("a1", {
      summarizeFn: vi.fn().mockResolvedValue("완료했다."),
      appendFn: vi.fn().mockResolvedValue(undefined),
      log,
    });
    expect(log.items("a1", "s1")).toHaveLength(0);
  });

  it("CLI 미설치는 조용히 cli-missing 폴백(로그 유지)", async () => {
    const log = seededLog();
    const result = await generateDiary("a1", {
      summarizeFn: vi.fn().mockRejectedValue(new Error("claude-not-found")),
      appendFn: vi.fn(),
      log,
    });
    expect(result).toEqual({ ok: false, reason: "cli-missing" });
    expect(log.items("a1")).toHaveLength(2); // 소진 안 함 — 다음에 재시도 가능
  });

  it("빈/깨진 응답은 failed(로그 유지)", async () => {
    const log = seededLog();
    const result = await generateDiary("a1", {
      summarizeFn: vi.fn().mockResolvedValue("   "),
      appendFn: vi.fn(),
      log,
    });
    expect(result).toEqual({ ok: false, reason: "failed" });
    expect(log.items("a1")).toHaveLength(2);
  });

  it("targetSessionId를 주면 그 세션 로그만 담아 기록하고 그 세션만 소진한다", async () => {
    const log = new WorkLog();
    log.append("a1", { at: 1, sessionId: "s1", kind: "prompt", text: "이전 세션 작업" });
    log.append("a1", { at: 2, sessionId: "s2", kind: "prompt", text: "종료된 세션 작업" });
    const summarizeFn = vi.fn().mockResolvedValue("종료된 세션을 정리했다.");
    const appendFn = vi.fn().mockResolvedValue(undefined);
    const result = await generateDiary("a1", { summarizeFn, appendFn, now: () => 9, log }, "s2");

    expect(result.ok).toBe(true);
    // s2 로그만 프롬프트에 담긴다.
    expect(summarizeFn.mock.calls[0][2]).toContain("종료된 세션 작업");
    expect(summarizeFn.mock.calls[0][2]).not.toContain("이전 세션 작업");
    expect(appendFn).toHaveBeenCalledWith("a1", { at: 9, sessionId: "s2", body: "종료된 세션을 정리했다." });
    // s2만 소진, s1은 남는다.
    expect(log.items("a1", "s2")).toHaveLength(0);
    expect(log.items("a1", "s1")).toHaveLength(1);
  });

  it("생성이 진행 중이면 두 번째 호출은 in-flight로 거절한다", async () => {
    const log = seededLog();
    let release!: () => void;
    const gate = new Promise<string>((r) => (release = () => r("완료했다.")));
    const summarizeFn = vi.fn().mockReturnValue(gate);
    const appendFn = vi.fn().mockResolvedValue(undefined);

    const first = generateDiary("a1", { summarizeFn, appendFn, log });
    const second = await generateDiary("a1", { summarizeFn, appendFn, log });
    expect(second).toEqual({ ok: false, reason: "in-flight" });
    release();
    expect((await first).ok).toBe(true);
    expect(summarizeFn).toHaveBeenCalledTimes(1);
  });
});
