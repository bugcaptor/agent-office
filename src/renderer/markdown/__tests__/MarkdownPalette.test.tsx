// @vitest-environment jsdom
//
// src/renderer/markdown/__tests__/MarkdownPalette.test.tsx
//
// 팔레트 키보드 내비게이션(이슈 #10): ↑/↓ 선택 이동, Enter 열기, Esc 닫기.
// tauriApi는 목(openFile이 markdownReadFile을 부른다) — 배선만 검증한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { listFiles, readFile } = vi.hoisted(() => ({
  listFiles: vi.fn().mockResolvedValue({ files: [], truncated: false }),
  readFile: vi.fn().mockResolvedValue({ content: "x", version: "v1" }),
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    markdownListFiles: (...a: unknown[]) => listFiles(...a),
    markdownReadFile: (...a: unknown[]) => readFile(...a),
    markdownWriteFile: vi.fn(),
  },
}));

const { useMarkdownStore } = await import("../markdownStore");
const { MarkdownPalette } = await import("../MarkdownPalette");

const initialState = useMarkdownStore.getState();

const FILES = [
  { relPath: "a.md", name: "a.md" },
  { relPath: "b.md", name: "b.md" },
  { relPath: "c.md", name: "c.md" },
];

beforeEach(() => {
  useMarkdownStore.setState(initialState, true);
  readFile.mockClear();
  // 팔레트 열림 + 목록 캐시 주입(백그라운드 재스캔은 무시).
  useMarkdownStore.setState({
    palette: { root: "/root", agentId: "agent1", query: "", selectedIndex: 0 },
    listing: { "/root": { files: FILES, truncated: false } },
  });
});

afterEach(() => cleanup());

describe("MarkdownPalette", () => {
  it("팔레트가 없으면 아무것도 렌더하지 않는다", () => {
    useMarkdownStore.setState({ palette: null });
    const { container } = render(<MarkdownPalette />);
    expect(container.firstChild).toBeNull();
  });

  it("빈 쿼리에서 파일을 relPath 사전순으로 보여주고 첫 항목이 선택된다", () => {
    render(<MarkdownPalette />);
    const options = screen.getAllByRole("option");
    expect(options.map((o) => o.getAttribute("aria-selected"))).toEqual(["true", "false", "false"]);
  });

  it("↓/↑가 선택을 이동하고 끝에서 클램프된다", () => {
    render(<MarkdownPalette />);
    const input = screen.getByRole("textbox");

    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(useMarkdownStore.getState().palette?.selectedIndex).toBe(1);
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "ArrowDown" }); // 끝(2)에서 더 못 감
    expect(useMarkdownStore.getState().palette?.selectedIndex).toBe(2);
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(useMarkdownStore.getState().palette?.selectedIndex).toBe(1);
  });

  it("Enter가 선택 파일을 연다(openFile→markdownReadFile)", () => {
    render(<MarkdownPalette />);
    const input = screen.getByRole("textbox");
    fireEvent.keyDown(input, { key: "ArrowDown" }); // b.md 선택
    fireEvent.keyDown(input, { key: "Enter" });
    expect(readFile).toHaveBeenCalledWith("/root", "b.md");
  });

  it("Esc가 팔레트를 닫는다", () => {
    render(<MarkdownPalette />);
    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
    expect(useMarkdownStore.getState().palette).toBeNull();
  });

  it("쿼리로 퍼지 필터한다", () => {
    render(<MarkdownPalette />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "b" } });
    const options = screen.getAllByRole("option");
    expect(options).toHaveLength(1);
    expect(options[0].textContent).toContain("b.md");
  });

  it("키 이벤트는 stopPropagation되어 상위로 새지 않는다", () => {
    const spy = vi.fn();
    const { container } = render(
      <div onKeyDown={spy}>
        <MarkdownPalette />
      </div>,
    );
    // 팔레트 입력에서 발생한 키가 래퍼로 버블되지 않아야 한다.
    fireEvent.keyDown(container.querySelector("input.md-palette-input")!, { key: "ArrowDown" });
    expect(spy).not.toHaveBeenCalled();
  });
});
