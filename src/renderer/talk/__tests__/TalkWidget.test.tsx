// @vitest-environment jsdom
//
// src/renderer/talk/__tests__/TalkWidget.test.tsx
//
// 하단바 동료 대화 표시(docs/agent-talk-design.md §7):
// - talkEnabled가 꺼져 있으면 아예 렌더되지 않고 상태 조회도 하지 않는다.
// - 켜져 있으면 열린 대화 수를 보여준다(끝난 대화는 세지 않는다).
// - 메뉴의 "대화 로그 보기"가 talk-log 모달을 연다.
// - "대화 전체 중지"가 talkEnabled:false로 저장한다(킬스위치).
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TalkStatus } from "@shared/types";

const talkStatus = vi.fn();
const setAppSettings = vi.fn((_settings: unknown) => Promise.resolve());
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    talkStatus: (...a: unknown[]) => talkStatus(...a),
    setAppSettings: (settings: unknown) => setAppSettings(settings),
  },
}));

const { TalkWidget } = await import("../TalkWidget");
const { useAppStore } = await import("../../store/appStore");

const initialState = useAppStore.getState();

function enableTalk(): void {
  useAppStore.setState({
    appSettings: { ...initialState.appSettings, talkEnabled: true },
  });
}

function status(overrides: Partial<TalkStatus> = {}): TalkStatus {
  return { enabled: true, queued: 0, conversations: [], ...overrides };
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  talkStatus.mockReset();
  talkStatus.mockResolvedValue(status());
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("TalkWidget: 표시 게이트", () => {
  it("talkEnabled가 꺼져 있으면 렌더도 상태 조회도 하지 않는다", () => {
    const { container } = render(<TalkWidget />);
    expect(container.textContent).toBe("");
    expect(talkStatus).not.toHaveBeenCalled();
  });

  it("켜져 있으면 열린 대화 수를 보여준다(끝난 대화는 제외)", async () => {
    enableTalk();
    talkStatus.mockResolvedValue(
      status({
        conversations: [
          { id: "c1", a: "a1", b: "a2", turns: 2, startedAt: 1 },
          { id: "c2", a: "a1", b: "a3", turns: 6, startedAt: 2, ended: "max-turns" },
        ],
      }),
    );
    const { getByRole } = render(<TalkWidget />);
    await waitFor(() => expect(getByRole("button").textContent).toContain("대화 1"));
  });

  it("전달 대기 건수가 있으면 덧붙인다", async () => {
    enableTalk();
    talkStatus.mockResolvedValue(status({ queued: 3 }));
    const { getByRole } = render(<TalkWidget />);
    await waitFor(() => expect(getByRole("button").textContent).toContain("+3"));
  });
});

describe("TalkWidget: 메뉴", () => {
  it("'대화 로그 보기'가 talk-log 모달을 연다", async () => {
    enableTalk();
    const { getByRole, getByText } = render(<TalkWidget />);
    await waitFor(() => expect(talkStatus).toHaveBeenCalled());
    fireEvent.click(getByRole("button", { name: "동료 대화" }));
    fireEvent.click(getByText("대화 로그 보기"));
    expect(useAppStore.getState().modal).toEqual({ kind: "talk-log" });
  });

  it("'대화 전체 중지'가 talkEnabled:false로 저장한다", async () => {
    enableTalk();
    const { getByRole, getByText } = render(<TalkWidget />);
    await waitFor(() => expect(talkStatus).toHaveBeenCalled());
    fireEvent.click(getByRole("button", { name: "동료 대화" }));
    fireEvent.click(getByText("대화 전체 중지"));
    expect(useAppStore.getState().appSettings.talkEnabled).toBe(false);
    expect(setAppSettings).toHaveBeenCalledWith(
      expect.objectContaining({ talkEnabled: false }),
    );
  });

  it("중지 후에는 표시 자체가 사라진다", async () => {
    enableTalk();
    const { getByRole, getByText, container } = render(<TalkWidget />);
    await waitFor(() => expect(talkStatus).toHaveBeenCalled());
    fireEvent.click(getByRole("button", { name: "동료 대화" }));
    fireEvent.click(getByText("대화 전체 중지"));
    await waitFor(() => expect(container.textContent).toBe(""));
  });
});
