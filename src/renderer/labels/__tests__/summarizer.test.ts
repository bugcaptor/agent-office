// src/renderer/labels/__tests__/summarizer.test.ts
//
// 요약기(통합 호출 계약): 프롬프트마다 한 번의 호출로 goal+current를 함께
// 받는다(이전 goal을 컨텍스트로 넘겨 후속 지시엔 목표 유지 바이어스). 캐시,
// stale 폐기, claude-not-found 영구 비활성, 실패 백오프, opt-in 게이트.
// summarizeFn/시계 전부 주입.
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
  LABEL_SYSTEM_PROMPT,
  deriveContextText,
  installTaskLabelSummarizer,
  sanitizeLabelPair,
  sanitizeSummary,
} from "../summarizer";
import type { AgentTaskLabel } from "../../store/types";
import type { ActivityEvent, SummaryProvider } from "@shared/types";

/** 통합 응답: 1줄 목표 + 2줄 현재. */
function pair(goal: string, current: string): string {
  return `${goal}\n${current}`;
}

/** 주입 summarizeFn 시그니처 — 인자 생략 구현이라도 mock.calls 튜플이 3인자로 추론되게. */
type SummarizeFn = (
  provider: SummaryProvider,
  instruction: string,
  text: string,
) => Promise<string>;

function promptEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000, text: "버그 고쳐줘", ...overrides };
}

function toolEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "tool", at: 9000, text: "Bash: ls", ...overrides };
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
      gitStatusEnabled: true,
    },
    false
  );
});

