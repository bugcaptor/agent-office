// src/renderer/diary/__tests__/diaryStore.test.ts
//
// 일기 오버레이 스토어: 열면 load 트리거, stale 로드 무시, writeNow가 성공 시
// refresh하고 실패 사유를 안내 문구로 매핑. tauriApi.loadDiary와
// generateDiary를 목으로 검증.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { loadDiary: vi.fn(), exportDiaryFile: vi.fn() },
}));
vi.mock("../diaryGenerator", () => ({ generateDiary: vi.fn() }));
// 공유 flusher는 목으로 — 백필 트리거만 검증하고, flush 자체는 diaryFlusher.test가 커버.
const hasPendingWork = vi.fn().mockReturnValue(false);
const flushAgent = vi.fn().mockResolvedValue(undefined);
vi.mock("../diaryFlusher", () => ({
  sharedDiaryFlusher: () => ({ hasPendingWork, flushAgent }),
}));

import { useDiaryStore } from "../diaryStore";
import { tauriApi } from "../../ipc/tauriApi";
import { generateDiary } from "../diaryGenerator";
import type { DiaryEntry } from "@shared/types";

const loadDiary = tauriApi.loadDiary as unknown as ReturnType<typeof vi.fn>;
const exportDiaryFile = tauriApi.exportDiaryFile as unknown as ReturnType<typeof vi.fn>;
const genDiary = generateDiary as unknown as ReturnType<typeof vi.fn>;

function entry(at: number, body: string): DiaryEntry {
  return { at, sessionId: "s1", body };
}

beforeEach(() => {
  loadDiary.mockReset();
  exportDiaryFile.mockReset().mockResolvedValue("/tmp/a.md");
  genDiary.mockReset();
  hasPendingWork.mockReset().mockReturnValue(false);
  flushAgent.mockReset().mockResolvedValue(undefined);
  useDiaryStore.setState({
    overlay: null,
    entries: [],
    loading: false,
    generating: false,
    backfilling: false,
    exporting: false,
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

  it("밀린(종료) 세션이 있으면 백필을 돌리고 끝나면 목록을 갱신한다", async () => {
    hasPendingWork.mockReturnValue(true);
    loadDiary.mockResolvedValue([entry(1, "복원분")]);

    useDiaryStore.getState().openDiary("a1", "컴파일러");
    // 백필 시작 → 배지 ON.
    await vi.waitFor(() => expect(useDiaryStore.getState().backfilling).toBe(true));
    expect(flushAgent).toHaveBeenCalledWith("a1", { includeLive: false, source: "open-diary" });
    // 완료 → 배지 OFF + refresh.
    await vi.waitFor(() => expect(useDiaryStore.getState().backfilling).toBe(false));
    expect(loadDiary).toHaveBeenCalledWith("a1");
  });

  it("밀린 세션이 없으면 백필도 배지도 없다", async () => {
    hasPendingWork.mockReturnValue(false);
    loadDiary.mockResolvedValue([]);
    useDiaryStore.getState().openDiary("a1", "컴파일러");
    await vi.waitFor(() => expect(useDiaryStore.getState().loading).toBe(false));
    expect(flushAgent).not.toHaveBeenCalled();
    expect(useDiaryStore.getState().backfilling).toBe(false);
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

describe("exportNow", () => {
  function openWith(entries: DiaryEntry[]) {
    useDiaryStore.setState({ overlay: { agentId: "a1", agentName: "컴파일러" }, entries });
  }

  it("두 포맷을 만들어 넘기고 저장 경로를 안내한다", async () => {
    openWith([entry(Date.UTC(2026, 6, 20), "첫 일기")]);
    exportDiaryFile.mockResolvedValue("/Users/me/컴파일러-일기.md");

    await useDiaryStore.getState().exportNow("a1");

    expect(exportDiaryFile).toHaveBeenCalledTimes(1);
    const [defaultName, markdown, json] = exportDiaryFile.mock.calls[0];
    expect(defaultName).toMatch(/^컴파일러-일기-\d{8}-\d{4}\.md$/);
    expect(markdown).toContain("# 컴파일러의 일기");
    expect(markdown).toContain("첫 일기");
    expect(JSON.parse(json)).toMatchObject({ kind: "agent-office.diary", agentName: "컴파일러" });
    expect(useDiaryStore.getState().notice).toContain("/Users/me/컴파일러-일기.md");
    expect(useDiaryStore.getState().exporting).toBe(false);
  });

  it("일기가 없으면 호출하지 않고 안내만 한다", async () => {
    openWith([]);
    await useDiaryStore.getState().exportNow("a1");
    expect(exportDiaryFile).not.toHaveBeenCalled();
    expect(useDiaryStore.getState().notice).toMatch(/내보낼 일기가 없습니다/);
  });

  it("다이얼로그 취소는 실패가 아니라 취소로 안내한다", async () => {
    openWith([entry(1, "일기")]);
    exportDiaryFile.mockResolvedValue(null);
    await useDiaryStore.getState().exportNow("a1");
    expect(useDiaryStore.getState().notice).toMatch(/취소/);
  });

  it("쓰기 실패는 안내로 잡고 진행 플래그를 되돌린다", async () => {
    openWith([entry(1, "일기")]);
    exportDiaryFile.mockRejectedValue(new Error("디스크 가득참"));
    await useDiaryStore.getState().exportNow("a1");
    expect(useDiaryStore.getState().notice).toMatch(/내보내지 못했습니다/);
    expect(useDiaryStore.getState().exporting).toBe(false);
  });

  it("내보내는 중이면 두 번째 호출은 무시한다", async () => {
    openWith([entry(1, "일기")]);
    useDiaryStore.setState({ exporting: true });
    await useDiaryStore.getState().exportNow("a1");
    expect(exportDiaryFile).not.toHaveBeenCalled();
  });

  it("다른 캐릭터를 열고 있으면(stale) 안내를 덮지 않는다", async () => {
    openWith([entry(1, "일기")]);
    let resolveExport!: (v: string | null) => void;
    exportDiaryFile.mockImplementation(() => new Promise((r) => (resolveExport = r)));

    const p = useDiaryStore.getState().exportNow("a1");
    // 저장 다이얼로그가 떠 있는 사이 다른 캐릭터로 전환.
    useDiaryStore.setState({ overlay: { agentId: "a2", agentName: "B" }, notice: null });
    resolveExport("/tmp/x.md");
    await p;

    expect(useDiaryStore.getState().notice).toBeNull();
    expect(useDiaryStore.getState().exporting).toBe(false);
  });
});
