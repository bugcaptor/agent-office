// src/renderer/memo/__tests__/memoStore.test.ts
//
// 포스트잇 메모(#79) 스토어: 로드/디바운스 자동저장/flush 경로(blur·전환·닫힘)·
// 한 장 넘기기·복사·아카이브 열람. tauriApi를 목으로 두고 "언제 어떤 커맨드가
// 어떤 인자로 불리는가"와 stale 가드를 검증한다.
//
// node 환경 — localStorage/navigator.clipboard는 최소 스텁을 globalThis에
// 주입한다(terminalViewMode.test.ts와 같은 관례).
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadMemo: vi.fn(),
    saveMemo: vi.fn(),
    archiveMemoSheet: vi.fn(),
    listMemoArchive: vi.fn(),
    readMemoSheet: vi.fn(),
    deleteMemos: vi.fn(),
  },
}));

import { useMemoStore, SAVE_DEBOUNCE_MS } from "../memoStore";
import { MEMO_VISIBLE_STORAGE_KEY } from "../memoVisibility";
import { tauriApi } from "../../ipc/tauriApi";
import type { MemoSheet, MemoSheetMeta } from "@shared/types";

const loadMemo = tauriApi.loadMemo as unknown as ReturnType<typeof vi.fn>;
const saveMemo = tauriApi.saveMemo as unknown as ReturnType<typeof vi.fn>;
const archiveMemoSheet = tauriApi.archiveMemoSheet as unknown as ReturnType<typeof vi.fn>;
const listMemoArchive = tauriApi.listMemoArchive as unknown as ReturnType<typeof vi.fn>;
const readMemoSheet = tauriApi.readMemoSheet as unknown as ReturnType<typeof vi.fn>;

function sheet(sheetId: string, content: string, archived?: string): MemoSheet {
  return {
    sheetId,
    created: "2026-07-30T09:00:00+09:00",
    updated: "2026-07-30T09:30:00+09:00",
    ...(archived ? { archived } : {}),
    content,
  };
}

function meta(sheetId: string, archived: string): MemoSheetMeta {
  return {
    sheetId,
    created: "2026-07-29T09:00:00+09:00",
    updated: "2026-07-29T18:00:00+09:00",
    archived,
  };
}

/** localStorage 스텁(node 환경). 실제 값 확인이 필요하므로 Map으로 뒤를 받친다. */
const lsBacking = new Map<string, string>();
const originalLocalStorage = (globalThis as { localStorage?: unknown }).localStorage;
const originalNavigator = (globalThis as { navigator?: unknown }).navigator;

function installLocalStorageStub(): void {
  lsBacking.clear();
  (globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (k: string) => (lsBacking.has(k) ? lsBacking.get(k)! : null),
    setItem: (k: string, v: string) => void lsBacking.set(k, v),
    removeItem: (k: string) => void lsBacking.delete(k),
    clear: () => lsBacking.clear(),
    key: () => null,
    length: 0,
  };
}

/** navigator.clipboard 스텁. `writeText`를 주면 성공, 생략하면 미지원 환경. */
function installClipboardStub(writeText?: ReturnType<typeof vi.fn>): void {
  Object.defineProperty(globalThis, "navigator", {
    value: writeText ? { clipboard: { writeText } } : {},
    configurable: true,
    writable: true,
  });
}

/** 스토어를 초기 상태로 되돌린다. flush()를 한 번 통과시켜 모듈 스코프의
 *  디바운스 타이머까지 취소한다(테스트 간 누수 방지). */
function resetStore(): void {
  useMemoStore.setState({ dirty: false });
  void useMemoStore.getState().flush();
  useMemoStore.setState({
    visible: false,
    agentId: null,
    sheet: null,
    draft: "",
    dirty: false,
    loading: false,
    archiving: false,
    notice: null,
    archive: null,
    archiveItems: [],
    archiveLoading: false,
    archiveSelected: null,
    archiveNotice: null,
  });
}

