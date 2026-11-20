// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Discussion, RolesFile } from "@shared/types";

const api = vi.hoisted(() => ({
  loadRoles: vi.fn<() => Promise<RolesFile | null>>(),
  saveRoles: vi.fn().mockResolvedValue(undefined),
  loadDiscussions: vi.fn<() => Promise<Discussion[]>>(),
  saveDiscussion: vi.fn<(d: Discussion) => Promise<void>>(),
  deleteDiscussion: vi.fn<(id: string) => Promise<void>>(),
  workAgentModels: vi.fn().mockResolvedValue([]),
  // 답한 뒤 실행기가 사회자 탭을 열려고 한다. 이 테스트는 열기 실패(멈춤)까지만 본다.
  openDiscussionSession: vi.fn().mockRejectedValue(new Error("work-send-create-failed")),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { ensure: vi.fn(), destroy: vi.fn(), get: vi.fn(), has: vi.fn() } }));
vi.mock("../../workpanels/AgentFace", () => ({ AgentFace: () => null }));

import { useAppStore } from "../../store/appStore";
import { resetRoleStoreForTest } from "../roleStore";
import { resetDiscussionStoreForTest, useDiscussionStore } from "../discussionStore";
import { DiscussionSection } from "../DiscussionSection";

const roles: RolesFile = {
  version: 1,
  roles: [
    { id: "mod", name: "사회자", perspective: "", stance: "moderator", criteria: "" },
    { id: "critic", name: "비판자", perspective: "", stance: "critical", criteria: "", agent: { cli: "codex", model: "gpt" } },
  ],
};
const agent = (id: string, name: string, extra = {}) => ({ id, name, role: "", seed: id, createdAt: 0, deskIndex: 0, ...extra });
const ready: Discussion = {
  id: "d1", topic: "회의실 이름", context: "맥락 글", participants: [
    { agentId: "a1", roleId: "mod", agent: { cli: "claude" } },
    { agentId: "a2", roleId: "gone", agent: { cli: "codex", model: "gpt" } },
  ],
  moderatorAgentId: "a1", limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true },
  state: "ready", round: 0, transcript: [], costUsd: 0, createdAt: 1, updatedAt: 1,
};

beforeEach(() => {
  resetRoleStoreForTest();
  resetDiscussionStoreForTest();
  api.loadRoles.mockReset().mockResolvedValue(structuredClone(roles));
  api.loadDiscussions.mockReset().mockResolvedValue([]);
  api.saveDiscussion.mockReset().mockResolvedValue(undefined);
  api.deleteDiscussion.mockReset().mockResolvedValue(undefined);
  useAppStore.setState({
    agents: {
      a1: agent("a1", "Ada", { defaultRoleId: "mod", cwd: "/work/ada" }),
      a2: agent("a2", "Bo", { defaultRoleId: "critic" }),
      a3: agent("a3", "Cy", { defaultAgent: { cli: "agy" } }),
    },
    agentOrder: ["a1", "a2", "a3"],
  });
});
afterEach(() => cleanup());

const dialog = () => screen.getByRole("dialog");

