// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
import { useWorkMonitorStore } from "../workMonitor";
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
    expect(screen.queryByRole("button", { name: "새 태스크 만들기" })).toBeNull();
    fireEvent.click(screen.getByRole("button",{name:"패널 펼치기"}));
    expect(screen.getByRole("button", { name: "새 태스크 만들기" })).toBeTruthy();
  });
  it("추가·저장·Esc 접기·삭제 확인을 제공하고 끄고 켜도 저장한 업무를 보존한다", async () => {
    useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}});
    render(<UIChrome />); await waitFor(() => expect(useWorkItemStore.getState().loaded).toBe(true));
    fireEvent.click(screen.getByRole("button", { name: "새 태스크 만들기" }));
    const input = screen.getByLabelText("제목");
    fireEvent.change(input,{target:{value:"Review feature"}}); fireEvent.submit(input.closest("form")!);
    expect(screen.getByText("Review feature")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("본문"),{target:{value:"Notes"}});
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    fireEvent.keyDown(screen.getByLabelText("본문"),{key:"Escape"}); expect(screen.queryByLabelText("본문")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:false}}));
    expect(screen.queryByRole("complementary")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}}));
    expect(useWorkItemStore.getState().items[0].body).toBe("Notes");
    fireEvent.click(screen.getByText("Review feature")); fireEvent.click(screen.getByRole("button",{name:"삭제"}));
    expect(screen.getByRole("alertdialog")).toBeTruthy(); fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button",{name:"취소"})); expect(useWorkItemStore.getState().items).toHaveLength(1);
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
  expect(useWorkItemStore.getState().items[0].agent).toBeUndefined();
  expect(screen.getByRole("button", { name: "보내기" }).hasAttribute("disabled")).toBe(true);
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  fireEvent.click(screen.getByRole("button", { name: "보내기" }));
  await waitFor(() => expect(screen.getByRole("alertdialog")).toBeTruthy());
  expect(screen.getByText("미제출 입력도 사라집니다.")).toBeTruthy();
  fireEvent.click(screen.getAllByRole("button", { name: "패널 접기" })[0]);
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "취소" }));
  await waitFor(() => expect(Object.keys(useWorkSendStore.getState().busy)).toHaveLength(0));
  expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(useWorkItemStore.getState().items[0].agent).toEqual({ cli: "pi", model: "custom/model", effort: "high" });
});


it("새 태스크는 추가할 때까지 저장하지 않고 취소하거나 접었다가 계속 쓸 수 있다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  render(<UIChrome />);
  await waitFor(() => expect(useWorkItemStore.getState().loaded).toBe(true));
  fireEvent.click(screen.getByRole("button", { name: "새 태스크 만들기" }));
  expect(screen.getByRole("button", { name: "추가" }).hasAttribute("disabled")).toBe(true);
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "새 태스크의 본문\n둘째 줄" } });
  fireEvent.blur(screen.getByLabelText("본문"));
  await act(() => useWorkItemStore.getState().flush());
  expect(useWorkItemStore.getState().items).toHaveLength(0);
  expect(api.saveWorkItems).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "태스크 접기" }));
  expect(screen.queryByLabelText("본문")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "새 태스크 작성 계속" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("새 태스크의 본문\n둘째 줄");
  fireEvent.click(screen.getByRole("button", { name: "취소" }));
  expect(screen.queryByLabelText("본문")).toBeNull();
  expect(useWorkItemStore.getState().items).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "새 태스크 만들기" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("");
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "제목 없는 태스크\n상세 설명" } });
  expect((screen.getByLabelText("제목") as HTMLInputElement).placeholder).toBe("제목 없는 태스크 상세 설명");
  fireEvent.click(screen.getByRole("button", { name: "추가" }));
  expect(useWorkItemStore.getState().items[0]).toMatchObject({ title: "", body: "제목 없는 태스크\n상세 설명" });
  expect(screen.getByText("제목 없는 태스크 상세 설명", { selector: "strong" }).classList.contains("work-title-derived")).toBe(true);
  await waitFor(() => expect(api.saveWorkItems).toHaveBeenCalledTimes(1));
});

