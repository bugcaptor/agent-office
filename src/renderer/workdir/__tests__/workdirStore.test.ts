// src/renderer/workdir/__tests__/workdirStore.test.ts
//
// 작업 폴더 스토어(이슈 #11) 상태 전이·배선 검증. tauriApi·appStore·markdownStore를
// 목으로 대체해 오케스트레이션만 확인한다(store 테스트 관례).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const {
  listFiles,
  searchFiles,
  gitStatus,
  openInVscode,
  openMarkdownFile,
  diffFile,
  fileHistory,
  diffCommit,
  commitFiles,
  repoLog,
  difftool,
} = vi.hoisted(() => ({
  listFiles: vi.fn(),
  searchFiles: vi.fn(),
  gitStatus: vi.fn(),
  openInVscode: vi.fn(),
  openMarkdownFile: vi.fn(),
  diffFile: vi.fn(),
  fileHistory: vi.fn(),
  diffCommit: vi.fn(),
  commitFiles: vi.fn(),
  repoLog: vi.fn(),
  difftool: vi.fn(),
}));

// gitStatusEnabled/fileIndexBackend를 테스트마다 바꾸기 위한 가변 셋팅.
const settings: { gitStatusEnabled: boolean; fileIndexBackend: "walker" | "everything" } = {
  gitStatusEnabled: true,
  fileIndexBackend: "walker",
};

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    workdirListFiles: (...a: unknown[]) => listFiles(...a),
    workdirSearchFiles: (...a: unknown[]) => searchFiles(...a),
    workdirGitStatus: (...a: unknown[]) => gitStatus(...a),
    openInVscode: (...a: unknown[]) => openInVscode(...a),
    workdirDiffFile: (...a: unknown[]) => diffFile(...a),
    workdirFileHistory: (...a: unknown[]) => fileHistory(...a),
    workdirDiffCommit: (...a: unknown[]) => diffCommit(...a),
    workdirCommitFiles: (...a: unknown[]) => commitFiles(...a),
    workdirRepoLog: (...a: unknown[]) => repoLog(...a),
    workdirDifftool: (...a: unknown[]) => difftool(...a),
  },
}));
vi.mock("../../store/appStore", () => ({
  useAppStore: { getState: () => ({ appSettings: settings }) },
}));
vi.mock("../../markdown/markdownStore", () => ({
  useMarkdownStore: { getState: () => ({ openFile: openMarkdownFile }) },
}));

import { useWorkdirStore, isMarkdownPath, joinPath } from "../workdirStore";

const initialState = useWorkdirStore.getState();

const cleanRepo = {
  isRepo: true,
  branch: "main",
  ahead: 0,
  behind: 0,
  entries: [],
  timedOut: false,
  truncated: false,
};

beforeEach(() => {
  useWorkdirStore.setState(initialState, true);
  settings.gitStatusEnabled = true;
  settings.fileIndexBackend = "walker";
  listFiles.mockReset().mockResolvedValue({ files: [], truncated: false });
  searchFiles.mockReset().mockResolvedValue({ files: [], truncated: false, usedIndex: false });
  gitStatus.mockReset().mockResolvedValue(cleanRepo);
  openInVscode.mockReset().mockResolvedValue(undefined);
  openMarkdownFile.mockReset().mockResolvedValue(undefined);
  diffFile
    .mockReset()
    .mockResolvedValue({ diff: "@@ -1 +1 @@\n-a\n+b\n", binary: false, truncated: false, timedOut: false });
  fileHistory.mockReset().mockResolvedValue({
    commits: [{ hash: "a".repeat(40), shortHash: "aaaaaaa", author: "A", date: "d", subject: "s" }],
    hasMore: false,
    timedOut: false,
  });
  diffCommit
    .mockReset()
    .mockResolvedValue({ diff: "diff --git\n", binary: false, truncated: false, timedOut: false });
  commitFiles.mockReset().mockResolvedValue({
    files: [
      { path: "src/a.rs", status: "M" },
      { path: "src/b.rs", status: "A" },
    ],
    hasMore: false,
    timedOut: false,
  });
  repoLog.mockReset().mockResolvedValue({
    commits: [{ hash: "c".repeat(40), shortHash: "ccccccc", author: "C", date: "d", subject: "feat: x" }],
    hasMore: false,
    timedOut: false,
  });
  difftool.mockReset().mockResolvedValue(undefined);
});

