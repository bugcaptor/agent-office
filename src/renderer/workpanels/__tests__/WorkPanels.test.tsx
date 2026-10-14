// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
const { api } = vi.hoisted(() => ({ api: {
  loadWorkItems: vi.fn().mockResolvedValue([]), saveWorkItems: vi.fn().mockResolvedValue(undefined),
  loadSessionEvents: vi.fn().mockResolvedValue([]), listSessionLogs: vi.fn().mockResolvedValue({ items: [], total: 0 }),
  workAgentModels: vi.fn().mockResolvedValue(["custom/model"]), saveState: vi.fn().mockResolvedValue(undefined),
  inspectWorkSendTarget: vi.fn().mockResolvedValue({ sessionId: "s", humanInputRevision: 3, restartRequired: true, hasUncommitted: true }),
  sendWorkItem: vi.fn().mockResolvedValue({ sessionId: "s", submittedAt: 100 }),
  setAppSettings: vi.fn().mockResolvedValue(undefined),
  exportWorkItemsFile: vi.fn(), importWorkItemsFile: vi.fn(),
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
import { usePanelLayoutStore, useWorkGroupStore } from "../panelLayoutStore";
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
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(screen.getByText("Review feature"));
    expect(screen.getByRole("dialog", { name: "Review feature" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("본문"),{target:{value:"Notes"}});
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    fireEvent.keyDown(screen.getByLabelText("본문"),{key:"Escape"}); expect(screen.queryByLabelText("본문")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:false}}));
    expect(screen.queryByRole("complementary")).toBeNull();
    act(() => useAppStore.setState({appSettings:{...initial.appSettings,workPanelsEnabled:true}}));
    expect(useWorkItemStore.getState().items[0].body).toBe("Notes");
    fireEvent.click(screen.getByText("Review feature")); fireEvent.click(screen.getByRole("button",{name:"삭제"}));
    expect(screen.getByRole("alertdialog")).toBeTruthy(); fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button",{name:"취소"})); expect(useWorkItemStore.getState().items).toHaveLength(1);
    fireEvent.click(screen.getByRole("button",{name:"삭제"})); fireEvent.click(within(screen.getByRole("alertdialog")).getByRole("button",{name:"삭제"})); expect(useWorkItemStore.getState().items).toHaveLength(0);
  });
});


it("모델/노력 편집만으로 입력하지 않고 재시작 확인에서 미제출 경고와 취소를 제공하며 보내기에 성공하면 팝업을 닫는다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: { a: { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0 } }, agentOrder: ["a"] });
  useWorkItemStore.setState({ loaded: true }); const id = useWorkItemStore.getState().add("Task")!;
  useWorkItemStore.getState().edit(id, { assigneeAgentId: "a" });
  render(<UIChrome />);
  fireEvent.change(screen.getByLabelText("업무 에이전트 지정"), { target: { value: "pi" } });
  fireEvent.change(screen.getByLabelText("모델"), { target: { value: "custom/model" } });
  fireEvent.change(screen.getByLabelText("노력"), { target: { value: "high" } });
  expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(api.inspectWorkSendTarget).not.toHaveBeenCalled();
  expect(useWorkItemStore.getState().items[0].agent).toBeUndefined();
  expect(screen.queryByRole("button", { name: "보내기" })).toBeNull();
  expect(screen.getByRole("button", { name: "저장 후 보내기" }).hasAttribute("disabled")).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  expect(screen.getByRole("button", { name: "저장" }).hasAttribute("disabled")).toBe(true);
  expect(screen.queryByRole("button", { name: "변경 되돌리기" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "보내기" }));
  await waitFor(() => expect(screen.getByRole("alertdialog")).toBeTruthy());
  expect(screen.getByText("미제출 입력도 사라집니다.")).toBeTruthy();
  fireEvent.click(screen.getAllByRole("button", { name: "패널 접기" })[0]);
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "취소" }));
  await waitFor(() => expect(Object.keys(useWorkSendStore.getState().busy)).toHaveLength(0));
  expect(api.sendWorkItem).not.toHaveBeenCalled(); expect(useWorkItemStore.getState().items[0].agent).toEqual({ cli: "pi", model: "custom/model", effort: "high" });
  expect(screen.getByRole("dialog", { name: "Task" })).toBeTruthy();
  api.inspectWorkSendTarget.mockResolvedValueOnce({ sessionId: "s", humanInputRevision: 3, restartRequired: false, hasUncommitted: false });
  fireEvent.click(screen.getByRole("button", { name: "보내기" }));
  await waitFor(() => expect(api.sendWorkItem).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(useWorkItemStore.getState().items[0]).toMatchObject({ status: "in_progress", sessionIds: ["s"] });
});


