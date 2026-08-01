// @vitest-environment jsdom
//
// src/renderer/shared/__tests__/useEscapeToClose.test.tsx
//
// 오버레이 공통 "Esc로 닫기" 훅 TDD. MemoArchiveDialog/SessionLogDialog/
// DiaryDialog가 각자 갖고 있던 동일 useEffect를 뽑은 것이라, 그 세 곳이
// 의존하던 성질을 그대로 지키는지 본다: (1) 캡처 단계에서 듣고 전파를 멈춘다
// — 터미널/전역 단축키로 새면 안 된다, (2) 닫혀 있으면 아무것도 안 한다,
// (3) 언마운트/닫힘 시 리스너를 뗀다.
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useEscapeToClose } from "../useEscapeToClose";

function Probe({ open, onClose }: { open: boolean; onClose: () => void }) {
  useEscapeToClose(open, onClose);
  return <div>probe</div>;
}

/** 캡처 단계 리스너가 멈춘 이벤트가 버블 단계 리스너까지 갔는지 본다. */
function withBubbleSpy(run: () => void): ReturnType<typeof vi.fn> {
  const bubbled = vi.fn();
  window.addEventListener("keydown", bubbled);
  try {
    run();
  } finally {
    window.removeEventListener("keydown", bubbled);
  }
  return bubbled;
}

/**
 * 실제 상황과 같은 경로로 쏜다 — 키 입력은 문서 안쪽(터미널 등)에서 나 window로
 * 올라온다. 캡처 단계에서 멈추면 버블 단계 리스너에는 닿지 않아야 한다.
 */
function pressEscape() {
  document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
}

afterEach(() => cleanup());

describe("useEscapeToClose", () => {
  it("열려 있을 때 Esc를 누르면 onClose를 부른다", () => {
    const onClose = vi.fn();
    render(<Probe open onClose={onClose} />);

    pressEscape();

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Esc 이벤트를 캡처 단계에서 멈춰 전역으로 새지 않게 한다", () => {
    const onClose = vi.fn();
    render(<Probe open onClose={onClose} />);

    const bubbled = withBubbleSpy(() => pressEscape());

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(bubbled).not.toHaveBeenCalled();
  });

  it("Esc가 아닌 키는 무시하고 전파도 막지 않는다", () => {
    const onClose = vi.fn();
    render(<Probe open onClose={onClose} />);

    const bubbled = withBubbleSpy(() => {
      document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    });

    expect(onClose).not.toHaveBeenCalled();
    expect(bubbled).toHaveBeenCalledTimes(1);
  });

  it("닫혀 있으면 리스너를 걸지 않는다", () => {
    const onClose = vi.fn();
    render(<Probe open={false} onClose={onClose} />);

    const bubbled = withBubbleSpy(() => pressEscape());

    expect(onClose).not.toHaveBeenCalled();
    expect(bubbled).toHaveBeenCalledTimes(1);
  });

  it("open이 false로 바뀌면 리스너를 뗀다", () => {
    const onClose = vi.fn();
    const { rerender } = render(<Probe open onClose={onClose} />);

    rerender(<Probe open={false} onClose={onClose} />);
    pressEscape();

    expect(onClose).not.toHaveBeenCalled();
  });

  it("언마운트되면 리스너를 뗀다", () => {
    const onClose = vi.fn();
    const { unmount } = render(<Probe open onClose={onClose} />);

    unmount();
    pressEscape();

    expect(onClose).not.toHaveBeenCalled();
  });
});