describe("DiscussionSection", () => {
  it("hides the card group when there is nothing and creates a ready discussion from the dialog", async () => {
    const { container } = render(<DiscussionSection />);
    await waitFor(() => expect(useDiscussionStore.getState().status).toBe("ready"));
    expect(container.querySelector(".discussion-group")).toBeNull();
    act(() => useDiscussionStore.getState().openDialog({ kind: "new" }));
    const topic = await screen.findByPlaceholderText(/무엇을 정하거나/);
    // 사회자는 기본 역할이 사회자인 Ada, 작업 폴더는 Ada의 폴더.
    expect((screen.getByDisplayValue("/work/ada") as HTMLInputElement).value).toBe("/work/ada");
    const create = within(dialog()).getByRole("button", { name: "만들기" }) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
    expect(screen.getByText("주제를 적으세요.")).toBeTruthy();
    fireEvent.change(topic, { target: { value: "저장 방식" } });
    expect(screen.getByText("토론자를 한 명 이상 넣으세요.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "토론자 추가" }));
    fireEvent.click(screen.getByRole("button", { name: "토론자 추가" }));
    expect(screen.getByText("토론자 (2/4)")).toBeTruthy();
    // 세 명이 다 들어가 더 넣을 팀원이 없다.
    expect((screen.getByRole("button", { name: "토론자 추가" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(within(dialog()).getByRole("spinbutton", { name: "최대 라운드" }), { target: { value: "20" } });
    expect(create.disabled).toBe(true);
    fireEvent.change(within(dialog()).getByRole("spinbutton", { name: "최대 라운드" }), { target: { value: "3" } });
    fireEvent.click(create);
    await waitFor(() => expect(api.saveDiscussion).toHaveBeenCalledTimes(1));
    const saved = api.saveDiscussion.mock.calls[0][0];
    expect(saved).toMatchObject({ topic: "저장 방식", state: "ready", moderatorAgentId: "a1", cwd: "/work/ada", limits: { maxRounds: 3, maxMinutes: 30, maxCostUsd: 5, usageGuard: true } });
    expect(saved.participants).toEqual([
      { agentId: "a1", roleId: "mod", agent: { cli: "claude" } },
      { agentId: "a2", roleId: "critic", agent: { cli: "codex", model: "gpt" } },
      { agentId: "a3", agent: { cli: "agy" } },
    ]);
    await waitFor(() => expect(container.querySelector(".discussion-card")).toBeTruthy());
    expect(useDiscussionStore.getState().dialog).toBeNull();
  });

  it("shows a card, opens the detail, edits a ready discussion and deletes it after confirming", async () => {
    api.loadDiscussions.mockResolvedValue([structuredClone(ready)]);
    const { container } = render(<DiscussionSection />);
    const card = await screen.findByRole("button", { name: /회의실 이름/ });
    expect(container.querySelector(".discussion-card small")?.textContent).toBe("최대 4라운드 · 30분 · $5");
    fireEvent.click(card);
    expect(within(dialog()).getByText("맥락 글")).toBeTruthy();
    // 지워진 역할은 "지워진 역할"로 읽는다.
    await waitFor(() => expect(within(dialog()).getByText(/지워진 역할 · codex · gpt/)).toBeTruthy());
    fireEvent.click(within(dialog()).getByRole("button", { name: "고치기" }));
    const topic = await screen.findByDisplayValue("회의실 이름");
    fireEvent.change(topic, { target: { value: "회의실 이름 정하기" } });
    fireEvent.click(within(dialog()).getByRole("button", { name: "저장" }));
    await waitFor(() => expect(api.saveDiscussion).toHaveBeenCalledTimes(1));
    const saved = api.saveDiscussion.mock.calls[0][0];
    expect(saved).toMatchObject({ id: "d1", topic: "회의실 이름 정하기", createdAt: 1 });
    expect(saved.participants).toEqual(ready.participants);
    // 저장하면 보기로 돌아온다.
    fireEvent.click(await within(dialog()).findByRole("button", { name: "삭제" }));
    expect(api.deleteDiscussion).not.toHaveBeenCalled();
    fireEvent.click(within(dialog()).getByRole("button", { name: "정말 삭제" }));
    await waitFor(() => expect(api.deleteDiscussion).toHaveBeenCalledWith("d1"));
    await waitFor(() => expect(container.querySelector(".discussion-card")).toBeNull());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("does not offer edit or delete while a discussion is running", async () => {
    api.loadDiscussions.mockResolvedValue([{ ...structuredClone(ready), state: "running", round: 2 }]);
    render(<DiscussionSection />);
    fireEvent.click(await screen.findByRole("button", { name: /회의실 이름/ }));
    expect(within(dialog()).getAllByText("2/4 라운드").length).toBeGreaterThan(0);
    expect(within(dialog()).queryByRole("button", { name: "고치기" })).toBeNull();
    expect(within(dialog()).queryByRole("button", { name: "삭제" })).toBeNull();
  });
});

describe("DiscussionSection run controls", () => {
  const open = async () => fireEvent.click(await screen.findByRole("button", { name: /회의실 이름/ }));

  it("offers start for a ready discussion and explains why it is blocked", async () => {
    api.loadDiscussions.mockResolvedValue([structuredClone(ready)]);
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, observerEnabled: false } }));
    render(<DiscussionSection />);
    await open();
    expect((within(dialog()).getByRole("button", { name: "시작" }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(dialog()).getByText(/관찰 훅이 꺼져/)).toBeTruthy();
    act(() => useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, observerEnabled: true } })));
    expect((within(dialog()).getByRole("button", { name: "시작" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("highlights a card whose moderator asked the person and takes the answer from the detail", async () => {
    const waiting: Discussion = { ...structuredClone(ready), state: "waiting_user", round: 2, run: { phase: "user", turns: [], seen: {}, question: "예산은 얼마인가요?" } };
    api.loadDiscussions.mockResolvedValue([waiting]);
    render(<DiscussionSection />);
    const card = (await screen.findByRole("button", { name: /회의실 이름/ })).closest("article")!;
    expect(card.classList.contains("attention")).toBe(true);
    fireEvent.click(within(card).getByRole("button", { name: /회의실 이름/ }));
    expect(within(dialog()).getByText("예산은 얼마인가요?")).toBeTruthy();
    const submit = within(dialog()).getByRole("button", { name: "답하기" }) as HTMLButtonElement;
    expect(submit.disabled).toBe(true);
    fireEvent.change(within(dialog()).getByRole("textbox", { name: "답" }), { target: { value: "10만 원" } });
    fireEvent.click(submit);
    const d = useDiscussionStore.getState().discussions[0];
    expect(d.transcript.slice(-1)[0]).toMatchObject({ kind: "answer", text: "10만 원" });
    expect(d.run?.phase).toBe("moderator");
  });

  it("shows why a discussion paused and offers resume and end", async () => {
    const paused: Discussion = { ...structuredClone(ready), state: "paused", round: 1, run: { phase: "opening", turns: [], seen: {}, pauseReason: "human-input" } };
    api.loadDiscussions.mockResolvedValue([paused]);
    render(<DiscussionSection />);
    await open();
    expect(within(dialog()).getByText(/쓰다 만 입력/)).toBeTruthy();
    expect(within(dialog()).getByRole("button", { name: "계속" })).toBeTruthy();
    fireEvent.click(within(dialog()).getByRole("button", { name: "끝내기" }));
    expect(within(dialog()).getByRole("button", { name: "정말 끝내기" })).toBeTruthy();
  });
});

describe("DiscussionEditor stray roles", () => {
  it("keeps a saved role that no longer fits its seat visible by name", async () => {
    // a2의 역할 critic이 나중에 사회자 스탠스로 바뀐 경우: 토론자 칸 선택지에는 없지만 값은 남는다.
    api.loadRoles.mockResolvedValue({ version: 1, roles: [roles.roles[0], { ...roles.roles[1], stance: "moderator" }] });
    api.loadDiscussions.mockResolvedValue([{ ...structuredClone(ready), participants: [ready.participants[0], { ...ready.participants[1], roleId: "critic" }] }]);
    render(<DiscussionSection />);
    await waitFor(() => expect(useDiscussionStore.getState().status).toBe("ready"));
    act(() => useDiscussionStore.getState().openDialog({ kind: "edit", id: "d1" }));
    await screen.findByDisplayValue("회의실 이름");
    const roleSelects = within(dialog()).getAllByRole("combobox", { name: "역할" }) as HTMLSelectElement[];
    expect(roleSelects[1].value).toBe("critic");
    expect(roleSelects[1].selectedOptions[0].textContent).toBe("비판자");
  });
});
