// src/renderer/store/__tests__/taskLabels.test.ts
//
// taskLabels 슬라이스: prompt 이벤트 축적, 세션 교체 리셋,
// 요약 반영, removeAgent 정리.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEvent } from "@shared/types";

// 일부 케이스(연속 prompt)는 턴을 정산하므로 appendSessionTurn이 호출된다 —
// 실 tauriApi(invoke)를 타지 않도록 모킹(다른 시간추적 테스트와 동일 컨벤션).
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { appendSessionTurn: vi.fn() } }));

import { useAppStore } from "../appStore";

function promptEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000, text: "첫 지시", ...overrides };
}

beforeEach(() => {
  useAppStore.setState({ taskLabels: {}, timeTracking: {}, agents: {}, sessions: {}, agentOrder: [] });
});

describe("applyActivityEvent → taskLabels", () => {
  it("첫 prompt는 first/latest를 함께 설정한다", () => {
    useAppStore.getState().applyActivityEvent(promptEvent());
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l).toEqual({
      sessionId: "s1",
      firstPromptText: "첫 지시",
      latestPromptText: "첫 지시",
      latestPromptAt: 1000,
    });
  });

  it("같은 세션의 후속 prompt는 latest만 갱신하고 currentSummary를 무효화한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.setTaskLabelSummary("a1", { goal: "버그 수정", currentSummary: "첫 요약" });
    s.applyActivityEvent(promptEvent({ text: "다음 지시", at: 2000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l.firstPromptText).toBe("첫 지시");
    expect(l.goal).toBe("버그 수정"); // 목표는 유지
    expect(l.latestPromptText).toBe("다음 지시");
    expect(l.latestPromptAt).toBe(2000);
    expect(l.currentSummary).toBeUndefined(); // 재요약 대상
  });

  it("sessionId가 바뀌면 전체 리셋 후 새로 시작한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.setTaskLabelSummary("a1", { goal: "버그 수정" });
    s.applyActivityEvent(promptEvent({ sessionId: "s2", text: "새 세션 지시", at: 3000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l).toEqual({
      sessionId: "s2",
      firstPromptText: "새 세션 지시",
      latestPromptText: "새 세션 지시",
      latestPromptAt: 3000,
    });
  });

  it("text 없는 prompt / tool 이벤트는 taskLabels를 건드리지 않는다 (timeTracking은 기존대로)", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent({ text: undefined }));
    s.applyActivityEvent(promptEvent({ kind: "tool", text: undefined, at: 1100 }));
    expect(useAppStore.getState().taskLabels).toEqual({});
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("working");
  });

  it("setTaskLabelSummary는 없는 agent에는 no-op", () => {
    useAppStore.getState().setTaskLabelSummary("ghost", { goal: "x" });
    expect(useAppStore.getState().taskLabels).toEqual({});
  });

  it("removeAgent가 taskLabels 엔트리도 정리한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.removeAgent("a1");
    expect(useAppStore.getState().taskLabels).toEqual({});
  });
});
