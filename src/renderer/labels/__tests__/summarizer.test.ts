// src/renderer/labels/__tests__/summarizer.test.ts
//
// 요약기: 첫 프롬프트 → goal+current 2호출, 캐시, stale 폐기,
// claude-not-found 영구 비활성, 실패 백오프. summarizeFn/시계 전부 주입.
import { beforeEach, describe, expect, it, vi } from "vitest";

// 연속 prompt는 이전 턴을 정산하므로 appendSessionTurn이 호출된다 — 실
// tauriApi(invoke)를 타지 않도록 모킹(다른 시간추적 테스트와 동일 컨벤션).
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    appendSessionTurn: vi.fn(),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import {
  CURRENT_SYSTEM_PROMPT,
  GOAL_SYSTEM_PROMPT,
  installTaskLabelSummarizer,
  sanitizeSummary,
} from "../summarizer";
import type { ActivityEvent, SummaryProvider } from "@shared/types";

function promptEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000, text: "버그 고쳐줘", ...overrides };
}

function deferred(): { promise: Promise<void>; release: () => void } {
  let release!: () => void;
  const promise = new Promise<void>((resolve) => (release = resolve));
  return { promise, release };
}

let teardown: (() => void) | null = null;

beforeEach(() => {
  teardown?.();
  teardown = null;
  useAppStore.setState({ taskLabels: {}, timeTracking: {} });
  // 기존 테스트들은 CLI ON을 전제 — opt-in 게이트 도입 이후 명시적으로 켜준다.
  useAppStore.getState().hydrateSettings(
    {
      version: 1,
      summarizerEnabled: true,
      summaryProvider: "claude",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
    },
    false
  );
});

