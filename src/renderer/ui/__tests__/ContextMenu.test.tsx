// @vitest-environment jsdom
//
// 범용 컨텍스트 메뉴 TDD: 항목 렌더/선택/외부 클릭/Escape 닫힘.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenu } from "../ContextMenu";

afterEach(() => cleanup());

describe("ContextMenu", () => {
  it("항목을 렌더하고 클릭 시 onSelect 후 onClose를 호출한다", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(
      <ContextMenu
        x={10}
        y={10}
        items={[{ label: "프로필 편집", onSelect }]}
        onClose={onClose}
      />
    );
    fireEvent.click(screen.getByRole("menuitem", { name: "프로필 편집" }));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("메뉴 밖 mousedown으로 닫힌다 (메뉴 안은 무시)", () => {
    const onClose = vi.fn();
    render(
      <ContextMenu x={10} y={10} items={[{ label: "항목", onSelect: () => {} }]} onClose={onClose} />
    );
    fireEvent.mouseDown(screen.getByRole("menuitem", { name: "항목" }));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.mouseDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Escape로 닫힌다", () => {
    const onClose = vi.fn();
    render(
      <ContextMenu x={10} y={10} items={[{ label: "항목", onSelect: () => {} }]} onClose={onClose} />
    );
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("disabled 항목은 클릭해도 onSelect/onClose가 호출되지 않는다", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    render(
      <ContextMenu
        x={10}
        y={10}
        items={[{ label: "비활성 항목", onSelect, disabled: true }]}
        onClose={onClose}
      />
    );
    const item = screen.getByRole("menuitem", { name: "비활성 항목" });
    expect(item).toHaveProperty("disabled", true);
    fireEvent.click(item);
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });
});