describe("순수 헬퍼", () => {
  it("isMarkdownPath는 md/mdx/markdown만 true", () => {
    expect(isMarkdownPath("a.md")).toBe(true);
    expect(isMarkdownPath("dir/B.MDX")).toBe(true);
    expect(isMarkdownPath("readme.markdown")).toBe(true);
    expect(isMarkdownPath("main.rs")).toBe(false);
    expect(isMarkdownPath("noext")).toBe(false);
  });

  it("joinPath는 후행 구분자를 중복 없이 붙인다", () => {
    expect(joinPath("/root", "a/b.txt")).toBe("/root/a/b.txt");
    expect(joinPath("/root/", "a.txt")).toBe("/root/a.txt");
    expect(joinPath("C:\\proj\\", "a.txt")).toBe("C:\\proj/a.txt");
  });
});

describe("팔레트 열기", () => {
  it("openPalette가 목록과 git 상태를 함께 조회한다", async () => {
    listFiles.mockResolvedValueOnce({
      files: [{ relPath: "a.rs", name: "a.rs" }],
      truncated: false,
    });
    gitStatus.mockResolvedValueOnce({ ...cleanRepo, entries: [{ path: "a.rs", status: "M", xy: ".M" }] });

    useWorkdirStore.getState().openPalette("/root", "agent1");
    expect(useWorkdirStore.getState().palette).toMatchObject({
      root: "/root",
      agentId: "agent1",
      changedOnly: false,
    });
    expect(listFiles).toHaveBeenCalledWith("/root");
    expect(gitStatus).toHaveBeenCalledWith("/root");

    await vi.waitFor(() => {
      expect(useWorkdirStore.getState().listing["/root"].files).toHaveLength(1);
      expect(useWorkdirStore.getState().git["/root"].entries).toHaveLength(1);
    });
  });

  it("setChangedOnly는 선택 인덱스를 0으로 리셋한다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setSelectedIndex(5);
    s.setChangedOnly(true);
    expect(useWorkdirStore.getState().palette).toMatchObject({ changedOnly: true, selectedIndex: 0 });
  });
});

describe("git 토글 존중", () => {
  it("gitStatusEnabled=false면 git 조회를 건너뛰고 캐시를 비운다", async () => {
    // 먼저 켠 상태로 캐시를 채운다.
    useWorkdirStore.getState().openPalette("/root", "agent1");
    await vi.waitFor(() => expect(useWorkdirStore.getState().git["/root"]).toBeDefined());

    // 끄고 재조회하면 조회 없이 캐시가 사라진다.
    settings.gitStatusEnabled = false;
    gitStatus.mockClear();
    await useWorkdirStore.getState().refreshGit("/root");

    expect(gitStatus).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().git["/root"]).toBeUndefined();
  });
});

describe("항목 열기", () => {
  it(".md는 인앱 마크다운 편집기로 위임하고 팔레트를 닫는다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openEntry("/root", "docs/x.md", "x.md");

    expect(openMarkdownFile).toHaveBeenCalledWith("/root", "docs/x.md", "agent1");
    expect(openInVscode).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().palette).toBeNull();
  });

  it("그 외 파일은 절대경로로 open_in_vscode에 넘긴다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openEntry("/root", "src/main.rs", "main.rs");

    expect(openInVscode).toHaveBeenCalledWith("/root/src/main.rs");
    expect(openMarkdownFile).not.toHaveBeenCalled();
    // 외부 에디터로 여는 경우 팔레트는 유지된다.
    expect(useWorkdirStore.getState().palette).not.toBeNull();
  });
});

