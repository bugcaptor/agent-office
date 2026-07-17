// @vitest-environment jsdom
//
// src/renderer/about/__tests__/AboutDialog.test.tsx
//
// SettingsDialog.test.tsx와 동일 패턴: modal.kind 게이트, 표시 내용, 닫기
// 동작만 확인하는 단순 표시용 모달이라 상태 변경 케이스는 없다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { AboutDialog } from "../AboutDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

describe("AboutDialog", () => {
  it("modal이 about이 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<AboutDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("about 모달일 때 이름과 버전을 표시한다", () => {
    useAppStore.getState().openModal({ kind: "about" });

    render(<AboutDialog />);
    expect(screen.getByText("Agent Office")).toBeTruthy();
    expect(screen.getByText(/버전 0\.4\.4/)).toBeTruthy();
  });

  it("닫기 버튼 클릭 시 modal이 none이 된다", () => {
    useAppStore.getState().openModal({ kind: "about" });

    render(<AboutDialog />);
    fireEvent.click(screen.getByRole("button", { name: "닫기" }));

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
