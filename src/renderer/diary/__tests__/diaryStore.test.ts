// src/renderer/diary/__tests__/diaryStore.test.ts
//
// 일기 오버레이 스토어: 열면 load 트리거, stale 로드 무시, writeNow가 성공 시
// refresh하고 실패 사유를 안내 문구로 매핑. tauriApi.loadDiary와
// generateDiary를 목으로 검증.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { loadDiary: vi.fn() },
}));
vi.mock("../diaryGenerator", () => ({ generateDiary: vi.fn() }));

import { useDiaryStore } from "../diaryStore";
import { tauriApi } from "../../ipc/tauriApi";
import { generateDiary } from "../diaryGenerator";
import type { DiaryEntry } from "@shared/types";

const loadDiary = tauriApi.loadDiary as unknown as ReturnType<typeof vi.fn>;
const genDiary = generateDiary as unknown as ReturnType<typeof vi.fn>;

function entry(at: number, body: string): DiaryEntry {
  return { at, sessionId: "s1", body };
}

beforeEach(() => {
  loadDiary.mockReset();
  genDiary.mockReset();
  useDiaryStore.setState({
    overlay: null,
    entries: [],
    loading: false,
    generating: false,
    notice: null,
  });
});

describe("openDiary / refresh", () => {
  it("열면 오버레이를 세팅하고 일기를 로드한다", async () => {
    loadDiary.mockResolvedValue([entry(1, "가"), entry(2, "나")]);
    useDiaryStore.getState().openDiary("a1", "컴파일러");
    expect(useDiaryStore.getState().overlay).toEqual({ agentId: "a1", agentName: "컴파일러" });
    await vi.waitFor(() => expect(useDiaryStore.getState().loading).toBe(false));
    expect(useDiaryStore.getState().entries).toHaveLength(2);
    expect(loadDiary).toHaveBeenCalledWith("a1");
  });

  it("로드 완료 전에 다른 캐릭터로 바뀌면 stale 결과를 무시한다", async () => {
    let resolveFirst!: (v: DiaryEntry[]) => void;
    loadDiary.mockImplementationOnce(() => new Promise((r) => (resolveFirst = r)));
    loadDiary.mockResolvedValueOnce([entry(9, "새 캐릭터")]);

    useDiaryStore.getState().openDiary("a1", "A");
    useDiaryStore.getState().openDiary("a2", "B"); // 두 번째 로드 시작
    await vi.waitFor(() => expect(useDiaryStore.getState().entries).toEqual([entry(9, "새 캐릭터")]));

    resolveFirst([entry(1, "옛 캐릭터")]); // 늦게 도착한 a1 결과
    // a1은 더 이상 열린 타깃이 아니므로 반영되지 않아야 한다.
    await Promise.resolve();
    expect(useDiaryStore.getState().entries).toEqual([entry(9, "새 캐릭터")]);
  });
});

describe("writeNow", () => {
  it("성공하면 안내를 세팅하고 refresh한다", async () => {
    useDiaryStore.setState({ overlay: { agentId: "a1", agentName: "A" } });
    genDiary.mockResolvedValue({ ok: true, entry: entry(3, "새 일기") });
    loadDiary.mockResolvedValue([entry(3, "새 일기")]);

    await useDiaryStore.getState().writeNow("a1");

    expect(genDiary).toHaveBeenCalledWith("a1");
    expect(useDiaryStore.getState().notice).toMatch(/썼습니다/);
    expect(useDiaryStore.getState().entries).toEqual([entry(3, "새 일기")]);
  });

  it("실패 사유를 사람이 읽는 안내로 매핑하고 refresh하지 않는다", async () => {
    useDiaryStore.setState({ overlay: { agentId: "a1", agentName: "A" } });
    genDiary.mockResolvedValue({ ok: false, reason: "disabled" });

    await useDiaryStore.getState().writeNow("a1");

    expect(useDiaryStore.getState().notice).toMatch(/설정/);
    expect(loadDiary).not.toHaveBeenCalled();
  });

  it("생성 중이면 두 번째 writeNow는 무시한다", async () => {
    useDiaryStore.setState({ overlay: { agentId: "a1", agentName: "A" }, generating: true });
    await useDiaryStore.getState().writeNow("a1");
    expect(genDiary).not.toHaveBeenCalled();
  });
});
