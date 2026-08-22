// @vitest-environment jsdom
//
// src/renderer/memo/__tests__/PostItWidget.test.tsx
//
// 포스트잇 위젯(#79) 렌더/상호작용:
// - 닫혀 있거나 활성 터미널이 없으면 아무것도 렌더하지 않는다.
// - 열리면 활성 탭의 장을 로드해 textarea에 담고, 탭을 바꾸면 그 캐릭터의
//   장으로 갈아탄다.
// - 타이핑은 store.edit으로, blur는 flush로 이어진다.
// - 위젯 안에서 누른 키가 터미널 오버레이의 전역 단축키(window keydown)로
//   새지 않는다 — 메모를 타이핑하다 탭이 바뀌는 사고 방지.
// - "한 장 넘기기"는 빈 장에서 비활성이다.
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
import type { MemoSheet } from "@shared/types";

const loadMemo = vi.fn();
const saveMemo = vi.fn();
const archiveMemoSheet = vi.fn();
const listMemoArchive = vi.fn();
const readMemoSheet = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadMemo: (...a: unknown[]) => loadMemo(...a),
    saveMemo: (...a: unknown[]) => saveMemo(...a),
    archiveMemoSheet: (...a: unknown[]) => archiveMemoSheet(...a),
    listMemoArchive: (...a: unknown[]) => listMemoArchive(...a),
    readMemoSheet: (...a: unknown[]) => readMemoSheet(...a),
  },
}));

const { PostItWidget } = await import("../PostItWidget");
const { useMemoStore } = await import("../memoStore");

function mkProfile(id: string): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

function sheet(sheetId: string, content: string): MemoSheet {
  return {
    sheetId,
    created: "2026-07-30T09:00:00+09:00",
    updated: "2026-07-30T09:30:00+09:00",
    content,
  };
}

const initialAppState = useAppStore.getState();
const initialMemoState = useMemoStore.getState();

beforeEach(() => {
  useAppStore.setState(initialAppState, true);
  useMemoStore.setState(initialMemoState, true);
  loadMemo.mockReset().mockResolvedValue(sheet("20260730T090000", ""));
  saveMemo.mockReset().mockResolvedValue(undefined);
  archiveMemoSheet.mockReset().mockResolvedValue(sheet("20260730T100000", ""));
  listMemoArchive.mockReset().mockResolvedValue([]);
  readMemoSheet.mockReset();
});

afterEach(() => cleanup());

/** 에이전트 하나를 심고 그 탭을 활성으로 만든다. */
function seedActive(id = "a1") {
  const s = useAppStore.getState();
  s.addAgent(mkProfile(id));
  s.openTerminal(id);
}

describe("게이트", () => {
  it("닫혀 있으면 아무것도 렌더하지 않는다", () => {
    seedActive();
    useMemoStore.setState({ visible: false });
    const { container } = render(<PostItWidget />);
    expect(container.firstChild).toBeNull();
    expect(loadMemo).not.toHaveBeenCalled();
  });

  it("열려 있어도 활성 터미널이 없으면 렌더하지 않는다", () => {
    useAppStore.getState().addAgent(mkProfile("a1")); // 탭을 열지 않음
    useMemoStore.setState({ visible: true });
    const { container } = render(<PostItWidget />);
    expect(container.firstChild).toBeNull();
    expect(loadMemo).not.toHaveBeenCalled();
  });
});