describe("상세(변경점) 페인", () => {
  it("openDetail은 추적 변경 파일에 worktreeVsHead diff를 로드한다", async () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openDetail("/root", "src/a.rs", "a.rs", "M");

    const d0 = useWorkdirStore.getState().detail;
    expect(d0).toMatchObject({ relPath: "src/a.rs", diffMode: "worktreeVsHead", isUntracked: false, tab: "diff" });
    expect(diffFile).toHaveBeenCalledWith("/root", "src/a.rs", "worktreeVsHead");

    await vi.waitFor(() => {
      expect(useWorkdirStore.getState().detail?.diff?.diff).toContain("+b");
    });
  });

  it("미추적(? ) 파일은 untracked 모드로 연다", () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "new.txt", "new.txt", "?");
    expect(useWorkdirStore.getState().detail).toMatchObject({ isUntracked: true, diffMode: "untracked" });
    expect(diffFile).toHaveBeenCalledWith("/root", "new.txt", "untracked");
  });

  it("setDiffMode는 관점을 바꿔 재조회하고 gen을 올려 stale 응답을 폐기한다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    const gen0 = useWorkdirStore.getState().detail!.gen;

    s.setDiffMode("indexVsHead");
    const d = useWorkdirStore.getState().detail!;
    expect(d.diffMode).toBe("indexVsHead");
    expect(d.gen).toBe(gen0 + 1);
    expect(diffFile).toHaveBeenLastCalledWith("/root", "src/a.rs", "indexVsHead");
  });

  it("히스토리 탭 최초 진입 시 지연 로드한다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    expect(fileHistory).not.toHaveBeenCalled();

    s.setDetailTab("history");
    expect(fileHistory).toHaveBeenCalledWith("/root", "src/a.rs", 50, 0);
    await vi.waitFor(() => {
      expect(useWorkdirStore.getState().detail?.history).toHaveLength(1);
    });
  });

  it("커밋 선택 시 그 커밋 diff를 로드한다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    await s.selectCommit("a".repeat(40));

    expect(diffCommit).toHaveBeenCalledWith("/root", "a".repeat(40), "src/a.rs");
    expect(useWorkdirStore.getState().detail).toMatchObject({ selectedCommit: "a".repeat(40) });
    expect(useWorkdirStore.getState().detail?.commitDiff?.diff).toContain("diff --git");
  });

  it("openDifftool은 현재 관점을 넘겨 외부 도구를 띄운다", () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    s.openDifftool();
    expect(difftool).toHaveBeenCalledWith("/root", "src/a.rs", "worktreeVsHead", undefined);
  });

  it("closeDetail/closePalette는 상세를 비운다", () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    expect(useWorkdirStore.getState().detail).not.toBeNull();
    s.closeDetail();
    expect(useWorkdirStore.getState().detail).toBeNull();

    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    s.closePalette();
    expect(useWorkdirStore.getState().detail).toBeNull();
  });
});

describe("메뉴 우선 진입(이슈 #54)", () => {
  it("변경 없는 파일은 기본 히스토리 탭으로 열고 로그를 로드한다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openDetail("/root", "src/clean.rs", "clean.rs", undefined);
    expect(useWorkdirStore.getState().detail).toMatchObject({
      relPath: "src/clean.rs",
      tab: "history",
    });
    // 히스토리 탭이 기본이라 즉시 히스토리를 로드한다.
    expect(fileHistory).toHaveBeenCalledWith("/root", "src/clean.rs", 50, 0);
  });

  it("openExternal은 .md도 강제로 외부 에디터로 연다(팔레트 유지)", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openDetail("/root", "docs/x.md", "x.md", undefined);
    s.openExternal();
    expect(openInVscode).toHaveBeenCalledWith("/root/docs/x.md");
    expect(openMarkdownFile).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().palette).not.toBeNull();
  });

  it("openInApp은 마크다운만 인앱으로 열고 팔레트를 닫는다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    // 마크다운.
    s.openDetail("/root", "docs/x.md", "x.md", undefined);
    s.openInApp();
    expect(openMarkdownFile).toHaveBeenCalledWith(
      "/root",
      "docs/x.md",
      "agent1",
      expect.any(Function),
    );
    expect(useWorkdirStore.getState().palette).toBeNull();

    // 비마크다운은 no-op.
    openMarkdownFile.mockClear();
    s.openPalette("/root", "agent1");
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    s.openInApp();
    expect(openMarkdownFile).not.toHaveBeenCalled();
  });

  it("인앱 뷰어를 닫으면 작업 폴더 탐색(팔레트+메뉴)으로 복귀한다", () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.openDetail("/root", "docs/x.md", "x.md", undefined);
    s.openInApp();
    // 뷰어가 열리며 탐색은 잠시 비워진다.
    expect(useWorkdirStore.getState().palette).toBeNull();
    expect(useWorkdirStore.getState().detail).toBeNull();

    // openFile에 넘어간 onClose 콜백을 실행하면 팔레트+메뉴가 복귀한다.
    const onClose = openMarkdownFile.mock.calls[0][3] as () => void;
    onClose();
    expect(useWorkdirStore.getState().palette).toMatchObject({ root: "/root", agentId: "agent1" });
    expect(useWorkdirStore.getState().detail).toMatchObject({ relPath: "docs/x.md" });
  });
});