beforeEach(() => {
  loadMemo.mockReset().mockResolvedValue(sheet("20260730T090000", ""));
  saveMemo.mockReset().mockResolvedValue(undefined);
  archiveMemoSheet.mockReset().mockResolvedValue(sheet("20260730T100000", ""));
  listMemoArchive.mockReset().mockResolvedValue([]);
  readMemoSheet.mockReset().mockResolvedValue(sheet("20260729T090000", "지난 장", "2026-07-30T09:00:00+09:00"));
  installLocalStorageStub();
  installClipboardStub();
  resetStore();
});

afterEach(() => {
  vi.useRealTimers();
  (globalThis as { localStorage?: unknown }).localStorage = originalLocalStorage;
  Object.defineProperty(globalThis, "navigator", {
    value: originalNavigator,
    configurable: true,
    writable: true,
  });
});

describe("focusAgent", () => {
  it("활성 탭의 현재 장을 로드해 draft에 담는다", async () => {
    loadMemo.mockResolvedValue(sheet("20260730T090000", "이어서 할 일"));

    await useMemoStore.getState().focusAgent("a1");

    expect(loadMemo).toHaveBeenCalledWith("a1");
    const s = useMemoStore.getState();
    expect(s.agentId).toBe("a1");
    expect(s.sheet?.sheetId).toBe("20260730T090000");
    expect(s.draft).toBe("이어서 할 일");
    expect(s.dirty).toBe(false);
    expect(s.loading).toBe(false);
  });

  it("같은 캐릭터를 다시 지목하면 재로드하지 않는다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    loadMemo.mockClear();

    await useMemoStore.getState().focusAgent("a1");

    expect(loadMemo).not.toHaveBeenCalled();
  });

  it("에이전트 전환 시 이전 캐릭터의 미저장 편집을 먼저 저장한다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    useMemoStore.getState().edit("a1의 미저장 메모");

    await useMemoStore.getState().focusAgent("a2");

    expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "a1의 미저장 메모");
    expect(useMemoStore.getState().agentId).toBe("a2");
  });

  it("로드 중 다시 전환되면 늦게 온 응답을 반영하지 않는다", async () => {
    let resolveFirst: (v: MemoSheet) => void = () => {};
    loadMemo.mockImplementationOnce(
      () => new Promise<MemoSheet>((res) => (resolveFirst = res))
    );
    const first = useMemoStore.getState().focusAgent("a1");

    loadMemo.mockResolvedValue(sheet("20260730T110000", "a2의 메모"));
    await useMemoStore.getState().focusAgent("a2");
    resolveFirst(sheet("20260730T090000", "a1의 메모"));
    await first;

    const s = useMemoStore.getState();
    expect(s.agentId).toBe("a2");
    expect(s.draft).toBe("a2의 메모");
  });

  it("로드 실패는 안내로 흡수한다", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    loadMemo.mockRejectedValue(new Error("boom"));

    await useMemoStore.getState().focusAgent("a1");

    const s = useMemoStore.getState();
    expect(s.loading).toBe(false);
    expect(s.sheet).toBeNull();
    expect(s.notice).toBe("메모를 불러오지 못했습니다.");
  });
});

