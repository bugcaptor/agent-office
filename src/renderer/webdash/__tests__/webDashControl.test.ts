import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "@shared/types";
const { api, workSend, autoRun } = vi.hoisted(() => ({
  api: { loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), clearNotifications: vi.fn() },
  workSend: { requestWorkSend: vi.fn(), workSendDisabledReason: vi.fn() },
  autoRun: { endAutoRun: vi.fn(), pauseAutoRun: vi.fn(), resumeAutoRun: vi.fn(), setAutoRunUsageGuard: vi.fn(), startAutoRun: vi.fn() },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("../../workpanels/workSendStore", () => workSend);
vi.mock("../../workpanels/autoRun", () => autoRun);
vi.mock("../../discussion/discussionControl", () => ({ handleDiscussionControl: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../../workpanels/workItemStore";
import { handleWebDashControl } from "../webDashControl";
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
  workSend.requestWorkSend.mockResolvedValue(undefined);
  workSend.workSendDisabledReason.mockReturnValue(undefined);
  autoRun.startAutoRun.mockReturnValue(undefined);
  useAppStore.setState(initialApp, true); useAppStore.setState({ agents: { a: profile }, agentOrder: ["a"] });
  useWorkItemStore.setState({ ...initialWork, loaded: true }, true);
});
describe("웹 상태판: 업무 완료", () => {
  it("업무를 완료로 옮기고 열린 업무 팝업과 담당 캐릭터 알림을 내린다", async () => {
    const id = useWorkItemStore.getState().add({ title: "Task", body: "", status: "todo", assigneeAgentId: "a" })!;
    useWorkItemStore.getState().edit(id, { status: "review" });
    expect(useWorkItemStore.getState().selectedId).toBe(id);
    useAppStore.getState().pushNotification({ id: "n1", agentId: "a", sessionId: "s", source: "stop", message: "done", at: 1 } as never);
    expect(useAppStore.getState().notifications).toHaveLength(1);
    expect(await handleWebDashControl({ requestId: "r", op: "work-done", itemId: id })).toEqual({});
    expect(useWorkItemStore.getState().items[0].status).toBe("done");
    expect(useWorkItemStore.getState().selectedId).toBeNull();
    expect(useAppStore.getState().notifications).toHaveLength(0);
    expect(api.clearNotifications).toHaveBeenCalledWith("a");
  });
  it("없는 업무는 거절한다", async () => {
    expect(await handleWebDashControl({ requestId: "r", op: "work-done", itemId: "nope" })).toEqual({ error: "task-not-found" });
  });
});
describe("웹 상태판: 업무 만들기·편집·삭제·순서", () => {
  it("만든 업무로 앱의 업무 팝업을 띄우지 않고 고르던 것을 지킨다", async () => {
    const r = await handleWebDashControl({
      requestId: "r", op: "work-create", title: "Web task", body: "세부 내용", assigneeAgentId: "a",
      agent: { cli: "codex", model: "gpt-5", effort: "high" }, maxFollowUps: 5, permissionWaitMinutes: 20, send: false,
    });
    expect(r.data).toEqual({ itemId: expect.any(String) });
    expect(useWorkItemStore.getState().selectedId).toBeNull();
    const created = useWorkItemStore.getState().items[0];
    expect(created.agent).toEqual({ cli: "codex", model: "gpt-5", effort: "high" });
    expect(created.maxFollowUps).toBe(5);
    expect(created.permissionWaitMinutes).toBe(20);
    const open = useWorkItemStore.getState().add({ title: "Open", body: "", status: "todo" })!;
    await handleWebDashControl({ requestId: "r2", op: "work-create", title: "Another", body: "", send: false });
    expect(useWorkItemStore.getState().selectedId).toBe(open);
  });
  it("업무 스토어가 미로드 상태여도 자동으로 불러온 뒤 처리한다", async () => {
    useWorkItemStore.setState({ ...initialWork, loaded: false }, true);
    const r = await handleWebDashControl({ requestId: "r", op: "work-create", title: "Lazy load", body: "", send: false });
    expect(r.error).toBeUndefined();
    expect(api.loadWorkItems).toHaveBeenCalledTimes(1);
    expect(useWorkItemStore.getState().items).toHaveLength(1);
  });
  it("업무 필드를 수정하고 null로 선택 값을 비우며 삭제·순서 변경을 반영한다", async () => {
    const a = useWorkItemStore.getState().add({ title: "First", body: "", status: "todo", assigneeAgentId: "a", agent: { cli: "claude", model: "opus" }, maxFollowUps: 4 })!;
    const b = useWorkItemStore.getState().add({ title: "Second", body: "", status: "todo", assigneeAgentId: "a" })!;
    expect(await handleWebDashControl({
      requestId: "r1", op: "work-edit", itemId: a,
      patch: { title: "Updated", body: "body", assigneeAgentId: null, agent: null, dependsOn: [b], plan: true, maxFollowUps: null },
    })).toEqual({});
    const edited = useWorkItemStore.getState().items.find((i) => i.id === a)!;
    expect(edited.title).toBe("Updated");
    expect(edited.assigneeAgentId).toBeUndefined();
    expect(edited.agent).toBeUndefined();
    expect(edited.dependsOn).toEqual([b]);
    expect(edited.plan).toBe(true);
    expect(edited.maxFollowUps).toBeUndefined();

    expect(await handleWebDashControl({ requestId: "r2", op: "work-reorder", itemId: b, targetId: a, after: false })).toEqual({});
    const orders = new Map(useWorkItemStore.getState().items.map((i) => [i.id, i.order]));
    expect(orders.get(b)!).toBeLessThan(orders.get(a)!);

    expect(await handleWebDashControl({ requestId: "r3", op: "work-delete", itemId: b })).toEqual({});
    expect(useWorkItemStore.getState().items).toHaveLength(1);
    expect(useWorkItemStore.getState().items[0].dependsOn).toBeUndefined();
  });
});
describe("웹 상태판: 보내기·자동 진행·예약", () => {
  it("플랜 루트·선행 대기 업무 보내기를 막고, 일반 업무는 restartConfirmed를 전달한다", async () => {
    const dep = useWorkItemStore.getState().add({ title: "Step 1", body: "", status: "todo", assigneeAgentId: "a" })!;
    const root = useWorkItemStore.getState().add({ title: "Root", body: "", status: "todo", assigneeAgentId: "a", dependsOn: [dep], plan: true })!;
    expect(await handleWebDashControl({ requestId: "r1", op: "work-send", itemId: root })).toEqual({ error: "work-send-plan-root" });
    useWorkItemStore.getState().edit(root, { plan: false });
    expect(await handleWebDashControl({ requestId: "r2", op: "work-send", itemId: root })).toEqual({ error: "work-send-blocked" });
    expect(await handleWebDashControl({ requestId: "r3", op: "work-send", itemId: dep, restartConfirmed: true })).toEqual({});
    expect(workSend.requestWorkSend).toHaveBeenCalledWith(dep, { restartConfirmed: true });
  });
  it("자동 진행 시작·일시정지·계속·사용량 대기 토글·끝내기를 중계한다", async () => {
    const dep = useWorkItemStore.getState().add({ title: "Step 1", body: "", status: "todo", assigneeAgentId: "a" })!;
    const root = useWorkItemStore.getState().add({ title: "Plan", body: "", status: "todo", assigneeAgentId: "a", dependsOn: [dep], plan: true })!;
    expect(await handleWebDashControl({ requestId: "r1", op: "autorun-start", itemId: root, usageGuard: false })).toEqual({});
    expect(autoRun.startAutoRun).toHaveBeenCalledWith(root, { usageGuard: false });
    useWorkItemStore.getState().setAutoRun(root, { state: "paused", judgeSessionId: "j", judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: 1, pauseReason: "user" });
    expect(await handleWebDashControl({ requestId: "r2", op: "autorun-pause", itemId: root })).toEqual({});
    expect(autoRun.pauseAutoRun).toHaveBeenCalledWith(root);
    expect(await handleWebDashControl({ requestId: "r3", op: "autorun-resume", itemId: root })).toEqual({});
    expect(autoRun.resumeAutoRun).toHaveBeenCalledWith(root);
    expect(await handleWebDashControl({ requestId: "r4", op: "autorun-usage-guard", itemId: root, usageGuard: false })).toEqual({});
    expect(autoRun.setAutoRunUsageGuard).toHaveBeenCalledWith(root, false);
    expect(await handleWebDashControl({ requestId: "r5", op: "autorun-end", itemId: root })).toEqual({});
    expect(autoRun.endAutoRun).toHaveBeenCalledWith(root);
  });
  it("예약을 설정·끄기·켜기·삭제하고 잘못된 규칙을 거절한다", async () => {
    const id = useWorkItemStore.getState().add({ title: "Scheduled", body: "", status: "todo", assigneeAgentId: "a" })!;
    expect(await handleWebDashControl({ requestId: "r0", op: "work-schedule", itemId: id, action: "set", rule: { kind: "interval", minutes: 1 } })).toEqual({ error: "schedule-invalid" });
    expect(await handleWebDashControl({ requestId: "r1", op: "work-schedule", itemId: id, action: "set", rule: { kind: "daily", time: "09:30" } })).toEqual({});
    expect(useWorkItemStore.getState().items[0].schedule?.enabled).toBe(true);
    expect(useWorkItemStore.getState().items[0].schedule?.nextAt).toBeTypeOf("number");
    expect(await handleWebDashControl({ requestId: "r2", op: "work-schedule", itemId: id, action: "disable" })).toEqual({});
    expect(useWorkItemStore.getState().items[0].schedule?.enabled).toBe(false);
    expect(await handleWebDashControl({ requestId: "r3", op: "work-schedule", itemId: id, action: "enable" })).toEqual({});
    expect(useWorkItemStore.getState().items[0].schedule?.enabled).toBe(true);
    expect(await handleWebDashControl({ requestId: "r4", op: "work-schedule", itemId: id, action: "remove" })).toEqual({});
    expect(useWorkItemStore.getState().items[0].schedule).toBeUndefined();
  });
});