describe("인라인 커밋 확장(이슈 #54)", () => {
  it("커밋을 펼치면 변경파일을 로드하고, 다시 누르면 접는다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    await s.toggleCommitExpand("f".repeat(40));
    expect(commitFiles).toHaveBeenCalledWith("/root", "f".repeat(40), 100, 0);
    expect(useWorkdirStore.getState().detail).toMatchObject({ expandedCommit: "f".repeat(40) });
    expect(useWorkdirStore.getState().detail?.commitFiles).toHaveLength(2);

    await s.toggleCommitExpand("f".repeat(40));
    expect(useWorkdirStore.getState().detail?.expandedCommit).toBeUndefined();
  });

  it("selectCommitFile은 그 커밋의 해당 파일 diff를 하단에 로드한다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    await s.selectCommitFile("f".repeat(40), "src/b.rs");
    expect(diffCommit).toHaveBeenCalledWith("/root", "f".repeat(40), "src/b.rs");
    expect(useWorkdirStore.getState().detail).toMatchObject({
      selectedCommit: "f".repeat(40),
      selectedCommitFile: "src/b.rs",
    });
  });

  it("selectCommit은 지금 파일을 selectedCommitFile로 쓴다", async () => {
    const s = useWorkdirStore.getState();
    s.openDetail("/root", "src/a.rs", "a.rs", "M");
    await s.selectCommit("f".repeat(40));
    expect(diffCommit).toHaveBeenCalledWith("/root", "f".repeat(40), "src/a.rs");
    expect(useWorkdirStore.getState().detail?.selectedCommitFile).toBe("src/a.rs");
  });
});

describe("커밋 로그 브라우저(이슈 #54)", () => {
  it("setViewMode('log') 최초 진입 시 로그를 로드한다", async () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setViewMode("log");
    expect(useWorkdirStore.getState().palette?.viewMode).toBe("log");
    expect(repoLog).toHaveBeenCalledWith("/root", 50, 0, false, "");
    await vi.waitFor(() => {
      expect(useWorkdirStore.getState().repoLog["/root"].commits).toHaveLength(1);
    });
  });

  it("커밋 선택→파일 선택→diff 흐름", async () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    await s.loadRepoLog(true);
    await s.selectRepoCommit("c".repeat(40));
    expect(commitFiles).toHaveBeenCalledWith("/root", "c".repeat(40), 100, 0);
    expect(useWorkdirStore.getState().repoLog["/root"].files).toHaveLength(2);

    await s.selectRepoFile("c".repeat(40), "src/a.rs");
    expect(diffCommit).toHaveBeenCalledWith("/root", "c".repeat(40), "src/a.rs");
    expect(useWorkdirStore.getState().repoLog["/root"].selectedFile).toBe("src/a.rs");
    expect(useWorkdirStore.getState().repoLog["/root"].fileDiff?.diff).toContain("diff --git");
  });

  it("검색어 변경은 첫 페이지부터 재조회한다", async () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    await s.loadRepoLog(true);
    repoLog.mockClear();
    s.setRepoLogQuery("feat");
    expect(useWorkdirStore.getState().repoLog["/root"].query).toBe("feat");
    expect(repoLog).toHaveBeenCalledWith("/root", 50, 0, false, "feat");
  });

  it("전체 브랜치 토글은 --all로 재조회한다", async () => {
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    await s.loadRepoLog(true);
    repoLog.mockClear();
    s.setRepoLogAllBranches(true);
    expect(repoLog).toHaveBeenCalledWith("/root", 50, 0, true, "");
  });

  it("더 보기는 다음 페이지를 이어 붙인다", async () => {
    repoLog.mockResolvedValueOnce({
      commits: [{ hash: "1".repeat(40), shortHash: "1111111", author: "A", date: "d", subject: "s1" }],
      hasMore: true,
      timedOut: false,
    });
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    await s.loadRepoLog(true);
    expect(useWorkdirStore.getState().repoLog["/root"].hasMore).toBe(true);

    repoLog.mockResolvedValueOnce({
      commits: [{ hash: "2".repeat(40), shortHash: "2222222", author: "B", date: "d", subject: "s2" }],
      hasMore: false,
      timedOut: false,
    });
    await s.loadRepoLog(false);
    expect(repoLog).toHaveBeenLastCalledWith("/root", 50, 1, false, "");
    expect(useWorkdirStore.getState().repoLog["/root"].commits).toHaveLength(2);
  });
});

