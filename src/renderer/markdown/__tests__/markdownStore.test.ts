// src/renderer/markdown/__tests__/markdownStore.test.ts
//
// 마크다운 스토어 상태 전이(이슈 #10): 열기/더티/저장/충돌. tauriApi는
// 목으로 대체해 오케스트레이션 배선만 검증한다(store 테스트 관례, appStore.test).
import { beforeEach, describe, expect, it, vi } from "vitest";

const { listFiles, readFile, writeFile } = vi.hoisted(() => ({
  listFiles: vi.fn(),
  readFile: vi.fn(),
  writeFile: vi.fn(),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    markdownListFiles: (...a: unknown[]) => listFiles(...a),
    markdownReadFile: (...a: unknown[]) => readFile(...a),
    markdownWriteFile: (...a: unknown[]) => writeFile(...a),
  },
}));

import { useMarkdownStore, isEditorDirty } from "../markdownStore";

const initialState = useMarkdownStore.getState();

beforeEach(() => {
  useMarkdownStore.setState(initialState, true);
  listFiles.mockReset().mockResolvedValue({ files: [], truncated: false });
  readFile.mockReset().mockResolvedValue({ content: "hello", version: "v1" });
  writeFile.mockReset().mockResolvedValue({ version: "v2" });
});

describe("팔레트", () => {
  it("openPalette가 팔레트를 열고 목록 재스캔을 트리거한다", async () => {
    listFiles.mockResolvedValueOnce({ files: [{ relPath: "a.md", name: "a.md" }], truncated: true });
    useMarkdownStore.getState().openPalette("/root", "agent1");

    const p = useMarkdownStore.getState().palette;
    expect(p).toMatchObject({ root: "/root", agentId: "agent1", query: "", selectedIndex: 0 });
    expect(listFiles).toHaveBeenCalledWith("/root");

    await vi.waitFor(() => {
      expect(useMarkdownStore.getState().listing["/root"]).toEqual({
        files: [{ relPath: "a.md", name: "a.md" }],
        truncated: true,
      });
    });
  });

  it("setQuery는 선택 인덱스를 0으로 리셋한다", () => {
    const s = useMarkdownStore.getState();
    s.openPalette("/root", "agent1");
    s.setSelectedIndex(3);
    s.setQuery("re");
    expect(useMarkdownStore.getState().palette).toMatchObject({ query: "re", selectedIndex: 0 });
  });
});

describe("파일 열기/더티", () => {
  it("openFile은 읽은 내용/버전으로 편집기를 열고 팔레트를 닫는다", async () => {
    const s = useMarkdownStore.getState();
    s.openPalette("/root", "agent1");
    await s.openFile("/root", "docs/x.md", "agent1");

    const ed = useMarkdownStore.getState().editor!;
    expect(ed).toMatchObject({
      root: "/root",
      relPath: "docs/x.md",
      content: "hello",
      baseline: "hello",
      version: "v1",
      loading: false,
    });
    expect(useMarkdownStore.getState().palette).toBeNull();
    expect(isEditorDirty(ed)).toBe(false);
  });

  it("읽기 실패면 loadError를 세팅한다", async () => {
    readFile.mockRejectedValueOnce("ENOENT");
    await useMarkdownStore.getState().openFile("/root", "gone.md", "agent1");
    expect(useMarkdownStore.getState().editor).toMatchObject({
      loading: false,
      loadError: "ENOENT",
    });
  });

  it("setContent이 baseline과 달라지면 더티", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("hello world");
    expect(isEditorDirty(useMarkdownStore.getState().editor)).toBe(true);
  });
});

describe("저장", () => {
  it("저장 성공 시 버전·기준선을 갱신하고 더티를 해제한다", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("edited");
    const res = await useMarkdownStore.getState().save();

    expect(res).toEqual({ ok: true });
    expect(writeFile).toHaveBeenCalledWith("/root", "x.md", "edited", "v1");
    const ed = useMarkdownStore.getState().editor!;
    expect(ed).toMatchObject({ version: "v2", baseline: "edited", saving: false });
    expect(isEditorDirty(ed)).toBe(false);
  });

  it("CONFLICT reject면 conflict 플래그를 세우고 SaveResult로 알린다", async () => {
    writeFile.mockRejectedValueOnce("CONFLICT: 파일이 변경됨");
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("edited");
    const res = await useMarkdownStore.getState().save();

    expect(res).toEqual({ ok: false, conflict: true });
    expect(useMarkdownStore.getState().editor).toMatchObject({ conflict: true, saving: false });
  });

  it("일반 실패는 conflict 없이 error 메시지를 담는다", async () => {
    writeFile.mockRejectedValueOnce("EACCES");
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("edited");
    const res = await useMarkdownStore.getState().save();

    expect(res).toEqual({ ok: false, conflict: false, error: "EACCES" });
    expect(useMarkdownStore.getState().editor?.conflict).toBe(false);
  });
});

describe("충돌 해결", () => {
  it("reloadFromDisk는 최신 내용/버전으로 교체하고 충돌을 해제한다", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("mine");
    useMarkdownStore.setState((st) => ({ editor: { ...st.editor!, conflict: true } }));

    readFile.mockResolvedValueOnce({ content: "theirs", version: "v9" });
    await useMarkdownStore.getState().reloadFromDisk();

    const ed = useMarkdownStore.getState().editor!;
    expect(ed).toMatchObject({ content: "theirs", baseline: "theirs", version: "v9", conflict: false });
    expect(isEditorDirty(ed)).toBe(false);
  });

  it("overwrite는 최신 버전을 다시 읽어 그 버전으로 재저장한다", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1"); // version v1
    s.setContent("mine");
    useMarkdownStore.setState((st) => ({ editor: { ...st.editor!, conflict: true } }));

    readFile.mockResolvedValueOnce({ content: "theirs", version: "v9" }); // 최신 버전
    writeFile.mockResolvedValueOnce({ version: "v10" });
    const res = await useMarkdownStore.getState().overwrite();

    expect(res).toEqual({ ok: true });
    // 최신 버전 v9로 덮어썼는지 확인(내 내용 유지).
    expect(writeFile).toHaveBeenLastCalledWith("/root", "x.md", "mine", "v9");
    const ed = useMarkdownStore.getState().editor!;
    expect(ed).toMatchObject({ content: "mine", baseline: "mine", version: "v10", conflict: false });
  });
});

describe("더티 가드", () => {
  it("더티면 requestClose가 확인 다이얼로그를 띄운다", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    s.setContent("edited");
    useMarkdownStore.getState().requestClose();
    expect(useMarkdownStore.getState().discardConfirm).toBe(true);
    expect(useMarkdownStore.getState().editor).not.toBeNull();
  });

  it("더티가 아니면 requestClose가 즉시 닫는다", async () => {
    const s = useMarkdownStore.getState();
    await s.openFile("/root", "x.md", "agent1");
    useMarkdownStore.getState().requestClose();
    expect(useMarkdownStore.getState().editor).toBeNull();
    expect(useMarkdownStore.getState().discardConfirm).toBe(false);
  });
});
