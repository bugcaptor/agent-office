import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ActivityEvent, NotificationEvent, SessionStateEvent, TurnUsageEvent } from "@shared/types";
const { callbacks, save } = vi.hoisted(() => ({ callbacks: {} as Record<string, (e: never) => void>, save: vi.fn() }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {
  saveWorkItems: save,
  onActivity: (cb: (e: ActivityEvent) => void) => { callbacks.activity = cb; return vi.fn(); },
  onNotification: (cb: (e: NotificationEvent) => void) => { callbacks.notification = cb; return vi.fn(); },
  onTurnUsage: (cb: (e: TurnUsageEvent) => void) => { callbacks.usage = cb; return vi.fn(); },
  onSessionState: (cb: (e: SessionStateEvent) => void) => { callbacks.session = cb; return vi.fn(); },
} }));
import { useWorkItemStore } from "../workItemStore";
import { installWorkMonitor, reconcileWorkMonitor, useWorkMonitorStore } from "../workMonitor";
const initial = useWorkItemStore.getState(); let off: () => void;
function activity(kind: ActivityEvent["kind"], at = 100, sessionId = "s") { callbacks.activity({ agentId: "a", sessionId, kind, at } as never); }
function notify(source: NotificationEvent["source"], at = 101, sessionId = "s") { callbacks.notification({ agentId: "a", sessionId, source, at } as never); }
function item() { const id = useWorkItemStore.getState().add("Task")!; useWorkItemStore.getState().recordSend(id, { sessionId: "s" }); return id; }
beforeEach(() => { vi.useFakeTimers(); save.mockResolvedValue(undefined); useWorkItemStore.setState({ ...initial, loaded: true }, true); useWorkMonitorStore.setState({ sessions: {} }); off = installWorkMonitor(); });
afterEach(async () => { off(); await useWorkItemStore.getState().flush(); vi.useRealTimers(); });
describe("연결 업무 모니터링", () => {
  it("질문은 강조만 하고 Stop→검토, 후속 prompt→진행으로 전이한다", () => {
    const id = item(); activity("prompt"); notify("hook");
    expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("waiting"); expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
    notify("stop", 102); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    activity("tool", 103); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    activity("prompt", 104); expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
    useWorkItemStore.getState().edit(id, { status: "done" }); notify("stop", 105); activity("prompt", 106); expect(useWorkItemStore.getState().items[0].status).toBe("done");
  });
  it("이전 연결 세션/늦게 도착한 이벤트/idle 신호는 최신 업무 상태를 바꾸지 않는다", () => {
    const id = item(); useWorkItemStore.getState().recordSend(id, { sessionId: "new" });
    notify("stop", 200); expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
    notify("stop", 202, "new"); activity("prompt", 201, "new"); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    activity("idle", 203, "new"); expect(useWorkMonitorStore.getState().sessions.new.phase).toBe("review");
  });
  it("턴 종료 뒤 idle_prompt 훅 알림은 질문 대기로 바꾸지 않고 다음 prompt 뒤 질문은 다시 강조한다", () => {
    item(); activity("prompt"); notify("stop", 102); notify("hook", 160);
    expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("review"); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    activity("prompt", 170); notify("hook", 171); expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("waiting");
    callbacks.session({ agentId: "a", sessionId: "s", at: 180, state: "exited" } as never); notify("hook", 181);
    expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("ended");
  });
  it("부분 사용량은 턴 종료로 보지 않고 실제 Stop 사용량은 검토로 바꾼다", () => {
    item(); callbacks.usage({ agentId: "a", sessionId: "s", at: 100, partial: true } as never); expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
    callbacks.usage({ agentId: "a", sessionId: "s", at: 101, partial: false } as never); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    callbacks.session({ agentId: "a", sessionId: "s", at: 102, state: "exited" } as never); expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("ended");
    expect(useWorkItemStore.getState().items[0].status).toBe("review");
  });
  it("태스크 대화 세션의 신호는 같은 sessionId여도 업무 상태에 넣지 않는다", () => {
    item(); activity("prompt");
    callbacks.notification({ agentId: "task-chat-t1", sessionId: "s", source: "stop", at: 300 } as never);
    callbacks.usage({ agentId: "task-chat-t1", sessionId: "s", at: 301, partial: false } as never);
    callbacks.session({ agentId: "task-chat-t1", sessionId: "c", at: 302, state: "exited" } as never);
    expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
    expect(useWorkMonitorStore.getState().sessions.s.phase).toBe("working");
    expect(useWorkMonitorStore.getState().sessions.c).toBeUndefined();
  });
  it("IPC 완료 전에 온 Stop도 연결 후 반영하지만 예전 시작 신호는 무시한다", () => {
    notify("stop", 110); item(); reconcileWorkMonitor("s", 100); expect(useWorkItemStore.getState().items[0].status).toBe("review");
    useWorkItemStore.getState().edit(useWorkItemStore.getState().items[0].id, { status: "in_progress" }); reconcileWorkMonitor("s", 120); expect(useWorkItemStore.getState().items[0].status).toBe("in_progress");
  });
});