describe("installTaskLabelSummarizer", () => {
  it("첫 프롬프트에 통합 호출 1회로 goal+current를 함께 반영한다", async () => {
    const summarizeFn = vi.fn<SummarizeFn>(async () => pair("버그 수정", "버그 고치는 중"));
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());

    await vi.waitFor(() => {
      const l = useAppStore.getState().taskLabels["a1"];
      expect(l.goal).toBe("버그 수정");
      expect(l.currentSummary).toBe("버그 고치는 중");
    });
    expect(summarizeFn).toHaveBeenCalledTimes(1);
    const [provider, instruction, text] = summarizeFn.mock.calls[0];
    expect(provider).toBe("claude");
    expect(instruction).toBe(LABEL_SYSTEM_PROMPT);
    // 첫 프롬프트 → 이전 목표는 (없음), 새 지시 원문 포함.
    expect(text).toContain("(없음)");
    expect(text).toContain("버그 고쳐줘");
  });

  it("후속 프롬프트는 이전 goal을 컨텍스트로 넘기고 목표를 유지할 수 있다", async () => {
    const summarizeFn = vi.fn(async (_p: SummaryProvider, _i: string, text: string) =>
      text.includes("테스트도") ? pair("버그 수정", "테스트 수정") : pair("버그 수정", "버그 고치는 중"),
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels.a1.goal).toBe("버그 수정"));

    useAppStore.getState().applyActivityEvent(promptEvent({ text: "테스트도 고쳐줘", at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("테스트 수정"),
    );
    // 목표는 유지, 현재만 갱신.
    expect(useAppStore.getState().taskLabels.a1.goal).toBe("버그 수정");
    expect(summarizeFn).toHaveBeenCalledTimes(2);
    // 두 번째 호출의 사용자 텍스트에 이전 목표가 컨텍스트로 들어간다.
    const secondText = summarizeFn.mock.calls[1][2];
    expect(secondText).toContain("버그 수정");
    expect(secondText).toContain("테스트도 고쳐줘");
    expect(secondText).not.toContain("(없음)");
  });

  it.each(["claude", "codex"] as const)(
    "%s provider도 프롬프트당 정확히 한 번 호출한다",
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
          gitStatusEnabled: true,
        },
        false,
      );
      const summarizeFn = vi.fn<SummarizeFn>(async () => pair("목표", "현재"));
      teardown = installTaskLabelSummarizer({ summarizeFn });
      useAppStore.getState().applyActivityEvent(promptEvent());
      await vi.waitFor(() =>
        expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("현재"),
      );
      expect(summarizeFn).toHaveBeenCalledTimes(1);
      expect(summarizeFn.mock.calls.every(([seen]) => seen === provider)).toBe(true);
    },
  );

  it("provider 변경만으로 기존 라벨을 재요약하지 않고 다음 프롬프트부터 적용한다", async () => {
    const summarizeFn = vi.fn<SummarizeFn>(async (provider) =>
      pair(`${provider} 목표`, `${provider} 현재`),
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    await Promise.resolve();
    expect(summarizeFn).toHaveBeenCalledTimes(1);

    useAppStore.getState().applyActivityEvent(promptEvent({ text: "다음 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(
        summarizeFn.mock.calls.some(
          ([provider, , text]) => provider === "codex" && text.includes("다음 지시"),
        ),
      ).toBe(true),
    );
  });

  it("진행 중인 동일 identity는 provider 변경 후 재sweep에서도 인플라이트 중 중복 요청하지 않는다", async () => {
    const gate = deferred();
    const summarizeFn = vi.fn<SummarizeFn>(async (provider) => {
      if (provider === "claude") {
        await gate.promise;
        return pair("Claude 목표", "Claude 현재");
      }
      return pair("Codex 목표", "Codex 현재");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    // provider를 바꾸고, 같은 프롬프트를 유지한 채 tool 이벤트로 재sweep을 유발.
    // (tool 이벤트는 정황도 실어 오지만, claude 요청이 인플라이트인 동안엔
    //  activeIdentityKeys 소유권이 같은 identity의 재요청을 막는다.)
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(toolEvent());
    await new Promise((resolve) => setTimeout(resolve, 0));
    // 인플라이트 중에는 activeIdentityKeys 소유권으로 codex 재요청이 차단된다.
    expect.soft(summarizeFn).toHaveBeenCalledTimes(1);
    expect.soft(summarizeFn.mock.calls.some(([provider]) => provider === "codex")).toBe(false);

    // 인플라이트가 끝나면 도착한 정황으로 딱 1회 재평가한다 — 이때 provider는 codex.
    gate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.goal).toBe("Codex 목표"),
    );
    expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 현재");
    expect(summarizeFn).toHaveBeenCalledTimes(2);
    const [enrichProvider, , enrichText] = summarizeFn.mock.calls[1];
    expect(enrichProvider).toBe("codex");
    expect(enrichText).toContain("[초기 작업 정황]");
  });

  it("Claude cache는 같은 원문의 새 Codex identity를 충족하지 않는다", async () => {
    const summarizeFn = vi.fn<SummarizeFn>(async (provider) =>
      pair(`${provider} 목표`, `${provider} 현재`),
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("claude 현재"),
    );
    expect(summarizeFn).toHaveBeenCalledTimes(1);

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("codex 현재"),
    );

    expect(summarizeFn).toHaveBeenCalledTimes(2);
    const [provider, instruction, text] = summarizeFn.mock.calls[1];
    expect(provider).toBe("codex");
    expect(instruction).toBe(LABEL_SYSTEM_PROMPT);
    expect(text).toContain("버그 고쳐줘");
  });

  it("같은 세션·원문의 더 최신 prompt는 이전 결과를 거부하고 새 provider 결과만 적용한다", async () => {
    const oldGate = deferred();
    const newGate = deferred();
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      await (provider === "claude" ? oldGate.promise : newGate.promise);
      return provider === "claude" ? pair("Claude 목표", "Claude 낡은 현재") : pair("Codex 목표", "Codex 새 현재");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent({ at: 1000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ at: 2000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));

    oldGate.release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    // 낡은 identity(at=1000) 결과는 최신 프롬프트(at=2000)에 반영되지 않는다.
    expect(useAppStore.getState().taskLabels.a1.latestPromptAt).toBe(2000);
    expect(useAppStore.getState().taskLabels.a1.currentSummary).toBeUndefined();

    newGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 새 현재"),
    );
    expect(useAppStore.getState().taskLabels.a1.goal).toBe("Codex 목표");
  });

  it("한 provider 미설치는 다른 provider를 비활성화하지 않는다", async () => {
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      if (provider === "claude") throw new Error("claude-not-found");
      return pair("Codex 목표", "Codex 현재");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "claude")).toBe(true);

    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "Codex 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 현재"),
    );
  });

  it("Codex 미설치 latch도 Claude와 분리된다", async () => {
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      if (provider === "codex") throw new Error("codex-not-found");
      return pair("Claude 목표", "Claude 현재");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });
    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));
    expect(summarizeFn.mock.calls.every(([provider]) => provider === "codex")).toBe(true);

    useAppStore.getState().updateAppSettings({ summaryProvider: "claude" });
    useAppStore.getState().applyActivityEvent(
      promptEvent({ text: "Claude 지시", at: 2000 }),
    );
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Claude 현재"),
    );
  });

  it("같은 원문·이전목표는 캐시로 재호출 없이 반영한다 (세션 재시작 후 동일 첫 프롬프트)", async () => {
    const summarizeFn = vi.fn<SummarizeFn>(async () => pair("목표", "현재"));
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels["a1"].goal).toBe("목표"));
    expect(summarizeFn).toHaveBeenCalledTimes(1);

    // 같은 텍스트, 새 세션 → 첫 프롬프트라 prevGoal (없음) 동일 → 캐시 히트.
    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s2", at: 2000 }));
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels["a1"].goal).toBe("목표"));
    expect(summarizeFn).toHaveBeenCalledTimes(1); // 캐시 히트, 추가 호출 없음
  });

  it("응답 도착 전에 최신 프롬프트가 바뀌면 stale 결과를 폐기한다", async () => {
    const gate = deferred();
    const summarizeFn = vi.fn(async (_p: SummaryProvider, _i: string, text: string) => {
      if (text.includes("버그 고쳐줘")) {
        await gate.promise; // 첫 요약을 붙잡아 둔다
        return pair("낡은 목표", "낡은 요약");
      }
      return pair("정상 목표", "정상 요약");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "새 지시", at: 2000 })); // gate 해제 전에 교체
    gate.release();

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
      return pair("복구 목표", "복구 요약");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => now });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    // 쿨다운 중: 새 프롬프트가 와도 호출하지 않는다
    now = 10_000;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "쿨다운 중 지시", at: 2000 }));
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(1);

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
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));
    const callsAfterDisable = summarizeFn.mock.calls.length;

    // 쿨다운과 무관하게, 완전히 새로운 agent/텍스트에도 더는 호출하지 않는다.
    useAppStore.getState().applyActivityEvent(
      promptEvent({ agentId: "a2", sessionId: "s3", text: "다른 지시", at: 5000 })
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(callsAfterDisable);
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
        gitStatusEnabled: true,
      },
      false
    );
    const summarizeFn = vi.fn(async () => pair("목표", "요약"));
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await new Promise((r) => setTimeout(r, 0));

    expect(summarizeFn).not.toHaveBeenCalled();
  });

  it("summarizer-disabled 에러는 영구 비활성/쿨다운 없이 무시된다", async () => {
    // ON 상태에서 요청이 나갔는데 백엔드가 disabled를 돌려준 경합 상황을 재현.
    // now()를 고정해 두면, 만약 구현이 실수로 쿨다운을 걸었을 때(now() + 30_000)
    // 아래 재요청이 `now() < cooldownUntil`에 막혀 실패하므로 검증력이 있다.
    const summarizeFn = vi.fn(async (_p: SummaryProvider, _i: string, text: string) => {
      if (text.includes("버그 고쳐줘")) {
        useAppStore.getState().hydrateSettings(
          { ...useAppStore.getState().appSettings, summarizerEnabled: false },
          false,
        );
        throw new Error("summarizer-disabled");
      }
      return pair("새 목표", "새 요약");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => 1000 });

    useAppStore.getState().applyActivityEvent(promptEvent()); // text: "버그 고쳐줘"
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    // 새 원문 → 쿨다운/영구비활성 없이 즉시 재요청되어 성공해야 한다.
    useAppStore.getState().updateAppSettings({ summarizerEnabled: true });
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "새 지시", at: 2000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("새 요약")
    );
  });

  it("두 줄 미만·메타·깨짐 응답은 반영하지 않고 쿨다운 후 폴백", async () => {
    let now = 0;
    let broken = true;
    const summarizeFn = vi.fn(async () => {
      if (broken) return "죄송하지만 요약할 수 없습니다"; // 한 줄뿐 + 메타
      return pair("복구 목표", "복구 요약");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn, now: () => now });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    expect(useAppStore.getState().taskLabels["a1"].goal).toBeUndefined();
    expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBeUndefined();

    // 쿨다운 중: 새 프롬프트가 와도 호출하지 않는다
    now = 10_000;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "쿨다운 중 지시", at: 2000 }));
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn.mock.calls.length).toBe(1);

    // 쿨다운 경과 후: 재시도해 성공 반영
    now = 40_000;
    broken = false;
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "재시도 지시", at: 3000 }));
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels["a1"].currentSummary).toBe("복구 요약")
    );
  });

  it("이전 세션의 동일 원문 결과는 새 세션 identity 재검사 후에만 적용한다", async () => {
    const oldGate = deferred();
    const newGate = deferred();
    const summarizeFn = vi.fn(async (provider: SummaryProvider) => {
      if (provider === "claude") {
        await oldGate.promise;
        return pair("Claude 이전 목표", "Claude 이전 현재");
      }
      await newGate.promise;
      return pair("Codex 새 목표", "Codex 새 현재");
    });
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s1", at: 1000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));
    useAppStore.getState().updateAppSettings({ summaryProvider: "codex" });
    useAppStore.getState().applyActivityEvent(promptEvent({ sessionId: "s2", at: 2000 }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(2));

    oldGate.release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    // 낡은 세션(s1) 결과는 새 세션(s2) identity에 반영되지 않는다.
    expect(useAppStore.getState().taskLabels.a1.sessionId).toBe("s2");
    expect(useAppStore.getState().taskLabels.a1.goal).toBeUndefined();
    expect(useAppStore.getState().taskLabels.a1.currentSummary).toBeUndefined();

    newGate.release();
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("Codex 새 현재"),
    );
    expect(useAppStore.getState().taskLabels.a1.goal).toBe("Codex 새 목표");
    expect(useAppStore.getState().taskLabels.a1.sessionId).toBe("s2");
  });

  it("정황이 도착하면 잠정 목표를 1회 재평가해 승격하고 [초기 작업 정황]을 주입한다 (#51)", async () => {
    const summarizeFn = vi.fn(async (_p: SummaryProvider, _i: string, text: string) =>
      text.includes("초기 작업 정황")
        ? pair("훅 설정 복구", "이슈 40 해결")
        : pair("이슈 40 해결", "이슈 40 해결"),
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    // 프롬프트만으로 만든 잠정 목표(정황 없음).
    useAppStore.getState().applyActivityEvent(promptEvent({ text: "이슈 40을 해결해" }));
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels.a1.goal).toBe("이슈 40 해결"));
    expect(summarizeFn).toHaveBeenCalledTimes(1);
    expect(summarizeFn.mock.calls[0][2]).not.toContain("초기 작업 정황");

    // assistant 내레이션 도착 → 딱 1회 재평가로 목표 승격.
    useAppStore.getState().applyActivityEvent(
      toolEvent({ text: undefined, assistantText: "Claude 훅 설정 파일을 복구하는 중" }),
    );
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels.a1.goal).toBe("훅 설정 복구"));
    expect(summarizeFn).toHaveBeenCalledTimes(2);
    const enrichText = summarizeFn.mock.calls[1][2];
    expect(enrichText).toContain("[초기 작업 정황]");
    expect(enrichText).toContain("Claude 훅 설정 파일을 복구하는 중");
  });

  it("정황이 없으면 프롬프트 기반 요약 1회만 낸다 (#51)", async () => {
    const summarizeFn = vi.fn<SummarizeFn>(async () => pair("목표", "현재"));
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent());
    await vi.waitFor(() =>
      expect(useAppStore.getState().taskLabels.a1.currentSummary).toBe("현재"),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn).toHaveBeenCalledTimes(1);
    expect(summarizeFn.mock.calls[0][2]).not.toContain("초기 작업 정황");
  });

  it("정황이 여러 번 갱신돼도 프롬프트당 재평가는 정확히 1회다 (#51)", async () => {
    const summarizeFn = vi.fn(async (_p: SummaryProvider, _i: string, text: string) =>
      text.includes("초기 작업 정황")
        ? pair("훅 설정 복구", "이슈 40 해결")
        : pair("이슈 40 해결", "이슈 40 해결"),
    );
    teardown = installTaskLabelSummarizer({ summarizeFn });

    useAppStore.getState().applyActivityEvent(promptEvent({ text: "이슈 40을 해결해" }));
    await vi.waitFor(() => expect(summarizeFn).toHaveBeenCalledTimes(1));

    useAppStore.getState().applyActivityEvent(
      toolEvent({ text: undefined, assistantText: "훅 설정 복구 중", at: 9000 }),
    );
    await vi.waitFor(() => expect(useAppStore.getState().taskLabels.a1.goal).toBe("훅 설정 복구"));
    expect(summarizeFn).toHaveBeenCalledTimes(2);

    // 이후 내레이션이 더 와도 같은 프롬프트는 다시 재평가하지 않는다.
    useAppStore.getState().applyActivityEvent(
      toolEvent({ text: undefined, assistantText: "테스트 실행 중", at: 10_000 }),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(summarizeFn).toHaveBeenCalledTimes(2);
  });
});