describe("installTaskLabelSummarizer", () => {
  it("첫 프롬프트에 goal/current 요약을 병렬 요청해 store에 반영한다", async () => {
    const summarizeFn = vi.fn(async (_provider: SummaryProvider, instruction: string, _text: string) => {
      return instruction === GOAL_SYSTEM_PROMPT ? "버그 수정" : "버그 고치는 중";
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());

    await vi.waitFor(() => {
      const l = useAppStore.getState().taskLabels["a1"];
      expect(l.goal).toBe("버그 수정");
      expect(l.currentSummary).toBe("버그 고치는 중");
    });
    expect(summarizeFn).toHaveBeenCalledTimes(2);
    const [provider, instruction, text] = summarizeFn.mock.calls[0];
    expect(provider).toBe("claude");
    expect(instruction === GOAL_SYSTEM_PROMPT || instruction === CURRENT_SYSTEM_PROMPT).toBe(true);
    expect(text).toBe("버그 고쳐줘");
  });

  it.each(["claude", "codex"] as const)(
    "%s provider도 첫 프롬프트에 정확히 두 번 호출한다",
    async (provider) => {
      useAppStore.getState().hydrateSettings(
        {
          version: 1,
          summarizerEnabled: true,
          summaryProvider: provider,
          observerEnabled: false,
          soundEnabled: true,
          soundVolume: 0.5,
          externalTerminal: "terminal",
          externalEditor: "system",
          attentionHoldMs: 5000,
        },
        false,
      );
      const summarizeFn = vi.fn(async (_provider: SummaryProvider, instruction: string) =>
        instruction === GOAL_SYSTEM_PROMPT ? "목표" : "현재",
      );
      teardown = installTaskLabelSummarizer({ summarizeFn });
      useAppStore.getState().applyActivityEvent(promptEvent());
      await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
      expect(summarizeFn.mock.calls.every(([seen]) => seen === provider)).toBe(true);
    },
  );

  it("provider 변경만으로 기존 라벨을 재요약하지 않고 다음 프롬프트부터 적용한다", async () => {
    const summarizeFn = vi.fn(
      async (provider: SummaryProvider, _instruction: string, _text: string) =>
        `${provider} 요약`,
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    await Promise.resolve();
    expect(summarizeFn).toHaveBeenCalledTimes(2);

    useAppStore.getState().applyActivityEvent(promptEvent({ text: "다음 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(
        summarizeFn.mock.calls.some(
          ([provider, , text]) => provider === "codex" && text === "다음 지시",
        ),
      ).toBe(true),
    );
  });

  it("진행 중인 동일 identity는 provider 변경 후 sibling 완료 sweep에서도 중복 요청하지 않는다", async () => {
    const goalGate = deferred();
    const currentGate = deferred();
    const summarizeFn = vi.fn(
      async (provider: SummaryProvider, instruction: string) => {
        const isGoal = instruction === GOAL_SYSTEM_PROMPT;
        if (provider === "claude") {
          await (isGoal ? goalGate.promise : currentGate.promise);
          return isGoal ? "Claude 목표" : "Claude 현재";
        }
        return isGoal ? "Codex 목표" : "Codex 현재";
      },
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });

    goalGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.goal).toBe("Claude 목표"),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect.soft(summarizeFn).toHaveBeenCalledTimes(2);
    expect.soft(summarizeFn.mock.calls.some(([provider]) => provider === "codex")).toBe(false);

    currentGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Claude 현재"),
    );
    expect(summarizeFn).toHaveBeenCalledTimes(2);
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "claude")).toBe(true);
  });

  it("Claude cache는 같은 원문의 새 Codex identity를 충족하지 않는다", async () => {
    const summarizeFn = vi.fn(
      async (provider: SummaryProvider, instruction: string) =>
        instruction === GOAL_SYSTEM_PROMPT ? `${provider} 목표` : `${provider} 현재`,
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("claude 현재"),
    );
    expect(summarizeFn).toHaveBeenCalledTimes(2);

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("codex 현재"),
    );

    expect(summarizeFn).toHaveBeenCalledTimes(3);
    expect(summarizeFn.mock.calls[2]).toEqual([
      "codex",
      CURRENT_SYSTEM_PROMPT,
      "버그 고쳐줘",
    ]);
  });

  it("같은 세션·원문의 더 최신 prompt는 이전 결과를 거부하고 새 provider 결과만 적용한다", async () => {
    const oldCurrentGate = deferred();
    const newCurrentGate = deferred();
    const summarizeFn = vi.fn(
      async (provider: SummaryProvider, instruction: string) => {
        if (instruction === GOAL_SYSTEM_PROMPT) return `${provider} 목표`;
        await (provider === "claude" ? oldCurrentGate.promise : newCurrentGate.promise);
        return provider === "claude" ? "Claude 낡은 현재" : "Codex 새 현재";
      },
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent({ at: 1000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ at: 2000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(3));

    oldCurrentGate.release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(useAppStore.getState().taskLabels.a1.latestPromptAt).toBe(2000);
    expect(useAppStore.getState().taskLabels.a1.currentSummary).toBeUndefined();

    newCurrentGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 새 현재"),
    );
    expect(
      summarizeFn.mock.calls.some(
        ([provider, instruction]) =>
          provider === "codex" && instruction === CURRENT_SYSTEM_PROMPT,
      ),
    ).toBe(true);
  });

  it("한 provider 미설치는 다른 provider를 비활성화하지 않는다", async () => {
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      if (provider === "claude") throw new Error("claude-not-found");
      return "Codex 요약";
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "claude")).toBe(true);

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "Codex 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 요약"),
    );
  });

  it("Codex 미설치 latch도 Claude와 분리된다", async () => {
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      if (provider === "codex") throw new Error("codex-not-found");
      return "Claude 요약";
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "codex")).toBe(true);

    useAppStore.getState().updateAppSettings({ summaryProvider: "claude" });
    useAppStore.getState().applyActivityEvent(
      promptEvent({ text: "Claude 지시", at: 2000 }),
    );
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Claude 요약"),
    );
  });

  it("같은 원문은 캐시로 재호출 없이 반영한다 (세션 재시작 후 동일 첫 프롬프트)", async () => {
    const summarizeFn = vi.fn(async () => "요약");
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels["a1"].goal).toBe("요약"));
    const callsAfterFirst = summarizeFn.mock.calls.length; // goal+current = 2

    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s2", at: 2000 })); // 같은 텍스트, 새 세션
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels["a1"].goal).toBe("요약"));
    expect(summarizeFn.mock.calls.length).toBe(callsAfterFirst); // 캐시 히트, 추가 호출 없음
  });

  it("응답 도착 전에 최신 프롬프트가 바뀌면 stale 결과를 폐기한다", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const summarizeFn = vi.fn(
      async (_provider: SummaryProvider, instruction: string, text: string) => {
        if (text === "버그 고쳐줘" && instruction === CURRENT_SYSTEM_PROMPT) {
          await gate; // 첫 current 요약을 붙잡아 둔다
          return "낡은 요약";
        }
        return "정상 요약";
      },
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "새 지시", at: 2000 })); // gate 해제 전에 교체
    release();

    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("정상 요약")
    );
    // 낡은 요약이 최신 요약을 덮어쓰지 않았음을 재확인
    await new Promise((r) => setTimeout(r, 0));
    expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("정상 요약");
  });

  it("실패 시 해당 agent는 30초 쿨다운, 이후 재시도한다", async () => {
    let now = 0;
    let fail = true;
    const summarizeFn = vi.fn(async () => {
      if (fail) throw new Error("network down");
      return "복구 요약";
    });
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => now });

    useAppStore.getState().applyActivityEvent(promptEvent());
    // goal + current 두 요청이 모두 실패로 정착할 때까지 기다린다(카운트 고정).
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    const failedCalls = 2;

    // 쿨다운 중: 새 프롬프트가 와도 호출하지 않는다
    now = 10_000;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "쿨다운 중 지시", at: 2000 }));
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(failedCalls);

    // 쿨다운 경과 후: 재시도해 성공 반영
    now = 40_000;
    fail = false;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "재시도 지시", at: 3000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("복구 요약")
    );
  });

  it("claude-not-found 에러는 Claude만 영구 비활성 — 이후 새 프롬프트에도 호출하지 않는다", async () => {
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      expect(provider).toBe("claude");
      throw new Error("claude-not-found");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    const callsAfterDisable = summarizeFn.mock.calls.length;
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "claude")).toBe(true);

    // 쿨다운과 무관하게, 완전히 새로운 agent/텍스트에도 더는 호출하지 않는다.
    useAppStore.getState().applyActivityEvent(
      promptEvent({ agentId: "a2", sessionId: "s3", text: "다른 지시", at: 5000 })
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(callsAfterDisable);
    expect(summarizeFn.mock.calls.some(([provider]) => provider === "codex")).toBe(false);
    expect(useAppStore.getState().taskLabels["a2"]?.goal).toBeUndefined();
  });

  it("summarizerEnabled=false면 라벨이 있어도 summarizeFn을 호출하지 않는다", async () => {
    useAppStore.getState().hydrateSettings(
      {
        version: 1,
        summarizerEnabled: false,
        summaryProvider: "claude",
        observerEnabled: false,
        soundEnabled: true,
        soundVolume: 0.5,
        externalTerminal: "terminal",
        externalEditor: "system",
        attentionHoldMs: 5000,
      },
      false
    );
    const summarizeFn = vi.fn(async () => "요약");
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await new Promise((r) => setTimeout(r, 0));

    expect(summarizeFn).not.toHaveBeenCalled();
  });

  it("summarizer-disabled 에러는 영구 비활성/쿨다운 없이 무시된다", async () => {
    // ON 상태에서 요청이 나갔는데 백엔드가 disabled를 돌려준 경합 상황을 재현.
    // now()를 고정해 두면, 만약 구현이 실수로 쿨다운을 걸었을 때(now() + 30_000)
    // 아래 재요청이 `now() < cooldownUntil`에 막혀 실패하므로 검증력이 있다.
    const summarizeFn = vi.fn(
      async (_provider: SummaryProvider, instruction: string, text: string) => {
        if (instruction === GOAL_SYSTEM_PROMPT) return "목표";
        if (text === "버그 고쳐줘") {
          useAppStore.getState().hydrateSettings(
            { ...useAppStore.getState().appSettings, summarizerEnabled: false },
            false,
          );
          throw new Error("summarizer-disabled");
        }
        return "새 요약";
      },
    );
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => 1000 });

    useAppStore.getState().applyActivityEvent(promptEvent()); // text: "버그 고쳐줘"
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels["a1"].goal).toBe("목표"));

    // 새 원문 → 쿨다운/영구비활성 없이 즉시 재요청되어 성공해야 한다.
    useAppStore.getState().updateAppSettings({ summarizerEnabled: true });
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "새 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("새 요약")
    );
  });

  it("메타·깨짐 응답은 반영하지 않고 쿨다운 후 폴백", async () => {
    let now = 0;
    let broken = true;
    const summarizeFn = vi.fn(async () => {
      if (broken) return "죄송하지만 요약할 수 없습니다";
      return "복구 요약";
    });
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => now });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    const failedCalls = 2;

    expect(useAppStore.getState().taskLabels["a1"].goal).toBeUndefined();
    expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBeUndefined();

    // 쿨다운 중: 새 프롬프트가 와도 호출하지 않는다
    now = 10_000;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "쿨다운 중 지시", at: 2000 }));
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(failedCalls);

    // 쿨다운 경과 후: 재시도해 성공 반영
    now = 40_000;
    broken = false;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "재시도 지시", at: 3000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("복구 요약")
    );
  });

  it("이전 세션의 동일 원문 결과는 새 세션 identity 재검사 후에만 적용한다", async () => {
    const oldGoalGate = deferred();
    const oldCurrentGate = deferred();
    const newGoalGate = deferred();
    const newCurrentGate = deferred();
    const summarizeFn = vi.fn(
      async (provider: SummaryProvider, instruction: string) => {
        const isGoal = instruction === GOAL_SYSTEM_PROMPT;
        if (provider === "claude") {
          await (isGoal ? oldGoalGate.promise : oldCurrentGate.promise);
          return isGoal ? "Claude 이전 목표" : "Claude 이전 현재";
        }
        await (isGoal ? newGoalGate.promise : newCurrentGate.promise);
        return isGoal ? "Codex 새 목표" : "Codex 새 현재";
      },
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s1", at: 1000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s2", at: 2000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(4));

    oldGoalGate.release();
    oldCurrentGate.release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(useAppStore.getState().taskLabels.a1.sessionId).toBe("s2");
    expect(useAppStore.getState().taskLabels.a1.goal).toBeUndefined();
    expect(useAppStore.getState().taskLabels.a1.currentSummary).toBeUndefined();

    newGoalGate.release();
    newCurrentGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 새 현재"),
    );
    expect(useAppStore.getState().taskLabels.a1.goal).toBe("Codex 새 목표");
    expect(useAppStore.getState().taskLabels.a1.sessionId).toBe("s2");
  });
});

describe("sanitizeSummary", () => {
  it("다중 줄 입력은 첫 줄만 취한다", () => {
    expect(sanitizeSummary("버그 수정\n부가 설명 줄")).toBe("버그 수정");
  });

  it("따옴표를 제거한다", () => {
    expect(sanitizeSummary('"버그 수정"')).toBe("버그 수정");
  });

  it("머리말(요약:/목표:)을 제거한다", () => {
    expect(sanitizeSummary("요약: 버그 수정")).toBe("버그 수정");
  });

  it("메타 발언 응답은 null", () => {
    expect(sanitizeSummary("죄송하지만 인코딩 오류로 요약할 수 없습니다")).toBeNull();
  });

  it("깨진 응답(물음표 반복)은 null", () => {
    expect(sanitizeSummary("?? ??? ???")).toBeNull();
  });

  it("40자 초과 응답은 null", () => {
    expect(sanitizeSummary("가".repeat(41))).toBeNull();
  });

  it("공백뿐인 응답은 null", () => {
    expect(sanitizeSummary("   \n  ")).toBeNull();
  });

  it("치환 문자(U+FFFD) 포함 응답은 null", () => {
    expect(sanitizeSummary("버그 � 수정")).toBeNull();
  });
});
