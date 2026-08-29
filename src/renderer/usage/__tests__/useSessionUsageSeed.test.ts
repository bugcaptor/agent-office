// @vitest-environment jsdom
//
// src/renderer/usage/__tests__/useSessionUsageSeed.test.ts
//
// useSessionUsageSeed 배선 검증(docs/session-analytics-design.md §11.3·11.5,
// 코드 리뷰 A/B/E 수정). 모듈 스코프 `attempted` 플래그가 앱 수명당 1회를
// 강제하므로, 테스트마다 `vi.resetModules()`로 훅과 스토어를 둘 다 새로
// import해 격리한다(둘이 같은 `useAppStore` 인스턴스를 봐야 하므로 훅과
// 스토어를 항상 같은 `load()` 호출에서 함께 가져온다).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook } from "@testing-library/react";

const { loadSessionEvents } = vi.hoisted(() => ({ loadSessionEvents: vi.fn() }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { loadSessionEvents } }));

beforeEach(() => {
  vi.resetModules();
  loadSessionEvents.mockReset();
  loadSessionEvents.mockResolvedValue([]);
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** 훅과 스토어를 같은 모듈 인스턴스로 함께 가져온다(위 헤더 주석 참조). */
async function load() {
  const { useAppStore } = await import("../../store/appStore");
  const { useSessionUsageSeed, SEED_WINDOW_MS } = await import("../useSessionUsageSeed");
  return { useAppStore, useSessionUsageSeed, SEED_WINDOW_MS };
}

/** 훅의 setTimeout(0) + 그 뒤 프라미스 체인을 한 번에 흘려보낸다. */
async function flush() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(0);
  });
}

describe("useSessionUsageSeed", () => {
  it("설정(sessionCostEnabled)이 꺼져 있으면 loadSessionEvents를 부르지 않는다", async () => {
    const { useAppStore, useSessionUsageSeed } = await load();
    useAppStore.setState({
      settingsHydrated: true,
      appSettings: { ...useAppStore.getState().appSettings, sessionCostEnabled: false },
    });

    renderHook(() => useSessionUsageSeed());
    await flush();

    expect(loadSessionEvents).not.toHaveBeenCalled();
  });

  it("하이드레이트 전에는 시딩하지 않고, 하이드레이트 후에 시딩한다", async () => {
    const { useAppStore, useSessionUsageSeed } = await load();
    // 초기 상태: settingsHydrated=false(스토어 생성 시점 기본값).
    expect(useAppStore.getState().settingsHydrated).toBe(false);

    const { rerender } = renderHook(() => useSessionUsageSeed());
    await flush();
    expect(loadSessionEvents).not.toHaveBeenCalled();

    act(() => {
      useAppStore.getState().hydrateSettings(useAppStore.getState().appSettings, false);
    });
    rerender();
    await flush();

    expect(loadSessionEvents).toHaveBeenCalledTimes(1);
  });

  it("앱 수명당 1회만 부른다(하이드레이트/상태 변경이 반복돼도 재호출 없음)", async () => {
    const { useAppStore, useSessionUsageSeed } = await load();
    useAppStore.setState({ settingsHydrated: true });

    const { rerender } = renderHook(() => useSessionUsageSeed());
    await flush();
    expect(loadSessionEvents).toHaveBeenCalledTimes(1);

    // 다시 하이드레이트하거나 다른 상태가 바뀌어도 재시도하지 않는다.
    act(() => {
      useAppStore.getState().hydrateSettings(useAppStore.getState().appSettings, false);
    });
    rerender();
    await flush();

    expect(loadSessionEvents).toHaveBeenCalledTimes(1);
  });

  it("sessionUsageFirstAt이 있으면 firstAt-1을 컷오프로 써서 그 이전 SEED_WINDOW_MS만 조회한다", async () => {
    const { useAppStore, useSessionUsageSeed, SEED_WINDOW_MS } = await load();
    useAppStore.setState({ settingsHydrated: true, sessionUsageFirstAt: 5000 });

    renderHook(() => useSessionUsageSeed());
    await flush();

    expect(loadSessionEvents).toHaveBeenCalledWith(5000 - 1 - SEED_WINDOW_MS, 5000 - 1, ["stop", "usage"]);
  });

  it("firstAt이 없으면 지금(Date.now())을 컷오프로 써서 조회한다", async () => {
    const now = 1_800_000_000_000;
    vi.setSystemTime(now);
    const { useAppStore, useSessionUsageSeed, SEED_WINDOW_MS } = await load();
    useAppStore.setState({ settingsHydrated: true });

    renderHook(() => useSessionUsageSeed());
    await flush();

    expect(loadSessionEvents).toHaveBeenCalledWith(now - SEED_WINDOW_MS, now, ["stop", "usage"]);
  });

  it("loadSessionEvents가 reject해도 조용히 삼키고 재시도하지 않는다", async () => {
    loadSessionEvents.mockRejectedValueOnce(new Error("boom"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { useAppStore, useSessionUsageSeed } = await load();
    useAppStore.setState({ settingsHydrated: true });

    const { rerender } = renderHook(() => useSessionUsageSeed());
    await flush();
    // reject 처리(.catch)까지 마이크로태스크 한 틱 더.
    await act(async () => {
      await Promise.resolve();
    });

    expect(useAppStore.getState().sessionUsageSeed).toBeNull();
    expect(warn).toHaveBeenCalled();

    rerender();
    await flush();
    expect(loadSessionEvents).toHaveBeenCalledTimes(1); // 재시도 없음

    warn.mockRestore();
  });
});