it("새 태스크는 추가할 때까지 저장하지 않고 초안을 버리거나 닫았다가 계속 쓸 수 있다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  render(<UIChrome />);
  await waitFor(() => expect(useWorkItemStore.getState().loaded).toBe(true));
  fireEvent.click(screen.getByRole("button", { name: "새 태스크 만들기" }));
  expect(screen.getByRole("button", { name: "추가" }).hasAttribute("disabled")).toBe(true);
  expect(screen.queryByRole("button", { name: "초안 버리기" })).toBeNull();
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "새 태스크의 본문\n둘째 줄" } });
  fireEvent.blur(screen.getByLabelText("본문"));
  await act(() => useWorkItemStore.getState().flush());
  expect(useWorkItemStore.getState().items).toHaveLength(0);
  expect(api.saveWorkItems).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "닫기" }));
  expect(screen.queryByLabelText("본문")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "새 태스크 작성 계속" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("새 태스크의 본문\n둘째 줄");
  fireEvent.click(screen.getByRole("button", { name: "초안 버리기" }));
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

it("편집 초안은 닫기와 카드 전환에 보존되며 되돌리기는 팝업을 연 채 모든 필드를 되돌린다", async () => {
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
  fireEvent.click(screen.getByRole("button", { name: "닫기" }));
  fireEvent.click(screen.getByText("Other", { selector: "strong" }));
  fireEvent.click(screen.getByText("Original", { selector: "strong" }));
  expect((screen.getByLabelText("제목") as HTMLInputElement).value).toBe("Draft");
  fireEvent.click(screen.getAllByRole("button", { name: "패널 접기" })[0]);
  fireEvent.click(screen.getByRole("button", { name: "패널 펼치기" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("Draft body");
  fireEvent.click(screen.getByRole("button", { name: "변경 되돌리기" }));
  expect(screen.getByRole("dialog", { name: "Original" })).toBeTruthy();
  expect(screen.queryByRole("button", { name: "변경 되돌리기" })).toBeNull();
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

it("편집기에서 선행 작업을 고르면 카드에 대기 표시가 나오고 보내기를 막는다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: {
    a: { id: "a", name: "Ada", role: "dev", seed: "s", createdAt: 1, deskIndex: 0 },
  } });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  const design = store.add("Design")!; const build = store.add("Build")!;
  store.edit(build, { assigneeAgentId: "a" }); store.select(build);
  render(<UIChrome />);
  const add = screen.getByLabelText("선행 작업 추가…") as HTMLSelectElement;
  expect(within(add).queryByText("Build")).toBeNull();
  fireEvent.change(add, { target: { value: design } });
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  expect(useWorkItemStore.getState().items.find((i) => i.id === build)?.dependsOn).toEqual([design]);
  expect(screen.getByText("선행 작업 대기: Design")).toBeTruthy();
  expect((screen.getByRole("button", { name: "보내기" }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole("button", { name: "보내기" }).getAttribute("aria-describedby")).toBe("work-send-hint");
  expect(screen.getByText("선행 작업을 모두 완료한 뒤 보내세요.")).toBeTruthy();
  expect(screen.getByText("후속 작업 1개가 기다림")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "“Design” 선행 작업 빼기" }));
  fireEvent.click(screen.getByRole("button", { name: "저장" }));
  expect(useWorkItemStore.getState().items.find((i) => i.id === build)?.dependsOn).toBeUndefined();
  expect(screen.queryByText("선행 작업 대기: Design")).toBeNull();
});

it("완료로 표시는 상태만 즉시 저장하고 나머지 초안은 남긴다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  useWorkItemStore.setState({ loaded: true });
  const id = useWorkItemStore.getState().add("Wrap up")!; useWorkItemStore.getState().select(id);
  render(<UIChrome />);
  expect(screen.getByRole("button", { name: "저장" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByRole("button", { name: "보내기" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByText("존재하는 담당 캐릭터를 선택하세요.")).toBeTruthy();
  fireEvent.change(screen.getByLabelText("본문"), { target: { value: "Draft notes" } });
  fireEvent.click(screen.getByRole("button", { name: "완료로 표시" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(useWorkItemStore.getState().items[0]).toMatchObject({ status: "done", body: "" });
  fireEvent.click(screen.getByText("Wrap up", { selector: "strong" }));
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("Draft notes");
  expect((screen.getByLabelText("상태") as HTMLSelectElement).value).toBe("done");
  expect(screen.queryByRole("button", { name: "완료로 표시" })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "닫기" }));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("내보내기·가져오기 팝업은 고른 상태만 내보내고 취소·실패를 알리며 가져올 업무를 확인 후 새로 추가한다", async () => {
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true } });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  store.add("Open task"); const done = store.add("Closed task")!; store.edit(done, { status: "done" }); store.select(null);
  render(<UIChrome />);
  fireEvent.click(screen.getByRole("button", { name: "내보내기·가져오기" }));
  const dialog = screen.getByRole("dialog");
  fireEvent.click(within(dialog).getByLabelText("완료 (1)"));
  api.exportWorkItemsFile.mockResolvedValueOnce(null);
  fireEvent.click(within(dialog).getByRole("button", { name: "파일로 내보내기 (1개)" }));
  await waitFor(() => expect(api.exportWorkItemsFile).toHaveBeenCalledTimes(1));
  const [name, filter, json] = api.exportWorkItemsFile.mock.calls[0];
  expect(name).toMatch(/^agent-office-work-items-\d{8}\.json$/);
  expect(filter).toBe("Agent Office 업무");
  expect(JSON.parse(json).items.map((i: { title: string }) => i.title)).toEqual(["Open task"]);
  expect(within(dialog).queryByRole("status")).toBeNull();
  api.exportWorkItemsFile.mockRejectedValueOnce(new Error("disk"));
  fireEvent.click(within(dialog).getByRole("button", { name: "파일로 내보내기 (1개)" }));
  expect((await within(dialog).findByRole("alert")).textContent).toBe("업무를 내보내지 못했습니다.");
  api.exportWorkItemsFile.mockResolvedValueOnce("/tmp/tasks.json");
  fireEvent.click(within(dialog).getByRole("button", { name: "파일로 내보내기 (1개)" }));
  expect((await within(dialog).findByRole("status")).textContent).toBe("업무 1개를 내보냈습니다: /tmp/tasks.json");

  api.importWorkItemsFile.mockResolvedValueOnce("not json");
  fireEvent.click(within(dialog).getByRole("button", { name: "파일 고르기…" }));
  expect((await within(dialog).findByRole("alert")).textContent).toBe("JSON 형식이 올바르지 않습니다.");
  api.importWorkItemsFile.mockResolvedValueOnce(json.replace('"Open task"', '"Edited task"').replace('"items": [', '"items": [{"title":""},'));
  fireEvent.click(within(dialog).getByRole("button", { name: "파일 고르기…" }));
  await within(dialog).findByText("업무 1개를 새로 추가합니다. 내용이 없거나 형식이 맞지 않는 항목 1개는 건너뜁니다.");
  expect(useWorkItemStore.getState().items).toHaveLength(2);
  fireEvent.click(within(dialog).getByRole("button", { name: "추가" }));
  expect(within(dialog).getByRole("status").textContent).toBe("업무 1개를 추가했습니다.");
  expect(useWorkItemStore.getState().items.map((i) => i.title)).toEqual(["Open task", "Closed task", "Edited task"]);
  expect(screen.getByText("Edited task", { selector: "strong" })).toBeTruthy();
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("내보내기는 담당 캐릭터를 골라 이름과 함께 쓰고, 가져오기는 없는 캐릭터의 업무를 고른 캐릭터로 한꺼번에 바꾼다", async () => {
  const profile = (id: string, name: string) => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0 });
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: { a: profile("a", "Ada"), b: profile("b", "Bob") }, agentOrder: ["a", "b"] });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  for (const [title, assigneeAgentId] of [["Ada 1", "a"], ["Ada 2", "a"], ["Bob 1", "b"], ["Free", undefined], ["Ghost", "gone"]] as const) {
    store.add({ title, body: "", status: "todo", assigneeAgentId });
  }
  store.select(null);
  render(<UIChrome />);
  fireEvent.click(screen.getByRole("button", { name: "내보내기·가져오기" }));
  const dialog = screen.getByRole("dialog");
  const assignees = within(dialog).getByRole("group", { name: "내보낼 담당 캐릭터" });
  expect(within(assignees).getAllByRole("checkbox").map((c) => c.parentElement!.textContent)).toEqual(["미배정 (1)", "Ada (2)", "Bob (1)", "삭제된 캐릭터 (1)"]);
  fireEvent.click(within(assignees).getByLabelText("Bob (1)"));
  fireEvent.click(within(assignees).getByLabelText("미배정 (1)"));
  fireEvent.click(within(assignees).getByLabelText("삭제된 캐릭터 (1)"));
  api.exportWorkItemsFile.mockResolvedValueOnce("/tmp/ada.json");
  fireEvent.click(within(dialog).getByRole("button", { name: "파일로 내보내기 (2개)" }));
  await within(dialog).findByText("업무 2개를 내보냈습니다: /tmp/ada.json");
  const exported = JSON.parse(api.exportWorkItemsFile.mock.lastCall![2]).items;
  expect(exported.map((i: { title: string; assigneeName?: string }) => [i.title, i.assigneeName])).toEqual([["Ada 1", "Ada"], ["Ada 2", "Ada"]]);

  // 다른 컴퓨터: 같은 이름의 캐릭터가 있으면 기본으로 그 캐릭터, 이름이 없으면 미배정.
  act(() => useAppStore.setState({ agents: { c: profile("c", "Cat"), d: profile("d", "Ada") }, agentOrder: ["c", "d"] }));
  const file = { items: [...exported, { title: "Other", assigneeAgentId: "zz" }, { title: "Mine", assigneeAgentId: "c" }] };
  api.importWorkItemsFile.mockResolvedValueOnce(JSON.stringify(file));
  fireEvent.click(within(dialog).getByRole("button", { name: "파일 고르기…" }));
  const replace = await within(dialog).findByRole("group", { name: "찾을 수 없는 캐릭터" });
  const ada = within(replace).getByLabelText("Ada (업무 2개) →") as HTMLSelectElement;
  const other = within(replace).getByLabelText("zz (업무 1개) →") as HTMLSelectElement;
  expect([ada.value, other.value]).toEqual(["d", ""]);
  fireEvent.change(ada, { target: { value: "c" } });
  fireEvent.click(within(dialog).getByRole("button", { name: "추가" }));
  const added = useWorkItemStore.getState().items.slice(-4).map((i) => [i.title, i.assigneeAgentId]);
  expect(added).toEqual([["Ada 1", "c"], ["Ada 2", "c"], ["Other", undefined], ["Mine", "c"]]);
});

it("업무를 캐릭터별로 묶고 묶음 접힘을 기억한다", async () => {
  const profile = (id: string, name: string) => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0 });
  useWorkGroupStore.setState({ collapsed: [] });
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: { a: profile("a", "Ada"), b: profile("b", "Bob") }, agentOrder: ["b", "a"] });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  store.add({ title: "Ada todo", body: "", status: "todo", assigneeAgentId: "a" });
  store.add({ title: "Ada done", body: "", status: "done", assigneeAgentId: "a" });
  store.add({ title: "Bob review", body: "", status: "review", assigneeAgentId: "b" });
  store.add({ title: "Nobody", body: "", status: "todo" });
  store.add({ title: "Ghost", body: "", status: "todo", assigneeAgentId: "gone" });
  store.select(null);
  render(<UIChrome />);
  const groups = [...document.querySelectorAll(".work-agent-group")] as HTMLDetailsElement[];
  const heads = (list: Element[]) => list.map((g) => `${g.querySelector(".work-group-name > strong")!.textContent} ${g.querySelector(".work-group-count")!.textContent}`);
  // 머리의 개수는 완료를 뺀 남은 업무 수다.
  expect(heads(groups)).toEqual(["Bob 1", "Ada 1", "미배정 1", "삭제된 캐릭터 1"]);
  const ada = groups[1];
  // 완료가 아닌 업무는 상태 칩을 단 카드로 바로 보이고, 완료만 접힌 묶음에 들어간다.
  expect([...ada.querySelectorAll(":scope > .work-card .work-status-chip")].map((s) => s.textContent)).toEqual(["할 일"]);
  const done = ada.querySelector(".work-done-group") as HTMLDetailsElement;
  expect(done.querySelector("summary")!.textContent).toBe("완료 (1)");
  expect(done.open).toBe(false);
  ada.open = false; fireEvent(ada, new Event("toggle"));
  expect(useWorkGroupStore.getState().collapsed).toEqual(["a"]);
  cleanup(); render(<UIChrome />);
  const reopened = document.querySelectorAll(".work-agent-group")[1] as HTMLDetailsElement;
  expect(reopened.open).toBe(false);
  reopened.open = true; fireEvent(reopened, new Event("toggle"));
  expect(useWorkGroupStore.getState().collapsed).toEqual([]);
});

