// @vitest-environment jsdom
//
// src/renderer/talk/__tests__/useTalkStatus.test.ts
//
// 동료 대화(docs/agent-talk-design.md §7) 상태 폴링 훅:
// - enabled가 꺼져 있으면 조회도, 8초 재폴링도 하지 않고 {open:0, queued:0}.
// - 켜져 있으면 즉시 1회 조회하고, 8초마다 재폴링한다.
// - 열린 대화 수는 `ended`가 붙지 않은 대화만 센다.
// - 조회 실패는 console.warn만 남기고 이전 값을 유지한다.
import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TalkStatus } from "@shared/types";

const talkStatus = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    talkStatus: (...a: unknown[]) => talkStatus(...a),
  },
}));

const { openConversationCount, useTalkStatus } = await import("../useTalkStatus");

function status(overrides: Partial<TalkStatus> = {}): TalkStatus {
  return { enabled: true, queued: 0, conversations: [], ...overrides };
}

beforeEach(() => {
  talkStatus.mockReset();
  talkStatus.mockResolvedValue(status());
});

afterEach(() => cleanup());

describe("openConversationCount", () => {
  it("끝난(ended) 대화는 빼고 센다", () => {
    const count = openConversationCount({
      conversations: [
        { id: "c1", ended: undefined },
        { id: "c2", ended: "max-turns" },
        { id: "c3" },
      ] as never,
    });
    expect(count).toBe(2);
  });
});

describe("useTalkStatus: enabled 게이트", () => {
  it("false면 조회하지 않고 {open:0, queued:0}을 반환한다", () => {
    const { result } = renderHook(() => useTalkStatus(false));
    expect(talkStatus).not.toHaveBeenCalled();
    expect(result.current).toEqual({ open: 0, queued: 0 });
  });

  it("true면 즉시 조회해 열린 대화 수/대기 수를 반영한다(끝난 대화는 제외)", async () => {
    talkStatus.mockResolvedValue(
      status({
        queued: 3,
        conversations: [
          { id: "c1", a: "a1", b: "a2", turns: 2, startedAt: 1 },
          { id: "c2", a: "a1", b: "a3", turns: 6, startedAt: 2, ended: "max-turns" },
        ],
      }),
    );
    const { result } = renderHook(() => useTalkStatus(true));
    await waitFor(() => expect(result.current).toEqual({ open: 1, queued: 3 }));
  });
});

describe("useTalkStatus: 폴링 주기", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => vi.useRealTimers());

  it("8초마다 재폴링한다", async () => {
    const { rerender } = renderHook(({ enabled }) => useTalkStatus(enabled), {
      initialProps: { enabled: true },
    });
    await vi.waitFor(() => expect(talkStatus).toHaveBeenCalledTimes(1));
    await vi.advanceTimersByTimeAsync(8_000);
    expect(talkStatus).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(8_000);
    expect(talkStatus).toHaveBeenCalledTimes(3);
    rerender({ enabled: false });
  });

  it("enabled가 꺼지면 재폴링을 멈춘다", async () => {
    const { rerender } = renderHook(({ enabled }) => useTalkStatus(enabled), {
      initialProps: { enabled: true },
    });
    await vi.waitFor(() => expect(talkStatus).toHaveBeenCalledTimes(1));
    rerender({ enabled: false });
    await vi.advanceTimersByTimeAsync(16_000);
    expect(talkStatus).toHaveBeenCalledTimes(1);
  });
});

describe("useTalkStatus: 조회 실패", () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }));
  afterEach(() => vi.useRealTimers());

  it("console.warn만 남기고 이전 값을 유지한다", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    talkStatus
      .mockResolvedValueOnce(status({ queued: 2 }))
      .mockRejectedValueOnce(new Error("boom"));
    const { result } = renderHook(() => useTalkStatus(true));
    await vi.waitFor(() => expect(result.current.queued).toBe(2));

    // 두 번째(8초 뒤) 폴링은 실패하지만 이전 값(queued: 2)이 유지돼야 한다.
    await vi.advanceTimersByTimeAsync(8_000);
    expect(talkStatus).toHaveBeenCalledTimes(2);
    expect(result.current.queued).toBe(2);
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
