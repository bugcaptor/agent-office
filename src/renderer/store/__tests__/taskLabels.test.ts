// src/renderer/store/__tests__/taskLabels.test.ts
//
// taskLabels 슬라이스: prompt 이벤트 축적, 세션 교체 리셋,
// 요약 반영, removeAgent 정리.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEvent, NotificationEvent } from "@shared/types";

// 일부 케이스(연속 prompt)는 턴을 정산하므로 appendSessionTurn이 호출된다 —
// 실 tauriApi(invoke)를 타지 않도록 모킹(다른 시간추적 테스트와 동일 컨벤션).
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { appendSessionTurn: vi.fn() } }));

import { useAppStore } from "../appStore";

function promptEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000, text: "첫 지시", ...overrides };
}

function toolEvent(overrides: Partial<ActivityEvent> = {}): ActivityEvent {
  return { agentId: "a1", sessionId: "s1", kind: "tool", at: 5000, ...overrides };
}

function stopEvent(overrides: Partial<NotificationEvent> = {}): NotificationEvent {
  return {
    id: "n1",
    sessionId: "s1",
    agentId: "a1",
    source: "stop",
    message: "완료",
    dedupKey: "k1",
    at: 9000,
    ...overrides,
  };
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

describe("applyActivityEvent(tool) → taskLabels 실황", () => {
  it("라벨이 있으면 도구 요약/assistant 내레이션을 반영한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ text: "Bash: npm test", assistantText: "테스트 실행 중", at: 5000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l.latestToolText).toBe("Bash: npm test");
    expect(l.latestToolAt).toBe(5000);
    expect(l.latestAssistantText).toBe("테스트 실행 중");
    // 프롬프트/세션은 유지.
    expect(l.firstPromptText).toBe("첫 지시");
    expect(l.sessionId).toBe("s1");
  });

  it("도구 요약은 2초 스로틀, assistantText는 즉시 반영한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ text: "Read: a.rs", at: 5000 }));
    // 1초 뒤 도구 이벤트: text는 스로틀에 걸려 무시되지만 assistantText는 반영.
    s.applyActivityEvent(toolEvent({ text: "Read: b.rs", assistantText: "코드 읽는 중", at: 6000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l.latestToolText).toBe("Read: a.rs"); // 스로틀로 갱신 안 됨
    expect(l.latestToolAt).toBe(5000);
    expect(l.latestAssistantText).toBe("코드 읽는 중"); // 즉시 반영
    // 2초 이상 지나면 갱신.
    s.applyActivityEvent(toolEvent({ text: "Read: b.rs", at: 7500 }));
    expect(useAppStore.getState().taskLabels["a1"].latestToolText).toBe("Read: b.rs");
  });

  it("동일 도구 텍스트는 스로틀 통과해도 갱신하지 않는다(no-op)", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ text: "Bash: build", at: 5000 }));
    s.applyActivityEvent(toolEvent({ text: "Bash: build", at: 9000 })); // 동일 텍스트
    expect(useAppStore.getState().taskLabels["a1"].latestToolAt).toBe(5000);
  });

  it("라벨이 없으면(프롬프트 없이 tool) 라벨을 만들지 않는다", () => {
    useAppStore.getState().applyActivityEvent(toolEvent({ text: "Bash: x" }));
    expect(useAppStore.getState().taskLabels).toEqual({});
  });

  it("sessionId가 다르면 무시한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ sessionId: "s2", text: "Bash: x" }));
    expect(useAppStore.getState().taskLabels["a1"].latestToolText).toBeUndefined();
  });

  it("새 프롬프트가 오면 이전 턴 실황(tool/assistant)을 리셋한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ text: "Bash: x", assistantText: "작업 중", at: 5000 }));
    s.applyActivityEvent(promptEvent({ text: "다음 지시", at: 8000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l.latestToolText).toBeUndefined();
    expect(l.latestAssistantText).toBeUndefined();
    expect(l.latestToolAt).toBeUndefined();
    expect(l.latestPromptText).toBe("다음 지시");
  });

  it("stop 알림(applyNotificationTiming)이 실황을 리셋한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent(promptEvent());
    s.applyActivityEvent(toolEvent({ text: "Bash: x", assistantText: "작업 중", at: 5000 }));
    s.applyNotificationTiming(stopEvent({ at: 9000 }));
    const l = useAppStore.getState().taskLabels["a1"];
    expect(l.latestToolText).toBeUndefined();
    expect(l.latestAssistantText).toBeUndefined();
    expect(l.latestToolAt).toBeUndefined();
    // 프롬프트 소스는 보존(idle이어도 목표/원문은 남는다).
    expect(l.firstPromptText).toBe("첫 지시");
  });
});
