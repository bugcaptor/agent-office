// @vitest-environment jsdom
//
// src/renderer/markdown/__tests__/MarkdownEditorOverlay.test.tsx
//
// 편집기 더티 가드(이슈 #10): 더티일 때 Esc가 확인 다이얼로그를 띄우고,
// "버리고 닫기"로 닫힌다. Cmd/Ctrl+S 저장 배선과 충돌 다이얼로그 렌더도 확인.
// tauriApi는 목 — 편집기 배선만 검증한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { readFile, writeFile } = vi.hoisted(() => ({
  readFile: vi.fn().mockResolvedValue({ content: "hello", version: "v1" }),
  writeFile: vi.fn().mockResolvedValue({ version: "v2" }),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    markdownListFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
    markdownReadFile: (...a: unknown[]) => readFile(...a),
    markdownWriteFile: (...a: unknown[]) => writeFile(...a),
  },
}));

const { useMarkdownStore } = await import("../markdownStore");
const { MarkdownEditorOverlay } = await import("../MarkdownEditorOverlay");

const initialState = useMarkdownStore.getState();

/** 로드 완료 상태의 편집기를 store에 직접 세팅한다(비동기 openFile 우회). */
function seedEditor(content = "hello") {
  useMarkdownStore.setState({
    editor: {
      root: "/root",
      relPath: "docs/x.md",
      agentId: "agent1",
      content,
      baseline: "hello",
      version: "v1",
      mode: "source",
      loading: false,
      saving: false,
      loadError: null,
      conflict: false,
    },
    discardConfirm: false,
  });
}

beforeEach(() => {
  useMarkdownStore.setState(initialState, true);
  writeFile.mockClear();
});
afterEach(() => cleanup());

describe("MarkdownEditorOverlay", () => {
  it("편집기가 없으면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<MarkdownEditorOverlay />);
    expect(container.firstChild).toBeNull();
  });

  it("relPath와 소스 textarea를 표시한다", () => {
    seedEditor();
    render(<MarkdownEditorOverlay />);
    expect(screen.getByText("docs/x.md")).toBeTruthy();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("hello");
  });

  it("더티가 아니면 Esc가 즉시 닫는다", () => {
    seedEditor("hello"); // baseline과 동일 → not dirty
    render(<MarkdownEditorOverlay />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
    expect(useMarkdownStore.getState().editor).toBeNull();
  });

  it("더티면 Esc가 확인 다이얼로그를 띄운다(닫지 않음)", () => {
    seedEditor();
    render(<MarkdownEditorOverlay />);
    // 내용 변경 → 더티
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "hello world" } });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });

    expect(screen.getByText("저장되지 않은 변경")).toBeTruthy();
    expect(useMarkdownStore.getState().editor).not.toBeNull();
  });

  it("확인 다이얼로그의 '버리고 닫기'가 편집기를 닫는다", () => {
    seedEditor();
    render(<MarkdownEditorOverlay />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "changed" } });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
    fireEvent.click(screen.getByRole("button", { name: "버리고 닫기" }));
    expect(useMarkdownStore.getState().editor).toBeNull();
  });

  it("더티 표시(●)가 변경 시 나타난다", () => {
    seedEditor();
    render(<MarkdownEditorOverlay />);
    expect(screen.queryByLabelText("저장되지 않은 변경")).toBeNull();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "x" } });
    expect(screen.getByLabelText("저장되지 않은 변경")).toBeTruthy();
  });

  it("Cmd/Ctrl+S가 저장을 호출한다", () => {
    seedEditor();
    render(<MarkdownEditorOverlay />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "edited" } });
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "s", ctrlKey: true });
    expect(writeFile).toHaveBeenCalledWith("/root", "docs/x.md", "edited", "v1");
  });

  it("미리보기 토글이 sanitize된 HTML을 렌더한다", () => {
    seedEditor("# 제목\n\n본문");
    render(<MarkdownEditorOverlay />);
    fireEvent.click(screen.getByRole("button", { name: "미리보기" }));
    // marked가 헤딩을 <h1>로 변환.
    expect(document.querySelector(".md-editor-preview h1")?.textContent).toContain("제목");
  });

  it("충돌 플래그가 서면 충돌 해결 다이얼로그가 뜬다", () => {
    seedEditor();
    useMarkdownStore.setState((s) => ({ editor: { ...s.editor!, conflict: true } }));
    render(<MarkdownEditorOverlay />);
    expect(screen.getByText("저장 충돌")).toBeTruthy();
    expect(screen.getByRole("button", { name: /내 내용으로 덮어쓰기/ })).toBeTruthy();
  });
});
