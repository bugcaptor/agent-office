// src/renderer/agent/__tests__/terminateSession.test.ts
//
// 터미널 종료 오케스트레이터: disposeSession → clearNotifications 순서와
// 각 단계 실패 관용(restartAgentSession.test.ts의 목 패턴 참고).
// 스토어 상태 전이는 백엔드 disposed 이벤트 → sessionBridge 정규화가 담당
// 하므로 여기서는 tauriApi 호출 배선만 검증한다.
import { beforeEach, describe, expect, it, vi } from "vitest";

const disposeSession = vi.fn().mockResolvedValue(undefined);
const clearNotifications = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    disposeSession: (...args: unknown[]) => disposeSession(...args),
    clearNotifications: (...args: unknown[]) => clearNotifications(...args),
  },
}));

const { terminateAgentSession } = await import("../terminateSession");

beforeEach(() => {
  disposeSession.mockClear();
  clearNotifications.mockClear();
  disposeSession.mockResolvedValue(undefined);
  clearNotifications.mockResolvedValue(undefined);
});

describe("terminateAgentSession 오케스트레이션", () => {
  it("disposeSession → clearNotifications 순서로 호출한다", async () => {
    const order: string[] = [];
    disposeSession.mockImplementationOnce(async (id: string) => {
      order.push(`dispose:${id}`);
    });
    clearNotifications.mockImplementationOnce(async (id: string) => {
      order.push(`clear:${id}`);
    });

    await terminateAgentSession("a1");

    expect(order).toEqual(["dispose:a1", "clear:a1"]);
  });

  it("disposeSession이 실패해도 clearNotifications는 호출된다", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    disposeSession.mockRejectedValueOnce(new Error("no such session"));

    await terminateAgentSession("a1");

    expect(clearNotifications).toHaveBeenCalledWith("a1");
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("clearNotifications가 실패해도 예외를 던지지 않는다", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    clearNotifications.mockRejectedValueOnce(new Error("ipc down"));

    await expect(terminateAgentSession("a1")).resolves.toBeUndefined();

    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });
});