describe("edit / flush (자동저장)", () => {
  it("타이핑은 디바운스 후 한 번만 저장한다", async () => {
    vi.useFakeTimers();
    await useMemoStore.getState().focusAgent("a1");

    useMemoStore.getState().edit("한");
    useMemoStore.getState().edit("한 줄");
    useMemoStore.getState().edit("한 줄 메모");
    expect(saveMemo).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS);

    expect(saveMemo).toHaveBeenCalledTimes(1);
    expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "한 줄 메모");
    expect(useMemoStore.getState().dirty).toBe(false);
  });

  it("flush는 디바운스를 앞당겨 즉시 저장한다(blur/닫힘 경로)", async () => {
    vi.useFakeTimers();
    await useMemoStore.getState().focusAgent("a1");
    useMemoStore.getState().edit("바로 저장");

    await useMemoStore.getState().flush();

    expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "바로 저장");
    // 예약돼 있던 타이머가 취소돼 중복 저장이 없다.
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS * 2);
    expect(saveMemo).toHaveBeenCalledTimes(1);
  });

  it("미저장 편집이 없으면 저장하지 않는다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    await useMemoStore.getState().flush();
    expect(saveMemo).not.toHaveBeenCalled();
  });

  it("저장 실패는 dirty를 되살려 다음 flush에서 재시도한다", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    await useMemoStore.getState().focusAgent("a1");
    saveMemo.mockRejectedValueOnce(new Error("io"));
    useMemoStore.getState().edit("실패할 저장");

    await useMemoStore.getState().flush();

    expect(useMemoStore.getState().dirty).toBe(true);
    expect(useMemoStore.getState().notice).toBe("메모를 저장하지 못했습니다.");

    await useMemoStore.getState().flush();
    expect(saveMemo).toHaveBeenCalledTimes(2);
    expect(useMemoStore.getState().dirty).toBe(false);
  });
});

describe("visible (localStorage 영속)", () => {
  it("토글은 상태와 localStorage를 함께 바꾼다", () => {
    useMemoStore.getState().toggleVisible();
    expect(useMemoStore.getState().visible).toBe(true);
    expect(lsBacking.get(MEMO_VISIBLE_STORAGE_KEY)).toBe("true");

    useMemoStore.getState().toggleVisible();
    expect(useMemoStore.getState().visible).toBe(false);
    expect(lsBacking.get(MEMO_VISIBLE_STORAGE_KEY)).toBe("false");
  });

  it("닫을 때 미저장 편집을 flush 한다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    useMemoStore.setState({ visible: true });
    useMemoStore.getState().edit("닫기 전 마지막 타이핑");

    useMemoStore.getState().setVisible(false);
    await vi.waitFor(() =>
      expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "닫기 전 마지막 타이핑")
    );
  });

  it("같은 값으로 setVisible 하면 아무 일도 하지 않는다", () => {
    useMemoStore.getState().setVisible(false);
    expect(lsBacking.has(MEMO_VISIBLE_STORAGE_KEY)).toBe(false);
  });
});

describe("archiveNow (한 장 넘기기)", () => {
  it("빈 장은 넘기지 않는다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    await useMemoStore.getState().archiveNow();
    expect(archiveMemoSheet).not.toHaveBeenCalled();
  });

  it("본문을 먼저 저장한 뒤 아카이브하고 새 빈 장으로 갈아탄다", async () => {
    loadMemo.mockResolvedValue(sheet("20260730T090000", "넘길 내용"));
    archiveMemoSheet.mockResolvedValue(sheet("20260730T120000", ""));
    await useMemoStore.getState().focusAgent("a1");
    useMemoStore.getState().edit("넘길 내용 + 마지막 한 줄");

    await useMemoStore.getState().archiveNow();

    expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "넘길 내용 + 마지막 한 줄");
    expect(archiveMemoSheet).toHaveBeenCalledWith("a1");
    const s = useMemoStore.getState();
    expect(s.sheet?.sheetId).toBe("20260730T120000");
    expect(s.draft).toBe("");
    expect(s.dirty).toBe(false);
    expect(s.archiving).toBe(false);
    expect(s.notice).toContain("한 장 넘겼습니다");
  });

  it("실패는 안내로 흡수하고 현재 장을 유지한다", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    loadMemo.mockResolvedValue(sheet("20260730T090000", "넘길 내용"));
    archiveMemoSheet.mockRejectedValue(new Error("io"));
    await useMemoStore.getState().focusAgent("a1");

    await useMemoStore.getState().archiveNow();

    const s = useMemoStore.getState();
    expect(s.sheet?.sheetId).toBe("20260730T090000");
    expect(s.archiving).toBe(false);
    expect(s.notice).toBe("한 장 넘기지 못했습니다.");
  });
});

