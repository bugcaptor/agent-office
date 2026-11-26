import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "@shared/types";
const { api } = vi.hoisted(() => ({ api: { loadWorkItems: vi.fn(), saveWorkItems: vi.fn(), clearNotifications: vi.fn() } }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("../../workpanels/workSendStore", () => ({ requestWorkSend: vi.fn(), workSendDisabledReason: vi.fn() }));
vi.mock("../../workpanels/autoRun", () => ({ endAutoRun: vi.fn(), startAutoRun: vi.fn() }));
vi.mock("../../discussion/discussionControl", () => ({ handleDiscussionControl: vi.fn() }));
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../../workpanels/workItemStore";
import { handleWebDashControl } from "../webDashControl";
const initialApp = useAppStore.getState(); const initialWork = useWorkItemStore.getState();
const profile: AgentProfile = { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0, cwd: "/work" };
beforeEach(() => {
  vi.clearAllMocks();
  api.loadWorkItems.mockResolvedValue([]); api.saveWorkItems.mockResolvedValue(undefined);
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
describe("웹 상태판: 업무 만들기", () => {
  it("만든 업무로 앱의 업무 팝업을 띄우지 않고 고르던 것을 지킨다", async () => {
    const r = await handleWebDashControl({ requestId: "r", op: "work-create", title: "Web task", body: "", assigneeAgentId: "a", send: false });
    expect(r.data).toEqual({ itemId: expect.any(String) });
    expect(useWorkItemStore.getState().selectedId).toBeNull();
    const open = useWorkItemStore.getState().add({ title: "Open", body: "", status: "todo" })!;
    await handleWebDashControl({ requestId: "r2", op: "work-create", title: "Another", body: "", send: false });
    expect(useWorkItemStore.getState().selectedId).toBe(open);
  });
});