it("캐릭터 묶음의 ＋는 업무가 없는 캐릭터에도 보이며 그 캐릭터를 담당으로 채운 새 태스크를 연다", async () => {
  const profile = (id: string, name: string) => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0 });
  useWorkGroupStore.setState({ collapsed: ["b"] });
  useAppStore.setState({ appSettings: { ...initial.appSettings, workPanelsEnabled: true }, agents: { a: profile("a", "Ada"), b: profile("b", "Bob") }, agentOrder: ["a", "b"] });
  useWorkItemStore.setState({ loaded: true });
  useWorkItemStore.getState().add({ title: "Ada todo", body: "", status: "todo", assigneeAgentId: "a" });
  useWorkItemStore.getState().select(null);
  render(<UIChrome />);
  const groups = [...document.querySelectorAll(".work-agent-group")] as HTMLDetailsElement[];
  expect(groups.map((g) => g.querySelector(".work-group-count")!.textContent)).toEqual(["1", "0"]);
  expect(groups[1].classList.contains("empty")).toBe(true);
  expect(screen.queryByRole("button", { name: /미배정/ })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Bob에게 새 태스크" }));
  // ＋는 묶음을 접거나 펴지 않는다.
  expect(groups[1].open).toBe(false);
  const dialog = screen.getByRole("dialog", { name: "새 태스크" });
  expect((within(dialog).getByLabelText("담당 캐릭터") as HTMLSelectElement).value).toBe("b");
  fireEvent.change(within(dialog).getByLabelText("본문"), { target: { value: "keep me" } });
  fireEvent.keyDown(within(dialog).getByLabelText("본문"), { key: "Escape" });
  // 쓰던 초안은 살리고 담당만 바꾼다.
  fireEvent.click(screen.getByRole("button", { name: "Ada에게 새 태스크" }));
  expect((screen.getByLabelText("담당 캐릭터") as HTMLSelectElement).value).toBe("a");
  expect((screen.getByLabelText("본문") as HTMLTextAreaElement).value).toBe("keep me");
  fireEvent.submit(screen.getByLabelText("본문").closest("form")!);
  expect(useWorkItemStore.getState().items.find((i) => i.body === "keep me")?.assigneeAgentId).toBe("a");
});

