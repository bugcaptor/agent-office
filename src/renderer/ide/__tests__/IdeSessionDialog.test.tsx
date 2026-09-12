// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, IdeSessionCandidate } from "@shared/types";

const mocks = vi.hoisted(() => ({
  listIdeSessions: vi.fn<(...args: unknown[]) => Promise<IdeSessionCandidate[]>>(),
  connectIdeSession: vi.fn().mockResolvedValue({ sessionId: "external-1" }),
  flushPersistence: vi.fn().mockResolvedValue(undefined),
}));
const { listIdeSessions, connectIdeSession, flushPersistence } = mocks;
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { listIdeSessions: mocks.listIdeSessions, connectIdeSession: mocks.connectIdeSession } }));
vi.mock("../../store/persist", () => ({ flushPersistence: () => mocks.flushPersistence() }));

import { useAppStore } from "../../store/appStore";
import { IdeSessionDialog } from "../IdeSessionDialog";

const initialState = useAppStore.getState();
const candidate: IdeSessionCandidate = { provider: "codex", sourceSessionId: "source-1", cwd: "/work/demo", file: "chat.json", updatedAt: 1, source: "vscode" };
function agent(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return { id: "a1", name: "연결이", role: "개발", seed: "seed", createdAt: 1, deskIndex: 0, cwd: "/work/demo", ...overrides };
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  useAppStore.setState((state) => ({ appSettings: { ...state.appSettings, observerEnabled: true, ideConnectionEnabled: true } }));
  listIdeSessions.mockReset(); listIdeSessions.mockResolvedValue([candidate]);
  connectIdeSession.mockReset(); connectIdeSession.mockResolvedValue({ sessionId: "external-1" });
  flushPersistence.mockReset(); flushPersistence.mockResolvedValue(undefined);
});
afterEach(cleanup);