describe("TTL/캐시 재사용(이슈 #67)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("5분 이내 재오픈은 캐시를 재사용하고 재스캔하지 않는다", async () => {
    vi.setSystemTime(0);
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "a.rs", name: "a.rs" }], truncated: false });
    await useWorkdirStore.getState().refreshListing("/root"); // 최초 채움(캐시 없음 → 즉시 스캔)
    expect(listFiles).toHaveBeenCalledTimes(1);

    vi.setSystemTime(4 * 60_000); // 4분 경과(TTL 이내)
    useWorkdirStore.getState().openPalette("/root", "agent1");
    // 캐시 hit이면 동기적으로 스킵되므로 마이크로태스크를 기다릴 필요조차 없다.
    expect(listFiles).toHaveBeenCalledTimes(1);
  });

  it("5분 초과 시 캐시를 즉시 보여주고 백그라운드로 1회 재스캔한다", async () => {
    vi.setSystemTime(0);
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "old.rs", name: "old.rs" }], truncated: false });
    await useWorkdirStore.getState().refreshListing("/root");

    vi.setSystemTime(6 * 60_000); // 6분 경과(TTL 초과)
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "new.rs", name: "new.rs" }], truncated: false });
    useWorkdirStore.getState().openPalette("/root", "agent1");

    // 재스캔이 끝나기 전에는 기존 캐시가 그대로 보인다(즉시 표시).
    expect(useWorkdirStore.getState().listing["/root"].files).toEqual([{ relPath: "old.rs", name: "old.rs" }]);

    await vi.runAllTimersAsync(); // 백그라운드 재스캔·git 조회(마이크로태스크) 완료까지 흘려보낸다.
    expect(listFiles).toHaveBeenCalledTimes(2);
    expect(useWorkdirStore.getState().listing["/root"].files).toEqual([{ relPath: "new.rs", name: "new.rs" }]);
  });

  it("force:true는 TTL을 무시하고 항상 재스캔한다", async () => {
    vi.setSystemTime(0);
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "old.rs", name: "old.rs" }], truncated: false });
    await useWorkdirStore.getState().refreshListing("/root");
    expect(listFiles).toHaveBeenCalledTimes(1);

    vi.setSystemTime(60_000); // 1분 후(TTL 이내라도)
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "new.rs", name: "new.rs" }], truncated: false });
    await useWorkdirStore.getState().refreshListing("/root", { force: true });

    expect(listFiles).toHaveBeenCalledTimes(2);
    expect(useWorkdirStore.getState().listing["/root"].files).toEqual([{ relPath: "new.rs", name: "new.rs" }]);
  });

  it("동시에 호출된 refreshListing은 in-flight dedupe로 1회만 실행된다", async () => {
    let resolveList!: (v: { files: { relPath: string; name: string }[]; truncated: boolean }) => void;
    listFiles.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveList = resolve;
        }),
    );
    const s = useWorkdirStore.getState();
    const p1 = s.refreshListing("/root");
    const p2 = s.refreshListing("/root"); // 이미 진행 중이므로 스킵되어야 한다.

    resolveList({ files: [{ relPath: "a.rs", name: "a.rs" }], truncated: false });
    await Promise.all([p1, p2]);

    expect(listFiles).toHaveBeenCalledTimes(1);
    expect(useWorkdirStore.getState().listing["/root"].files).toEqual([{ relPath: "a.rs", name: "a.rs" }]);
  });
});

