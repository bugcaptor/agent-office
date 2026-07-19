// src/renderer/workdir/__tests__/workdirStore.test.ts
//
// 작업 폴더 스토어(이슈 #11) 상태 전이·배선 검증. tauriApi·appStore·markdownStore를
// 목으로 대체해 오케스트레이션만 확인한다(store 테스트 관례).
import { beforeEach, describe, expect, it, vi } from "vitest";

const { listFiles, gitStatus, openInVscode, openMarkdownFile } = vi.hoisted(() => ({
  listFiles: vi.fn(),
  gitStatus: vi.fn(),
  openInVscode: vi.fn(),
  openMarkdownFile: vi.fn(),
}));

// gitStatusEnabled를 테스트마다 바꾸기 위한 가변 셋팅.
const settings = { gitStatusEnabled: true };

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    workdirListFiles: (...a: unknown[]) => listFiles(...a),
    workdirGitStatus: (...a: unknown[]) => gitStatus(...a),
    openInVscode: (...a: unknown[]) => openInVscode(...a),
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