describe("copyAll", () => {
  it("본문을 클립보드로 복사한다", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    installClipboardStub(writeText);
    loadMemo.mockResolvedValue(sheet("20260730T090000", "복사할 메모"));
    await useMemoStore.getState().focusAgent("a1");

    await useMemoStore.getState().copyAll();

    expect(writeText).toHaveBeenCalledWith("복사할 메모");
    expect(useMemoStore.getState().notice).toBe("메모 전체를 복사했습니다.");
  });

  it("빈 본문은 복사하지 않고 안내만 남긴다", async () => {
    const writeText = vi.fn();
    installClipboardStub(writeText);
    await useMemoStore.getState().focusAgent("a1");

    await useMemoStore.getState().copyAll();

    expect(writeText).not.toHaveBeenCalled();
    expect(useMemoStore.getState().notice).toBe("복사할 내용이 없습니다.");
  });

  it("클립보드 미지원 환경은 실패 안내로 흡수한다", async () => {
    installClipboardStub(); // clipboard 없음
    loadMemo.mockResolvedValue(sheet("20260730T090000", "복사할 메모"));
    await useMemoStore.getState().focusAgent("a1");

    await useMemoStore.getState().copyAll();

    expect(useMemoStore.getState().notice).toBe("복사에 실패했습니다.");
  });
});

describe("아카이브 열람", () => {
  it("열면 목록을 로드한다", async () => {
    listMemoArchive.mockResolvedValue([
      meta("20260729T090000", "2026-07-30T09:00:00+09:00"),
      meta("20260728T090000", "2026-07-29T09:00:00+09:00"),
    ]);

    await useMemoStore.getState().openArchive("a1", "컴파일러");

    expect(listMemoArchive).toHaveBeenCalledWith("a1");
    const s = useMemoStore.getState();
    expect(s.archive).toEqual({ agentId: "a1", agentName: "컴파일러" });
    expect(s.archiveItems).toHaveLength(2);
    expect(s.archiveLoading).toBe(false);
  });

  it("열기 전에 현재 장을 flush 한다", async () => {
    await useMemoStore.getState().focusAgent("a1");
    useMemoStore.getState().edit("아카이브 열기 전 타이핑");

    await useMemoStore.getState().openArchive("a1", "컴파일러");

    expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "아카이브 열기 전 타이핑");
  });

  it("장을 고르면 본문을 읽어 온다", async () => {
    await useMemoStore.getState().openArchive("a1", "컴파일러");

    await useMemoStore.getState().selectSheet("20260729T090000");

    expect(readMemoSheet).toHaveBeenCalledWith("a1", "20260729T090000");
    expect(useMemoStore.getState().archiveSelected?.content).toBe("지난 장");
  });

  it("고른 장을 복사한다", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    installClipboardStub(writeText);
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    await useMemoStore.getState().selectSheet("20260729T090000");

    await useMemoStore.getState().copySelected();

    expect(writeText).toHaveBeenCalledWith("지난 장");
    expect(useMemoStore.getState().archiveNotice).toBe("이 장을 복사했습니다.");
  });

  it("목록 로드 실패는 안내로 흡수한다", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    listMemoArchive.mockRejectedValue(new Error("io"));

    await useMemoStore.getState().openArchive("a1", "컴파일러");

    expect(useMemoStore.getState().archiveLoading).toBe(false);
    expect(useMemoStore.getState().archiveNotice).toBe("아카이브를 불러오지 못했습니다.");
  });

  it("닫으면 목록/선택을 비운다", async () => {
    listMemoArchive.mockResolvedValue([meta("20260729T090000", "2026-07-30T09:00:00+09:00")]);
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    await useMemoStore.getState().selectSheet("20260729T090000");

    useMemoStore.getState().closeArchive();

    const s = useMemoStore.getState();
    expect(s.archive).toBeNull();
    expect(s.archiveItems).toEqual([]);
    expect(s.archiveSelected).toBeNull();
  });
});