it("편집 초안은 접기와 카드 전환에 보존되며 취소는 모든 필드를 되돌린다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: {
    a: { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0 },
  } });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  const id = store.add("Original")!;
  store.add("Other"); store.select(id);
  await store.flush(); api.saveWorkItems.mockClear();
  render(<UIChrome />);
  fireEvent.change(screen.getByLabelText("제목"), { target: { value: "Draft" } });
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "Draft body" } });
  fireEvent.change(screen.getByLabelText("담당 캐릭터"), { target: { value: "a" } });
  fireEvent.change(screen.getByLabelText("업무 에이전트 지정"), { target: { value: "codex" } });
  fireEvent.change(screen.getByLabelText("모델"), { target: { value: "custom/model" } });
  fireEvent.change(screen.getByLabelText("노력"), { target: { value: "high" } });
  fireEvent.change(screen.getByLabelText("상태"), { target: { value: "done" } });
  fireEvent.blur(screen.getByLabelText("본문"));
  await act(() => store.flush());
  expect(api.saveWorkItems).not.toHaveBeenCalled();
  expect(useWorkItemStore.getState().items.find((item) => item.id === id)).toMatchObject({ title: "Original", body: "", status: "todo" });
  fireEvent.click(screen.getByRole("button", { name: "태스크 접기" }));
  fireEvent.click(screen.getByText("Other"));
  fireEvent.click(screen.getByText("Original"));
  expect((screen.getByLabelText("제목") as HTMLInputElement).value).toBe("Draft");
  fireEvent.click(screen.getAllByRole("button", { name: "패널 접기" })[0]);
  fireEvent.click(screen.getByRole("button", { name: "패널 펼치기" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("Draft body");
  fireEvent.click(screen.getByRole("button", { name: "취소" }));
  fireEvent.click(screen.getByText("Original"));
  expect((screen.getByLabelText("제목") as HTMLInputElement).value).toBe("Original");
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("");
  expect((screen.getByLabelText("담당 캐릭터") as HTMLSelectElement).value).toBe("");
  expect((screen.getByLabelText("업무 에이전트 지정") as HTMLSelectElement).value).toBe("");
  expect((screen.getByLabelText("상태") as HTMLSelectElement).value).toBe("todo");
});

it("본문으로 만든 제목은 수정 저장 후 갱신되며 편집 중 자동 상태 전이를 덮어쓰지 않는다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  const id = store.add("Explicit")!;
  render(<UIChrome />);
  fireEvent.change(screen.getByLabelText("제목"), { target: { value: "" } });
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "First body" } });
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  expect(screen.getByText("First body", { selector: "strong" }).classList.contains("work-title-derived")).toBe(true);
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "Updated body" } });
  act(() => store.recordSend(id, { sessionId: "session" }));
  expect((screen.getByLabelText("상태") as HTMLSelectElement).value).toBe("in_progress");
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  expect(screen.getByText("Updated body", { selector: "strong" }).classList.contains("work-title-derived")).toBe(true);
  expect(useWorkItemStore.getState().items[0]).toMatchObject({ title: "", body: "Updated body", status: "in_progress", sessionIds: ["session"] });
});

it("완료로 옮긴 업무는 살아 있는 세션의 질문 대기 신호가 남아도 완료로 보이고 강조하지 않는다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  useWorkItemStore.setState({ loaded: true });
  const id = useWorkItemStore.getState().add("Finished")!;
  useWorkItemStore.getState().recordSend(id, { sessionId: "live" });
  useSessionSummaryStore.setState({ liveIds: ["live"] });
  useWorkMonitorStore.setState({ sessions: { live: { phase: "waiting", at: 1, agentId: "a", transition: false } } });
  render(<UIChrome />);
  const card = document.getElementById(`work-item-${id}`)!;
  expect(within(card).getByText("질문 대기")).toBeTruthy(); expect(card.classList.contains("waiting")).toBe(true);
  act(() => useWorkItemStore.getState().edit(id, { status: "done" }));
  const doneCard = document.getElementById(`work-item-${id}`)!;
  expect(within(doneCard).queryByText("질문 대기")).toBeNull(); expect(doneCard.classList.contains("waiting")).toBe(false);
  expect(doneCard.querySelector(".work-card-monitor")?.textContent).toContain("완료");
});
