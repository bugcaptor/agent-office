// src/renderer/store/__tests__/appStore.shellActivity.test.ts
//
// 셸 포그라운드 명령 감지(kbm #2f9)가 렌더러 쪽에서 만나는 지점은 딱 둘이다:
// 시작은 기존 prompt activity를 그대로 타고, 종료는 새 `idle` activity로 온다.
// `idle`은 완료 알림(source="stop")과 같은 정산·실황 정리를 하되 알림은 만들지
// 않는다 — 셸 명령마다 알림 목록이 불어나면 안 되기 때문이다.
import { beforeEach, describe, expect, it, vi } from "vitest";

const { appendSessionTurnMock } = vi.hoisted(() => ({
  appendSessionTurnMock: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { appendSessionTurn: appendSessionTurnMock },
}));

import { useAppStore } from "../appStore";

const initialState = useAppStore.getState();

beforeEach(() => {
  appendSessionTurnMock.mockClear();
  useAppStore.setState(initialState, true);
});

describe("appStore: 셸 작업중 신호(prompt → idle)", () => {
  it("셸 명령 시작(prompt)이 턴을 열고 명령줄을 라벨 목표로 싣는다", () => {
    useAppStore.getState().applyActivityEvent({
      agentId: "a1",
      sessionId: "s1",
      kind: "prompt",
      at: 1000,
      text: "npm test",
    });
    const state = useAppStore.getState();
    expect(state.timeTracking["a1"].phase).toBe("working");
    expect(state.timeTracking["a1"].turnStartedAt).toBe(1000);
    expect(state.taskLabels["a1"].latestPromptText).toBe("npm test");
  });

  it("idle이 열린 턴을 정산하고 알림은 만들지 않는다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "idle", at: 4000 });

    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("idle");
    expect(t.turns).toBe(1);
    expect(t.totalMs).toBe(4000);
    expect(t.workedMs).toBe(4000);
    // 알림은 별도 채널(notification-new)이라 activity만으로는 생기지 않는다.
    expect(useAppStore.getState().notifications).toEqual([]);
    expect(appendSessionTurnMock).toHaveBeenCalledTimes(1);
    expect(appendSessionTurnMock.mock.calls[0][0]).toMatchObject({
      agentId: "a1",
      startedAt: 0,
      endedAt: 4000,
    });
  });

  it("idle이 턴 중 실황(도구 요약/내레이션)을 지운다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0, text: "make" });
    s.applyActivityEvent({
      agentId: "a1",
      sessionId: "s1",
      kind: "tool",
      at: 3000,
      text: "Bash: make",
      assistantText: "빌드 중",
    });
    expect(useAppStore.getState().taskLabels["a1"].latestToolText).toBe("Bash: make");

    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "idle", at: 6000 });
    const label = useAppStore.getState().taskLabels["a1"];
    // 목표(무엇을 시켰나)는 남고 실황만 사라진다 — 완료 알림과 같은 규칙.
    expect(label.latestPromptText).toBe("make");
    expect(label.latestToolText).toBeUndefined();
    expect(label.latestAssistantText).toBeUndefined();
    expect(label.latestToolAt).toBeUndefined();
  });

  it("이미 유휴인 캐릭터에 온 idle은 no-op(반쪽 턴을 만들지 않는다)", () => {
    useAppStore
      .getState()
      .applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "idle", at: 4000 });
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("idle");
    expect(t.turns).toBe(0);
    expect(t.totalMs).toBe(0);
    expect(appendSessionTurnMock).not.toHaveBeenCalled();
  });

  it("라벨이 아직 없는 세션의 idle도 안전하게 정산만 한다", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 }); // text 없음 → 라벨 미생성
    expect(useAppStore.getState().taskLabels["a1"]).toBeUndefined();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "idle", at: 2000 });
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("idle");
    expect(useAppStore.getState().taskLabels["a1"]).toBeUndefined();
  });
});