describe("서버사이드 검색(이슈 #67)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("Everything 백엔드 + 2글자 이상 쿼리는 250ms 디바운스 후 서버 검색을 호출한다", async () => {
    settings.fileIndexBackend = "everything";
    searchFiles.mockResolvedValueOnce({
      files: [{ relPath: "src/workdir.tsx", name: "workdir.tsx" }],
      truncated: false,
      usedIndex: true,
    });
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wd");
    // 디바운스 대기 중에는 아직 호출되지 않지만 로딩 표시는 즉시 켜진다.
    expect(searchFiles).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().searchLoading).toBe(true);

    await vi.advanceTimersByTimeAsync(250);
    expect(searchFiles).toHaveBeenCalledWith("/root", "wd");
    expect(useWorkdirStore.getState().search).toEqual({
      root: "/root",
      query: "wd",
      files: [{ relPath: "src/workdir.tsx", name: "workdir.tsx" }],
      truncated: false,
    });
    expect(useWorkdirStore.getState().searchLoading).toBe(false);
  });

  it("usedIndex=false면 search를 null로 두어 클라이언트 필터로 폴백시킨다", async () => {
    settings.fileIndexBackend = "everything";
    searchFiles.mockResolvedValueOnce({ files: [], truncated: false, usedIndex: false });
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wd");
    await vi.advanceTimersByTimeAsync(250);
    expect(useWorkdirStore.getState().search).toBeNull();
    expect(useWorkdirStore.getState().searchLoading).toBe(false);
  });

  it("Walker 백엔드는 서버 검색을 시도하지 않는다", async () => {
    settings.fileIndexBackend = "walker";
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wd");
    await vi.advanceTimersByTimeAsync(500);
    expect(searchFiles).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().search).toBeNull();
  });

  it("2글자 미만 쿼리는 시도하지 않는다", async () => {
    settings.fileIndexBackend = "everything";
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("w");
    await vi.advanceTimersByTimeAsync(500);
    expect(searchFiles).not.toHaveBeenCalled();
    expect(useWorkdirStore.getState().searchLoading).toBe(false);
  });

  it("변경만(changedOnly) 필터에서는 시도하지 않는다", async () => {
    settings.fileIndexBackend = "everything";
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setChangedOnly(true);
    s.setQuery("wd");
    await vi.advanceTimersByTimeAsync(500);
    expect(searchFiles).not.toHaveBeenCalled();
  });

  it("타이핑 중 더 최신 쿼리가 나가면 이전 디바운스 타이머는 취소된다", async () => {
    settings.fileIndexBackend = "everything";
    searchFiles.mockResolvedValue({ files: [], truncated: false, usedIndex: true });
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wo");
    await vi.advanceTimersByTimeAsync(100); // 아직 250ms가 안 지남.
    s.setQuery("wor"); // 이전 타이머를 취소하고 새로 250ms 대기.
    await vi.advanceTimersByTimeAsync(250);

    expect(searchFiles).toHaveBeenCalledTimes(1);
    expect(searchFiles).toHaveBeenCalledWith("/root", "wor");
  });

  it("응답이 늦게 도착해도 그 사이 더 최신 요청이 나갔으면(stale) 폐기한다", async () => {
    settings.fileIndexBackend = "everything";
    let resolveFirst!: (v: { files: unknown[]; truncated: boolean; usedIndex: boolean }) => void;
    searchFiles.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
    );
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wo");
    await vi.advanceTimersByTimeAsync(250); // 첫 요청 발사(아직 pending).

    searchFiles.mockResolvedValueOnce({
      files: [{ relPath: "b.tsx", name: "b.tsx" }],
      truncated: false,
      usedIndex: true,
    });
    s.setQuery("wor"); // 더 최신 요청.
    await vi.advanceTimersByTimeAsync(250);
    expect(useWorkdirStore.getState().search).toMatchObject({ query: "wor" });

    // 첫 요청 응답이 뒤늦게 도착 -- gen이 최신이 아니므로 무시돼야 한다.
    resolveFirst({ files: [{ relPath: "stale.tsx", name: "stale.tsx" }], truncated: false, usedIndex: true });
    await vi.advanceTimersByTimeAsync(0);
    expect(useWorkdirStore.getState().search).toMatchObject({ query: "wor" });
  });

  it("closePalette/openPalette는 검색 상태를 초기화한다", async () => {
    settings.fileIndexBackend = "everything";
    searchFiles.mockResolvedValueOnce({
      files: [{ relPath: "a.tsx", name: "a.tsx" }],
      truncated: false,
      usedIndex: true,
    });
    const s = useWorkdirStore.getState();
    s.openPalette("/root", "agent1");
    s.setQuery("wd");
    await vi.advanceTimersByTimeAsync(250);
    expect(useWorkdirStore.getState().search).not.toBeNull();

    s.closePalette();
    expect(useWorkdirStore.getState().search).toBeNull();
    expect(useWorkdirStore.getState().searchLoading).toBe(false);
  });
});
