// @vitest-environment jsdom
// 아키타입 콤보박스 — 자유 텍스트 계약(목록에 없는 종족을 그대로 적기).
import { describe, it, expect, vi, afterEach } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { ArchetypePicker, filterArchetypeOptions } from "../ArchetypePicker";
import {
  archetypeInputText,
  normalizeArchetypeInput,
  customArchetypeSubject,
} from "../../office/gen/archetypes";

afterEach(() => cleanup());

describe("아키타입 값 변환", () => {
  it("알려진 id는 한국어 라벨로 보이고, 라벨/id 어느 쪽을 쳐도 id로 접힌다", () => {
    expect(archetypeInputText("orc")).toBe("오크");
    expect(archetypeInputText("auto")).toBe("자동(시드)");
    expect(normalizeArchetypeInput("오크")).toBe("orc");
    expect(normalizeArchetypeInput("  ORC ")).toBe("orc");
    // 비우면 빈 값 그대로 — "auto"로 접으면 입력칸이 "자동(시드)"로 되채워져
    // 새 종족을 적을 수가 없다.
    expect(normalizeArchetypeInput("")).toBe("");
    expect(normalizeArchetypeInput("   ")).toBe("");
    expect(archetypeInputText("")).toBe("");
  });

  it("목록에 없는 값은 친 그대로 남고 커스텀으로 판정된다", () => {
    expect(normalizeArchetypeInput("드래곤")).toBe("드래곤");
    expect(archetypeInputText("드래곤")).toBe("드래곤");
    expect(customArchetypeSubject("드래곤")).toBe("드래곤");
    expect(customArchetypeSubject("orc")).toBeNull();
    expect(customArchetypeSubject("auto")).toBeNull();
    expect(customArchetypeSubject(undefined)).toBeNull();
  });

  it("걸리는 후보가 없으면 목록을 비우지 않고 전부 보여 준다", () => {
    expect(filterArchetypeOptions("오크").map((o) => o.value)).toEqual(["orc"]);
    expect(filterArchetypeOptions("드래곤")).toHaveLength(9);
    expect(filterArchetypeOptions("")).toHaveLength(9);
  });
});

describe("ArchetypePicker", () => {
  it("타이핑한 자유 텍스트를 그대로 올려보내고 안내를 띄운다", () => {
    const onChange = vi.fn();
    render(<ArchetypePicker value="auto" onChange={onChange} />);
    const input = screen.getByLabelText("아키타입") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "고양이 마법사" } });
    expect(onChange).toHaveBeenCalledWith("고양이 마법사");

    cleanup();
    render(<ArchetypePicker value="고양이 마법사" onChange={onChange} />);
    expect(screen.getByText(/목록에 없는 종족/)).toBeTruthy();
  });

  it("새로 적으려고 비워도 '자동(시드)'로 되채워지지 않는다", () => {
    const onChange = vi.fn();
    const { rerender } = render(<ArchetypePicker value="드래곤" onChange={onChange} />);
    const input = screen.getByLabelText("아키타입") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "" } });
    expect(onChange).toHaveBeenCalledWith("");
    rerender(<ArchetypePicker value="" onChange={onChange} />);
    expect((screen.getByRole("combobox") as HTMLInputElement).value).toBe("");
    expect(screen.queryByText(/목록에 없는 종족/)).toBeNull();
  });

  it("커스텀을 치고 있어도 목록에서 다시 고를 수 있다", () => {
    const onChange = vi.fn();
    render(<ArchetypePicker value="드래곤" onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: "아키타입 목록" }));
    const list = screen.getByRole("listbox", { name: "아키타입" });
    fireEvent.click(within(list).getByRole("option", { name: "엘프" }));
    expect(onChange).toHaveBeenCalledWith("elf");
  });
});