describe("로드/전환", () => {
  it("열리면 활성 탭의 장을 로드해 본문에 담는다", async () => {
    seedActive();
    loadMemo.mockResolvedValue(sheet("20260730T090000", "이어서 할 일"));
    useMemoStore.setState({ visible: true });

    const { getByLabelText } = render(<PostItWidget />);

    await waitFor(() => expect(loadMemo).toHaveBeenCalledWith("a1"));
    const box = getByLabelText("포스트잇 메모 본문") as HTMLTextAreaElement;
    await waitFor(() => expect(box.value).toBe("이어서 할 일"));
    expect(box.disabled).toBe(false);
  });

  it("탭을 바꾸면 그 캐릭터의 장으로 갈아탄다", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.addAgent(mkProfile("a2"));
    s.openTerminal("a1");
    loadMemo.mockResolvedValue(sheet("20260730T090000", "a1의 메모"));
    useMemoStore.setState({ visible: true });
    const { getByLabelText } = render(<PostItWidget />);
    await waitFor(() => expect(loadMemo).toHaveBeenCalledWith("a1"));

    loadMemo.mockResolvedValue(sheet("20260730T110000", "a2의 메모"));
    useAppStore.getState().openTerminal("a2");

    await waitFor(() => expect(loadMemo).toHaveBeenCalledWith("a2"));
    const box = getByLabelText("포스트잇 메모 본문") as HTMLTextAreaElement;
    await waitFor(() => expect(box.value).toBe("a2의 메모"));
  });

  it("로드 전에는 편집을 막는다(빈 draft로 덮어쓰기 방지)", () => {
    seedActive();
    // 로드가 끝나지 않은 상태를 강제(sheet 없음).
    useMemoStore.setState({ visible: true, agentId: "a1", sheet: null, loading: true });
    const { getByLabelText } = render(<PostItWidget />);
    expect((getByLabelText("포스트잇 메모 본문") as HTMLTextAreaElement).disabled).toBe(true);
  });
});

describe("편집/저장", () => {
  it("타이핑은 store의 draft를 갱신한다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const { getByLabelText } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());

    fireEvent.change(getByLabelText("포스트잇 메모 본문"), { target: { value: "새 메모" } });

    expect(useMemoStore.getState().draft).toBe("새 메모");
    expect(useMemoStore.getState().dirty).toBe(true);
  });

  it("blur는 즉시 저장으로 이어진다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const { getByLabelText } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());
    const box = getByLabelText("포스트잇 메모 본문");
    fireEvent.change(box, { target: { value: "blur 전 타이핑" } });

    fireEvent.blur(box);

    await waitFor(() =>
      expect(saveMemo).toHaveBeenCalledWith("a1", "20260730T090000", "blur 전 타이핑")
    );
  });

  it("위젯 안의 키 입력은 window 단축키 핸들러로 새지 않는다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const onWindowKey = vi.fn();
    window.addEventListener("keydown", onWindowKey);
    try {
      const { getByLabelText } = render(<PostItWidget />);
      await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());

      fireEvent.keyDown(getByLabelText("포스트잇 메모 본문"), { key: "w", metaKey: true });

      expect(onWindowKey).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", onWindowKey);
    }
  });
});

describe("액션", () => {
  it("빈 장에서는 ‘한 장 넘기기’와 ‘전체 복사’가 비활성이다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const { getByText } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());

    expect((getByText("한 장 넘기기") as HTMLButtonElement).disabled).toBe(true);
    expect((getByText("전체 복사") as HTMLButtonElement).disabled).toBe(true);
  });

  it("내용이 있으면 ‘한 장 넘기기’가 아카이브 커맨드를 부른다", async () => {
    seedActive();
    loadMemo.mockResolvedValue(sheet("20260730T090000", "넘길 내용"));
    useMemoStore.setState({ visible: true });
    const { getByText } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().draft).toBe("넘길 내용"));

    fireEvent.click(getByText("한 장 넘기기"));

    await waitFor(() => expect(archiveMemoSheet).toHaveBeenCalledWith("a1"));
  });

  it("× 버튼은 위젯을 닫는다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const { getByLabelText, container } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());

    fireEvent.click(getByLabelText("포스트잇 닫기"));

    expect(useMemoStore.getState().visible).toBe(false);
    await waitFor(() => expect(container.firstChild).toBeNull());
  });

  it("‘아카이브’ 버튼은 아카이브 다이얼로그 타깃을 세운다", async () => {
    seedActive();
    useMemoStore.setState({ visible: true });
    const { getByText } = render(<PostItWidget />);
    await waitFor(() => expect(useMemoStore.getState().sheet).not.toBeNull());

    fireEvent.click(getByText("아카이브"));

    await waitFor(() =>
      expect(useMemoStore.getState().archive).toEqual({ agentId: "a1", agentName: "Agent a1" })
    );
    expect(listMemoArchive).toHaveBeenCalledWith("a1");
  });
});
