// @vitest-environment jsdom
//
// src/renderer/talk/__tests__/TalkLogDialog.test.tsx
//
// 대화 감사 로그 다이얼로그(docs/agent-talk-design.md §7):
// - self-gate(모달이 talk-log가 아니면 null 렌더), Esc/닫기 버튼으로 닫힘.
// - 날짜 목록에서 최신 날짜를 자동으로 고르고, 다른 날짜를 고르면 다시 읽는다.
// - 로그를 대화(convId) 단위로 묶어 보여주고, 받는 쪽 이름은 프로필로 푼다.
// - 만료 줄은 "전달 실패(만료)"로 표시, 로그가 없으면 빈 상태 문구.
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TalkLogEntry } from "@shared/types";
import type { AgentProfile } from "../../store/types";

const listTalkLogDates = vi.fn();
const readTalkLog = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    listTalkLogDates: (...a: unknown[]) => listTalkLogDates(...a),
    readTalkLog: (...a: unknown[]) => readTalkLog(...a),
  },
}));

const { TalkLogDialog } = await import("../TalkLogDialog");
const { useAppStore } = await import("../../store/appStore");

const initialState = useAppStore.getState();

function mkProfile(id: string, name: string): AgentProfile {
  return {
    id,
    name,
    role: "eng",
    seed: id,
    createdAt: 0,
    deskIndex: 0,
  };
}

function entry(over: Partial<TalkLogEntry> & { convId: string; at: number }): TalkLogEntry {
  return {
    kind: "send",
    id: `m${over.at}`,
    from: "a1",
    fromName: "하나",
    to: "a2",
    text: "본문",
    ...over,
  };
}

function openDialog(): void {
  useAppStore.setState({
    agents: { a1: mkProfile("a1", "하나"), a2: mkProfile("a2", "두리") },
    modal: { kind: "talk-log" },
  });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  listTalkLogDates.mockReset();
  readTalkLog.mockReset();
  listTalkLogDates.mockResolvedValue([]);
  readTalkLog.mockResolvedValue([]);
});

afterEach(() => cleanup());

describe("TalkLogDialog: 게이팅", () => {
  it("모달이 talk-log가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<TalkLogDialog />);
    expect(container.textContent).toBe("");
    expect(listTalkLogDates).not.toHaveBeenCalled();
  });

  it("닫기 버튼이 모달을 닫는다", async () => {
    openDialog();
    const { getByText } = render(<TalkLogDialog />);
    await waitFor(() => expect(listTalkLogDates).toHaveBeenCalled());
    fireEvent.click(getByText("닫기"));
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("Esc로 닫힌다", async () => {
    openDialog();
    render(<TalkLogDialog />);
    await waitFor(() => expect(listTalkLogDates).toHaveBeenCalled());
    fireEvent.keyDown(window, { key: "Escape" });
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});

describe("TalkLogDialog: 빈 상태", () => {
  it("로그가 아예 없으면 빈 상태 문구", async () => {
    openDialog();
    const { findByText } = render(<TalkLogDialog />);
    expect(await findByText("기록된 대화가 없습니다.")).toBeTruthy();
    expect(readTalkLog).not.toHaveBeenCalled(); // 고를 날짜가 없으면 읽지 않는다
  });

  it("날짜는 있는데 그 날 로그가 비었으면 역시 빈 상태 문구", async () => {
    openDialog();
    listTalkLogDates.mockResolvedValue(["2026-08-22"]);
    const { findByText } = render(<TalkLogDialog />);
    expect(await findByText("기록된 대화가 없습니다.")).toBeTruthy();
  });
});

describe("TalkLogDialog: 대화 단위 묶기", () => {
  it("뒤섞인 줄을 convId별 묶음으로 나누고 받는 쪽 이름을 프로필로 푼다", async () => {
    openDialog();
    listTalkLogDates.mockResolvedValue(["2026-08-22", "2026-08-21"]);
    readTalkLog.mockResolvedValue([
      entry({ convId: "cB", at: 200, text: "두 번째 대화" }),
      entry({ convId: "cA", at: 100, text: "첫 대화 첫 줄" }),
      entry({ convId: "cA", at: 300, kind: "deliver", text: "첫 대화 둘째 줄" }),
    ]);
    const { container, findByText } = render(<TalkLogDialog />);
    await findByText("첫 대화 첫 줄");

    const convs = container.querySelectorAll(".talk-log-conv");
    expect(convs.length).toBe(2);
    // 시작 시각 순: cA(100) → cB(200).
    expect(convs[0].textContent).toContain("conv=cA");
    expect(convs[0].textContent).toContain("첫 대화 첫 줄");
    expect(convs[0].textContent).toContain("첫 대화 둘째 줄");
    expect(convs[1].textContent).toContain("conv=cB");
    // 받는 쪽 이름(a2 -> 두리)은 로그에 없어 프로필로 푼다.
    expect(convs[0].textContent).toContain("하나 → 두리");
  });

  it("만료 줄은 '전달 실패(만료)'로 표시한다", async () => {
    openDialog();
    listTalkLogDates.mockResolvedValue(["2026-08-22"]);
    readTalkLog.mockResolvedValue([entry({ convId: "cA", at: 100, kind: "expire" })]);
    const { findByText } = render(<TalkLogDialog />);
    expect(await findByText("전달 실패(만료)")).toBeTruthy();
  });

  it("프로필에 없는 수신자는 agentId를 그대로 보여준다", async () => {
    openDialog();
    listTalkLogDates.mockResolvedValue(["2026-08-22"]);
    readTalkLog.mockResolvedValue([entry({ convId: "cA", at: 100, to: "삭제된-id" })]);
    const { findByText } = render(<TalkLogDialog />);
    expect(await findByText("하나 → 삭제된-id")).toBeTruthy();
  });
});

describe("TalkLogDialog: 날짜 선택", () => {
  it("가장 최신 날짜를 자동으로 고르고, 다른 날짜를 고르면 그 날을 다시 읽는다", async () => {
    openDialog();
    listTalkLogDates.mockResolvedValue(["2026-08-22", "2026-08-21"]);
    readTalkLog.mockResolvedValue([entry({ convId: "cA", at: 100 })]);
    const { findByText, getByText } = render(<TalkLogDialog />);
    await findByText("본문");
    expect(readTalkLog).toHaveBeenLastCalledWith("2026-08-22");

    fireEvent.click(getByText("2026-08-21"));
    await waitFor(() => expect(readTalkLog).toHaveBeenLastCalledWith("2026-08-21"));
  });
});
