// @vitest-environment jsdom
//
// src/renderer/workdir/__tests__/WorkdirPalette.test.tsx
//
// 작업 폴더 오버레이(이슈 #11) 렌더·상호작용: git 뱃지 표시, [전체|변경만] 필터,
// 키보드 내비게이션. tauriApi·appStore·markdownStore는 목으로 대체한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { listFiles, gitStatus, openInVscode, updateSettings, diffFile, fileHistory, diffCommit, difftool } =
  vi.hoisted(() => ({
    listFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
    gitStatus: vi.fn().mockResolvedValue({
      isRepo: true,
      branch: "main",
      ahead: 0,
      behind: 0,
      entries: [],
      timedOut: false,
    }),
    openInVscode: vi.fn().mockResolvedValue(undefined),
    updateSettings: vi.fn(),
    diffFile: vi
      .fn()
      .mockResolvedValue({ diff: "@@ -1 +1 @@\n-a\n+b\n", binary: false, truncated: false, timedOut: false }),
    fileHistory: vi.fn().mockResolvedValue({ commits: [], hasMore: false, timedOut: false }),
    diffCommit: vi.fn().mockResolvedValue({ diff: "", binary: false, truncated: false, timedOut: false }),
    difftool: vi.fn().mockResolvedValue(undefined),
  }));

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
// useAppStore는 selector 훅으로도, getState로도 쓰인다. 두 경로 모두 지원.
vi.mock("../../store/appStore", () => {
  const state = () => ({ appSettings: settings, updateAppSettings: updateSettings });
  const useAppStore = (sel?: (s: ReturnType<typeof state>) => unknown) =>
    sel ? sel(state()) : state();
  useAppStore.getState = state;
  return { useAppStore };
});
vi.mock("../../markdown/markdownStore", () => ({
  useMarkdownStore: { getState: () => ({ openFile: vi.fn() }) },
}));

const { useWorkdirStore } = await import("../workdirStore");
const { WorkdirPalette } = await import("../WorkdirPalette");

const initialState = useWorkdirStore.getState();

const FILES = [
  { relPath: "src/a.rs", name: "a.rs" },
  { relPath: "src/b.rs", name: "b.rs" },
  { relPath: "README.md", name: "README.md" },
];
const GIT = {
  isRepo: true,
  branch: "main",
  ahead: 2,
  behind: 1,
  entries: [
    { path: "src/a.rs", status: "M", xy: ".M" },
    { path: "deleted.rs", status: "D", xy: "D." },
  ],
  timedOut: false,
};

beforeEach(() => {
  useWorkdirStore.setState(initialState, true);
  settings.gitStatusEnabled = true;
  openInVscode.mockClear();
  updateSettings.mockClear();
  useWorkdirStore.setState({
    palette: { root: "/root", agentId: "agent1", query: "", selectedIndex: 0, changedOnly: false },
    listing: { "/root": { files: FILES, truncated: false } },
    git: { "/root": GIT },
    gitLoading: {},
  });
});

afterEach(() => cleanup());

describe("WorkdirPalette", () => {
  it("브랜치 요약과 ahead/behind·변경 수를 보여준다", () => {
    render(<WorkdirPalette />);
    expect(screen.getByText(/main/)).toBeTruthy();
    expect(screen.getByText(/↑2/)).toBeTruthy();
    expect(screen.getByText(/↓1/)).toBeTruthy();
    expect(screen.getByText(/변경 2개/)).toBeTruthy();
  });

  it("전체 뷰에서 파일에 git 뱃지를 매칭해 얹는다", () => {
    render(<WorkdirPalette />);
    // 전체 목록 3개가 보인다.
    expect(screen.getByText("a.rs")).toBeTruthy();
    expect(screen.getByText("b.rs")).toBeTruthy();
    // src/a.rs 행에 M 뱃지.
    const badges = screen.getAllByText("M");
    expect(badges.length).toBeGreaterThanOrEqual(1);
  });

  it("변경만 필터는 git 엔트리(삭제 포함)를 목록으로 쓴다", () => {
    render(<WorkdirPalette />);
    fireEvent.click(screen.getByText("변경만"));
    // git 엔트리 기준: a.rs(M), deleted.rs(D). 루트 파일이라 name·path 두 span에
    // 모두 나타나므로 getAllByText로 확인한다.
    expect(screen.getAllByText("deleted.rs").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("D")).toBeTruthy();
    // 변경 없는 b.rs는 사라진다.
    expect(screen.queryByText("b.rs")).toBeNull();
  });

  it("git 토글 해제는 updateAppSettings를 부른다", () => {
    render(<WorkdirPalette />);
    const checkbox = screen.getByRole("checkbox");
    fireEvent.click(checkbox);
    expect(updateSettings).toHaveBeenCalledWith({ gitStatusEnabled: false });
  });

  it("Esc로 팔레트를 닫는다", () => {
    render(<WorkdirPalette />);
    const input = screen.getByPlaceholderText(/검색/);
    fireEvent.keyDown(input, { key: "Escape" });
    expect(useWorkdirStore.getState().palette).toBeNull();
  });

  it("일반 파일 클릭은 open_in_vscode로 절대경로를 넘긴다", () => {
    render(<WorkdirPalette />);
    fireEvent.mouseDown(screen.getByText("b.rs"));
    expect(openInVscode).toHaveBeenCalledWith("/root/src/b.rs");
  });

  it("변경된 파일 클릭은 열지 않고 변경점 상세를 연다", async () => {
    render(<WorkdirPalette />);
    // a.rs는 M 상태 → 곧장 열지 않고 diff 상세 페인.
    fireEvent.mouseDown(screen.getByText("a.rs"));
    expect(openInVscode).not.toHaveBeenCalled();
    expect(diffFile).toHaveBeenCalledWith("/root", "src/a.rs", "worktreeVsHead");
    // 상세 페인(탭/버튼)이 렌더된다.
    expect(screen.getByText("변경점")).toBeTruthy();
    expect(screen.getByText("실제 파일 열기")).toBeTruthy();
    // diff 내용이 도착하면 추가/삭제 줄이 보인다.
    await screen.findByText("+b");
  });

  it("상세가 열려 있으면 Esc는 상세만 먼저 닫는다", () => {
    render(<WorkdirPalette />);
    fireEvent.mouseDown(screen.getByText("a.rs"));
    expect(useWorkdirStore.getState().detail).not.toBeNull();

    const input = screen.getByPlaceholderText(/검색/);
    fireEvent.keyDown(input, { key: "Escape" });
    // 상세만 닫히고 팔레트는 유지.
    expect(useWorkdirStore.getState().detail).toBeNull();
    expect(useWorkdirStore.getState().palette).not.toBeNull();
  });
});