describe("deriveContextText", () => {
  const labelWith = (patch: Partial<AgentTaskLabel>): AgentTaskLabel => ({
    sessionId: "s1",
    ...patch,
  });

  it("assistant 내레이션을 도구 요약보다 우선한다", () => {
    expect(
      deriveContextText(
        labelWith({ latestAssistantText: "원인 좁히는 중", latestToolText: "Bash: ls" }),
      ),
    ).toBe("원인 좁히는 중");
  });

  it("assistant가 없으면 도구 요약으로 폴백한다", () => {
    expect(deriveContextText(labelWith({ latestToolText: "Bash: npm test" }))).toBe(
      "Bash: npm test",
    );
  });

  it("둘 다 없으면 undefined", () => {
    expect(deriveContextText(labelWith({}))).toBeUndefined();
  });

  it("120자 초과는 버리지 않고 절단한다", () => {
    const out = deriveContextText(labelWith({ latestAssistantText: "가".repeat(200) }));
    expect(out && Array.from(out).length).toBe(120);
  });
});

describe("sanitizeLabelPair", () => {
  it("정상 두 줄을 목표/현재로 나눈다", () => {
    expect(sanitizeLabelPair("버그 수정\n테스트 수정")).toEqual({
      goal: "버그 수정",
      current: "테스트 수정",
    });
  });

  it("비공백 줄 앞 2개만 취한다(중간 빈 줄·꼬리 무시)", () => {
    expect(sanitizeLabelPair("버그 수정\n\n테스트 수정\n부가 설명")).toEqual({
      goal: "버그 수정",
      current: "테스트 수정",
    });
  });

  it("한 줄뿐이면 null", () => {
    expect(sanitizeLabelPair("버그 수정")).toBeNull();
  });

  it("공백뿐이면 null", () => {
    expect(sanitizeLabelPair("  \n  \n ")).toBeNull();
  });

  it("머리말(1줄:/2줄:/목표:/요약:)을 각 줄에서 제거한다", () => {
    expect(sanitizeLabelPair("1줄: 버그 수정\n2줄: 테스트 수정")).toEqual({
      goal: "버그 수정",
      current: "테스트 수정",
    });
    expect(sanitizeLabelPair("목표: 버그 수정\n요약: 테스트 수정")).toEqual({
      goal: "버그 수정",
      current: "테스트 수정",
    });
  });

  it("한 줄이라도 40자 초과면 전체 null", () => {
    expect(sanitizeLabelPair(`버그 수정\n${"가".repeat(41)}`)).toBeNull();
  });

  it("한 줄이라도 메타 발언이면 전체 null", () => {
    expect(sanitizeLabelPair("버그 수정\n죄송하지만 요약할 수 없습니다")).toBeNull();
  });

  it("한 줄이라도 깨짐(치환 문자/물음표 반복)이면 전체 null", () => {
    expect(sanitizeLabelPair("버그 � 수정\n테스트 수정")).toBeNull();
    expect(sanitizeLabelPair("버그 수정\n?? ??? ???")).toBeNull();
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
