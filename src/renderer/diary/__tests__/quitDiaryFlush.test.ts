// src/renderer/diary/__tests__/quitDiaryFlush.test.ts
//
// 앱 종료 flush 오케스트레이션(#60): 대상 선별(pendingDiaryAgents)과 flush 실행
// (runQuitDiaryFlush) — 시작·종료 flushNow(캔슬 안전망·happy path 정리), 대상별
// flushAgent 호출, 진행 콜백, 실패 삼킴. flusher/persister는 주입.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { loadDiary: vi.fn().mockResolvedValue([]) },
}));

import { useAppStore } from "../../store/appStore";
import {
  pendingDiaryAgents,
  runQuitDiaryFlush,
  type QuitDiaryFlushDeps,
} from "../quitDiaryFlush";
import type { DiaryFlusher } from "../diaryFlusher";

function fakeFlusher(over: Partial<DiaryFlusher> = {}): DiaryFlusher {
  return {
    hasPendingWork: vi.fn().mockReturnValue(false),
    flushAgent: vi.fn().mockResolvedValue(undefined),
    ...over,
  } as unknown as DiaryFlusher;
}

describe("pendingDiaryAgents", () => {
  beforeEach(() => {
    useAppStore.setState({ agentOrder: ["a1", "a2", "a3"] });
  });

  it("hasPendingWork가 참인 캐릭터만, agentOrder 순서로 고른다", () => {
    const flusher = fakeFlusher({
      hasPendingWork: vi.fn((id: string) => id !== "a2"),
    });
    expect(pendingDiaryAgents(flusher)).toEqual(["a1", "a3"]);
  });

  it("아무도 자격이 없으면 빈 배열(=즉시 종료 신호)", () => {
    expect(pendingDiaryAgents(fakeFlusher())).toEqual([]);
  });
});

describe("runQuitDiaryFlush", () => {
  let flushNow: ReturnType<typeof vi.fn>;
  let deps: QuitDiaryFlushDeps;
  let flushAgent: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    flushNow = vi.fn().mockResolvedValue(undefined);
    flushAgent = vi.fn().mockResolvedValue(undefined);
    deps = {
      flusher: fakeFlusher({ flushAgent }),
      persister: () => ({ flushNow, dispose: vi.fn() }),
    };
  });

  it("각 대상에 quit flush를 돌리고 진행률을 보고한다", async () => {
    const progress: Array<[number, number]> = [];
    await runQuitDiaryFlush(["a1", "a2"], { ...deps, onProgress: (d, t) => progress.push([d, t]) });

    expect(flushAgent).toHaveBeenCalledWith("a1", { includeLive: false, source: "quit" });
    expect(flushAgent).toHaveBeenCalledWith("a2", { includeLive: false, source: "quit" });
    expect(progress[0]).toEqual([0, 2]);
    expect(progress[progress.length - 1]).toEqual([2, 2]);
  });

  it("시작과 끝에 flushNow로 작업 로그를 디스크에 안착시킨다", async () => {
    await runQuitDiaryFlush(["a1"], deps);
    expect(flushNow).toHaveBeenCalledTimes(2);
  });

  it("대상이 없어도 flushNow는 부른다(캔슬 안전망)", async () => {
    await runQuitDiaryFlush([], deps);
    expect(flushNow).toHaveBeenCalledTimes(2);
    expect(flushAgent).not.toHaveBeenCalled();
  });

  it("한 캐릭터 flush가 던져도 나머지를 계속하고 완료한다", async () => {
    flushAgent.mockImplementation((id: string) =>
      id === "a1" ? Promise.reject(new Error("boom")) : Promise.resolve(),
    );
    await expect(runQuitDiaryFlush(["a1", "a2"], deps)).resolves.toBeUndefined();
    expect(flushAgent).toHaveBeenCalledTimes(2);
  });

  it("persister가 없어도(설치 전) 안전하게 동작한다", async () => {
    await expect(
      runQuitDiaryFlush(["a1"], { flusher: fakeFlusher({ flushAgent }), persister: () => null }),
    ).resolves.toBeUndefined();
    expect(flushAgent).toHaveBeenCalledWith("a1", { includeLive: false, source: "quit" });
  });
});
