// src/renderer/memo/__tests__/memoCleanup.test.ts
//
// 캐릭터 삭제 → 메모 폴더 정리 브리지(#79). portraitCache/spriteCache와 같은
// 지점(appStore `agents` 구독)이므로 같은 방식으로 검증한다: 사라진 id에 대해
// deleteMemos가 한 번, 남은/새로 생긴 id에 대해서는 불리지 않는다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const deleteMemos = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { deleteMemos: (...a: unknown[]) => deleteMemos(...a) },
}));

const { installMemoCleanup } = await import("../memoCleanup");
const { useAppStore } = await import("../../store/appStore");
import type { AgentProfile } from "../../store/types";

function mkProfile(id: string): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    seed: id,
    createdAt: 1,
    deskIndex: 0,
  };
}

const initialState = useAppStore.getState();
let dispose: (() => void) | null = null;

beforeEach(() => {
  useAppStore.setState(initialState, true);
  deleteMemos.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  dispose?.();
  dispose = null;
});

describe("installMemoCleanup", () => {
  it("사라진 캐릭터의 메모 폴더를 지운다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    useAppStore.getState().addAgent(mkProfile("a2"));
    dispose = installMemoCleanup();

    useAppStore.getState().removeAgent("a1");

    expect(deleteMemos).toHaveBeenCalledTimes(1);
    expect(deleteMemos).toHaveBeenCalledWith("a1");
  });

  it("캐릭터 추가는 아무것도 지우지 않는다", () => {
    dispose = installMemoCleanup();
    useAppStore.getState().addAgent(mkProfile("a1"));
    expect(deleteMemos).not.toHaveBeenCalled();
  });

  it("구독을 해제하면 더 이상 반응하지 않는다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));
    const off = installMemoCleanup();
    off();

    useAppStore.getState().removeAgent("a1");

    expect(deleteMemos).not.toHaveBeenCalled();
  });

  it("삭제 실패는 조용히 흡수한다(콘솔 경고만)", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    deleteMemos.mockRejectedValue(new Error("io"));
    useAppStore.getState().addAgent(mkProfile("a1"));
    dispose = installMemoCleanup();

    useAppStore.getState().removeAgent("a1");
    await vi.waitFor(() => expect(warn).toHaveBeenCalled());
    warn.mockRestore();
  });
});