it("캐릭터 묶음 머리에 담당 프로젝트를 보이고 상태 칩을 검토 대기 → 진행 중 → 할 일 순으로 단다", async () => {
  const profile = (id: string, name: string, cwd?: string) => ({ id, name, role: "dev", seed: id, createdAt: 1, deskIndex: 0, cwd });
  useWorkGroupStore.setState({ collapsed: [] });
  useAppStore.setState({
    appSettings: { ...initial.appSettings, workPanelsEnabled: true },
    agents: { a: profile("a", "Ada", "/dev/agent-office"), b: profile("b", "Bob", "/dev/kbm"), c: profile("c", "Cy") },
    agentOrder: ["a", "b", "c"],
    // 프로필 폴더 안(워크트리)이면 프로필 이름을 지키고, 밖으로 나가면 세션 폴더를 따른다.
    taskLabels: { a: { sessionId: "s1", cwd: "/dev/agent-office/.claude/worktrees/x" }, b: { sessionId: "s2", cwd: "/dev/other" } },
  });
  useWorkItemStore.setState({ loaded: true });
  const store = useWorkItemStore.getState();
  store.add({ title: "T", body: "", status: "todo", assigneeAgentId: "a" });
  store.add({ title: "P", body: "", status: "in_progress", assigneeAgentId: "a" });
  store.add({ title: "R", body: "", status: "review", assigneeAgentId: "a" });
  store.select(null);
  render(<UIChrome />);
  const projects = [...document.querySelectorAll(".work-group-project")];
  expect(projects.map((p) => p.textContent)).toEqual(["agent-office", "other", "홈 폴더"]);
  expect(projects[0].getAttribute("title")).toBe("/dev/agent-office");
  const ada = document.querySelector(".work-agent-group")!;
  expect([...ada.querySelectorAll(".work-card")].map((c) => c.className.match(/status-(\w+)/)![1])).toEqual(["review", "in_progress", "todo"]);
  expect(screen.getByLabelText("업무 3개").textContent).toBe("3");
});
