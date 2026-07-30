// @vitest-environment jsdom
//
// src/renderer/memo/__tests__/MemoArchiveDialog.test.tsx
//
// 메모 아카이브 다이얼로그(#79):
// - self-gate(타깃 없으면 null 렌더), backdrop 클릭/닫기 버튼으로 닫힘.
// - Esc는 캡처 단계에서 멈춘다(터미널/전역으로 새지 않게).
// - 목록은 최신순 그대로 보여주고, 고르면 본문을 읽어 렌더한다.
// - 빈 아카이브는 안내 문구.
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MemoSheet, MemoSheetMeta } from "@shared/types";

const listMemoArchive = vi.fn();
const readMemoSheet = vi.fn();
const saveMemo = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    listMemoArchive: (...a: unknown[]) => listMemoArchive(...a),
    readMemoSheet: (...a: unknown[]) => readMemoSheet(...a),
    saveMemo: (...a: unknown[]) => saveMemo(...a),
  },
}));

const { MemoArchiveDialog } = await import("../MemoArchiveDialog");
const { useMemoStore } = await import("../memoStore");

function meta(sheetId: string, archived: string): MemoSheetMeta {
  return {
    sheetId,
    created: "2026-07-29T09:00:00+09:00",
    updated: "2026-07-29T18:00:00+09:00",
    archived,
  };
}

function sheet(sheetId: string, content: string): MemoSheet {
  return {
    sheetId,
    created: "2026-07-29T09:00:00+09:00",
    updated: "2026-07-29T18:00:00+09:00",
    archived: "2026-07-30T09:00:00+09:00",
    content,
  };
}

const initialMemoState = useMemoStore.getState();

beforeEach(() => {
  useMemoStore.setState(initialMemoState, true);
  listMemoArchive.mockReset().mockResolvedValue([]);
  readMemoSheet.mockReset().mockResolvedValue(sheet("20260729T090000", "지난 장 본문"));
  saveMemo.mockReset().mockResolvedValue(undefined);
});

afterEach(() => cleanup());

describe("게이트/닫기", () => {
  it("타깃이 없으면 렌더하지 않는다", () => {
    const { container } = render(<MemoArchiveDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("닫기 버튼으로 닫힌다", async () => {
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { getByText, container } = render(<MemoArchiveDialog />);

    fireEvent.click(getByText("닫기"));

    expect(useMemoStore.getState().archive).toBeNull();
    await waitFor(() => expect(container.firstChild).toBeNull());
  });

  it("backdrop mousedown으로 닫힌다(패널 내부 클릭은 무해)", async () => {
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { container, getByRole } = render(<MemoArchiveDialog />);

    fireEvent.mouseDown(getByRole("dialog"), { button: 0 });
    expect(useMemoStore.getState().archive).not.toBeNull();

    fireEvent.mouseDown(container.firstChild as Element, { button: 0 });
    expect(useMemoStore.getState().archive).toBeNull();
  });

  it("Esc로 닫히고, 그 이벤트는 전역(버블 단계)으로 새지 않는다", async () => {
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const bubbled = vi.fn();
    window.addEventListener("keydown", bubbled); // 버블 단계 리스너
    try {
      render(<MemoArchiveDialog />);

      fireEvent.keyDown(window, { key: "Escape" });

      expect(useMemoStore.getState().archive).toBeNull();
      expect(bubbled).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", bubbled);
    }
  });
});

describe("목록/열람", () => {
  it("빈 아카이브는 안내 문구를 보여준다", async () => {
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { getByText } = render(<MemoArchiveDialog />);
    expect(getByText(/아직 넘긴 장이 없습니다/)).toBeTruthy();
  });

  it("목록을 스토어 순서(최신순) 그대로 보여준다", async () => {
    listMemoArchive.mockResolvedValue([
      meta("20260729T090000", "2026-07-30T09:00:00+09:00"),
      meta("20260728T090000", "2026-07-29T09:00:00+09:00"),
    ]);
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { getAllByRole } = render(<MemoArchiveDialog />);

    const items = getAllByRole("button").filter((b) =>
      b.className.includes("memo-archive-item")
    );
    expect(items).toHaveLength(2);
    expect(items[0].textContent).toContain("2026-07-30 09:00 넘김");
    expect(items[1].textContent).toContain("2026-07-29 09:00 넘김");
  });

  it("장을 고르면 본문을 읽어 렌더하고, 복사 버튼이 활성화된다", async () => {
    listMemoArchive.mockResolvedValue([meta("20260729T090000", "2026-07-30T09:00:00+09:00")]);
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { getAllByRole, getByText, container } = render(<MemoArchiveDialog />);
    expect((getByText("복사") as HTMLButtonElement).disabled).toBe(true);

    const item = getAllByRole("button").find((b) => b.className.includes("memo-archive-item"))!;
    fireEvent.click(item);

    await waitFor(() => expect(readMemoSheet).toHaveBeenCalledWith("a1", "20260729T090000"));
    // 본문은 <pre>로 렌더된다(미리보기 한 줄과 문자열이 겹칠 수 있으므로 직접 지목).
    await waitFor(() =>
      expect(container.querySelector(".memo-archive-view-body")?.textContent).toBe("지난 장 본문")
    );
    expect((getByText("복사") as HTMLButtonElement).disabled).toBe(false);
  });

  it("아무 장도 고르지 않았으면 안내를 보여준다", async () => {
    listMemoArchive.mockResolvedValue([meta("20260729T090000", "2026-07-30T09:00:00+09:00")]);
    await useMemoStore.getState().openArchive("a1", "컴파일러");
    const { getByText } = render(<MemoArchiveDialog />);
    expect(getByText("왼쪽에서 장을 고르세요.")).toBeTruthy();
  });
});
