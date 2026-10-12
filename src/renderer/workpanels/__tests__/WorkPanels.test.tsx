// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const { api } = vi.hoisted(() => ({ api: {
  loadWorkItems: vi.fn().mockResolvedValue([]), saveWorkItems: vi.fn().mockResolvedValue(undefined),
  loadSessionEvents: vi.fn().mockResolvedValue([]), listSessionLogs: vi.fn().mockResolvedValue({ items: [], total: 0 }),
  automationCliModels: vi.fn().mockResolvedValue(["custom/model"]), saveState: vi.fn().mockResolvedValue(undefined),
  inspectWorkSendTarget: vi.fn().mockResolvedValue({ sessionId: "s", humanInputRevision: 3, restartRequired: true, hasUncommitted: true }),
  sendWorkItem: vi.fn().mockResolvedValue({ sessionId: "s", submittedAt: 100 }),
  setAppSettings: vi.fn().mockResolvedValue(undefined),
} }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { destroy: vi.fn() } }));
vi.mock("../AgentFace", () => ({ AgentFace: () => null }));
vi.mock("../../layout/TopBar", () => ({ TopBar: () => null }));
vi.mock("../../layout/BottomBar", () => ({ BottomBar: () => null }));
vi.mock("../../timeline/SessionTimePanel", () => ({ SessionTimePanel: () => null }));
vi.mock("../../notification/NotificationTicker", () => ({ NotificationTicker: () => null }));
import { UIChrome } from "../../layout/UIChrome";
import { SystemTab } from "../../settings/SystemTab";
import { useAppStore } from "../../store/appStore";
import { useWorkItemStore } from "../workItemStore";
import { useSessionSummaryStore } from "../sessionSummaryStore";
import { settleWorkSends, useWorkSendStore } from "../workSendStore";
import { usePanelLayoutStore } from "../panelLayoutStore";
const initial = useAppStore.getState();
const initialWork = useWorkItemStore.getState();
beforeEach(() => {
  useAppStore.setState(initial,true); useWorkItemStore.setState(initialWork,true);
  useSessionSummaryStore.setState({summaries:{},liveIds:[],error:false});
  usePanelLayoutStore.setState({ panels:{left:{width:280,collapsed:false},right:{width:280,collapsed:false}} });
  useWorkSendStore.setState({ busy: {}, confirmation: null });
  api.sendWorkItem.mockClear(); api.inspectWorkSendTarget.mockClear();
  api.loadWorkItems.mockClear(); api.saveWorkItems.mockClear(); api.loadSessionEvents.mockClear();
});
afterEach(async () => { window.history.replaceState({}, "", "/"); cleanup(); await settleWorkSends(); await useWorkItemStore.getState().flush(); });
describe("업무 패널 UI", () => {
  it("데스크톱 원격 창에서는 호스트 설정이 켜져 있어도 로컬 업무 패널을 만들지 않는다", () => {
    window.history.replaceState({}, "", "/?remote=1");
    useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}});
    render(<UIChrome />); expect(screen.queryByRole("complementary")).toBeNull(); expect(api.loadSessionEvents).not.toHaveBeenCalled();
  });
  it("기본 OFF에서는 패널도 이벤트 조회도 만들지 않는다", () => {
    render(<UIChrome />); expect(screen.queryByRole("complementary")).toBeNull(); expect(api.loadSessionEvents).not.toHaveBeenCalled();
  });
  it("설정 토글로 켜며 접기·펼치기와 키보드 폭 조절을 기억한다", async () => {
    render(<><SystemTab /><UIChrome /></>);
    fireEvent.click(screen.getByText("업무 패널"));
    await waitFor(() => expect(screen.getAllByRole("complementary")).toHaveLength(2));
    fireEvent.keyDown(screen.getAllByRole("separator")[0],{key:"ArrowRight"});
    expect(usePanelLayoutStore.getState().panels.left.width).toBe(290);
    fireEvent.click(screen.getAllByRole("button",{name:"패널 접기"})[0]);
    expect(usePanelLayoutStore.getState().panels.left.collapsed).toBe(true);
    expect(screen.queryByPlaceholderText("새 업무 제목 · Enter로 추가")).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:"패널 펼치기"}));
    expect(screen.getByPlaceholderText("새 업무 제목 · Enter로 추가")).toBeTruthy();
  });
  it("Enter 추가·Esc 접기·삭제 확인을 제공하고 끄고 켜도 업무를 보존한다", async () => {
    useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}});
    render(<UIChrome />); await waitFor(() => expect(useWorkItemStore.getState().loaded).toBe(true));
    const input=screen.getByPlaceholderText("새 업무 제목 · Enter로 추가");
    fireEvent.change(input,{target:{value:"Review feature"}}); fireEvent.submit(input.closest("form")!);
    expect(screen.getByText("Review feature")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("본문"),{target:{value:"Notes"}});
    fireEvent.keyDown(screen.getByLabelText("본문"),{key:"Escape"}); expect(screen.queryByLabelText("본문")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:false}}));
    expect(screen.queryByRole("complementary")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}}));
    expect(useWorkItemStore.getState().items[0].body).toBe("Notes");
    fireEvent.click(screen.getByText("Review feature")); fireEvent.click(screen.getByRole("button",{name:"삭제"}));
    expect(screen.getByRole("alertdialog")).toBeTruthy(); fireEvent.click(screen.getByRole("button",{name:"취소"})); expect(useWorkItemStore.getState().items).toHaveLength(1);
    fireEvent.click(screen.getByRole("button",{name:"삭제"})); fireEvent.click(screen.getAllByRole("button",{name:"삭제"})[1]); expect(useWorkItemStore.getState().items).toHaveLength(0);
  });
});


it("모델/노력 편집만으로 입력하지 않고 재시작 확인에서 미제출 경고와 취소를 제공한다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: { a: { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0 } }, agentOrder: ["a"] });
  useWorkItemStore.setState({ loaded: true }); const id = useWorkItemStore.getState().add("Task")!;
  useWorkItemStore.getState().edit(id, { assigneeAgentId: "a" });
  render(<UIChrome />);
  fireEvent.change(screen.getByLabelText("업무 에이전트 지정"), { target: { value: "pi" } });
  fireEvent.change(screen.getByLabelText("모델"), { target: { value: "custom/model" } });
  fireEvent.change(screen.getByLabelText("노력"), { target: { value: "high" } });
  expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(api.inspectWorkSendTarget).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "보내기" }));
  await waitFor(() => expect(screen.getByRole("alertdialog")).toBeTruthy());
  expect(screen.getByText("미제출 입력도 사라집니다.")).toBeTruthy();
  fireEvent.click(screen.getAllByRole("button", { name: "패널 접기" })[0]);
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "취소" }));
  await waitFor(() => expect(Object.keys(useWorkSendStore.getState().busy)).toHaveLength(0));
  expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(useWorkItemStore.getState().items[0].agent).toEqual({ cli: "pi", model: "custom/model", effort: "high" });
});