describe("IdeSessionDialog", () => {
  it("preselects the requested Codex character and discovers only its folder", async () => {
    useAppStore.getState().addAgent(agent({ cwd: "/work/other" }), { startSession: false });
    useAppStore.getState().addAgent(agent({ id: "a2", name: "두번째", cwd: "/work/demo" }), { startSession: false });
    useAppStore.getState().openModal({
      kind: "ide-session",
      initialAgentId: "a2",
      initialCwd: "/work/demo",
      initialProvider: "codex",
    });
    render(<IdeSessionDialog />);
    await waitFor(() => expect(listIdeSessions).toHaveBeenLastCalledWith({ provider: "codex", cwd: "/work/demo" }));
    expect((await screen.findByTitle("chat.json")).getAttribute("aria-pressed")).toBe("true");
    expect((screen.getByLabelText("캐릭터") as HTMLSelectElement).value).toBe("a2");
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await waitFor(() => expect(connectIdeSession).toHaveBeenCalledWith({ agentId: "a2", provider: "codex", file: "chat.json", sourceSessionId: "source-1" }));
  });

  it("filters and connects a Kilo Code VS Code session with its own provider", async () => {
    const kilo = { ...candidate, provider: "kilo" as const, source: "kilo-shared" as const, sourceSessionId: "kilo-1", file: "kilo.db" };
    listIdeSessions.mockResolvedValue([kilo]);
    useAppStore.getState().addAgent(agent(), { startSession: false });
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.change(screen.getByLabelText("공급자"), { target: { value: "kilo" } });
    await waitFor(() => expect(listIdeSessions).toHaveBeenLastCalledWith({ provider: "kilo" }));
    const item = await screen.findByTitle("kilo.db");
    expect(item.textContent).toContain("Kilo Code (VS Code)");
    expect(screen.getByText(/Kilo Code는 VS Code와 CLI가 기록을 공유합니다/)).toBeTruthy();
    fireEvent.click(item);
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await waitFor(() => expect(connectIdeSession).toHaveBeenCalledWith({ agentId: "a1", provider: "kilo", file: "kilo.db", sourceSessionId: "kilo-1" }));
    expect(useAppStore.getState().sessions.a1.kind).toBe("external");
  });

  it("lists a candidate and connects only a matching idle character without a PTY", async () => {
    useAppStore.getState().addAgent(agent(), { startSession: false });
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    await waitFor(() => expect(screen.getByTitle("chat.json")).toBeTruthy());
    expect(screen.getByText("후보는 최근 IDE 기록입니다. 목록에 있어도 세션이 이미 끝났을 수 있습니다.")).toBeTruthy();
    expect(screen.getByText("연결한 뒤의 새 활동만 Agent Office에 표시됩니다. 이전 대화 기록은 재생하지 않습니다.")).toBeTruthy();
    fireEvent.click(screen.getByTitle("chat.json"));
    expect(screen.getByTitle("chat.json").getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await waitFor(() => expect(connectIdeSession).toHaveBeenCalledWith({ agentId: "a1", provider: "codex", file: "chat.json", sourceSessionId: "source-1" }));
    expect(flushPersistence.mock.invocationCallOrder[0]).toBeLessThan(connectIdeSession.mock.invocationCallOrder[0]);
    expect(useAppStore.getState().sessions.a1.kind).toBe("external");
    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
  });

  it("offers running, external, different-folder, unset-folder and clocked-out characters", async () => {
    for (const profile of [agent({ id: "running" }), agent({ id: "external" }), agent({ id: "different", cwd: "/other" }), agent({ id: "unset", cwd: undefined }), agent({ id: "off", clockedOut: true })]) {
      useAppStore.getState().addAgent(profile);
    }
    useAppStore.getState().setSessionState({ agentId: "external", status: "running", external: true });
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    const options = (screen.getByLabelText("캐릭터") as HTMLSelectElement).options;
    expect(Array.from(options).map((option) => option.value)).toEqual(["running", "external", "different", "unset", "off"]);
    expect(connectIdeSession).not.toHaveBeenCalled();
  });

  it("asks only at connect time and cancellation keeps the old terminal and profile", async () => {
    connectIdeSession.mockResolvedValueOnce({ replacement: { sessionId: "old-pty", kind: "pty" } });
    useAppStore.getState().addAgent(agent({ cwd: "/original" }));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "running" });
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(connectIdeSession).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByRole("alertdialog");
    expect(screen.getByText(/실행 중인 터미널과 작업을 종료/)).toBeTruthy();
    expect(useAppStore.getState().agents.a1.cwd).toBe("/original");
    fireEvent.click(screen.getByRole("button", { name: "취소" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(connectIdeSession).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().sessions.a1.status).toBe("running");
    expect(useAppStore.getState().agents.a1.cwd).toBe("/original");
    expect(useAppStore.getState().modal.kind).toBe("ide-session");
  });

  it("confirms the exact session before replacing and updates the folder after success", async () => {
    connectIdeSession.mockResolvedValueOnce({ replacement: { sessionId: "old-pty", kind: "pty" } });
    useAppStore.getState().addAgent(agent({ cwd: "/original" }));
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    fireEvent.click(await screen.findByRole("button", { name: "터미널 종료 후 연결" }));
    await waitFor(() => expect(connectIdeSession).toHaveBeenLastCalledWith({ agentId: "a1", provider: "codex", file: "chat.json", sourceSessionId: "source-1", replaceSessionId: "old-pty" }));
    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));
    expect(useAppStore.getState().agents.a1.cwd).toBe("/work/demo");
    expect(useAppStore.getState().sessions.a1.kind).toBe("external");
  });

  it("describes external replacement as detaching and revives an off-duty character only on success", async () => {
    connectIdeSession.mockResolvedValueOnce({ replacement: { sessionId: "old-external", kind: "external" } });
    useAppStore.getState().addAgent(agent({ cwd: undefined }), { startSession: false });
    useAppStore.getState().clockOut("a1");
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    const confirm = await screen.findByRole("button", { name: "기존 연결 해제 후 연결" });
    expect(screen.getByText(/원본 외부 앱은 종료하지 않습니다/)).toBeTruthy();
    expect(useAppStore.getState().agents.a1.clockedOut).toBe(true);
    fireEvent.click(confirm);
    await waitFor(() => expect(useAppStore.getState().sessions.a1.kind).toBe("external"));
    expect(useAppStore.getState().agents.a1.clockedOut).toBeUndefined();
    expect(useAppStore.getState().agents.a1.cwd).toBe(candidate.cwd);
  });

  it.each([
    ["observed-replacement-changed", "세션이 바뀌었습니다"],
    ["observed-replacement-kill-failed", "터미널을 종료하지 못해"],
    ["observed-replacement-tmux-kill-failed", "터미널을 종료하지 못해"],
  ])("leaves the profile alone after replacement failure: %s", async (reason, message) => {
    connectIdeSession.mockResolvedValueOnce({ replacement: { sessionId: "old", kind: "pty" } });
    connectIdeSession.mockRejectedValueOnce(reason);
    useAppStore.getState().addAgent(agent({ cwd: "/original" }));
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    fireEvent.click(await screen.findByRole("button", { name: "터미널 종료 후 연결" }));
    expect((await screen.findByRole("alert")).textContent).toContain(message);
    expect(useAppStore.getState().agents.a1.cwd).toBe("/original");
    expect(screen.getByRole("button", { name: "연결" })).toBeTruthy();
  });

  it("drops pending confirmation if IDE observation is disabled", async () => {
    connectIdeSession.mockResolvedValueOnce({ replacement: { sessionId: "old", kind: "pty" } });
    useAppStore.getState().addAgent(agent());
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.click(await screen.findByTitle("chat.json"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByRole("alertdialog");
    act(() => useAppStore.setState((state) => ({ appSettings: { ...state.appSettings, ideConnectionEnabled: false } })));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(connectIdeSession).toHaveBeenCalledTimes(1);
  });

  it("keeps the selected candidate after a connect failure", async () => {
    connectIdeSession.mockRejectedValueOnce(new Error("nope"));
    useAppStore.getState().addAgent(agent(), { startSession: false });
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    await waitFor(() => expect(screen.getByTitle("chat.json")).toBeTruthy());
    fireEvent.click(screen.getByTitle("chat.json"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByRole("alert");
    expect(screen.getByTitle("chat.json").className).toContain("selected");
  });

  it("shows the observer setting requirement without loading candidates", () => {
    useAppStore.setState((state) => ({ appSettings: { ...state.appSettings, observerEnabled: false, ideConnectionEnabled: true } }));
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    expect(screen.getByRole("alert").textContent).toContain("설정");
    expect(listIdeSessions).not.toHaveBeenCalled();
  });

  it("shows the IDE connection setting requirement without loading candidates", () => {
    useAppStore.setState((state) => ({ appSettings: { ...state.appSettings, ideConnectionEnabled: false, observerEnabled: true } }));
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    expect(screen.getByRole("alert").textContent).toContain("IDE 세션 연결");
    expect(listIdeSessions).not.toHaveBeenCalled();
  });

  it("uses the latest provider refresh result when an earlier request finishes late", async () => {
    let resolveAll!: (value: IdeSessionCandidate[]) => void;
    listIdeSessions.mockImplementationOnce(() => new Promise((resolve) => { resolveAll = resolve; }));
    listIdeSessions.mockResolvedValueOnce([{ ...candidate, provider: "claude", sourceSessionId: "claude-1", file: "claude.json" }]);
    useAppStore.getState().openModal({ kind: "ide-session" });
    render(<IdeSessionDialog />);
    fireEvent.change(screen.getByLabelText("공급자"), { target: { value: "claude" } });
    await waitFor(() => expect(screen.getByTitle("claude.json")).toBeTruthy());
    await act(async () => resolveAll([candidate]));
    expect(screen.queryByTitle("chat.json")).toBeNull();
  });
});
