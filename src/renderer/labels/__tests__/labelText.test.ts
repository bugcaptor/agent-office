// src/renderer/labels/__tests__/labelText.test.ts
import { describe, expect, it } from "vitest";
import { firstLine, projectNameFromCwd } from "../labelText";

describe("projectNameFromCwd", () => {
  it("basename을 돌려준다 (POSIX)", () => {
    expect(projectNameFromCwd("/Users/me/dev/agent-office")).toBe("agent-office");
  });
  it("트레일링 슬래시를 무시한다", () => {
    expect(projectNameFromCwd("/Users/me/dev/agent-office/")).toBe("agent-office");
  });
  it("윈도우 구분자도 처리한다", () => {
    expect(projectNameFromCwd("C:\\dev\\my-proj")).toBe("my-proj");
  });
  it("빈/undefined/루트는 undefined", () => {
    expect(projectNameFromCwd(undefined)).toBeUndefined();
    expect(projectNameFromCwd("")).toBeUndefined();
    expect(projectNameFromCwd("/")).toBeUndefined();
  });
});

describe("firstLine", () => {
  it("첫 줄만 취해 max 초과 시 …로 절단한다", () => {
    expect(firstLine("버그를 고쳐줘\n그리고 테스트도", 30)).toBe("버그를 고쳐줘");
    expect(firstLine("아주 긴 지시문입니다 정말로 깁니다", 8)).toBe("아주 긴 지시문…");
  });
  it("빈/공백뿐/undefined는 undefined", () => {
    expect(firstLine(undefined, 10)).toBeUndefined();
    expect(firstLine("   \n  ", 10)).toBeUndefined();
  });
});
