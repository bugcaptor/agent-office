// src/renderer/workdir/__tests__/workdirStore.test.ts
//
// 작업 폴더 스토어(이슈 #11) 상태 전이·배선 검증. tauriApi·appStore·markdownStore를
// 목으로 대체해 오케스트레이션만 확인한다(store 테스트 관례).
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  listFiles,
  gitStatus,
  openInVscode,
  openMarkdownFile,
  diffFile,
  fileHistory,
  diffCommit,
  difftool,
} = vi.hoisted(() => ({
  listFiles: vi.fn(),
  gitStatus: vi.fn(),
  openInVscode: vi.fn(),
  openMarkdownFile: vi.fn(),
  diffFile: vi.fn(),
  fileHistory: vi.fn(),
  diffCommit: vi.fn(),
  difftool: vi.fn(),
}));

// gitStatusEnabled를 테스트마다 바꾸기 위한 가변 셋팅.
const settings = { gitStatusEnabled: true };

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    workdirListFiles: (...a: unknown[]) => listFiles(...a),
    workdirGitStatus: (...a: unknown[]) => gitStatus(...a),
    openInVscode: (...a: unknown[]) => openInVscode(...a),
    workdirDiffFile: (...a: unknown[]) => diffFile(...a),
    workdirFileHistory: (...a: unknown[]) => fileHistory(...a),
    workdirDiffCommit: (...a: unknown[]) => diffCommit(...a),
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
};

beforeEach(() => {
  useWorkdirStore.setState(initialState, true);
  settings.gitStatusEnabled = true;
  listFiles.mockReset().mockResolvedValue({ files: [], truncated: false });
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
